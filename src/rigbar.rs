//! Kopfbereich: Menüleiste (bei Touch ein ☰-Knopf), Ansichten und die Funkgerät-Leiste mit allem,
//! was man im Betrieb anfasst – Betriebsart, Frequenz, Mode, Leistung, Verbindung, Audio, Aufnahme, RX/TX.

use super::*;
use egui::containers::menu::{MenuButton, MenuConfig};

pub(crate) const OPS: [(Op, &str); 6] = [(Op::Sstv, "SSTV"), (Op::Rtty, "RTTY"), (Op::Cw, "CW"), (Op::Aprs, "APRS"), (Op::Pocsag, "POCSAG"), (Op::Packet, "PACKET")];

/// 14230000 -> "14.230.000"
pub(crate) fn fmt_freq(f: u64) -> String { format!("{}.{:03}.{:03}", f / 1_000_000, (f / 1000) % 1000, f % 1000) }

impl App {
    pub(crate) fn ui_top(&mut self, ctx: &egui::Context) {
        if self.icon_tex.is_none() {
            let load = |bytes: &[u8], name: &str| image::load_from_memory(bytes).ok().map(|i| {
                let i = i.to_rgba8();
                ctx.load_texture(name, ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], i.as_raw()), TextureOptions::LINEAR)
            });
            self.icon_tex = load(include_bytes!("../assets/logo/icon-64.png"), "logo_icon");
            self.logo_tex = load(include_bytes!("../assets/logo/logo-dark.png"), "logo_wort");
        }
        let touch = self.touch == Some(true);
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            if !touch {
                egui::MenuBar::new().ui(ui, |ui| {
                    self.ui_menus(ui, ctx);
                    ui.separator();
                    self.ui_views(ui);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label(RichText::new("WAVE").strong());
                        ui.label(RichText::new("RUST").strong().color(ORANGE));
                        ui.add_space(6.0);
                        if let Some(t) = &self.icon_tex { ui.add(egui::Image::new(t).fit_to_exact_size(egui::vec2(18.0, 18.0))); }
                    });
                });
            }
            ui.horizontal_wrapped(|ui| {
                if touch {
                    // ein Knopf statt Menüleiste: mehr Platz für die Funkgerät-Leiste
                    MenuButton::new(RichText::new("☰").size(22.0)).ui(ui, |ui| {
                        ui.set_min_width(220.0);
                        self.ui_menus(ui, ctx);
                    }).0.on_hover_text("Menü");
                    self.ui_views(ui);
                    ui.separator();
                }
                self.ui_rigbar(ui);
            });
        });
    }

    /// Betrieb / Vorlagen / Logbuch
    fn ui_views(&mut self, ui: &mut egui::Ui) {
        for (t, label) in [(Tab::Operate, "Betrieb"), (Tab::Templates, "Vorlagen"), (Tab::Logbook, "Logbuch"), (Tab::Gallery, "Galerie")] {
            if ui.selectable_label(self.tab == t, RichText::new(label).size(15.0)).clicked() { self.tab = t; }
        }
    }

    fn ui_rigbar(&mut self, ui: &mut egui::Ui) {
        let idle = self.tx.is_none() && self.cw_tx.is_none();
        for (i, (o, label)) in OPS.iter().enumerate() {
            let txt = RichText::new(*label).size(15.0).strong();
            let b = if self.op == *o { egui::Button::new(txt.color(Color32::BLACK)).fill(ORANGE) } else { egui::Button::new(txt) };
            let key = match i { 0..=3 => format!("F{}", i + 1), 4 => "F9".into(), _ => "F10".into() };
            if ui.add_enabled(idle, b).on_hover_text(format!("Betriebsart {label} ({key})")).clicked() && self.op != *o { self.switch_op(*o); }
        }
        ui.separator();
        // Frequenz: Antippen öffnet Eingabe, Betriebsart und Bänder
        let txt = match self.freq_shown {
            Some(f) => RichText::new(fmt_freq(f)).monospace().size(19.0).strong(),
            None => RichText::new("––.–––.–––").monospace().size(19.0).weak(),
        };
        MenuButton::from_button(egui::Button::new(txt))
            .config(MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
            .ui(ui, |ui| { ui.set_min_width(300.0); self.ui_freq_menu(ui); })
            .0.on_hover_text("Frequenz eingeben, Mode und Bänder");
        if let Some(m) = self.poll.mode { ui.label(RichText::new(m.0).monospace().size(15.0)); }
        // Leistungsregler und Messwerte sind Gruppen: vorher umbrechen, wenn sie nicht mehr in die Zeile passen (Breite vom letzten Bild)
        self.bar_group(ui, 0, |s, ui| s.ui_power(ui));
        self.bar_group(ui, 1, |s, ui| s.ui_meters(ui));
        self.ui_rig_extras(ui);
        self.ui_voice_bar(ui);
        if ui.selectable_label(self.sat.show, "Sat").on_hover_text("Satelliten und ISS: Überflüge, Richtung, Doppler").clicked() { self.sat.show = !self.sat.show; }
        self.ui_clock(ui);

        // rechte Gruppe: Breite vom letzten Bild merken und mit Abstand nach rechts schieben;
        // reicht der Platz nicht, rutscht sie in die nächste Zeile statt den Rest zu überdecken
        let need = self.right_w + 8.0;
        // im umbrechenden Layout meldet available_width die ganze Zeilenbreite: Rest selbst ausrechnen
        let left = |ui: &egui::Ui| ui.max_rect().right() - ui.cursor().left();
        if left(ui) < need { ui.end_row(); }
        let rest = left(ui) - need;
        if rest > 0.0 { ui.add_space(rest); }
        let r = ui.horizontal(|ui| {
            let x0 = ui.min_rect().left();
            if self.touch == Some(true) && ui.button("⌨").on_hover_text("Bildschirmtastatur einblenden").clicked() {
                touchkbd::show(self.ev_tx.clone());
            }
            // Verbindung zum Funkgerät
            let (ctxt, col) = if self.net_connecting { ("Verbinde …".to_string(), ORANGE) }
                else if let Some(r) = &self.rig { (format!("{}{}", r.model.name, if r.is_net() { " WLAN" } else { "" }), GREEN) }
                else if self.reconnect.is_some() { ("Verbindung verloren".into(), ORANGE) }
                else { ("Funkgerät verbinden".into(), ui.visuals().weak_text_color()) };
            MenuButton::new(RichText::new(ctxt).color(col)).ui(ui, |ui| {
                self.ui_connect_row(ui);
                if ui.button("Einstellungen › Transceiver …").clicked() { self.set_page = Some(SetPage::Transceiver); }
            });
            // Audio
            let (atxt, aon) = if self.net.is_some() { ("Audio WLAN".to_string(), true) }
                else { self.input.as_ref().map_or(("Audio aus".into(), false), |i| (format!("Audio {:.0} kHz", i.fs / 1000.0), true)) };
            let atxt = if self.if_rx && aon { format!("{atxt} · ZF") } else { atxt };
            MenuButton::new(RichText::new(atxt).color(if aon { GREEN } else { ORANGE })).ui(ui, |ui| {
                if self.net.is_some() { ui.label("Audio läuft über WLAN/LAN"); }
                else if ui.button(if self.input.is_some() { "Audio neu starten" } else { "Audio starten" }).clicked() { let c = ui.ctx().clone(); self.start_audio(&c); }
                ui.separator();
                self.ui_if_switch(ui);
                if ui.button("Einstellungen › Audio …").clicked() { self.set_page = Some(SetPage::Audio); }
            });
            // Aufnahme
            if self.recording() {
                let secs = self.rx_sh.rec_n.load(Relaxed) / self.rx_sh.rec_fs.load(Relaxed).max(1) as u64;
                let b = egui::Button::new(RichText::new(format!("⏹ {}:{:02}", secs / 60, secs % 60)).color(Color32::WHITE).strong()).fill(RED);
                if ui.add(b).on_hover_text("Aufnahme beenden und speichern").clicked() { self.toggle_record(); }
                ui.ctx().request_repaint_after(Duration::from_millis(500));
            } else if ui.button("⏺").on_hover_text(format!("Empfangenes Audio als WAV mitschneiden – landet in {}", save_dir().display())).clicked() {
                self.toggle_record();
            }
            if ui.button("📁").on_hover_text(format!("Ordner mit Bildern und Aufnahmen öffnen\n{}", save_dir().display())).clicked() { self.open_save_dir(); }
            let tx = !idle;
            ui.label(RichText::new(if tx { "● TX" } else { "RX" }).monospace().strong().color(if tx { RED } else { ui.visuals().weak_text_color() }));
            // Breite des Inhalts selbst messen: das Rechteck der Gruppe wird am Fensterrand abgeschnitten, und mit
            // der abgeschnittenen Breite bliebe die Gruppe dauerhaft zu weit rechts
            ui.min_rect().right() - x0
        });
        self.right_w = r.inner;
    }

    /// Gruppe der Funkgerät-Leiste: in die nächste Zeile, wenn sie nicht mehr passt
    fn bar_group(&mut self, ui: &mut egui::Ui, k: usize, f: impl FnOnce(&mut Self, &mut egui::Ui)) {
        let left = ui.max_rect().right() - ui.cursor().left();
        if self.bar_ws[k] > 0.0 && left < self.bar_ws[k] + 4.0 && ui.cursor().left() > ui.max_rect().left() + 1.0 { ui.end_row(); }
        let w = ui.horizontal(|ui| {
            let x0 = ui.min_rect().left();
            f(self, ui);
            ui.min_rect().right() - x0
        }).inner;
        self.bar_ws[k] = w;
    }

    /// Schalter: Funkgerät liefert 12-kHz-ZF statt NF
    pub(crate) fn ui_if_switch(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.if_auto, "ZF am Funkgerät automatisch umschalten").on_hover_text(
            "IC-705, IC-7300, IC-9700 per CAT: bei POCSAG, APRS UKW und Packet über FM liefert das Gerät die 12-kHz-ZF (Rustwave demoduliert FM \
             selbst), bei SSTV, RTTY, CW und Kurzwelle NF. Umgeschaltet wird der Ausgang der aktuellen Verbindung (WLAN/LAN oder USB); \
             beim Trennen und Beenden stellt Rustwave wieder NF ein.");
        if self.if_managed() {
            ui.label(RichText::new(format!("ZF ist {} (für diese Betriebsart)", if self.if_rx { "an" } else { "aus" })).small().weak());
            return;
        }
        ui.checkbox(&mut self.if_rx, "ZF-Empfang (12 kHz, FM selbst demodulieren)").on_hover_text(
            "Für Icom-Geräte mit ZF-Ausgabe (IC-705, IC-7300, IC-9700 …): am Gerät SET > Connectors > WLAN bzw. USB AF/IF Output > \
             Output Select = IF. Rustwave demoduliert dann FM selbst – ohne Deemphasis und ohne den Hochpass gegen CTCSS, \
             wie ein SDR. Ideal für POCSAG und 9600-Bd-Packet. Nur für FM; für SSB-Betrieb (SSTV, RTTY, CW auf KW) am Gerät wieder AF wählen.");
    }

    fn ui_freq_menu(&mut self, ui: &mut egui::Ui) {
        let has_cat = self.rig.as_ref().is_some_and(|r| r.model.proto != cat::Proto::None);
        if !has_cat {
            ui.label("Kein Funkgerät mit CAT verbunden.");
            self.ui_connect_row(ui);
            if ui.button("Einstellungen › Transceiver …").clicked() { self.set_page = Some(SetPage::Transceiver); ui.close(); }
            return;
        }
        ui.horizontal(|ui| {
            ui.label("kHz");
            let r = ui.add(egui::TextEdit::singleline(&mut self.freq_khz).desired_width(110.0).font(egui::TextStyle::Monospace));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Setzen").clicked() || enter {
                match self.freq_khz.replace(',', ".").trim().parse::<f64>() {
                    Ok(k) => { self.tune_to(k, None); ui.close(); }
                    Err(_) => self.log("Frequenz in kHz eingeben, z. B. 14230"),
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Mode");
            for (sb, l) in [(cat::Sb::Usb, "USB"), (cat::Sb::Lsb, "LSB"), (cat::Sb::Cw, "CW"), (cat::Sb::Fm, "FM")] {
                if ui.button(l).clicked() { if let Some(r) = &mut self.rig { let _ = r.set_mode(sb); } }
            }
        });
        ui.separator();
        self.ui_band_list(ui);
        ui.separator();
        if ui.button("Frequenz neu lesen").clicked() { self.read_freq(); }
    }

    /// Bänder bzw. Anruffrequenzen der aktuellen Betriebsart
    pub(crate) fn ui_band_list(&mut self, ui: &mut egui::Ui) {
        if self.op == Op::Cw {
            ui.label(RichText::new("CW-Bereich (IARU Region 1)").strong());
            self.ui_cw_bands(ui);
            return;
        }
        if self.op == Op::Pocsag { self.ui_pocsag_bands(ui); return; }
        if self.op == Op::Packet { self.ui_packet_bands(ui); return; }
        ui.label(RichText::new(match self.op { Op::Sstv => "SSTV-Frequenzen (USB)", Op::Rtty => "RTTY-Frequenzen", _ => "APRS-Frequenzen" }).strong());
        egui::Grid::new("qf").show(ui, |ui| {
            // SSTV in USB; RTTY nach Konvention in LSB (mit "Umkehren" in USB); APRS je nach Eintrag
            let f = |l: &[u32], sb| l.iter().map(|&k| (k as f64, sb, None)).collect::<Vec<(f64, cat::Sb, Option<bool>)>>();
            let list = match self.op {
                Op::Rtty => f(&RTTY_KHZ, if self.rtty.reverse { cat::Sb::Usb } else { cat::Sb::Lsb }),
                Op::Aprs => APRS_LIST.iter().map(|&(k, sb, hf)| (k, sb, Some(hf))).collect(),
                _ => f(&QUICK_KHZ, cat::Sb::Usb),
            };
            for (i, (k, sb, hf)) in list.into_iter().enumerate() {
                let label = if self.op == Op::Aprs { format!("{:.4} {sb:?}{}", k / 1000.0, if hf == Some(true) { " KW" } else { "" }).to_uppercase() } else { format!("{:.3}", k / 1000.0) };
                if ui.add_enabled(self.rig.is_some(), egui::Button::new(RichText::new(label).monospace()))
                    .on_hover_text(format!("Frequenz + {sb:?}").to_uppercase()).clicked() {
                    if let Some(hf) = hf { self.aprs.hf = hf; }
                    self.tune_to(k, Some(sb));
                    ui.close();
                }
                if i % 3 == 2 { ui.end_row(); }
            }
        });
    }
}
