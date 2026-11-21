//! CW-Oberfläche: Empfangsfenster, Wasserfall zum Abstimmen, Bandvorlagen für den CW-Bereich jedes Bandes.

use crate::rtty_ui::terminal;
use crate::template::{self, Phase};
use crate::{cat, cw, App, GREEN, ORANGE, RED, WF_MAX_HZ};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use eframe::egui::{self, pos2, vec2, Color32, RichText, Sense, Stroke};
use std::sync::atomic::Ordering::Relaxed;

/// Bandvorlagen: Anfang des CW-Bereichs nach IARU-Region-1-Bandplan (kHz) und Ende des CW-Teils
pub const CW_BANDS: [(&str, f64, f64); 12] = [
    ("160 m", 1810.0, 1838.0), ("80 m", 3500.0, 3570.0), ("60 m", 5351.5, 5354.0), ("40 m", 7000.0, 7040.0),
    ("30 m", 10100.0, 10130.0), ("20 m", 14000.0, 14070.0), ("17 m", 18068.0, 18095.0), ("15 m", 21000.0, 21070.0),
    ("12 m", 24890.0, 24915.0), ("10 m", 28000.0, 28070.0), ("6 m", 50000.0, 50100.0), ("2 m", 144000.0, 144150.0),
];
const MAX_LINES: usize = 400;

/// Standardmakros je Phase (CQ, Antwort, Rapport, 73); „^SK“ = Schlusszeichen ohne Zeichenpause
pub const DEFAULT_MACROS: [&str; 4] = [
    "CQ CQ CQ DE %m %m %m PSE K",
    "%c DE %m %m K",
    "%c DE %m %g TNX FER CALL UR RST %r %r NAME %o %o QTH %a %a HW? %c DE %m K",
    "%c DE %m TNX FER QSO %n 73 ES GL %c DE %m ^SK",
];

/// laufende CW-Aussendung über das Funkgerät (in Stücken zu 30 Zeichen)
pub struct CwTx { chunks: VecDeque<String>, next_at: Instant, end_at: Instant, phase: Option<Phase>, total: usize, done: usize }

impl App {
    pub(crate) fn cw_can_send(&self) -> bool { self.rig.as_ref().is_some_and(|r| r.model.proto == cat::Proto::Civ) }

    /// Text über das Funkgerät als CW geben (Icom CI-V 17); das Gerät tastet selbst
    pub(crate) fn cw_send(&mut self, text: &str, phase: Option<Phase>) {
        let t = cw::sanitize(&template::expand(text, &self.macros(false)));
        if t.is_empty() { self.log("Nichts zu senden"); return; }
        let wpm = self.cw_wpm;
        let Some(rig) = &mut self.rig else { return };
        // Tempo setzen und Break-in einschalten (Semi), sonst sendet das Gerät den Text nicht
        let (sp, bk) = (cat::cw_speed(&rig.model, wpm), cat::cw_breakin(&rig.model, 1));
        if let Err(e) = rig.send_raw(&sp).and_then(|_| { std::thread::sleep(Duration::from_millis(30)); rig.send_raw(&bk) }) {
            self.log(format!("CAT-Fehler: {e}")); return;
        }
        let chunks: VecDeque<String> = cw::chunks(&t, 30).into();
        self.log(format!("Sende CW mit {wpm} WpM (Break-in am Funkgerät auf Semi gestellt) …"));
        let now = Instant::now() + Duration::from_millis(60);
        self.cw_tx = Some(CwTx { total: chunks.len(), done: 0, chunks, next_at: now, end_at: now, phase });
        self.rx_sh.paused.store(true, std::sync::atomic::Ordering::Relaxed); // eigenes Signal nicht dekodieren
        self.cw_lines.push((t.replace('^', ""), true));
        self.cw_lines.push((String::new(), false));
    }
    pub(crate) fn cw_stop(&mut self) {
        if let Some(r) = &mut self.rig { let b = cat::cw_stop(&r.model); let _ = r.send_raw(&b); }
        self.cw_tx = None;
        self.rx_sh.paused.store(false, std::sync::atomic::Ordering::Relaxed);
        self.log("CW abgebrochen");
    }
    /// je Bild: nächstes Stück senden, wenn das vorige (rechnerisch) fertig ist
    pub(crate) fn cw_tick(&mut self, ctx: &egui::Context) {
        let Some(t) = &mut self.cw_tx else { return };
        ctx.request_repaint_after(Duration::from_millis(100));
        let now = Instant::now();
        if now < t.next_at { return; }
        if let Some(c) = t.chunks.pop_front() {
            let dur = cw::duration_s(&c, self.cw_wpm as f64);
            // kleine Reserve: lieber eine etwas längere Wortpause als ein abgewiesener Befehl
            t.next_at = now + Duration::from_secs_f64(dur + 7.0 * 1.2 / self.cw_wpm as f64 + 0.1);
            t.end_at = t.next_at;
            t.done += 1;
            let Some(rig) = &mut self.rig else { self.cw_tx = None; return };
            let b = cat::cw_text(&rig.model, &c);
            if let Err(e) = rig.send_raw(&b) { self.log(format!("CAT-Fehler: {e}")); self.cw_tx = None; }
            return;
        }
        if now >= t.end_at {
            let phase = t.phase;
            self.cw_tx = None;
            self.rx_sh.paused.store(false, std::sync::atomic::Ordering::Relaxed);
            self.log("CW gesendet");
            match phase {
                Some(Phase::Final) if !self.qso.call.is_empty() => self.log_qso(),
                Some(p) => self.select_phase(p.next()),
                None => {}
            }
        }
    }

    pub(crate) fn cw_rx(&mut self, t: &str) {
        if self.cw_lines.is_empty() { self.cw_lines.push((String::new(), false)); }
        for c in t.chars() {
            let line = &mut self.cw_lines.last_mut().unwrap().0;
            line.push(c);
            // lange Zeilen am Wortende umbrechen
            if c == ' ' && line.chars().count() > 60 { self.cw_lines.push((String::new(), false)); }
        }
        let n = self.cw_lines.len();
        if n > MAX_LINES { self.cw_lines.drain(..n - MAX_LINES); }
    }

    /// Bandvorlagen: Frequenz und Betriebsart CW einstellen
    pub(crate) fn ui_cw_bands(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("cw_bands").spacing([6.0, 4.0]).show(ui, |ui| {
            for (i, (band, start, end)) in CW_BANDS.iter().enumerate() {
                let b = egui::Button::new(RichText::new(format!("{band:>5} {:.4}", start / 1000.0)).monospace());
                if ui.add_enabled(self.rig.is_some(), b)
                    .on_hover_text(format!("CW-Bereich {start:.1}–{end:.1} kHz (IARU Region 1) – stellt Frequenz und CW ein"))
                    .on_disabled_hover_text("Zuerst das Funkgerät verbinden").clicked() {
                    self.tune_to(*start, Some(cat::Sb::Cw));
                    ui.close();
                }
                if i % 3 == 2 { ui.end_row(); }
            }
        });
    }

    /// Frequenz eines Signals, das bei der Tonhöhe `pitch` zu hören ist (aus Gerätefrequenz und Betriebsart)
    pub(crate) fn rf_of(&self, pitch: f64) -> Option<u64> {
        let dial = self.freq_shown? as f64;
        let off = match self.poll.mode?.0 {
            "CW" | "CW-U" => pitch - self.cw.pitch,
            "CW-R" | "CW-L" => self.cw.pitch - pitch,
            "USB" | "DATA-U" => pitch,
            "LSB" | "DATA-L" => -pitch,
            _ => return None,
        };
        Some((dial + off).round() as u64)
    }

    /// Skimmer: alle CW-Signale im Durchlass; Zeile antippen = Dekoder darauf, „QSY“ = Funkgerät dorthin
    fn ui_skimmer(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.skim_on, RichText::new("Skimmer").strong())
                .on_hover_text("dekodiert alle CW-Signale im Durchlassbereich gleichzeitig (am besten mit breitem Filter, z. B. 2,4 kHz)");
            if self.skim_on { ui.label(RichText::new(format!("{} Signale", self.skim.len())).small().weak()); }
        });
        if !self.skim_on { return; }
        let rows = self.skim.clone();
        let mut action: Option<(f64, Option<u64>, bool)> = None; // Tonhöhe, Frequenz, QSY?
        let mut take_call = None;
        egui::ScrollArea::vertical().id_salt("skim").max_height(150.0).auto_shrink([false, true]).show(ui, |ui| {
            egui::Grid::new("skim_g").striped(true).num_columns(5).spacing([8.0, 2.0]).show(ui, |ui| {
                for r in &rows {
                    let rf = self.rf_of(r.pitch);
                    let f = rf.map_or(format!("{:+.0} Hz", r.pitch - self.cw.pitch), |f| format!("{:.2}", f as f64 / 1000.0));
                    let sel = (r.pitch - self.cw.pitch).abs() < 15.0;
                    if ui.selectable_label(sel, RichText::new(f).monospace()).on_hover_text("Dekoder oben auf dieses Signal").clicked() { action = Some((r.pitch, rf, false)); }
                    ui.label(RichText::new(format!("{:.0} WpM", r.wpm)).monospace().weak()).on_hover_text(format!("{:.0} dB über dem Rauschen", r.snr));
                    match &r.call {
                        Some(c) => if ui.small_button(RichText::new(c).monospace().color(GREEN)).on_hover_text("in die QSO-Leiste").clicked() { take_call = Some(c.clone()); },
                        None => { ui.label(""); }
                    }
                    if ui.add_enabled(rf.is_some(), egui::Button::new("QSY").small()).on_hover_text("Funkgerät auf dieses Signal abstimmen").clicked() { action = Some((r.pitch, rf, true)); }
                    let tail: String = r.text.chars().rev().take(48).collect::<Vec<_>>().into_iter().rev().collect();
                    ui.add(egui::Label::new(RichText::new(tail).monospace().size(12.0)).truncate());
                    ui.end_row();
                }
            });
            if rows.is_empty() { ui.label(RichText::new("noch keine CW-Signale").small().weak()); }
        });
        if let Some(c) = take_call { self.pick_call(&c); }
        match action {
            // QSY: danach liegt das Signal auf der eingestellten Tonhöhe (in CW genau auf der Gerätefrequenz)
            Some((_, Some(f), true)) => {
                let p = self.cw.pitch;
                let dial = match self.poll.mode.map(|m| m.0) { Some("USB" | "DATA-U") => f as f64 - p, Some("LSB" | "DATA-L") => f as f64 + p, _ => f as f64 };
                self.tune_to(dial / 1000.0, None);
            }
            Some((p, _, _)) => self.cw.pitch = p.round(),
            None => {}
        }
    }

    pub(crate) fn ui_cw_rx(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let snr = f32::from_bits(self.rx_sh.cw_snr.load(Relaxed));
        let wpm = f32::from_bits(self.rx_sh.cw_wpm.load(Relaxed));
        let open = snr as f64 >= self.cw.squelch;
        ui.horizontal(|ui| {
            ui.heading("Empfang CW");
            ui.add(egui::ProgressBar::new((snr / 30.0).clamp(0.0, 1.0)).desired_width(90.0).desired_height(10.0)
                .fill(if open { GREEN } else { Color32::from_gray(90) }))
                .on_hover_text(format!("Signal/Rauschen {snr:.0} dB – über der Rauschsperre wird Text angezeigt"));
            ui.label(RichText::new(if open { format!("{wpm:.0} WpM") } else { "–".into() }).monospace().strong())
                .on_hover_text("erkanntes Tempo (Wörter pro Minute)");
            ui.menu_button("…", |ui| {
                if ui.button("Empfangsfenster leeren").clicked() { self.cw_lines = vec![(String::new(), false)]; }
                if ui.button("WAV dekodieren …").clicked() { self.pick_wav(ctx); }
            }).response.on_hover_text("weitere Funktionen");
        }).response.on_hover_text("Klick auf ein Rufzeichen übernimmt es in die QSO-Leiste");
        let h = (ui.ctx().content_rect().height() * 0.38).max(150.0);
        if let Some(w) = terminal(ui, &self.cw_lines, "cw_rx", h) { self.pick_call(&w); }
        self.ui_skimmer(ui);

        // Wasserfall mit Markierung der Tonhöhe, Klick = Abstimmen
        crate::set_tex(ctx, &mut self.wf_tex, "wf", self.wf.clone());
        let w = ui.available_width();
        let (resp, painter) = ui.allocate_painter(vec2(w, 120.0), Sense::click_and_drag());
        let r = resp.rect;
        if let Some(t) = &self.wf_tex { painter.image(t.id(), r, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
        let x = r.left() + (self.cw.pitch as f32 / WF_MAX_HZ) * r.width();
        let half = 50.0 / WF_MAX_HZ * r.width(); // Durchlassbereich ±50 Hz
        painter.rect_filled(egui::Rect::from_min_max(pos2(x - half, r.top()), pos2(x + half, r.bottom())), 0.0, Color32::from_rgba_unmultiplied(255, 160, 0, 40));
        painter.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.5_f32, ORANGE));
        if resp.clicked() || resp.dragged() {
            if let Some(p) = resp.interact_pointer_pos() {
                let f = ((p.x - r.left()) / r.width() * WF_MAX_HZ) as f64;
                self.cw.pitch = f.clamp(200.0, 2500.0).round();
            }
        }
        resp.on_hover_text("Klicken oder ziehen: auf die Spur des CW-Signals abstimmen");
        ui.add(egui::ProgressBar::new((self.level * 2.0).min(1.0)).desired_height(6.0));

        ui.horizontal(|ui| {
            ui.label("Tonhöhe").on_hover_text("muss zum CW-Pitch des Funkgeräts passen (Icom: 600 Hz). Ein schmales CW-Filter hilft bei vollem Band, für den Skimmer das Filter breit lassen.");
            ui.add(egui::DragValue::new(&mut self.cw.pitch).range(200.0..=2500.0).speed(1.0).suffix(" Hz"));
            if ui.button("Stärkstes Signal").on_hover_text("auf das stärkste Signal zwischen 300 und 1200 Hz abstimmen").clicked() {
                if let Some(f) = self.wf_peak(300.0, 1200.0) { self.cw.pitch = f.round(); }
            }
            ui.label("Rauschsperre");
            ui.add(egui::Slider::new(&mut self.cw.squelch, 3.0..=20.0).fixed_decimals(0).suffix(" dB"))
                .on_hover_text("niedriger: schwächere Signale lesbar, aber Zufallszeichen bei Rauschen");
        });
    }

    pub(crate) fn ui_cw_tx(&mut self, ui: &mut egui::Ui) {
        let sending = self.cw_tx.is_some();
        let can = self.cw_can_send();
        self.ui_phase_bar(ui);
        let i = Phase::ALL.iter().position(|&x| x == self.phase).unwrap_or(0);
        const BOX_H: f32 = 70.0;
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Makro „{}“", self.phase.label())).small().weak())
                .on_hover_text("Platzhalter wie %c, %n, %r wie in den Vorlagen; ^SK = Schlusszeichen ohne Zeichenpause");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!sending, |ui| ui.menu_button("…", |ui| {
                    if ui.button("Makro zurücksetzen").clicked() { self.cw_macros[i] = DEFAULT_MACROS[i].to_string(); }
                }));
            });
        });
        egui::ScrollArea::vertical().id_salt("cw_m").max_height(BOX_H).show(ui, |ui| {
            ui.add_enabled(!sending, egui::TextEdit::multiline(&mut self.cw_macros[i]).font(egui::TextStyle::Monospace).desired_rows(3).desired_width(f32::INFINITY));
        });
        let preview = cw::sanitize(&template::expand(&self.cw_macros[i], &self.macros(false)));
        egui::Frame::new().fill(Color32::from_gray(20)).inner_margin(6.0).corner_radius(4.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(BOX_H * 0.7);
            ui.label(RichText::new(&preview).monospace().color(ORANGE));
            ui.label(RichText::new(format!("Dauer etwa {:.0} s", cw::duration_s(&preview, self.cw_wpm as f64))).small().weak());
        });
        ui.horizontal(|ui| {
            let b = if sending { egui::Button::new(RichText::new("Stopp").color(Color32::WHITE).strong()).fill(RED) }
                else { egui::Button::new(RichText::new(format!("Senden: {}", self.phase.label())).color(Color32::BLACK).strong()).fill(ORANGE) };
            let r = ui.add_enabled(can || sending, b.min_size(egui::vec2(200.0, 26.0)))
                .on_disabled_hover_text("CW senden geht über ein per CAT verbundenes Icom-Gerät");
            if r.clicked() { if sending { self.cw_stop() } else { let (m, p) = (self.cw_macros[i].clone(), self.phase); self.cw_send(&m, Some(p)); } }
            ui.label("Tempo");
            ui.add_enabled(!sending, egui::DragValue::new(&mut self.cw_wpm).range(6..=48).suffix(" WpM"));
        });
        let p = self.cw_tx.as_ref().map_or(0.0, |t| t.done as f32 / t.total.max(1) as f32);
        ui.add(egui::ProgressBar::new(p).desired_height(6.0));
        ui.add_space(6.0);
        ui.label(RichText::new("Freier Text").strong());
        ui.add_enabled(!sending, egui::TextEdit::multiline(&mut self.cw_input).font(egui::TextStyle::Monospace).desired_rows(2)
            .desired_width(f32::INFINITY).hint_text("Text tippen … Makros wie %c funktionieren auch hier"));
        ui.horizontal(|ui| {
            if ui.add_enabled(can && !sending && !self.cw_input.trim().is_empty(), egui::Button::new("Text senden")).clicked() {
                let t = std::mem::take(&mut self.cw_input);
                self.cw_send(&t, None);
            }
            if ui.add_enabled(!sending, egui::Button::new("Leeren")).clicked() { self.cw_input.clear(); }
        });
        if !can { ui.label(RichText::new("Zum Senden ein Icom-Gerät per CAT (USB oder WLAN) verbinden. Das Funkgerät tastet selbst; Break-in wird dabei auf Semi gestellt.").small().color(ORANGE)); }
    }
}
