//! Bandspektrum vom Funkgerät (Icom „27 00“): Kurve und Wasserfall, Klick stimmt das Gerät ab.
//! Zwei Ansichten: „Band“ (Festmodus mit eigenen Kanten: ganzes Band bzw. ein verschiebbarer, zoombarer Ausschnitt,
//! steht still beim Abstimmen) und „Mitte“ (Mittenmodus: Gerätefrequenz in der Mitte, Spannweite wählbar).

use super::*;
use eframe::egui::pos2;

const ROWS: usize = 60;
/// Befehlskennungen für die OK/NG-Auswertung
const TAG_EDGES: u8 = 1;
/// Kante, die wir im Funkgerät für die Bandansicht belegen
const EDGE: u8 = 3;
/// Amateurfunkbänder (IARU Region 1)
pub(crate) const BANDS: [(&str, u64, u64); 13] = [
    ("160 m", 1_810_000, 2_000_000), ("80 m", 3_500_000, 3_800_000), ("60 m", 5_351_500, 5_366_500), ("40 m", 7_000_000, 7_200_000),
    ("30 m", 10_100_000, 10_150_000), ("20 m", 14_000_000, 14_350_000), ("17 m", 18_068_000, 18_168_000), ("15 m", 21_000_000, 21_450_000),
    ("12 m", 24_890_000, 24_990_000), ("10 m", 28_000_000, 29_700_000), ("6 m", 50_000_000, 52_000_000),
    ("2 m", 144_000_000, 146_000_000), ("70 cm", 430_000_000, 440_000_000)];
/// Zoomstufen der Bandansicht (Breite in Hz)
const WIDTHS: [u64; 10] = [10_000, 20_000, 50_000, 100_000, 200_000, 500_000, 1_000_000, 2_000_000, 5_000_000, 10_000_000];

fn band_of(f: u64) -> Option<(&'static str, u64, u64)> { BANDS.iter().copied().find(|b| (b.1..=b.2).contains(&f)) }

#[derive(PartialEq, Clone, Copy, Debug)]
enum Want { Center(u64), Fixed(u64, u64) }

pub(crate) struct ScopeView {
    /// Bandansicht statt Mittenansicht
    pub band: bool,
    /// Mittenansicht: Spannweite ± Hz
    pub half_span: u64,
    /// Bandansicht: gewünschter Ausschnitt und Breite (None = ganzes Band)
    win: Option<(u64, u64)>,
    width: Option<u64>,
    /// größte Breite, die das Gerät mit festen Kanten annimmt (bei Ablehnung gelernt)
    max_w: u64,
    /// Gerät kann keine festen Kanten setzen
    no_fixed: bool,
    sent: Option<Want>,
    last_range: (u64, u64),
    /// seit wann das Spektrum angefordert ist (für den Hinweis, wenn nichts kommt)
    since: Option<Instant>,
}

impl Default for ScopeView {
    fn default() -> Self { ScopeView { band: true, half_span: 25_000, win: None, width: None, max_w: 10_000_000, no_fixed: false, sent: None, last_range: (0, 0), since: None } }
}

impl ScopeView {
    /// Breite der Bandansicht im Band (lo, hi)
    fn eff_width(&self, blo: u64, bhi: u64) -> u64 { self.width.unwrap_or(u64::MAX).min(bhi - blo).min(self.max_w) }
    /// Ausschnitt der Breite `w` um `mid`, ins Band geschoben, auf 1 kHz gerundet
    fn place(mid: u64, w: u64, blo: u64, bhi: u64) -> (u64, u64) {
        if w >= bhi - blo { return (blo, bhi); }
        let lo = (mid.saturating_sub(w / 2) / 1000 * 1000).clamp(blo, bhi - w);
        (lo, lo + w)
    }
}

impl App {
    /// je Bild: Spektrum am Gerät ein-/ausschalten, Ansicht einstellen und neue Bilder in den Wasserfall schieben
    pub(crate) fn scope_tick(&mut self) {
        let Some(rig) = &mut self.rig else { return };
        if rig.model.proto != cat::Proto::Civ { return; }
        let model = rig.model.clone();
        if self.scope_want != self.poll.scope_on {
            for c in cat::scope_cmds(&model, self.scope_want) { let _ = rig.send_raw(&c); std::thread::sleep(Duration::from_millis(20)); }
            self.poll.scope_on = self.scope_want;
            self.scope_v.since = Some(Instant::now());
            self.scope_v.sent = None; // nach dem Einschalten Ansicht neu setzen
        }
        if !self.scope_want { return; }
        let v = &mut self.scope_v;
        // abgelehnte Kanten: zu breit -> auf 1 MHz begrenzen, sonst kann das Gerät es nicht
        for tag in std::mem::take(&mut self.poll.cmd_ng) {
            if tag != TAG_EDGES { continue; }
            match v.sent {
                Some(Want::Fixed(lo, hi)) if hi - lo > 1_000_000 => { v.max_w = 1_000_000; v.win = None; }
                _ => { v.no_fixed = true; self.status = "Spektrum: Das Funkgerät nimmt keine festen Kanten an – Mittenansicht".into(); }
            }
            v.sent = None;
        }
        let want = match (v.band && !v.no_fixed && cat::scope_fixed_ok(&model), self.freq_shown.and_then(|f| band_of(f).map(|b| (f, b)))) {
            (true, Some((f, (_, blo, bhi)))) => {
                // neues Band (oder noch nichts gewählt): Ausschnitt um die Gerätefrequenz
                if v.win.is_none_or(|(lo, hi)| lo < blo || hi > bhi) { v.win = Some(ScopeView::place(f, v.eff_width(blo, bhi), blo, bhi)); }
                let (lo, hi) = v.win.unwrap();
                Want::Fixed(lo, hi)
            }
            _ => Want::Center(v.half_span),
        };
        if v.sent != Some(want) {
            match want {
                Want::Fixed(lo, hi) => if let Some(cmds) = cat::scope_fixed_cmds(&model, lo, hi, EDGE) {
                    for (i, c) in cmds.into_iter().enumerate() { self.poll.cmdq.push_back((if i == 0 { TAG_EDGES } else { 0 }, c)); }
                },
                Want::Center(s) => {
                    self.poll.cmdq.push_back((0, cat::scope_mode(&model, false)));
                    self.poll.cmdq.push_back((0, cat::scope_span(&model, s)));
                }
            }
            v.sent = Some(want);
        }
        if self.poll.scope_frames != self.scope_seen {
            self.scope_seen = self.poll.scope_frames;
            if let Some(f) = &self.poll.scope {
                let w = f.data.len().max(1);
                // anderer Ausschnitt: alter Wasserfall passt nicht mehr
                if self.scope_wf.size != [w, ROWS] || v.last_range != (f.lo_hz, f.hi_hz) {
                    self.scope_wf = ColorImage::new([w, ROWS], vec![Color32::BLACK; w * ROWS]);
                    v.last_range = (f.lo_hz, f.hi_hz);
                }
                self.scope_wf.pixels.copy_within(0..w * (ROWS - 1), w);
                for (x, val) in f.data.iter().enumerate() { self.scope_wf.pixels[x] = heat(((*val as u32 * 255) / 160).min(255) as u8); }
                self.scope_dirty = true;
            }
        }
    }

    /// Bedienzeile: Band/Mitte, verschieben, zoomen
    fn ui_scope_bar(&mut self, ui: &mut egui::Ui) {
        let fixed_ok = self.rig.as_ref().is_some_and(|r| cat::scope_fixed_ok(&r.model)) && !self.scope_v.no_fixed;
        let band = self.freq_shown.and_then(band_of);
        let v = &mut self.scope_v;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Spektrum").strong());
            ui.add_enabled_ui(fixed_ok, |ui| {
                // ohne Bandansicht (z. B. IC-7300) gilt die Mittenansicht – dann auch so anzeigen
                if ui.selectable_label(v.band && fixed_ok, "Band").on_hover_text("Ganzes Band bzw. ein Ausschnitt – steht still, wenn du abstimmst. Bandwechsel über die Frequenz oben.")
                    .on_disabled_hover_text("Bandansicht derzeit nur mit dem IC-705").clicked() { v.band = true; }
            });
            if ui.selectable_label(!(v.band && fixed_ok), "Mitte").on_hover_text("Gerätefrequenz immer in der Mitte").clicked() { v.band = false; }
            ui.separator();
            if v.band && fixed_ok {
                let Some((name, blo, bhi)) = band else { ui.label(RichText::new("außerhalb der Amateurfunkbänder – Mittenansicht").small().weak()); return };
                let (lo, hi) = v.win.unwrap_or((blo, bhi));
                let w = hi - lo;
                ui.label(RichText::new(name).strong());
                if ui.add_enabled(lo > blo, egui::Button::new("◀")).on_hover_text("Ausschnitt nach unten schieben").clicked() {
                    v.win = Some(ScopeView::place(lo, w, blo, bhi)); // Mitte auf die bisherige Unterkante: halbe Breite nach unten
                }
                if ui.add_enabled(hi < bhi, egui::Button::new("▶")).on_hover_text("Ausschnitt nach oben schieben").clicked() {
                    v.win = Some(ScopeView::place(hi, w, blo, bhi));
                }
                let max = (bhi - blo).min(v.max_w).max(WIDTHS[0]);
                let zoom = |v: &mut ScopeView, nw: u64| { v.width = Some(nw); v.win = Some(ScopeView::place(lo + w / 2, nw.min(max), blo, bhi)); };
                if ui.add_enabled(w > WIDTHS[0], egui::Button::new("+")).on_hover_text("hineinzoomen").clicked() {
                    let nw = WIDTHS.iter().rev().copied().find(|&x| x < w).unwrap_or(WIDTHS[0]);
                    zoom(v, nw);
                }
                if ui.add_enabled(w < max, egui::Button::new("−")).on_hover_text("herauszoomen").clicked() {
                    let nw = WIDTHS.iter().copied().find(|&x| x > w).unwrap_or(max).min(max);
                    zoom(v, nw);
                }
                if ui.button("VFO").on_hover_text("Ausschnitt um die Gerätefrequenz legen").clicked() {
                    if let Some(f) = self.freq_shown { v.win = Some(ScopeView::place(f, w, blo, bhi)); }
                }
                ui.label(RichText::new(format!("{:.3}–{:.3} MHz", lo as f64 / 1e6, hi as f64 / 1e6)).monospace().small());
                if v.max_w < bhi - blo && w >= v.max_w {
                    ui.label(RichText::new(format!("(Gerät zeigt höchstens {:.0} kHz auf einmal)", v.max_w as f64 / 1000.0)).small().weak());
                }
            } else {
                let i = cat::SCOPE_SPANS.iter().position(|&s| s == v.half_span).unwrap_or(3);
                if ui.add_enabled(i > 0, egui::Button::new("+")).on_hover_text("hineinzoomen").clicked() { v.half_span = cat::SCOPE_SPANS[i - 1]; }
                if ui.add_enabled(i + 1 < cat::SCOPE_SPANS.len(), egui::Button::new("−")).on_hover_text("herauszoomen").clicked() { v.half_span = cat::SCOPE_SPANS[i + 1]; }
                ui.label(RichText::new(format!("±{} kHz", v.half_span as f64 / 1000.0)).monospace().small());
            }
        });
    }

    pub(crate) fn ui_scope(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.scope_want || !self.rig.as_ref().is_some_and(|r| r.model.proto == cat::Proto::Civ) { return; }
        // die Daten kommen laufend über CAT/WLAN: regelmäßig neu zeichnen, nicht nur bei Mausbewegung
        self.repaint_tick(ctx);
        self.ui_scope_bar(ui);
        let Some(f) = self.poll.scope.clone() else {
            ui.label(RichText::new("Spektrum: warte auf Daten vom Funkgerät …").small().weak());
            // über USB schickt Icom das Spektrum nur mit 115200 Bd und „Unlink from [REMOTE]“
            let serial = self.rig.as_ref().is_some_and(|r| !r.is_net());
            if serial && self.scope_v.since.is_some_and(|t| t.elapsed() > Duration::from_secs(4)) {
                ui.label(RichText::new(format!(
                    "Es kommen keine Spektrumdaten. Über USB liefert das Funkgerät das Spektrum nur mit 115200 Bd (verbunden ist mit {} Bd). \
                     Am Gerät: MENU › SET › Connectors › CI-V › CI-V USB Port = „Unlink from [REMOTE]“ und CI-V USB Baud Rate = 115200 (oder Auto). \
                     Außerdem muss das Spektrum am Gerät eingeschaltet sein.", self.baud)).small().color(ORANGE));
                if self.baud != 115200 && ui.button("Hier auf 115200 Bd umstellen und neu verbinden").clicked() {
                    self.baud = 115200;
                    self.connect(); // trennen
                    self.connect(); // mit neuer Baudrate verbinden
                    self.scope_v.since = Some(Instant::now());
                }
            }
            return;
        };
        if self.scope_dirty { set_tex(ctx, &mut self.scope_tex, "scope", self.scope_wf.clone()); self.scope_dirty = false; }
        let w = ui.available_width();
        let (resp, painter) = ui.allocate_painter(egui::vec2(w, 120.0), egui::Sense::click());
        let r = resp.rect;
        let (top, bottom) = r.split_top_bottom_at_y(r.top() + 60.0);
        painter.rect_filled(top, 0.0, Color32::from_gray(10));
        if let Some(t) = &self.scope_tex { painter.image(t.id(), bottom, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
        // Kurve
        let n = f.data.len().max(2);
        let pts: Vec<egui::Pos2> = f.data.iter().enumerate()
            .map(|(i, v)| pos2(top.left() + i as f32 / (n - 1) as f32 * top.width(), top.bottom() - *v as f32 / 160.0 * top.height())).collect();
        painter.add(egui::Shape::line(pts, egui::Stroke::new(1.0_f32, Color32::from_rgb(120, 200, 255))));
        let span = (f.hi_hz - f.lo_hz).max(1) as f32;
        let x_of = |hz: u64| r.left() + (hz.saturating_sub(f.lo_hz)) as f32 / span * r.width();
        let font = egui::FontId::monospace(10.0);
        // eigene Frequenz; außerhalb des Ausschnitts ein Pfeil am Rand
        match self.freq_shown {
            Some(fr) if (f.lo_hz..=f.hi_hz).contains(&fr) => {
                let x = x_of(fr);
                painter.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], egui::Stroke::new(1.5_f32, RED));
            }
            Some(fr) => {
                let (pos, al, t) = if fr < f.lo_hz { (pos2(r.left() + 2.0, r.top() + 16.0), egui::Align2::LEFT_TOP, "◀ VFO") } else { (pos2(r.right() - 2.0, r.top() + 16.0), egui::Align2::RIGHT_TOP, "VFO ▶") };
                painter.text(pos, al, t, font.clone(), RED);
            }
            None => {}
        }
        let lbl = |hz: u64| format!("{:.3}", hz as f64 / 1e6);
        painter.text(pos2(r.left() + 2.0, r.top() + 2.0), egui::Align2::LEFT_TOP, lbl(f.lo_hz), font.clone(), Color32::GRAY);
        painter.text(pos2(r.right() - 2.0, r.top() + 2.0), egui::Align2::RIGHT_TOP, lbl(f.hi_hz), font.clone(), Color32::GRAY);
        let hz_at = |x: f32| f.lo_hz + ((x - r.left()) / r.width() * span).max(0.0) as u64;
        if let Some(p) = resp.hover_pos() {
            painter.line_segment([pos2(p.x, r.top()), pos2(p.x, r.bottom())], egui::Stroke::new(1.0_f32, Color32::from_gray(120)));
            painter.text(pos2(p.x + 4.0, r.top() + 14.0), egui::Align2::LEFT_TOP, format!("{:.4} MHz", hz_at(p.x) as f64 / 1e6), font, Color32::WHITE);
        }
        if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                // auf 100 Hz runden
                let hz = (hz_at(p.x) + 50) / 100 * 100;
                self.tune_to(hz as f64 / 1000.0, None);
            }
        }
        resp.on_hover_text("Spektrum des Funkgeräts – Klick stimmt das Funkgerät auf diese Frequenz ab");
    }
}
