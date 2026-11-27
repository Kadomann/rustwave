//! SSTV-Kern: Modustabellen, Encoder, Decoder (Port der getesteten JS-Version).

use std::f64::consts::TAU;

const F_SYNC: f64 = 1200.0;
const F_BLACK: f64 = 1500.0;
const F_WHITE: f64 = 2300.0;
const F_LOW: f64 = 1350.0;
/// Verzögerung des Diskriminators in Sekunden: nahe der ersten Nullstelle der Rausch-Autokorrelation
/// hinter dem 700-Hz-Kanalfilter, damit Rauschen die gemittelte Frequenz nicht zur Mitte (1700 Hz) zieht.
const LAG_S: f64 = 0.000708;

/// RGB-Bild, 3 Bytes pro Pixel.
#[derive(Clone)]
pub struct Img {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

impl Img {
    pub fn new(w: usize, h: usize) -> Self {
        Img { w, h, data: vec![0; w * h * 3] }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Ch { R, G, B, Y, RY, BY, Y0, Y1 }

#[derive(Clone, Copy)]
enum Seg {
    Tone { f: f64, ms: f64, sync: bool },
    Scan { ch: Ch, ms: f64 },
}
use Seg::*;

impl Seg {
    fn ms(&self) -> f64 {
        match *self { Tone { ms, .. } | Scan { ms, .. } => ms }
    }
}
const fn tone(f: f64, ms: f64) -> Seg { Tone { f, ms, sync: false } }
const fn sync(ms: f64) -> Seg { Tone { f: F_SYNC, ms, sync: true } }

#[derive(Clone, Copy, PartialEq, Debug)]
enum Family { Martin, Scottie, Robot36, Robot72, Pd, Wraase, Pasokon, Bw }

#[derive(Debug)]
pub struct Mode {
    pub name: &'static str,
    pub vis: u8,
    pub w: usize,
    pub h: usize,
    family: Family,
    /// Martin/Scottie/Wraase: Scanzeit je Farbe (ms); PD: Pixelzeit (ms); Pasokon: Zeiteinheit (ms); S/W: Scanzeit (ms)
    t: f64,
    /// Bildzeilen je übertragener Zeile (Martin M3/M4 und Robot 8 S/W: 2, die Zeile wird doppelt gezeigt)
    lh: usize,
}

/// Neue Modi werden hinten angehängt: der gewählte Sendemodus ist als Index gespeichert.
/// Zeiten nach JL Barber N7CXI (Dayton 2000) bzw. KB4YZ, wie in slowrx/MMSSTV/QSSTV.
pub static MODES: [Mode; 23] = [
    Mode { name: "Martin M1", vis: 44, w: 320, h: 256, family: Family::Martin, t: 146.432, lh: 1 },
    Mode { name: "Martin M2", vis: 40, w: 320, h: 256, family: Family::Martin, t: 73.216, lh: 1 },
    Mode { name: "Scottie S1", vis: 60, w: 320, h: 256, family: Family::Scottie, t: 138.24, lh: 1 },
    Mode { name: "Scottie S2", vis: 56, w: 320, h: 256, family: Family::Scottie, t: 88.064, lh: 1 },
    Mode { name: "Scottie DX", vis: 76, w: 320, h: 256, family: Family::Scottie, t: 345.6, lh: 1 },
    Mode { name: "Robot 36", vis: 8, w: 320, h: 240, family: Family::Robot36, t: 0.0, lh: 1 },
    Mode { name: "Robot 72", vis: 12, w: 320, h: 240, family: Family::Robot72, t: 0.0, lh: 1 },
    Mode { name: "PD 50", vis: 93, w: 320, h: 256, family: Family::Pd, t: 0.286, lh: 1 },
    Mode { name: "PD 90", vis: 99, w: 320, h: 256, family: Family::Pd, t: 0.532, lh: 1 },
    Mode { name: "PD 120", vis: 95, w: 640, h: 496, family: Family::Pd, t: 0.19, lh: 1 },
    Mode { name: "PD 160", vis: 98, w: 512, h: 400, family: Family::Pd, t: 0.382, lh: 1 },
    Mode { name: "PD 180", vis: 96, w: 640, h: 496, family: Family::Pd, t: 0.286, lh: 1 },
    Mode { name: "PD 240", vis: 97, w: 640, h: 496, family: Family::Pd, t: 0.382, lh: 1 },
    Mode { name: "PD 290", vis: 94, w: 800, h: 616, family: Family::Pd, t: 0.286, lh: 1 },
    Mode { name: "Martin M3", vis: 36, w: 320, h: 256, family: Family::Martin, t: 146.432, lh: 2 },
    Mode { name: "Martin M4", vis: 32, w: 320, h: 256, family: Family::Martin, t: 73.216, lh: 2 },
    Mode { name: "Wraase SC2-120", vis: 63, w: 320, h: 256, family: Family::Wraase, t: 156.4925, lh: 1 },
    Mode { name: "Wraase SC2-180", vis: 55, w: 320, h: 256, family: Family::Wraase, t: 235.0, lh: 1 },
    Mode { name: "Pasokon P3", vis: 113, w: 640, h: 496, family: Family::Pasokon, t: 1000.0 / 4800.0, lh: 1 },
    Mode { name: "Pasokon P5", vis: 114, w: 640, h: 496, family: Family::Pasokon, t: 1000.0 / 3200.0, lh: 1 },
    Mode { name: "Pasokon P7", vis: 115, w: 640, h: 496, family: Family::Pasokon, t: 1000.0 / 2400.0, lh: 1 },
    Mode { name: "Robot 8 SW", vis: 2, w: 320, h: 240, family: Family::Bw, t: 59.9, lh: 2 },
    Mode { name: "Robot 24 SW", vis: 10, w: 320, h: 240, family: Family::Bw, t: 93.0, lh: 1 },
];

pub fn mode_by_vis(code: u8) -> Option<&'static Mode> {
    MODES.iter().find(|m| m.vis == code)
}

impl Mode {
    /// Anzahl übertragener Zeilen (PD: 2 Bildzeilen je Übertragungszeile)
    pub fn lines(&self) -> usize {
        if self.family == Family::Pd { self.h / 2 } else { self.h / self.lh }
    }
    /// Schwarzweiß-Modus (Robot 8/24 S/W)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_bw(&self) -> bool { self.family == Family::Bw }

    fn preamble(&self) -> Vec<Seg> {
        if self.family == Family::Scottie { vec![tone(F_SYNC, 9.0)] } else { vec![] }
    }

    fn layout(&self, i: usize) -> Vec<Seg> {
        let s = self.t;
        match self.family {
            Family::Martin => {
                let p = 0.572;
                vec![sync(4.862), tone(F_BLACK, p), Scan { ch: Ch::G, ms: s }, tone(F_BLACK, p),
                     Scan { ch: Ch::B, ms: s }, tone(F_BLACK, p), Scan { ch: Ch::R, ms: s }, tone(F_BLACK, p)]
            }
            Family::Scottie => vec![tone(F_BLACK, 1.5), Scan { ch: Ch::G, ms: s }, tone(F_BLACK, 1.5),
                Scan { ch: Ch::B, ms: s }, sync(9.0), tone(F_BLACK, 1.5), Scan { ch: Ch::R, ms: s }],
            Family::Robot36 => {
                let odd = i % 2 == 1;
                vec![sync(9.0), tone(F_BLACK, 3.0), Scan { ch: Ch::Y, ms: 88.0 },
                     tone(if odd { F_WHITE } else { F_BLACK }, 4.5), tone(1900.0, 1.5),
                     Scan { ch: if odd { Ch::BY } else { Ch::RY }, ms: 44.0 }]
            }
            Family::Robot72 => vec![sync(9.0), tone(F_BLACK, 3.0), Scan { ch: Ch::Y, ms: 138.0 },
                tone(F_BLACK, 4.5), tone(1900.0, 1.5), Scan { ch: Ch::RY, ms: 69.0 },
                tone(F_WHITE, 4.5), tone(1900.0, 1.5), Scan { ch: Ch::BY, ms: 69.0 }],
            Family::Pd => {
                let d = self.w as f64 * s;
                vec![sync(20.0), tone(F_BLACK, 2.08), Scan { ch: Ch::Y0, ms: d }, Scan { ch: Ch::RY, ms: d },
                     Scan { ch: Ch::BY, ms: d }, Scan { ch: Ch::Y1, ms: d }]
            }
            // Sync, Schwarzschulter, dann R, G, B ohne Trennpausen
            Family::Wraase => vec![sync(5.5225), tone(F_BLACK, 0.5), Scan { ch: Ch::R, ms: s },
                Scan { ch: Ch::G, ms: s }, Scan { ch: Ch::B, ms: s }],
            // in Zeiteinheiten u: Sync 25u, je 5u Schwarz vor, zwischen und nach den Farben, je Farbe 640u
            Family::Pasokon => {
                let (d, p) = (self.w as f64 * s, 5.0 * s);
                vec![sync(25.0 * s), tone(F_BLACK, p), Scan { ch: Ch::R, ms: d }, tone(F_BLACK, p),
                     Scan { ch: Ch::G, ms: d }, tone(F_BLACK, p), Scan { ch: Ch::B, ms: d }, tone(F_BLACK, p)]
            }
            Family::Bw => vec![sync(7.0), Scan { ch: Ch::Y, ms: s }],
        }
    }

    pub fn line_ms(&self) -> f64 { self.layout(0).iter().map(|s| s.ms()).sum() }
    fn sync_info(&self) -> (f64, f64) { // (Offset bis Sync, Sync-Dauer) in ms
        let lay = self.layout(0);
        let k = lay.iter().position(|s| matches!(s, Tone { sync: true, .. })).unwrap();
        (lay[..k].iter().map(|s| s.ms()).sum(), lay[k].ms())
    }
    fn pre_ms(&self) -> f64 { self.preamble().iter().map(|s| s.ms()).sum() }
    /// Gesamtdauer inkl. VIS in Sekunden
    pub fn seconds(&self) -> f64 { (self.pre_ms() + self.lines() as f64 * self.line_ms() + 910.0) / 1000.0 }

    /// Kanalwerte (0..255) der Übertragungszeile i
    fn channels(&self, img: &Img, i: usize) -> [Vec<f32>; 8] {
        let mut c: [Vec<f32>; 8] = Default::default();
        match self.family {
            Family::Martin | Family::Scottie | Family::Wraase | Family::Pasokon => {
                let (r, g, b) = if self.lh == 2 {
                    let (a, b) = (row_rgb(img, 2 * i), row_rgb(img, 2 * i + 1));
                    (avg(&a.0, &b.0), avg(&a.1, &b.1), avg(&a.2, &b.2))
                } else { row_rgb(img, i) };
                c[Ch::R as usize] = r; c[Ch::G as usize] = g; c[Ch::B as usize] = b;
            }
            Family::Bw => {
                let g = row_gray(img, i * self.lh);
                c[Ch::Y as usize] = if self.lh == 2 { avg(&g, &row_gray(img, i * 2 + 1)) } else { g };
            }
            Family::Robot36 => {
                let a = row_yuv(img, i);
                let b = row_yuv(img, i ^ 1);
                c[Ch::Y as usize] = a.0;
                c[Ch::RY as usize] = avg(&a.1, &b.1);
                c[Ch::BY as usize] = avg(&a.2, &b.2);
            }
            Family::Robot72 => {
                let a = row_yuv(img, i);
                c[Ch::Y as usize] = a.0; c[Ch::RY as usize] = a.1; c[Ch::BY as usize] = a.2;
            }
            Family::Pd => {
                let a = row_yuv(img, 2 * i);
                let b = row_yuv(img, 2 * i + 1);
                c[Ch::RY as usize] = avg(&a.1, &b.1);
                c[Ch::BY as usize] = avg(&a.2, &b.2);
                c[Ch::Y0 as usize] = a.0; c[Ch::Y1 as usize] = b.0;
            }
        }
        c
    }

    /// Aus empfangenen Kanälen -> fertige Bildzeilen (y, RGB)
    fn rows(&self, i: usize, c: &[Vec<f32>; 8], st: &mut (Vec<f32>, Vec<f32>)) -> Vec<(usize, Vec<u8>)> {
        let g = |ch: Ch| &c[ch as usize];
        let rep = |row: Vec<u8>| (0..self.lh).map(|k| (i * self.lh + k, row.clone())).collect::<Vec<_>>();
        match self.family {
            Family::Martin | Family::Scottie | Family::Wraase | Family::Pasokon => rep(from_rgb(g(Ch::R), g(Ch::G), g(Ch::B))),
            Family::Bw => rep(g(Ch::Y).iter().flat_map(|v| { let v = v.round() as u8; [v, v, v] }).collect()),
            Family::Robot36 => {
                if i % 2 == 0 {
                    *st = (g(Ch::Y).clone(), g(Ch::RY).clone());
                    vec![]
                } else {
                    vec![(i - 1, from_yuv(&st.0, &st.1, g(Ch::BY))), (i, from_yuv(g(Ch::Y), &st.1, g(Ch::BY)))]
                }
            }
            Family::Robot72 => vec![(i, from_yuv(g(Ch::Y), g(Ch::RY), g(Ch::BY)))],
            Family::Pd => vec![(2 * i, from_yuv(g(Ch::Y0), g(Ch::RY), g(Ch::BY))),
                               (2 * i + 1, from_yuv(g(Ch::Y1), g(Ch::RY), g(Ch::BY)))],
        }
    }
}

// ---- Farbräume (BT.601, wie MMSSTV/QSSTV) ----
fn clamp(v: f64) -> f64 { v.clamp(0.0, 255.0) }
pub fn rgb_to_yuv(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    (16.0 + (65.738 * r + 129.057 * g + 25.064 * b) / 256.0,
     128.0 + (112.439 * r - 94.154 * g - 18.285 * b) / 256.0,
     128.0 + (-37.945 * r - 74.494 * g + 112.439 * b) / 256.0)
}
pub fn yuv_to_rgb(y: f64, ry: f64, by: f64) -> [u8; 3] {
    let (y, ry, by) = (1.164 * (y - 16.0), ry - 128.0, by - 128.0);
    [clamp(y + 1.596 * ry).round() as u8, clamp(y - 0.813 * ry - 0.392 * by).round() as u8, clamp(y + 2.017 * by).round() as u8]
}
fn row_rgb(img: &Img, y: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let row = &img.data[y * img.w * 3..(y + 1) * img.w * 3];
    (row.chunks(3).map(|p| p[0] as f32).collect(),
     row.chunks(3).map(|p| p[1] as f32).collect(),
     row.chunks(3).map(|p| p[2] as f32).collect())
}
/// Helligkeit 0..255 (volle Skala, wie Robot S/W)
fn row_gray(img: &Img, y: usize) -> Vec<f32> {
    img.data[y * img.w * 3..(y + 1) * img.w * 3].chunks(3)
        .map(|p| (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32).clamp(0.0, 255.0)).collect()
}
fn row_yuv(img: &Img, y: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let (r, g, b) = row_rgb(img, y);
    let mut out = (vec![0f32; img.w], vec![0f32; img.w], vec![0f32; img.w]);
    for x in 0..img.w {
        let (yy, ry, by) = rgb_to_yuv(r[x] as f64, g[x] as f64, b[x] as f64);
        out.0[x] = yy as f32; out.1[x] = ry as f32; out.2[x] = by as f32;
    }
    out
}
fn avg(a: &[f32], b: &[f32]) -> Vec<f32> { a.iter().zip(b).map(|(x, y)| (x + y) / 2.0).collect() }
fn from_rgb(r: &[f32], g: &[f32], b: &[f32]) -> Vec<u8> {
    (0..r.len()).flat_map(|x| [r[x].round() as u8, g[x].round() as u8, b[x].round() as u8]).collect()
}
fn from_yuv(y: &[f32], ry: &[f32], by: &[f32]) -> Vec<u8> {
    (0..y.len()).flat_map(|x| yuv_to_rgb(y[x] as f64, ry[x] as f64, by[x] as f64)).collect()
}

fn vis_segments(code: u8) -> Vec<Seg> {
    let mut v = vec![tone(1900.0, 300.0), tone(1200.0, 10.0), tone(1900.0, 300.0), tone(1200.0, 30.0)];
    let mut par = 0;
    for k in 0..7 {
        let b = (code >> k) & 1;
        par ^= b;
        v.push(tone(if b == 1 { 1100.0 } else { 1300.0 }, 30.0));
    }
    v.push(tone(if par == 1 { 1100.0 } else { 1300.0 }, 30.0)); // gerade Parität
    v.push(tone(1200.0, 30.0));
    v
}

// ---- Encoder: Bild (exakt mode.w × mode.h) -> Audio ----
pub fn encode(mode: &Mode, img: &Img, fs: f64, amp: f32) -> Vec<f32> {
    assert!(img.w == mode.w && img.h == mode.h, "Bild muss {}×{} sein", mode.w, mode.h);
    enum Part { F(f64, f64), V(Vec<f32>, f64) }
    let mut parts: Vec<Part> = vis_segments(mode.vis).into_iter().chain(mode.preamble())
        .map(|s| match s { Tone { f, ms, .. } => Part::F(f, ms), Scan { .. } => unreachable!() }).collect();
    for i in 0..mode.lines() {
        let mut ch = mode.channels(img, i);
        for s in mode.layout(i) {
            parts.push(match s {
                Tone { f, ms, .. } => Part::F(f, ms),
                Scan { ch: c, ms } => Part::V(std::mem::take(&mut ch[c as usize]), ms), // jeder Kanal einmal je Zeile
            });
        }
    }
    let total: f64 = parts.iter().map(|p| match p { Part::F(_, ms) | Part::V(_, ms) => *ms }).sum();
    let n = (total * fs / 1000.0).ceil() as usize;
    let mut out = vec![0f32; n];
    let (mut t, mut idx, mut ph) = (0.0f64, 0usize, 0.0f64);
    for p in &parts {
        let ms = match p { Part::F(_, ms) | Part::V(_, ms) => *ms };
        let t1 = t + ms;
        let end = ((t1 * fs / 1000.0).round() as usize).min(n);
        while idx < end {
            let f = match p {
                Part::F(f, _) => *f,
                Part::V(v, _) => {
                    let w = v.len() as f64;
                    let j = (((idx as f64 * 1000.0 / fs - t) / ms * w).floor().max(0.0) as usize).min(v.len() - 1);
                    F_BLACK + clamp(v[j] as f64) * (800.0 / 255.0)
                }
            };
            ph += TAU * f / fs;
            if ph > TAU { ph -= TAU; }
            out[idx] = amp * ph.sin() as f32;
            idx += 1;
        }
        t = t1;
    }
    out
}

// ---- Decoder ----
pub(crate) struct Biquad { b0: f64, b1: f64, a1: f64, a2: f64, z1: f64, z2: f64 }
impl Biquad {
    pub(crate) fn lowpass(fs: f64, fc: f64, q: f64) -> Self {
        let w = TAU * fc / fs;
        let (c, a) = (w.cos(), w.sin() / (2.0 * q));
        let a0 = 1.0 + a;
        Biquad { b0: (1.0 - c) / 2.0 / a0, b1: (1.0 - c) / a0, a1: -2.0 * c / a0, a2: (1.0 - a) / a0, z1: 0.0, z2: 0.0 }
    }
    #[cfg(test)]
    pub(crate) fn highpass(fs: f64, fc: f64, q: f64) -> Self {
        let w = TAU * fc / fs;
        let (c, a) = (w.cos(), w.sin() / (2.0 * q));
        let a0 = 1.0 + a;
        Biquad { b0: (1.0 + c) / 2.0 / a0, b1: -(1.0 + c) / a0, a1: -2.0 * c / a0, a2: (1.0 - a) / a0, z1: 0.0, z2: 0.0 }
    }
    pub(crate) fn run(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b0 * x - self.a2 * y;
        y
    }
}

pub enum Event {
    Start(&'static Mode),
    Line { y: usize, rgb: Vec<u8> },
    End { mode: &'static Mode, img: Img, complete: bool },
}

struct ImageState {
    mode: &'static Mode,
    img: Img,
    period: f64,
    sync_off: f64,
    sync_len: f64,
    line: usize,
    miss: usize,
    cand: Option<(usize, f64, usize)>, // (Zeile, Versatz, Anzahl übereinstimmender Zeilen)
    good: Vec<(usize, f64)>,
    sums: [f64; 4], // sx, sy, sxx, sxy
    st: (Vec<f32>, Vec<f32>),
    line_end: f64,
}

pub struct Decoder {
    fs: f64,
    fc: f64,
    k: f64,
    dph: f64,
    ph: f64,
    lp: [Biquad; 4],
    lag: usize,              // Verzögerung des Phasendiskriminators in Samples
    hist: Vec<(f64, f64)>,   // Ringpuffer der letzten `lag` I/Q-Werte
    hpos: usize,
    p_re: Vec<f64>, // Präfixsummen von z[n]·conj(z[n-lag]), Realteil
    p_sr: Vec<f64>, // dito mit halber Verzögerung (zeitgleich zentriert), Real-/Imaginärteil:
    p_si: Vec<f64>, // für Flanken, Sync-Schwelle und Mischfenster (kein Phasenumschlag bei 1100/1900 Hz)
    p_f: Vec<f64>,  // … Imaginärteil
    p_l: Vec<f64>, // Präfixsummen "Frequenz < 1350 Hz"
    base: f64,     // absoluter Index von p_f[0]
    scan_pos: f64,
    afc: f64, // beim VIS gemessene Verstimmung in Hz, wird beim Bild abgezogen
    img: Option<ImageState>,
    pub events: Vec<Event>,
}

impl Decoder {
    pub fn new(fs: f64) -> Self {
        let fc = 1700.0;
        let lp = |q| Biquad::lowpass(fs, 700.0, q);
        let lag = ((LAG_S * fs).round() as usize).max(1);
        Decoder {
            fs, fc, k: fs / TAU, dph: TAU * fc / fs, ph: 0.0,
            lp: [lp(0.5412), lp(1.3066), lp(0.5412), lp(1.3066)], // je 4. Ordnung Butterworth für I und Q
            lag, hist: vec![(0.0, 0.0); lag], hpos: 0, p_re: vec![0.0], p_sr: vec![0.0], p_si: vec![0.0],
            p_f: vec![0.0], p_l: vec![0.0], base: 0.0, scan_pos: 0.0, afc: 0.0, img: None, events: vec![],
        }
    }
    fn s(&self, ms: f64) -> f64 { ms * self.fs / 1000.0 }
    fn len(&self) -> usize { self.p_f.len() - 1 }
    fn end(&self) -> f64 { self.base + self.len() as f64 }

    pub fn push(&mut self, x: &[f32]) {
        for &v in x {
            let v = v as f64;
            self.ph += self.dph;
            if self.ph > TAU { self.ph -= TAU; }
            let i0 = self.lp[0].run(v * self.ph.cos());
            let i = self.lp[1].run(i0);
            let q0 = self.lp[2].run(-v * self.ph.sin());
            let q = self.lp[3].run(q0);
            let (lag, h0) = (self.lag, self.hpos);
            let (pi, pq) = std::mem::replace(&mut self.hist[h0], (i, q));
            self.hpos = (h0 + 1) % lag;
            let (re, im) = (i * pi + q * pq, q * pi - i * pq);
            // kurze Verzögerung: z[n-a]·conj(z[n-a-lag/2]), Mitte wie beim langen Produkt bei n-lag/2
            let (a, b) = (lag / 4, lag / 4 + lag / 2);
            let at = |m: usize| if m == 0 { (i, q) } else { self.hist[(h0 + lag - m) % lag] };
            let ((ai, aq), (bi, bq)) = (at(a), at(b));
            let (sr, si) = (ai * bi + aq * bq, aq * bi - ai * bq);
            let f = self.fc + si.atan2(sr) * self.k / (b - a) as f64;
            let (lr, lf, ll) = (*self.p_re.last().unwrap(), *self.p_f.last().unwrap(), *self.p_l.last().unwrap());
            let (l1, l2) = (*self.p_sr.last().unwrap(), *self.p_si.last().unwrap());
            self.p_sr.push(l1 + sr);
            self.p_si.push(l2 + si);
            self.p_re.push(lr + re);
            self.p_f.push(lf + im);
            self.p_l.push(ll + if f - self.afc < F_LOW { 1.0 } else { 0.0 });
        }
        self.run();
    }

    fn drop_before(&mut self, abs: f64) {
        let k = (abs - self.base).floor();
        if k < self.fs * 5.0 { return; }
        let k = k as usize;
        for a in [&mut self.p_re, &mut self.p_f, &mut self.p_l, &mut self.p_sr, &mut self.p_si] {
            let off = a[k];
            a.drain(..k);
            for v in a.iter_mut() { *v -= off; }
        }
        self.base += k as f64;
    }
    fn idx(&self, a: f64) -> usize { (a.round() - self.base).clamp(0.0, self.len() as f64) as usize }
    fn avg_of(&self, low: bool, a: f64, b: f64) -> f64 {
        let (mut ia, mut ib) = (self.idx(a), self.idx(b));
        if ib <= ia {
            if ia >= self.len() { ia = self.len() - 1; }
            ib = ia + 1;
        }
        if low { return (self.p_l[ib] - self.p_l[ia]) / (ib - ia) as f64; }
        let (re, im) = (self.p_re[ib] - self.p_re[ia], self.p_f[ib] - self.p_f[ia]);
        self.fc + im.atan2(re) * self.k / self.lag as f64
    }
    /// Verstimmung übernehmen und die Sync-Schwelle ab `from` (abs.) rückwirkend neu auswerten
    fn set_afc(&mut self, df: f64, from: f64) {
        self.afc = df;
        let i0 = self.idx(from).max(1);
        let (fc, k) = (self.fc, self.k / (self.lag / 2) as f64);
        for n in i0..=self.len() {
            let (re, im) = (self.p_sr[n] - self.p_sr[n - 1], self.p_si[n] - self.p_si[n - 1]);
            let low = (fc + im.atan2(re) * k - df < F_LOW) as u8 as f64;
            self.p_l[n] = self.p_l[n - 1] + low;
        }
    }
    /// mittlere Frequenz mit kurzer Verzögerung: gröber im Rauschen, aber eindeutig auch über 1900/1200-Wechsel hinweg
    fn mean_s(&self, a: f64, b: f64) -> f64 {
        let (mut ia, mut ib) = (self.idx(a), self.idx(b));
        if ib <= ia {
            if ia >= self.len() { ia = self.len() - 1; }
            ib = ia + 1;
        }
        let (re, im) = (self.p_sr[ib] - self.p_sr[ia], self.p_si[ib] - self.p_si[ia]);
        self.fc + im.atan2(re) * self.k / (self.lag / 2) as f64
    }
    fn mean(&self, a: f64, b: f64) -> f64 { self.avg_of(false, a, b) }
    fn low(&self, a: f64, b: f64) -> f64 { self.avg_of(true, a, b) }

    fn run(&mut self) {
        loop {
            let progressed = if self.img.is_none() { self.find_vis() } else { self.decode_lines() };
            if !progressed { return; }
        }
    }

    fn find_vis(&mut self) -> bool {
        if self.len() < 2 { return false; }
        let step = self.s(1.0);
        let last = self.end() - self.s(300.0);
        let mut p = self.scan_pos.max(self.base + self.s(260.0));
        let fs = self.fs;
        let s = |ms: f64| ms * fs / 1000.0;
        while p <= last {
            // grob: Vorton 1900 Hz (±120 Hz Verstimmung) und danach das Startbit 700 Hz tiefer
            let lead = self.mean(p - s(250.0), p - s(20.0));
            if (lead - 1900.0).abs() <= 120.0 && (self.mean(p + s(4.0), p + s(26.0)) - lead + 700.0).abs() <= 90.0 {
                if let Some((mode, t0, df)) = self.vis_at(p, lead) {
                    let t1 = t0 + s(300.0);
                    self.set_afc(df, t0);
                    self.scan_pos = t1;
                    self.start_image(mode, t1);
                    return true;
                }
            }
            p += step;
        }
        self.scan_pos = self.scan_pos.max(p);
        self.drop_before(self.scan_pos - self.s(400.0));
        false
    }

    /// Feinprüfung eines VIS-Kandidaten: Flanke 1900→1200 suchen, Verstimmung schätzen, Bits lesen,
    /// Lage über das ganze Bitmuster nachführen und dann streng prüfen.
    /// Liefert (Modus, Beginn des Startbits, Frequenzversatz).
    fn vis_at(&self, p: f64, lead: f64) -> Option<(&'static Mode, f64, f64)> {
        let s = |ms: f64| ms * self.fs / 1000.0;
        // grobe Flanke: größter Abfall zwischen je 20 ms davor/danach, auf 0,25 ms genau
        let (mut t0, mut best) = (p, f64::MIN);
        let mut q = p - s(15.0);
        while q <= p + s(15.0) {
            let d = self.mean_s(q - s(20.0), q) - self.mean_s(q, q + s(20.0));
            if d > best { best = d; t0 = q; }
            q += s(0.25);
        }
        let start = self.mean(t0 + s(3.0), t0 + s(27.0));
        let df = ((lead - 1900.0) + (start - 1200.0)) / 2.0;
        if (lead - start - 700.0).abs() > 80.0 || df.abs() > 120.0 { return None; }
        // Bitfenster k (0 = Startbit, 1..=8 Daten+Parität, 9 = Stoppbit), Rand `m` ms
        let bit = |t: f64, k: usize, m: f64| self.mean(t + s(30.0 * k as f64 + m), t + s(30.0 * (k + 1) as f64 - m)) - df;
        let cost = |t: f64, pat: &[f64]| -> f64 {
            let mut c = (self.mean(t - s(28.0), t - s(2.0)) - df - 1900.0).powi(2);
            for (k, f) in pat.iter().enumerate() { c += (bit(t, k, 2.0) - f).powi(2); }
            c
        };
        for _ in 0..2 {
            // vorläufige Bits (nur Vorzeichen) → Sollmuster → Lage ±6 ms in 0,125-ms-Schritten nachführen
            let pat: Vec<f64> = (0..10).map(|k| match k {
                0 | 9 => 1200.0,
                _ => if bit(t0, k, 3.0) < 1200.0 { 1100.0 } else { 1300.0 },
            }).collect();
            let (mut tb, mut cb) = (t0, f64::MAX);
            let mut q = t0 - s(6.0);
            while q <= t0 + s(6.0) {
                let c = cost(q, &pat);
                if c < cb { cb = c; tb = q; }
                q += s(0.125);
            }
            t0 = tb;
        }
        let (mut code, mut par) = (0u8, 0u8);
        for k in 0..8 {
            let m = bit(t0, k + 1, 3.0);
            // 1100 Hz = 1, 1300 Hz = 0; nahe 1200 Hz ist das Bit unsicher → verwerfen
            if !(1030.0..=1370.0).contains(&m) || (m - 1200.0).abs() < 35.0 { return None; }
            let b = (m < 1200.0) as u8;
            par ^= b;
            if k < 7 { code |= b << k; }
        }
        if par != 0 || (bit(t0, 9, 3.0) - 1200.0).abs() > 70.0 || (bit(t0, 0, 3.0) - 1200.0).abs() > 70.0 { return None; }
        mode_by_vis(code).map(|m| (m, t0, df))
    }

    fn start_image(&mut self, mode: &'static Mode, t: f64) {
        let (off_ms, sync_ms) = mode.sync_info();
        let sync_off = self.s(off_ms);
        self.img = Some(ImageState {
            mode, img: Img::new(mode.w, mode.h),
            period: self.s(mode.line_ms()), sync_off, sync_len: self.s(sync_ms),
            line: 0, miss: 0, cand: None,
            good: vec![(0, t + self.s(mode.pre_ms()) + sync_off)], // VIS-Timing als Startpunkt
            sums: [0.0; 4], st: Default::default(), line_end: 0.0,
        });
        self.events.push(Event::Start(mode));
    }

    /// Sync-Position (abs.) für Zeile i: Gerade durch akzeptierte Syncs (Slant-Korrektur)
    fn fit(st: &ImageState, i: usize) -> f64 {
        let (g, p) = (&st.good, st.period);
        let (j0, p0) = g[0];
        let n = g.len() as f64;
        if g.len() < 3 {
            let (j, pj) = g[g.len() - 1];
            return pj + p * (i as f64 - j as f64);
        }
        let [sx, sy, sxx, sxy] = st.sums;
        // Steigung mit Vorwissen "nominale Zeilendauer" (Ridge): wenige verrauschte Syncs kippen die Gerade nicht
        const PRIOR: f64 = 400.0;
        let (vxx, vxy) = (sxx - sx * sx / n, sxy - sx * sy / n);
        let slope = ((vxy + PRIOR * p) / (vxx + PRIOR)).clamp(p * 0.995, p * 1.005);
        p0 + (sy - slope * sx) / n + slope * (i as f64 - j0 as f64)
    }
    fn accept(st: &mut ImageState, i: usize, p: f64) {
        let (j0, p0) = st.good[0];
        let (x, y) = (i as f64 - j0 as f64, p - p0);
        st.good.push((i, p));
        let s = &mut st.sums;
        s[0] += x; s[1] += y; s[2] += x * x; s[3] += x * y;
    }

    /// Sync-Güte bei p: Anteil tiefer Frequenzen im Puls minus davor/danach (beide Flanken, 1 = ideal)
    fn sync_score(&self, p: f64, l: f64) -> f64 {
        self.low(p, p + l) - 0.5 * (self.low(p - l / 2.0, p) + self.low(p + l, p + 1.5 * l))
    }

    fn decode_lines(&mut self) -> bool {
        let mut st = self.img.take().unwrap();
        let m = st.mode;
        let l = st.sync_len;
        while st.line < m.lines() {
            let i = st.line;
            let pred = Self::fit(&st, i);
            // nach 2 verpassten Syncs breit suchen (Audio-Aussetzer verschieben die Zeitachse)
            let wide = st.miss >= 2;
            let w = if wide { st.period / 2.0 } else { self.s(8.0).max(st.period * 0.015) };
            if self.end() < (pred + w + 1.5 * l).max(pred - st.sync_off + st.period + w) {
                self.img = Some(st);
                return false;
            }
            let (mut best, mut bp) = (-1.0, pred);
            let mut p = (pred - w).round();
            while p <= pred + w {
                let sc = self.sync_score(p, l);
                if sc > best { best = sc; bp = p; }
                p += 1.0;
            }
            // nur Syncs nahe der Vorhersage übernehmen; ferne Treffer sind meist Rauschen
            if best > 0.5 && (bp - pred).abs() < self.s(2.0) {
                Self::accept(&mut st, i, bp); st.miss = 0; st.cand = None;
            } else if wide && best > 0.3 {
                // neu einrasten, wenn aufeinanderfolgende Zeilen denselben Versatz zeigen:
                // klarer Sync (> 0,7) zweimal, schwacher (im Rauschen) dreimal
                let off = bp - pred;
                let n = match st.cand {
                    Some((ci, coff, n)) if ci + 1 == i && (off - coff).abs() < self.s(1.0) => n + 1,
                    _ => 1,
                };
                let strong = best > 0.7 && st.cand.is_some_and(|c| c.2 >= 1 && n >= 2);
                if strong || n >= 3 {
                    for g in st.good.iter_mut() { g.1 += off; } // Gerade verschieben, Steigung bleibt
                    Self::accept(&mut st, i, bp); st.miss = 0; st.cand = None;
                } else {
                    st.cand = Some((i, off, n)); st.miss += 1;
                }
            } else {
                st.miss += 1;
            }
            let mut t = Self::fit(&st, i) - st.sync_off;
            let mut ch: [Vec<f32>; 8] = Default::default();
            for s in m.layout(i) {
                let len = self.s(s.ms());
                if let Scan { ch: c, .. } = s {
                    let px = len / m.w as f64;
                    ch[c as usize] = (0..m.w).map(|x| {
                        let x = x as f64;
                        clamp((self.mean(t + x * px, t + (x + 1.0) * px) - self.afc - F_BLACK) * (255.0 / 800.0)) as f32
                    }).collect();
                }
                t += len;
            }
            for (y, row) in m.rows(i, &ch, &mut st.st) {
                st.img.data[y * m.w * 3..(y + 1) * m.w * 3].copy_from_slice(&row);
                self.events.push(Event::Line { y, rgb: row });
            }
            st.line += 1;
            st.line_end = t;
            if st.miss > 25 { break; } // Signal verloren
        }
        self.finish(st);
        true
    }

    fn finish(&mut self, st: ImageState) {
        if st.line_end > 0.0 { self.scan_pos = st.line_end; }
        let complete = st.line >= st.mode.lines();
        self.events.push(Event::End { mode: st.mode, img: st.img, complete });
        self.drop_before(self.scan_pos - self.s(400.0));
    }

    /// Laufendes Bild sofort beenden (Teilbild wird ausgegeben)
    pub fn abort(&mut self) {
        if let Some(st) = self.img.take() { self.finish(st); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_image(w: usize, h: usize) -> Img {
        let mut img = Img::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 3;
                img.data[o] = (255.0 * x as f64 / w as f64) as u8;
                img.data[o + 1] = (255.0 * y as f64 / h as f64) as u8;
                img.data[o + 2] = (128.0 + 100.0 * (x as f64 / 25.0).sin() * (y as f64 / 30.0).cos()) as u8;
            }
        }
        img
    }
    struct Rng(u32);
    impl Rng {
        fn u(&mut self) -> f64 { self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345); self.0 as f64 / 4294967296.0 }
        fn gauss(&mut self) -> f64 { (-2.0 * (self.u() + 1e-12).ln()).sqrt() * (TAU * self.u()).cos() }
    }

    /// Bild -> Audio (Taktfehler, Rauschen, optional Lücke) -> Decoder -> (komplett, richtiger Modus, PSNR)
    fn run(mode: &'static Mode, ppm: f64, noise: f64, gap_ms: f64) -> (bool, f64) {
        let fs = 48000.0;
        let mut img = test_image(mode.w, mode.h);
        if mode.is_bw() { // Schwarzweiß-Modi: Vergleich mit dem Graubild
            for p in img.data.chunks_mut(3) {
                let g = (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32).round() as u8;
                p.copy_from_slice(&[g, g, g]);
            }
        }
        let sig = encode(mode, &img, fs * (1.0 + ppm * 1e-6), 0.5);
        let pad = vec![0f32; fs as usize];
        let mut x: Vec<f32> = pad.iter().chain(&sig).chain(&pad).copied().collect();
        if gap_ms != 0.0 { // Audio-Aussetzer mitten im Bild
            let at = pad.len() + sig.len() / 2;
            let n = (gap_ms.abs() * fs / 1000.0).round() as usize;
            if gap_ms > 0.0 { x.splice(at..at, std::iter::repeat_n(0.0, n)); } else { x.drain(at..at + n); }
        }
        let mut rng = Rng(1);
        for v in x.iter_mut() { *v += (noise * rng.gauss()) as f32; }
        let mut dec = Decoder::new(fs);
        let mut res = None;
        for c in x.chunks(4096) {
            dec.push(c);
            for e in dec.events.drain(..) {
                if let Event::End { mode: m, img, complete } = e { res = Some((m, img, complete)); }
            }
        }
        let (m, out, complete) = res.expect("kein Bild dekodiert");
        let se: f64 = img.data.iter().zip(&out.data).map(|(a, b)| (*a as f64 - *b as f64).powi(2)).sum();
        let psnr = 10.0 * (255.0f64.powi(2) / (se / img.data.len() as f64)).log10();
        (complete && std::ptr::eq(m, mode), psnr)
    }

    /// weißes Rauschen auf 300–2700 Hz begrenzt (wie hinter einem SSB-Filter), Leistung so, dass im Band `snr_db` gilt
    fn band_noise(n: usize, fs: f64, snr_db: f64, sig_amp: f64, seed: u32) -> Vec<f32> {
        let mut rng = Rng(seed);
        let mut hp = [Biquad::highpass(fs, 300.0, 0.5412), Biquad::highpass(fs, 300.0, 1.3066)];
        let mut lp = [Biquad::lowpass(fs, 2700.0, 0.5412), Biquad::lowpass(fs, 2700.0, 1.3066)];
        let mut x: Vec<f64> = (0..n).map(|_| { let mut v = rng.gauss(); for f in hp.iter_mut().chain(lp.iter_mut()) { v = f.run(v); } v }).collect();
        let p = x.iter().map(|v| v * v).sum::<f64>() / n as f64;
        let want = sig_amp * sig_amp / 2.0 / 10f64.powf(snr_db / 10.0);
        let k = (want / p).sqrt();
        for v in x.iter_mut() { *v *= k; }
        x.into_iter().map(|v| v as f32).collect()
    }

    /// (VIS erkannt, komplett, PSNR) bei SNR im 2,4-kHz-Band und Frequenzversatz (verstimmter SSB-Empfänger)
    pub(super) fn bench_one(mode: &'static Mode, snr_db: f64, df: f64, seed: u32) -> (bool, bool, f64) {
        let fs = 48000.0;
        let img = test_image(mode.w, mode.h);
        let img = if mode.is_bw() { let mut g = img.clone(); for p in g.data.chunks_mut(3) {
            let v = (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32).round() as u8; p.copy_from_slice(&[v, v, v]); } g } else { img };
        let mut sig = encode(mode, &img, fs, 0.5);
        if df != 0.0 { // Frequenzversatz per Einseitenband-Verschiebung (Hilbert über FFT wäre genauer; hier: analytisch per Mischung)
            sig = shift(&sig, fs, df);
        }
        let pad = vec![0f32; fs as usize];
        let mut x: Vec<f32> = pad.iter().chain(&sig).chain(&pad).copied().collect();
        let nz = band_noise(x.len(), fs, snr_db, 0.5, seed);
        for (v, n) in x.iter_mut().zip(nz) { *v += n; }
        let mut dec = Decoder::new(fs);
        let (mut vis, mut res) = (false, None);
        for c in x.chunks(4096) {
            dec.push(c);
            for e in dec.events.drain(..) {
                match e {
                    Event::Start(m) if std::ptr::eq(m, mode) => vis = true,
                    Event::End { mode: m, img, complete } if std::ptr::eq(m, mode) && res.is_none() => res = Some((img, complete)),
                    _ => {}
                }
            }
        }
        let Some((out, complete)) = res else { return (vis, false, 0.0) };
        let se: f64 = img.data.iter().zip(&out.data).map(|(a, b)| (*a as f64 - *b as f64).powi(2)).sum();
        (vis, complete, 10.0 * (255.0f64.powi(2) / (se / img.data.len() as f64)).log10())
    }

    /// Frequenz eines reellen Signals um df verschieben (Mischung mit analytischem Signal über einen einfachen Hilbert-FIR)
    fn shift(x: &[f32], fs: f64, df: f64) -> Vec<f32> {
        let n = 127usize;
        let h: Vec<f64> = (0..n).map(|k| { let m = k as i64 - (n / 2) as i64;
            if m % 2 == 0 { 0.0 } else { 2.0 / (std::f64::consts::PI * m as f64) * (0.54 - 0.46 * (TAU * k as f64 / (n - 1) as f64).cos()) } }).collect();
        (0..x.len()).map(|i| {
            let re = x[i] as f64;
            let im: f64 = (0..n).filter(|&k| i + n / 2 >= k && i + n / 2 - k < x.len()).map(|k| h[k] * x[i + n / 2 - k] as f64).sum(); // Σ h(m)·x[i-m], m = k - n/2
            let w = TAU * df * i as f64 / fs;
            (re * w.cos() - im * w.sin()) as f32
        }).collect()
    }

    fn seed0() -> u32 { std::env::var("SEED").map(|v| v.parse().unwrap()).unwrap_or(11) }
    /// Messstand: cargo test --release sstv_rauschen -- --ignored --nocapture
    #[test]
    #[ignore]
    fn sstv_rauschen() {
        let modes = ["Martin M1", "Scottie S1", "Robot 36", "Robot 72", "PD 120", "PD 90"];
        let snrs: Vec<f64> = std::env::var("SNRS").map(|v| v.split(',').map(|x| x.parse().unwrap()).collect())
            .unwrap_or(vec![12.0, 8.0, 5.0, 3.0, 1.0, -1.0]);
        print!("{:11}", "SNR (dB)");
        for s in &snrs { print!(" {s:>15}"); }
        println!();
        let (mut tot, mut nvis, mut ncomp, mut n) = (0.0, 0, 0, 0);
        for name in modes.iter().copied() {
            let m = MODES.iter().find(|m| m.name == name).unwrap();
            print!("{name:11}");
            for (k, &snr) in snrs.iter().enumerate() {
                let df = if k % 2 == 0 { 0.0 } else { 40.0 };
                let (v, c, p) = bench_one(m, snr, df, seed0() + k as u32);
                print!(" {}{} {p:5.1} dB{:>4}", if v { 'V' } else { '-' }, if c { 'K' } else { '-' }, if df != 0.0 { "+40" } else { "" });
                tot += p; nvis += v as usize; ncomp += c as usize; n += 1;
                use std::io::Write; std::io::stdout().flush().ok();
            }
            println!();
        }
        println!("SUMME: VIS {nvis}/{n}, komplett {ncomp}/{n}, mittleres PSNR {:.2} dB", tot / n as f64);
    }

    #[test]
    fn roundtrip_alle_modi() {
        let cases = [(0.0, 0.0, 25.0), (500.0, 0.0, 25.0), (300.0, 0.3, 13.0), (-500.0, 0.5, 10.0)];
        let mut fail = 0;
        for m in MODES.iter() {
            let mut line = format!("{:11}", m.name);
            for &(ppm, noise, min) in &cases {
                let (ok, psnr) = run(m, ppm, noise, 0.0);
                let pass = ok && psnr >= min;
                if !pass { fail += 1; }
                line += &format!("  {} {ppm}ppm σ{noise}: {psnr:.1} dB", if pass { '✓' } else { '✗' });
            }
            println!("{line}");
        }
        assert_eq!(fail, 0);
    }

    #[test]
    fn aussetzer_wieder_einrasten() {
        for name in ["Martin M1", "Scottie S1", "Robot 36", "PD 120", "Wraase SC2-180", "Pasokon P3", "Martin M3"] {
            let m = MODES.iter().find(|m| m.name == name).unwrap();
            for gap in [10.0, -10.0, 40.0] {
                let (ok, psnr) = run(m, 200.0, 0.2, gap);
                println!("{name:11} Lücke {gap} ms: {psnr:.1} dB {ok}");
                assert!(ok && psnr >= 15.0, "{name} Lücke {gap}");
            }
        }
    }

    #[test]
    fn rauschen_keine_fehlstarts() {
        let mut dec = Decoder::new(48000.0);
        let mut rng = Rng(7);
        let mut starts = 0;
        for _ in 0..(120 * 48000 / 4096) {
            let x: Vec<f32> = (0..4096).map(|_| (0.4 * rng.gauss()) as f32).collect();
            dec.push(&x);
            starts += dec.events.drain(..).filter(|e| matches!(e, Event::Start(_))).count();
        }
        assert_eq!(starts, 0);
    }

    /// auch bandbegrenztes Rauschen (wie hinter dem Empfängerfilter) verschiedener Stärke darf nichts auslösen
    #[test]
    fn bandrauschen_keine_fehlstarts() {
        for (k, amp) in [0.05, 0.3, 1.0].into_iter().enumerate() {
            let x = band_noise(100 * 48000, 48000.0, -10.0 * (2.0 * amp * amp / 0.25f64).log10(), 0.5, 100 + k as u32);
            let mut dec = Decoder::new(48000.0);
            let mut starts = 0;
            for c in x.chunks(4096) {
                dec.push(c);
                starts += dec.events.drain(..).filter(|e| matches!(e, Event::Start(_))).count();
            }
            assert_eq!(starts, 0, "Pegel {amp}");
        }
    }

    /// Gegenprobe mit fremden Encodern (z. B. PySSTV):
    /// SSTV_IN=bild.wav SSTV_REF=bild.png cargo test --release fremd_wav -- --ignored --nocapture
    #[test]
    #[ignore]
    fn fremd_wav() {
        let mut rd = hound::WavReader::open(std::env::var("SSTV_IN").unwrap()).unwrap();
        let fs = rd.spec().sample_rate as f64;
        let max = (1i64 << (rd.spec().bits_per_sample - 1)) as f32;
        let x: Vec<f32> = rd.samples::<i32>().map(|s| s.unwrap() as f32 / max).chain(std::iter::repeat_n(0.0, 2 * fs as usize)).collect();
        let mut dec = Decoder::new(fs);
        let mut res = None;
        for c in x.chunks(4096) {
            dec.push(c);
            for e in dec.events.drain(..) { if let Event::End { mode, img, complete } = e { res = Some((mode, img, complete)); } }
        }
        let (m, out, complete) = res.expect("kein Bild");
        let r = image::open(std::env::var("SSTV_REF").unwrap()).unwrap().to_rgb8();
        let r = image::imageops::resize(&r, m.w as u32, m.h as u32, image::imageops::FilterType::Nearest);
        let r = if m.is_bw() { image::DynamicImage::ImageRgb8(r).grayscale().to_rgb8() } else { r };
        let se: f64 = r.as_raw().iter().zip(&out.data).map(|(a, b)| (*a as f64 - *b as f64).powi(2)).sum();
        let psnr = 10.0 * (255.0f64.powi(2) / (se / out.data.len() as f64)).log10();
        println!("ERGEBNIS {} komplett={complete} PSNR={psnr:.1} dB", m.name);
    }
}
