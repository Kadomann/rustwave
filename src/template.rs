//! Bildvorlagen nach dem Vorbild von MMSSTV: Ebenen (Text mit Makros, Bilder, Rechtecke),
//! jede mit Drehung, Perspektive und Deckkraft. Koordinaten relativ (0..1), damit eine
//! Vorlage in jedem SSTV-Modus passt.

use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};
use image::{RgbImage, RgbaImage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const FONTS: [(&str, &[u8]); 5] = [
    ("Sans Fett", include_bytes!("../assets/DejaVuSans-Bold.ttf")),
    ("Sans", include_bytes!("../assets/DejaVuSans.ttf")),
    ("Sans Schmal Fett", include_bytes!("../assets/DejaVuSansCondensed-Bold.ttf")),
    ("Serif Fett", include_bytes!("../assets/DejaVuSerif-Bold.ttf")),
    ("Mono Fett", include_bytes!("../assets/DejaVuSansMono-Bold.ttf")),
];

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct Rgba(pub [u8; 4]);
impl Rgba {
    pub const WHITE: Rgba = Rgba([255, 255, 255, 255]);
    pub const BLACK: Rgba = Rgba([0, 0, 0, 255]);
    fn f(self) -> [f32; 4] { self.0.map(|v| v as f32 / 255.0) }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum Fill {
    Solid(Rgba),
    /// Verlauf von a nach b, Winkel in Grad (0 = links→rechts, 90 = oben→unten)
    Gradient { a: Rgba, b: Rgba, angle: f32 },
}
impl Fill {
    /// Farbe an relativer Position (u, v) in 0..1 des Ebenen-Bitmaps
    fn at(&self, u: f32, v: f32) -> [f32; 4] {
        match *self {
            Fill::Solid(c) => c.f(),
            Fill::Gradient { a, b, angle } => {
                let (s, c) = angle.to_radians().sin_cos();
                // Projektion auf die Verlaufsrichtung, auf 0..1 normiert
                let t = (((u - 0.5) * c + (v - 0.5) * s) / (c.abs() + s.abs()) + 0.5).clamp(0.0, 1.0);
                let (a, b) = (a.f(), b.f());
                [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t)
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum Align { Left, Center, Right }

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum Source {
    /// das aktuell gewählte Sendefoto
    Photo,
    /// das zuletzt empfangene (bzw. in der Galerie markierte) Bild – "Bild im Bild"
    Received,
    /// feste Bilddatei (Pfad in `path`)
    File,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum Fit { Cover, Contain, Stretch }

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub enum Kind {
    Text {
        text: String,
        font: usize,
        /// Schriftgröße als Anteil der Bildhöhe
        size: f32,
        fill: Fill,
        /// Randfarbe und -breite (Anteil der Schriftgröße)
        outline: Option<(Rgba, f32)>,
        /// Schattenfarbe und Versatz (Anteil der Schriftgröße)
        shadow: Option<(Rgba, f32)>,
        align: Align,
    },
    Picture { source: Source, path: String, fit: Fit, border: Option<(Rgba, f32)> },
    Rect { fill: Fill, border: Option<(Rgba, f32)>, radius: f32 },
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Layer {
    pub name: String,
    pub kind: Kind,
    /// Mittelpunkt, relativ zu Bildbreite/-höhe
    pub x: f32,
    pub y: f32,
    /// Größe relativ (bei Text automatisch aus dem Inhalt)
    pub w: f32,
    pub h: f32,
    /// Drehung in Grad
    pub rotation: f32,
    /// Perspektive: >0 verjüngt die Oberkante, <0 die Unterkante
    pub keystone: f32,
    pub opacity: f32,
    pub visible: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Phase { Cq, Answer, Report, Final }
impl Phase {
    pub const ALL: [Phase; 4] = [Phase::Cq, Phase::Answer, Phase::Report, Phase::Final];
    pub fn label(self) -> &'static str {
        match self { Phase::Cq => "CQ", Phase::Answer => "Antwort", Phase::Report => "Rapport", Phase::Final => "73" }
    }
    pub fn next(self) -> Phase {
        match self { Phase::Cq => Phase::Answer, Phase::Answer => Phase::Report, Phase::Report => Phase::Final, Phase::Final => Phase::Final }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Template {
    pub name: String,
    pub phase: Option<Phase>,
    pub layers: Vec<Layer>,
}

// ---------------- Makros ----------------

/// Makros (an MMSSTV angelehnt). `(Kürzel, Bedeutung)` – auch für die Einfügeknöpfe im Editor.
pub const MACROS: [(char, &str); 17] = [
    ('c', "Rufzeichen Gegenstation"), ('n', "Name Gegenstation"), ('q', "QTH Gegenstation"),
    ('r', "Rapport gesendet (RSV)"), ('s', "Rapport empfangen"), ('x', "Notiz"),
    ('m', "eigenes Rufzeichen"), ('o', "eigener Name"), ('a', "eigener QTH"), ('l', "eigener Locator"),
    ('g', "Gruß GM/GA/GE"), ('t', "Uhrzeit UTC"), ('d', "Datum UTC"), ('f', "Frequenz MHz"),
    ('b', "Band"), ('v', "SSTV-Modus"), ('w', "Anzahl früherer QSOs"),
];

/// Ersetzt %x-Makros; unbekannte bleiben stehen, `%%` ergibt `%`.
pub fn expand(text: &str, vals: &HashMap<char, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        if c != '%' { out.push(c); continue; }
        match it.peek().copied() {
            Some('%') => { it.next(); out.push('%'); }
            Some(k) if vals.contains_key(&k) => { it.next(); out.push_str(&vals[&k]); }
            _ => out.push('%'),
        }
    }
    out
}

/// Gruß nach Ortszeit-Stunde
pub fn greeting(hour: u32) -> &'static str {
    match hour { 4..=11 => "GM", 12..=17 => "GA", _ => "GE" }
}

// ---------------- Rendern ----------------

/// Alles, was eine Vorlage zum Rendern braucht.
pub struct Env<'a> {
    pub macros: HashMap<char, String>,
    pub photo: Option<&'a RgbImage>,
    pub received: Option<&'a RgbImage>,
    pub files: &'a HashMap<String, RgbImage>,
}

/// Lage einer Ebene im fertigen Bild (Eckpunkte in Pixeln: oben-links, oben-rechts, unten-rechts, unten-links)
pub type Quad = [[f32; 2]; 4];

/// Rendert die Vorlage in ein w×h-Bild. Liefert zusätzlich die Lage jeder Ebene (für Auswahl/Ziehen im Editor).
pub fn render(t: &Template, w: u32, h: u32, env: &Env) -> (RgbImage, Vec<Option<Quad>>) {
    let mut canvas = RgbImage::new(w, h);
    let mut quads = Vec::with_capacity(t.layers.len());
    for layer in &t.layers {
        if !layer.visible { quads.push(None); continue; }
        let bmp = layer_bitmap(layer, w, h, env);
        let q = layer_quad(layer, bmp.width() as f32, bmp.height() as f32, w as f32, h as f32);
        composite(&mut canvas, &bmp, &q, layer.opacity);
        quads.push(Some(q));
    }
    (canvas, quads)
}

/// Eckpunkte der Ebene: Bitmap um den Mittelpunkt, Perspektive, dann Drehung
pub fn layer_quad(l: &Layer, bw: f32, bh: f32, w: f32, h: f32) -> Quad {
    let (cx, cy) = (l.x * w, l.y * h);
    let k = l.keystone.clamp(-0.95, 0.95);
    let (top, bot) = if k >= 0.0 { (1.0 - k, 1.0) } else { (1.0, 1.0 + k) };
    let pts = [[-bw / 2.0 * top, -bh / 2.0], [bw / 2.0 * top, -bh / 2.0], [bw / 2.0 * bot, bh / 2.0], [-bw / 2.0 * bot, bh / 2.0]];
    let (s, c) = l.rotation.to_radians().sin_cos();
    pts.map(|[x, y]| [cx + x * c - y * s, cy + x * s + y * c])
}

fn layer_bitmap(l: &Layer, w: u32, h: u32, env: &Env) -> RgbaImage {
    let pw = ((l.w * w as f32).round() as u32).max(1);
    let ph = ((l.h * h as f32).round() as u32).max(1);
    match &l.kind {
        Kind::Text { text, font, size, fill, outline, shadow, align } => {
            text_bitmap(&expand(text, &env.macros), *font, size * h as f32, fill, *outline, *shadow, *align)
        }
        Kind::Rect { fill, border, radius } => rect_bitmap(pw, ph, fill, border.map(|(c, b)| (c, (b * h as f32).max(1.0))), *radius * h as f32),
        Kind::Picture { source, path, fit, border } => {
            let src = match source {
                Source::Photo => env.photo,
                Source::Received => env.received,
                Source::File => env.files.get(path),
            };
            let mut bmp = match src {
                Some(img) => fit_picture(img, pw, ph, *fit),
                None => placeholder(pw, ph),
            };
            if let Some((c, bw)) = border { draw_border(&mut bmp, *c, (bw * h as f32).max(1.0)); }
            bmp
        }
    }
}

fn placeholder(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| if (x / 8 + y / 8) % 2 == 0 { image::Rgba([90, 90, 90, 255]) } else { image::Rgba([60, 60, 60, 255]) })
}

fn fit_picture(img: &RgbImage, w: u32, h: u32, fit: Fit) -> RgbaImage {
    use image::imageops::{crop_imm, resize, FilterType::Triangle};
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    match fit {
        Fit::Stretch => image::DynamicImage::ImageRgb8(resize(img, w, h, Triangle)).to_rgba8(),
        Fit::Cover => {
            let k = (w as f32 / iw).max(h as f32 / ih);
            let (nw, nh) = (((iw * k).round() as u32).max(w), ((ih * k).round() as u32).max(h));
            let r = resize(img, nw, nh, Triangle);
            image::DynamicImage::ImageRgb8(crop_imm(&r, (nw - w) / 2, (nh - h) / 2, w, h).to_image()).to_rgba8()
        }
        Fit::Contain => {
            let k = (w as f32 / iw).min(h as f32 / ih);
            let (nw, nh) = (((iw * k).round() as u32).clamp(1, w), ((ih * k).round() as u32).clamp(1, h));
            let r = image::DynamicImage::ImageRgb8(resize(img, nw, nh, Triangle)).to_rgba8();
            let mut out = RgbaImage::new(w, h); // transparent
            image::imageops::overlay(&mut out, &r, ((w - nw) / 2) as i64, ((h - nh) / 2) as i64);
            out
        }
    }
}

fn draw_border(bmp: &mut RgbaImage, c: Rgba, width: f32) {
    let (w, h) = bmp.dimensions();
    let b = width.round() as u32;
    for (x, y, p) in bmp.enumerate_pixels_mut() {
        if x < b || y < b || x + b >= w || y + b >= h { *p = image::Rgba(c.0); }
    }
}

/// Rechteck mit abgerundeten Ecken; Randbreite und Radius in Pixeln
fn rect_bitmap(w: u32, h: u32, fill: &Fill, border: Option<(Rgba, f32)>, radius: f32) -> RgbaImage {
    let r = radius.min(w.min(h) as f32 / 2.0).max(0.0);
    let bw = border.map_or(0.0, |(_, b)| b);
    RgbaImage::from_fn(w, h, |x, y| {
        // vorzeichenbehafteter Abstand zur abgerundeten Rechteckkante (negativ = innen)
        let (px, py) = (x as f32 + 0.5 - w as f32 / 2.0, y as f32 + 0.5 - h as f32 / 2.0);
        let (qx, qy) = (px.abs() - (w as f32 / 2.0 - r), py.abs() - (h as f32 / 2.0 - r));
        let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
        let inside = (0.5 - d).clamp(0.0, 1.0);
        let mut c = fill.at(x as f32 / w as f32, y as f32 / h as f32);
        if let Some((bc, _)) = border {
            let k = (0.5 - (d + bw)).clamp(0.0, 1.0); // 1 tief innen, 0 im Randbereich
            let bc = bc.f();
            c = [0, 1, 2, 3].map(|i| bc[i] + (c[i] - bc[i]) * k);
        }
        image::Rgba([(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, (c[3] * inside * 255.0) as u8])
    })
}

/// Textbitmap: Schatten unter Rand unter Füllung, mehrzeilig mit Ausrichtung
fn text_bitmap(text: &str, font: usize, px: f32, fill: &Fill, outline: Option<(Rgba, f32)>, shadow: Option<(Rgba, f32)>, align: Align) -> RgbaImage {
    let font = FontRef::try_from_slice(FONTS[font.min(FONTS.len() - 1)].1).expect("Schrift");
    let px = px.max(4.0);
    let sf = font.as_scaled(PxScale::from(px));
    let lines: Vec<&str> = text.split('\n').collect();
    let line_h = sf.height() + sf.line_gap();
    let width_of = |s: &str| {
        let mut x = 0.0;
        let mut prev = None;
        for c in s.chars() {
            let id = sf.glyph_id(c);
            if let Some(p) = prev { x += sf.kern(p, id); }
            x += sf.h_advance(id);
            prev = Some(id);
        }
        x
    };
    let widths: Vec<f32> = lines.iter().map(|l| width_of(l)).collect();
    let tw = widths.iter().cloned().fold(1.0, f32::max);
    let th = line_h * lines.len() as f32;
    let ow = outline.map_or(0.0, |(_, w)| (w * px).max(1.0));
    let so = shadow.map_or(0.0, |(_, o)| (o * px).max(1.0));
    let pad = ow.ceil() + 1.0;
    let (bw, bh) = ((tw + 2.0 * pad + so).ceil() as usize, (th + 2.0 * pad + so).ceil() as usize);

    let mut mask = vec![0f32; bw * bh];
    for (li, line) in lines.iter().enumerate() {
        let mut x = pad + match align { Align::Left => 0.0, Align::Center => (tw - widths[li]) / 2.0, Align::Right => tw - widths[li] };
        let y0 = pad + li as f32 * line_h + sf.ascent();
        let mut prev = None;
        for c in line.chars() {
            let id = sf.glyph_id(c);
            if let Some(p) = prev { x += sf.kern(p, id); }
            if let Some(o) = font.outline_glyph(id.with_scale_and_position(px, point(x, y0))) {
                let b = o.px_bounds();
                o.draw(|gx, gy, cov| {
                    let (xx, yy) = (b.min.x as i64 + gx as i64, b.min.y as i64 + gy as i64);
                    if xx >= 0 && yy >= 0 && (xx as usize) < bw && (yy as usize) < bh {
                        let m = &mut mask[yy as usize * bw + xx as usize];
                        *m = m.max(cov);
                    }
                });
            }
            x += sf.h_advance(id);
            prev = Some(id);
        }
    }
    let edge = if ow > 0.0 { dilate(&mask, bw, bh, ow) } else { mask.clone() };
    let mut out = RgbaImage::new(bw as u32, bh as u32);
    let s = so.round() as usize;
    for y in 0..bh {
        for x in 0..bw {
            let mut px = [0f32; 4]; // nicht vormultipliziert
            if let Some((sc, _)) = shadow {
                if x >= s && y >= s { over(&mut px, sc.f(), edge[(y - s) * bw + x - s]); }
            }
            if let Some((oc, _)) = outline { over(&mut px, oc.f(), edge[y * bw + x]); }
            over(&mut px, fill.at(x as f32 / bw as f32, y as f32 / bh as f32), mask[y * bw + x]);
            out.put_pixel(x as u32, y as u32, image::Rgba(px.map(|v| (v * 255.0).round() as u8)));
        }
    }
    out
}

/// Farbe c mit Abdeckung cov über px legen (beides nicht vormultipliziert)
fn over(px: &mut [f32; 4], c: [f32; 4], cov: f32) {
    let a = c[3] * cov;
    if a <= 0.0 { return; }
    let out_a = a + px[3] * (1.0 - a);
    for i in 0..3 { px[i] = (c[i] * a + px[i] * px[3] * (1.0 - a)) / out_a; }
    px[3] = out_a;
}

fn dilate(mask: &[f32], w: usize, h: usize, r: f32) -> Vec<f32> {
    let ri = r.ceil() as i64;
    let mut out = vec![0f32; w * h];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let mut m = 0f32;
            for dy in -ri..=ri {
                for dx in -ri..=ri {
                    let d = ((dx * dx + dy * dy) as f32).sqrt();
                    if d > r + 0.5 { continue; }
                    let (xx, yy) = (x + dx, y + dy);
                    if xx >= 0 && yy >= 0 && xx < w as i64 && yy < h as i64 {
                        m = m.max(mask[yy as usize * w + xx as usize] * (r + 0.5 - d).clamp(0.0, 1.0));
                    }
                }
            }
            out[y as usize * w + x as usize] = m;
        }
    }
    out
}

// ---- Homographie: bildet die Bitmap-Ecken auf das Ziel-Viereck ab ----

/// Löst H mit H·src_i ~ dst_i (8×8-Gleichungssystem); Ergebnis zeilenweise, h33 = 1
fn homography(src: &Quad, dst: &Quad) -> Option<[f64; 9]> {
    let mut a = [[0f64; 9]; 8];
    for i in 0..4 {
        let ([x, y], [u, v]) = (src[i].map(|v| v as f64), dst[i].map(|v| v as f64));
        a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    for col in 0..8 { // Gauß mit Pivotsuche
        let piv = (col..8).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[piv][col].abs() < 1e-12 { return None; }
        a.swap(col, piv);
        for r in 0..8 {
            if r != col {
                let f = a[r][col] / a[col][col];
                for c in col..9 { a[r][c] -= f * a[col][c]; }
            }
        }
    }
    let h: Vec<f64> = (0..8).map(|i| a[i][8] / a[i][i]).collect();
    Some([h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7], 1.0])
}
fn apply(h: &[f64; 9], x: f64, y: f64) -> (f64, f64) {
    let w = h[6] * x + h[7] * y + h[8];
    ((h[0] * x + h[1] * y + h[2]) / w, (h[3] * x + h[4] * y + h[5]) / w)
}

/// Punkt im Viereck? (für Mausauswahl)
pub fn quad_contains(q: &Quad, p: [f32; 2]) -> bool {
    let mut sign = 0f32;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        let cr = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        if cr != 0.0 {
            if sign == 0.0 { sign = cr.signum(); } else if cr.signum() != sign { return false; }
        }
    }
    true
}

fn composite(canvas: &mut RgbImage, bmp: &RgbaImage, q: &Quad, opacity: f32) {
    let (bw, bh) = (bmp.width() as f32, bmp.height() as f32);
    let src: Quad = [[0.0, 0.0], [bw, 0.0], [bw, bh], [0.0, bh]];
    let Some(h) = homography(q, &src) else { return }; // Ziel -> Quelle
    let (cw, ch) = canvas.dimensions();
    let x0 = q.iter().map(|p| p[0]).fold(f32::MAX, f32::min).floor().max(0.0) as u32;
    let x1 = (q.iter().map(|p| p[0]).fold(f32::MIN, f32::max).ceil() as i64).clamp(0, cw as i64) as u32;
    let y0 = q.iter().map(|p| p[1]).fold(f32::MAX, f32::min).floor().max(0.0) as u32;
    let y1 = (q.iter().map(|p| p[1]).fold(f32::MIN, f32::max).ceil() as i64).clamp(0, ch as i64) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let (u, v) = apply(&h, x as f64 + 0.5, y as f64 + 0.5);
            let (u, v) = (u as f32 - 0.5, v as f32 - 0.5);
            if u < -0.5 || v < -0.5 || u > bw - 0.5 || v > bh - 0.5 { continue; }
            let s = bilinear(bmp, u, v);
            let a = s[3] * opacity.clamp(0.0, 1.0);
            if a <= 0.0 { continue; }
            let p = canvas.get_pixel_mut(x, y);
            for i in 0..3 { p.0[i] = (s[i] / s[3].max(1e-6) * a + p.0[i] as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8; }
        }
    }
}

/// bilineare Abtastung, vormultipliziert (0..255 für Farbe, 0..1 für Alpha)
fn bilinear(b: &RgbaImage, u: f32, v: f32) -> [f32; 4] {
    let (w, h) = (b.width() as i64, b.height() as i64);
    let (x0, y0) = (u.floor() as i64, v.floor() as i64);
    let (fx, fy) = (u - x0 as f32, v - y0 as f32);
    let mut acc = [0f32; 4];
    for (dx, dy, wt) in [(0, 0, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)] {
        let (xx, yy) = ((x0 + dx).clamp(0, w - 1), (y0 + dy).clamp(0, h - 1));
        let p = b.get_pixel(xx as u32, yy as u32).0;
        let a = p[3] as f32 / 255.0;
        for i in 0..3 { acc[i] += p[i] as f32 * a * wt; }
        acc[3] += a * wt;
    }
    acc
}

// ---------------- Standardvorlagen ----------------

fn text_layer(name: &str, text: &str, x: f32, y: f32, size: f32, fill: Fill, align: Align) -> Layer {
    Layer {
        name: name.into(),
        kind: Kind::Text { text: text.into(), font: 0, size, fill, outline: Some((Rgba::BLACK, 0.08)), shadow: Some((Rgba([0, 0, 0, 160]), 0.06)), align },
        x, y, w: 0.0, h: 0.0, rotation: 0.0, keystone: 0.0, opacity: 1.0, visible: true,
    }
}
fn photo_layer() -> Layer {
    Layer { name: "Sendefoto".into(), kind: Kind::Picture { source: Source::Photo, path: String::new(), fit: Fit::Cover, border: None },
        x: 0.5, y: 0.5, w: 1.0, h: 1.0, rotation: 0.0, keystone: 0.0, opacity: 1.0, visible: true }
}
fn band_layer(y: f32, h: f32) -> Layer {
    Layer { name: "Balken".into(), kind: Kind::Rect { fill: Fill::Gradient { a: Rgba([0, 0, 0, 200]), b: Rgba([0, 0, 0, 40]), angle: 0.0 }, border: None, radius: 0.0 },
        x: 0.5, y, w: 1.0, h, rotation: 0.0, keystone: 0.0, opacity: 1.0, visible: true }
}

pub fn defaults() -> Vec<Template> {
    let yellow = Fill::Gradient { a: Rgba([255, 230, 80, 255]), b: Rgba([255, 140, 0, 255]), angle: 90.0 };
    let white = Fill::Solid(Rgba::WHITE);
    let pip = Layer {
        name: "Dein Bild".into(),
        kind: Kind::Picture { source: Source::Received, path: String::new(), fit: Fit::Cover, border: Some((Rgba::WHITE, 0.008)) },
        x: 0.78, y: 0.36, w: 0.36, h: 0.36, rotation: -6.0, keystone: 0.0, opacity: 1.0, visible: true,
    };
    vec![
        Template { name: "CQ".into(), phase: Some(Phase::Cq), layers: vec![
            photo_layer(), band_layer(0.14, 0.26),
            text_layer("CQ", "CQ SSTV de %m", 0.5, 0.14, 0.12, yellow, Align::Center),
            text_layer("Locator", "%a  %l", 0.5, 0.91, 0.075, white, Align::Center),
        ]},
        Template { name: "Antwort".into(), phase: Some(Phase::Answer), layers: vec![
            photo_layer(), band_layer(0.12, 0.22),
            text_layer("Rufzeichen", "%c de %m", 0.5, 0.12, 0.11, yellow, Align::Center),
            text_layer("Gruß", "%g %n", 0.5, 0.88, 0.08, white, Align::Center),
        ]},
        Template { name: "Rapport".into(), phase: Some(Phase::Report), layers: vec![
            photo_layer(), band_layer(0.12, 0.22),
            text_layer("Rufzeichen", "%c de %m", 0.5, 0.12, 0.11, yellow, Align::Center),
            pip,
            text_layer("Rapport", "UR %r", 0.25, 0.55, 0.14, white, Align::Center),
            text_layer("Name", "Name %o  QTH %a", 0.5, 0.9, 0.075, white, Align::Center),
        ]},
        Template { name: "73".into(), phase: Some(Phase::Final), layers: vec![
            photo_layer(), band_layer(0.5, 0.36),
            text_layer("73", "TNX %n · 73!", 0.5, 0.45, 0.13, yellow, Align::Center),
            text_layer("Rufzeichen", "%c de %m", 0.5, 0.6, 0.07, white, Align::Center),
        ]},
        Template { name: "Nur Foto + Rufzeichen".into(), phase: None, layers: vec![
            photo_layer(),
            text_layer("Rufzeichen", "%m", 0.2, 0.1, 0.11, white, Align::Left),
        ]},
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with<'a>(files: &'a HashMap<String, RgbImage>) -> Env<'a> {
        let macros = [('c', "DL1ABC"), ('m', "DK0XY"), ('n', "Hans")].into_iter().map(|(k, v)| (k, v.to_string())).collect();
        Env { macros, photo: None, received: None, files }
    }
    fn count_bright(img: &RgbImage, x0: u32, x1: u32, y0: u32, y1: u32) -> usize {
        (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| img.get_pixel(x, y).0[0] > 128).count()
    }

    #[test]
    fn makros() {
        let v: HashMap<char, String> = [('c', "DL1ABC".to_string()), ('r', "595".to_string())].into();
        assert_eq!(expand("%c UR %r 100%% %z", &v), "DL1ABC UR 595 100% %z");
        assert_eq!(expand("ende%", &v), "ende%");
        assert_eq!(greeting(9), "GM");
        assert_eq!(greeting(15), "GA");
        assert_eq!(greeting(21), "GE");
    }

    #[test]
    fn homographie_bildet_ecken_ab() {
        let src: Quad = [[0.0, 0.0], [10.0, 0.0], [10.0, 5.0], [0.0, 5.0]];
        let dst: Quad = [[3.0, 2.0], [17.0, 4.0], [15.0, 11.0], [1.0, 9.0]];
        let h = homography(&src, &dst).unwrap();
        for i in 0..4 {
            let (u, v) = apply(&h, src[i][0] as f64, src[i][1] as f64);
            assert!((u - dst[i][0] as f64).abs() < 1e-6 && (v - dst[i][1] as f64).abs() < 1e-6);
        }
        assert!(quad_contains(&dst, [9.0, 6.0]) && !quad_contains(&dst, [0.0, 0.0]));
    }

    fn white_rect(rot: f32, key: f32) -> Template {
        Template { name: "t".into(), phase: None, layers: vec![Layer {
            name: "r".into(), kind: Kind::Rect { fill: Fill::Solid(Rgba::WHITE), border: None, radius: 0.0 },
            x: 0.5, y: 0.5, w: 0.5, h: 0.2, rotation: rot, keystone: key, opacity: 1.0, visible: true }] }
    }

    #[test]
    fn drehung_und_perspektive() {
        let files = HashMap::new();
        let env = env_with(&files);
        // Rechteck 160×40 in 320×200, mittig
        let (flat, _) = render(&white_rect(0.0, 0.0), 320, 200, &env);
        assert!(count_bright(&flat, 80, 240, 80, 120) > 6000);
        assert_eq!(count_bright(&flat, 150, 170, 0, 70), 0);
        // um 90° gedreht: jetzt 40 breit, 160 hoch
        let (rot, _) = render(&white_rect(90.0, 0.0), 320, 200, &env);
        assert!(count_bright(&rot, 150, 170, 25, 70) > 800, "Drehung wirkt nicht");
        assert_eq!(count_bright(&rot, 80, 130, 90, 110), 0);
        // Perspektive: Oberkante schmaler als Unterkante
        let (ks, _) = render(&white_rect(0.0, 0.5), 320, 200, &env);
        let top = count_bright(&ks, 0, 320, 81, 83);
        let bot = count_bright(&ks, 0, 320, 117, 119);
        assert!(top * 10 < bot * 7, "Perspektive: oben {top}, unten {bot}");
    }

    #[test]
    fn text_mit_makro_und_deckkraft() {
        let files = HashMap::new();
        let env = env_with(&files);
        let mut t = Template { name: "t".into(), phase: None, layers: vec![text_layer("a", "%c", 0.5, 0.5, 0.2, Fill::Solid(Rgba::WHITE), Align::Center)] };
        let (img, quads) = render(&t, 320, 256, &env);
        let q = quads[0].unwrap();
        let wq = q[1][0] - q[0][0];
        assert!(wq > 150.0 && wq < 260.0, "Textbreite für 'DL1ABC' bei 51 px: {wq}");
        assert!(count_bright(&img, 0, 320, 100, 156) > 1500);
        t.layers[0].opacity = 0.0;
        assert_eq!(count_bright(&render(&t, 320, 256, &env).0, 0, 320, 0, 256), 0);
    }

    #[test]
    fn standardvorlagen_json_roundtrip_und_render() {
        let files = HashMap::new();
        let env = env_with(&files);
        for t in defaults() {
            let s = serde_json::to_string_pretty(&t).unwrap();
            let back: Template = serde_json::from_str(&s).unwrap();
            assert_eq!(back, t);
            let (img, _) = render(&t, 320, 256, &env);
            assert_eq!(img.dimensions(), (320, 256));
        }
    }

    /// Sichtprüfung: SSTV_VORSCHAU=/ordner cargo test --release vorschau -- --ignored
    #[test]
    #[ignore]
    fn vorschau() {
        let dir = std::env::var("SSTV_VORSCHAU").unwrap();
        let photo = image::open(format!("{dir}/foto.jpg")).unwrap().to_rgb8();
        let rx = image::open(format!("{dir}/rx.png")).unwrap().to_rgb8();
        let files = HashMap::new();
        let macros = [('c', "DL1ABC"), ('n', "Hans"), ('r', "595"), ('m', "DK0XY"), ('o', "Roman"), ('a', "Berlin"), ('l', "JO62QM"), ('g', "GA")]
            .into_iter().map(|(k, v)| (k, v.to_string())).collect();
        let env = Env { macros, photo: Some(&photo), received: Some(&rx), files: &files };
        for (i, t) in defaults().iter().enumerate() { render(t, 320, 256, &env).0.save(format!("{dir}/vorlage{i}.png")).unwrap(); }
    }
}
