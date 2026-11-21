//! Reiter „Betrieb“: QSO-Leiste, SSTV-Empfang, Phasenleiste und SSTV-Senden.

use super::*;

impl App {
    // ================= Reiter „Betrieb“ =================
    pub(crate) fn ui_qso_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let field = |ui: &mut egui::Ui, label: &str, v: &mut String, w: f32, hint: &str| {
                ui.label(label);
                ui.add(egui::TextEdit::singleline(v).desired_width(w).hint_text(hint)).changed()
            };
            ui.label(RichText::new("QSO").strong().color(ORANGE));
            let call_resp = {
                ui.label("Rufzeichen");
                ui.add(egui::TextEdit::singleline(&mut self.qso.call).desired_width(90.0).hint_text("DL1ABC").font(egui::TextStyle::Monospace))
            };
            if call_resp.changed() { self.call_changed(); self.logged = false; }
            let mut ch = false;
            ch |= field(ui, "Name", &mut self.qso.name, 80.0, "");
            ch |= field(ui, "QTH", &mut self.qso.qth, 90.0, "");
            ui.label("RSV gesendet");
            egui::ComboBox::from_id_salt("rsv_s").width(56.0).selected_text(if self.qso.rsv_sent.is_empty() { "—" } else { &self.qso.rsv_sent })
                .show_ui(ui, |ui| {
                    let vals: &[&str] = if matches!(self.op, Op::Rtty | Op::Cw) { &["599", "589", "579", "569", "559", "549", "539"] } else { &["595", "585", "575", "555", "545", "535", "525", "515"] };
                    for v in vals { ch |= ui.selectable_value(&mut self.qso.rsv_sent, v.to_string(), *v).changed(); }
                });
            ch |= field(ui, "empfangen", &mut self.qso.rsv_rcvd, 40.0, if matches!(self.op, Op::Rtty | Op::Cw) { "599" } else { "595" });
            ch |= field(ui, "Notiz", &mut self.qso.note, 110.0, "");
            if ch { self.logged = false; }
            if ui.button("Neues QSO").on_hover_text("Felder leeren und zurück zu CQ").clicked() { self.new_qso(); }
            let log_label = if self.logged { "✔ geloggt" } else { "Ins Log" };
            if ui.add_enabled(!self.qso.call.is_empty(), egui::Button::new(log_label)).on_hover_text("Speichert das QSO (passiert beim Senden der 73-Vorlage automatisch)").clicked() { self.log_qso(); }
        });
        if let Some(k) = &self.known {
            let when = chrono::DateTime::<chrono::Utc>::from_timestamp(k.last, 0).map_or(String::new(), |d| d.format("%d.%m.%Y").to_string());
            ui.label(RichText::new(format!("{} schon {}× gearbeitet, zuletzt am {} – Name/QTH übernommen", self.qso.call, k.count, when)).small().color(GREEN));
        }
    }

    pub(crate) fn ui_rx(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Empfang SSTV");
        ui.add(egui::Label::new(RichText::new(&self.rx_info).monospace().weak()).truncate());
        if self.rx_dirty || self.rx_tex.is_none() {
            set_tex(ctx, &mut self.rx_tex, "rx", self.rx_img.clone());
            self.rx_dirty = false;
        }
        let w = ui.available_width();
        if let Some(t) = &self.rx_tex { ui.add(egui::Image::new(t).fit_to_exact_size(pic_size(ui, w, 0.42))); }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Bild beenden").on_hover_text("laufenden Empfang abbrechen und auf das nächste Bild warten").clicked() { self.rx_sh.ctl.store(CTL_ABORT, Relaxed); }
            ui.menu_button("…", |ui| {
                if ui.button("WAV dekodieren …").clicked() { self.pick_wav(ctx); }
                if ui.button("Bilderordner öffnen").clicked() { self.open_save_dir(); }
            }).response.on_hover_text("weitere Funktionen");
        });
        set_tex(ctx, &mut self.wf_tex, "wf", self.wf.clone());
        if let Some(t) = &self.wf_tex { ui.add(egui::Image::new(t).fit_to_exact_size(egui::vec2(w, 80.0)).maintain_aspect_ratio(false)); }
        let r = ui.horizontal(|ui| ui.allocate_space(egui::vec2(w, 12.0))).inner.1;
        for f in [1200.0, 1500.0, 1900.0, 2300.0] {
            ui.painter().text(egui::pos2(r.left() + f / WF_MAX_HZ * w, r.center().y), egui::Align2::CENTER_CENTER,
                format!("{f}"), egui::FontId::monospace(10.0), ui.visuals().weak_text_color());
        }
        ui.add(egui::ProgressBar::new((self.level * 2.0).min(1.0)).desired_height(6.0));
        let mut clicked = None;
        egui::ScrollArea::vertical().id_salt("gal").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (i, g) in self.gallery.iter().enumerate() {
                    let sel = self.gallery_sel == Some(i);
                    let resp = ui.add(egui::Image::new(&g.tex).fit_to_exact_size(egui::vec2(96.0, 77.0)).sense(egui::Sense::click()))
                        .on_hover_text(format!("{}\n{}\nKlick: als „Bild im Bild“ für deine Antwort · Rechtsklick: Datei öffnen", g.label, g.path.display()));
                    if sel { ui.painter().rect_stroke(resp.rect.expand(2.0), 2.0, egui::Stroke::new(2.0_f32, ORANGE), egui::StrokeKind::Outside); }
                    if resp.clicked() { clicked = Some(i); }
                    if resp.secondary_clicked() { open_path(&g.path); }
                }
            });
        });
        if let Some(i) = clicked {
            if let Ok(img) = image::open(&self.gallery[i].path) {
                self.received = Some(img.to_rgb8());
                self.received_ver += 1;
                self.gallery_sel = Some(i);
            }
        }
    }

    /// Phasen-Leiste CQ › Antwort › Rapport › 73 (SSTV: Vorlage je Phase, RTTY: Makro je Phase)
    pub(crate) fn ui_phase_bar(&mut self, ui: &mut egui::Ui) {
        let busy = self.tx.is_some() || self.cw_tx.is_some();
        ui.horizontal(|ui| {
            for (i, p) in template::Phase::ALL.iter().enumerate() {
                if i > 0 { ui.label(RichText::new("›").weak()); }
                let text_mode = matches!(self.op, Op::Rtty | Op::Cw); // Makros statt Bildvorlagen
                let has = text_mode || self.templates.iter().any(|t| t.phase == Some(*p));
                let active = self.phase == *p && (text_mode || self.templates[self.sel_tpl].phase == Some(*p));
                let txt = RichText::new(p.label()).strong().size(15.0);
                let b = if active { egui::Button::new(txt.color(Color32::BLACK)).fill(ORANGE) } else { egui::Button::new(txt) };
                if ui.add_enabled(has && !busy, b.min_size(egui::vec2(70.0, 26.0)))
                    .on_hover_text(if has { "Vorlage dieser Phase wählen" } else { "Keine Vorlage für diese Phase – im Reiter Vorlagen zuweisen" }).clicked() {
                    self.select_phase(*p);
                }
            }
        });
    }

    pub(crate) fn ui_tx(&mut self, ui: &mut egui::Ui) {
        let busy = self.tx.is_some();
        self.ui_phase_bar(ui);
        let w = ui.available_width();
        if let Some(t) = &self.tx_tex { ui.add(egui::Image::new(t).fit_to_exact_size(pic_size(ui, w, 0.42))).on_hover_text("Sendebild – ein Foto ins Fenster ziehen, um es zu verwenden"); }
        // kleine Bildschirme: Senden direkt unter die Vorschau, damit es ohne Rollen erreichbar ist
        let r = ui.ctx().content_rect();
        let send_first = r.width() < 1150.0 || r.height() < 700.0;
        if send_first { self.ui_send_row(ui); }
        // freie Ablage
        let mut pick = None;
        egui::ScrollArea::horizontal().id_salt("strip").show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, (t, tex)) in self.templates.iter().zip(&self.thumbs).enumerate() {
                    ui.vertical(|ui| {
                        let resp = ui.add_enabled(!busy, egui::Image::new(tex).fit_to_exact_size(egui::vec2(80.0, 64.0)).sense(egui::Sense::click()));
                        if i == self.sel_tpl { ui.painter().rect_stroke(resp.rect.expand(2.0), 2.0, egui::Stroke::new(2.0_f32, ORANGE), egui::StrokeKind::Outside); }
                        if resp.clicked() { pick = Some(i); }
                        let tag = t.phase.map_or(String::new(), |p| format!("[{}] ", p.label()));
                        ui.add(egui::Label::new(RichText::new(format!("{tag}{}", t.name)).small()).truncate());
                    });
                }
            });
        });
        if let Some(i) = pick { self.select_template(i); }
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal(|ui| {
                let m = &sstv::MODES[self.tx_mode];
                egui::ComboBox::from_id_salt("mode").width(200.0).selected_text(format!("{} — {}×{}, {:.0} s", m.name, m.w, m.h, m.seconds()))
                    .show_ui(ui, |ui| {
                        for (i, md) in sstv::MODES.iter().enumerate() {
                            ui.selectable_value(&mut self.tx_mode, i, format!("{} — {}×{}, {:.0} s", md.name, md.w, md.h, md.seconds()));
                        }
                    });
                #[cfg(windows)]
                if ui.button("Foto wählen").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Bilder", &["png", "jpg", "jpeg", "bmp"]).pick_file() { self.load_photo(&p); }
                }
                if ui.button("Vorlage bearbeiten").clicked() { self.tab = Tab::Templates; }
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.fskid_tx, "FSKID senden").on_hover_text("hängt das eigene Rufzeichen als FSK-Kennung an (wie MMSSTV) – die Gegenstation sieht es automatisch");
        });
        if !send_first { self.ui_send_row(ui); }
        self.ui_gain(ui);
    }

    /// Senden/Stopp und Abstimmton mit Fortschritt
    pub(crate) fn ui_send_row(&mut self, ui: &mut egui::Ui) {
        let busy = self.tx.is_some();
        ui.horizontal(|ui| {
            let kind = self.tx.as_ref().map(|t| t.kind);
            let red = |t: &str| egui::Button::new(RichText::new(t).color(Color32::WHITE).strong()).fill(RED);
            let send = if kind == Some(TxKind::Image) { red("Stopp") } else {
                egui::Button::new(RichText::new(format!("Senden: {}", self.templates[self.sel_tpl].name)).color(Color32::BLACK).strong()).fill(ORANGE)
            };
            if ui.add_enabled_ui(kind.is_none() || kind == Some(TxKind::Image), |ui| ui.add_sized([200.0, 26.0], send)).inner.clicked() {
                if busy { self.stop_tx() } else { self.start_tx(TxKind::Image) }
            }
            let tune = if kind == Some(TxKind::Tune) { red("Ton aus") } else { egui::Button::new("Abstimmton 1750 Hz") };
            if ui.add_enabled_ui(kind.is_none() || kind == Some(TxKind::Tune), |ui| ui.add_sized([150.0, 26.0], tune)).inner.clicked() {
                if busy { self.stop_tx() } else { self.start_tx(TxKind::Tune) }
            }
        });
        let p = self.tx.as_ref().map_or(0.0, |t| t.state.progress());
        ui.add(egui::ProgressBar::new(p).desired_height(6.0));
    }
}
