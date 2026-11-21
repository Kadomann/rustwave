//! RTTY-Oberfläche im Reiter „Betrieb“: Empfangsfenster, Wasserfall zum Abstimmen, Makros, freier Text.

use crate::template::{self, Phase};
use crate::{App, TxKind, GREEN, ORANGE, WF_MAX_HZ};
use eframe::egui::{self, pos2, vec2, Color32, RichText, Sense, Stroke};

/// Standardmakros je Phase (CQ, Antwort, Rapport, 73) – Makros wie in den SSTV-Vorlagen
pub const DEFAULT_MACROS: [&str; 4] = [
    "CQ CQ CQ DE %m %m %m\nCQ CQ CQ DE %m %m %m PSE K",
    "%c %c DE %m %m %m K",
    "%c DE %m\n%g %n, TNX FER CALL. UR RST %r %r\nNAME %o %o QTH %a %a LOC %l\nHW? %c DE %m K",
    "%c DE %m\nTNX FER QSO %n, 73 ES GL\n%c DE %m SK",
];
const MAX_LINES: usize = 400;

/// Sieht das Wort wie ein Rufzeichen aus? (Buchstaben und Ziffern gemischt, 3–10 Zeichen, optional /P)
pub fn is_call(w: &str) -> bool {
    let w = w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '/');
    let core = w.split('/').max_by_key(|p| p.len()).unwrap_or("");
    (3..=10).contains(&core.len()) && core.chars().all(|c| c.is_ascii_alphanumeric())
        && core.chars().any(|c| c.is_ascii_digit()) && core.chars().any(|c| c.is_ascii_alphabetic())
        && !is_locator(core)
}
/// Maidenhead-Locator (JO62 / JO62QM) – sieht aus wie ein Rufzeichen, ist aber keins
fn is_locator(w: &str) -> bool {
    let b = w.as_bytes();
    (b.len() == 4 || b.len() == 6)
        && b[..2].iter().all(|c| (b'A'..=b'R').contains(&c.to_ascii_uppercase()))
        && b[2..4].iter().all(|c| c.is_ascii_digit())
        && b[4..].iter().all(|c| (b'A'..=b'X').contains(&c.to_ascii_uppercase()))
}

/// Textfenster mit anklickbaren Rufzeichen (eigene Aussendungen orange); liefert das angeklickte Wort
pub fn terminal(ui: &mut egui::Ui, lines: &[(String, bool)], id: &str, h: f32) -> Option<String> {
    let mut picked = None;
    egui::Frame::new().fill(Color32::from_gray(12)).inner_margin(6.0).corner_radius(4.0).show(ui, |ui| {
        ui.set_min_height(h);
        egui::ScrollArea::vertical().id_salt(id).stick_to_bottom(true).max_height(h).auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
            for (line, tx) in lines {
                ui.horizontal_wrapped(|ui| {
                    let color = if *tx { ORANGE } else { Color32::from_gray(225) };
                    for (k, w) in line.split(' ').enumerate() {
                        if k > 0 { ui.label(RichText::new(" ").monospace()); }
                        if w.is_empty() { continue; }
                        if !*tx && is_call(w) {
                            let r = ui.add(egui::Label::new(RichText::new(w).monospace().color(color).underline()).sense(Sense::click()))
                                .on_hover_text("als Rufzeichen übernehmen");
                            if r.clicked() { picked = Some(w.to_string()); }
                        } else {
                            ui.label(RichText::new(w).monospace().color(color));
                        }
                    }
                    if line.is_empty() { ui.label(RichText::new(" ").monospace()); }
                });
            }
        });
    });
    picked
}

fn phase_idx(p: Phase) -> usize { Phase::ALL.iter().position(|&x| x == p).unwrap_or(0) }

impl App {
    /// angeklicktes Rufzeichen in die QSO-Leiste übernehmen
    pub(crate) fn pick_call(&mut self, w: &str) {
        self.qso.call = w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '/').to_string();
        self.call_changed();
        self.logged = false;
    }
    /// Text ins Empfangsfenster schreiben (tx = selbst gesendet)
    pub(crate) fn rtty_rx(&mut self, t: &str, tx: bool) {
        for (i, part) in t.split('\n').enumerate() {
            if i > 0 || self.rtty_lines.last().is_none_or(|l| l.1 != tx) { self.rtty_lines.push((String::new(), tx)); }
            self.rtty_lines.last_mut().unwrap().0.push_str(part);
        }
        let n = self.rtty_lines.len();
        if n > MAX_LINES { self.rtty_lines.drain(..n - MAX_LINES); }
    }
    /// Gesendeten Text im Empfangsfenster zeigen und danach in einer neuen Empfangszeile weiterschreiben
    pub(crate) fn rtty_echo(&mut self, t: &str) {
        self.rtty_rx(t.trim_end(), true);
        self.rtty_lines.push((String::new(), false));
    }
    pub(crate) fn rtty_send(&mut self, text: String, phase: Option<Phase>) {
        if text.trim().is_empty() { self.log("Nichts zu senden"); return; }
        self.rtty_out = Some((template::expand(&text, &self.macros(false)), phase));
        self.start_tx(TxKind::Rtty);
    }

    pub(crate) fn ui_rtty_rx(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let q = f32::from_bits(self.rx_sh.rtty_quality.load(std::sync::atomic::Ordering::Relaxed));
        ui.horizontal(|ui| {
            ui.heading("Empfang RTTY");
            ui.add(egui::ProgressBar::new(q.clamp(0.0, 1.0)).desired_width(90.0).desired_height(10.0)
                .fill(if q >= self.rtty.squelch { GREEN } else { Color32::from_gray(90) }))
                .on_hover_text("Signalgüte – über der Rauschsperre wird Text angezeigt");
            ui.menu_button("…", |ui| {
                if ui.button("Empfangsfenster leeren").clicked() { self.rtty_lines = vec![(String::new(), false)]; }
                if ui.button("WAV dekodieren …").clicked() { self.pick_wav(ctx); }
            }).response.on_hover_text("weitere Funktionen");
        }).response.on_hover_text("Klick auf ein Rufzeichen übernimmt es in die QSO-Leiste · eigene Aussendungen orange");
        let h = (ui.ctx().content_rect().height() * 0.38).max(150.0);
        let picked = terminal(ui, &self.rtty_lines, "rtty_rx", h);
        if let Some(w) = picked { self.pick_call(&w); }

        // Wasserfall mit Mark/Space-Markierung, Klick = Abstimmen auf die Mitte
        crate::set_tex(ctx, &mut self.wf_tex, "wf", self.wf.clone());
        let w = ui.available_width();
        let (resp, painter) = ui.allocate_painter(vec2(w, 120.0), Sense::click_and_drag());
        let r = resp.rect;
        if let Some(t) = &self.wf_tex { painter.image(t.id(), r, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
        let x_of = |f: f64| r.left() + (f as f32 / WF_MAX_HZ) * r.width();
        for (f, c, label) in [(self.rtty.mark(), GREEN, "M"), (self.rtty.space(), ORANGE, "S")] {
            painter.line_segment([pos2(x_of(f), r.top()), pos2(x_of(f), r.bottom())], Stroke::new(1.5_f32, c));
            painter.text(pos2(x_of(f), r.top() + 2.0), egui::Align2::CENTER_TOP, label, egui::FontId::monospace(11.0), c);
        }
        if resp.clicked() || resp.dragged() {
            if let Some(p) = resp.interact_pointer_pos() {
                let f = ((p.x - r.left()) / r.width() * WF_MAX_HZ) as f64;
                self.rtty.center = (f.clamp(300.0, 2900.0) / 5.0).round() * 5.0;
            }
        }
        resp.on_hover_text("Klicken oder ziehen: auf die Mitte zwischen den beiden Spuren abstimmen");
        ui.add(egui::ProgressBar::new((self.level * 2.0).min(1.0)).desired_height(6.0));

        // Parameter
        ui.horizontal_wrapped(|ui| {
            ui.label("Baud");
            egui::ComboBox::from_id_salt("baud_r").width(60.0).selected_text(format!("{}", self.rtty.baud)).show_ui(ui, |ui| {
                for b in [45.45, 50.0, 75.0] { ui.selectable_value(&mut self.rtty.baud, b, format!("{b}")); }
            });
            ui.label("Shift");
            egui::ComboBox::from_id_salt("shift_r").width(60.0).selected_text(format!("{} Hz", self.rtty.shift)).show_ui(ui, |ui| {
                for s in [170.0, 200.0, 425.0, 850.0] { ui.selectable_value(&mut self.rtty.shift, s, format!("{s} Hz")); }
            });
            ui.label("Mitte");
            ui.add(egui::DragValue::new(&mut self.rtty.center).range(300.0..=2900.0).speed(1.0).suffix(" Hz"));
            ui.checkbox(&mut self.rtty.reverse, "Umkehren").on_hover_text("Mark/Space tauschen – nötig, wenn in USB statt LSB gearbeitet wird");
        });
        ui.horizontal(|ui| {
            ui.label("Rauschsperre");
            ui.add(egui::Slider::new(&mut self.rtty.squelch, 0.2..=0.8).fixed_decimals(2))
                .on_hover_text("niedriger: schwächere Signale lesbar, aber mehr Zufallszeichen bei Rauschen");
        });
    }

    pub(crate) fn ui_rtty_tx(&mut self, ui: &mut egui::Ui) {
        let busy = self.tx.is_some();
        self.ui_phase_bar(ui);
        let i = phase_idx(self.phase);
        // feste Höhen: der Senden-Knopf bleibt in jeder Phase an derselben Stelle
        const BOX_H: f32 = 78.0;
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Makro „{}“", self.phase.label())).small().weak())
                .on_hover_text("Platzhalter wie %c (Rufzeichen der Gegenstation), %n, %r – Liste im Reiter Vorlagen");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!busy, |ui| ui.menu_button("…", |ui| {
                    if ui.button("Makro zurücksetzen").on_hover_text("Standardtext dieses Makros wiederherstellen").clicked() { self.rtty_macros[i] = DEFAULT_MACROS[i].to_string(); }
                }));
            });
        });
        egui::ScrollArea::vertical().id_salt("m_edit").max_height(BOX_H).show(ui, |ui| {
            ui.add_enabled(!busy, egui::TextEdit::multiline(&mut self.rtty_macros[i]).font(egui::TextStyle::Monospace)
                .desired_rows(4).desired_width(f32::INFINITY));
        });
        let preview = template::expand(&self.rtty_macros[i], &self.macros(false));
        egui::Frame::new().fill(Color32::from_gray(20)).inner_margin(6.0).corner_radius(4.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(BOX_H);
            egui::ScrollArea::vertical().id_salt("m_prev").max_height(BOX_H).show(ui, |ui| {
                ui.label(RichText::new(preview.to_uppercase()).monospace().color(ORANGE));
            });
        });
        ui.horizontal(|ui| {
            let kind = self.tx.as_ref().map(|t| t.kind);
            let red = |t: &str| egui::Button::new(RichText::new(t).color(Color32::WHITE).strong()).fill(crate::RED);
            let send = if kind == Some(TxKind::Rtty) { red("Stopp") } else {
                egui::Button::new(RichText::new(format!("Senden: {}", self.phase.label())).color(Color32::BLACK).strong()).fill(ORANGE)
            };
            if ui.add_enabled_ui(kind.is_none() || kind == Some(TxKind::Rtty), |ui| ui.add_sized([200.0, 26.0], send)).inner.clicked() {
                if busy { self.stop_tx() } else { let (m, p) = (self.rtty_macros[i].clone(), self.phase); self.rtty_send(m, Some(p)); }
            }
            let tune = if kind == Some(TxKind::Tune) { red("Ton aus") } else { egui::Button::new("Abstimmton 1750 Hz") };
            if ui.add_enabled_ui(kind.is_none() || kind == Some(TxKind::Tune), |ui| ui.add_sized([150.0, 26.0], tune)).inner.clicked() {
                if busy { self.stop_tx() } else { self.start_tx(TxKind::Tune) }
            }
        });
        let p = self.tx.as_ref().map_or(0.0, |t| t.state.progress());
        ui.add(egui::ProgressBar::new(p).desired_height(6.0));
        self.ui_gain(ui).on_hover_text("wie bei SSTV: so einstellen, dass die ALC gerade nicht ausschlägt");
        ui.add_space(8.0);
        ui.separator();
        ui.label(RichText::new("Freier Text").strong());
        ui.add_enabled(!busy, egui::TextEdit::multiline(&mut self.rtty_input).font(egui::TextStyle::Monospace)
            .desired_rows(3).desired_width(f32::INFINITY).hint_text("Text tippen … Makros wie %c funktionieren auch hier"));
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy && !self.rtty_input.trim().is_empty(), egui::Button::new("Text senden")).clicked() {
                let t = std::mem::take(&mut self.rtty_input);
                self.rtty_send(t, None);
            }
            if ui.add_enabled(!busy, egui::Button::new("Leeren")).clicked() { self.rtty_input.clear(); }
        });
        ui.add_space(6.0);
        ui.label(RichText::new(format!("Mark {:.0} Hz · Space {:.0} Hz · {} · Funkgerät auf {}", self.rtty.mark(), self.rtty.space(),
            if self.rtty.reverse { "umgekehrt" } else { "normal" }, if self.rtty.reverse { "USB" } else { "LSB" })).small().weak());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rufzeichen_erkennen() {
        for c in ["DL1ABC", "dk0xy", "F4XYZ,", "OE/DL1ABC", "DL1ABC/P", "9A1AA", "2E0XYZ"] { assert!(is_call(c), "{c}"); }
        for w in ["599", "CQ", "DE", "K", "QTH", "JO62QM.", "JO62", "14.085", "73"] { assert!(!is_call(w), "{w}"); }
    }
}
