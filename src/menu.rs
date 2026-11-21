//! Menüs (Datei, Bearbeiten, Ansicht, Funkgerät, Einstellungen, Hilfe), Tastenkürzel und Hilfefenster.

use super::*;
use egui::{Key, Modifiers};

/// Tastenkürzel: (Taste, Bedeutung) – auch für Hilfe › Tastenkürzel
const KEYS: [(&str, &str); 9] = [
    ("F1 … F4", "Betriebsart SSTV, RTTY, CW, APRS"),
    ("F9", "Betriebsart POCSAG (Funkruf)"),
    ("F10", "Betriebsart Packet Radio"),
    ("F5 … F8", "CQ, Antwort, Rapport, 73 senden (SSTV-Vorlage bzw. RTTY-/CW-Makro)"),
    ("Strg+Enter", "Senden: SSTV-Bild, freien Text (sonst Makro der Phase), APRS-Nachricht bzw. Bake, Funkruf"),
    ("Esc", "Senden abbrechen, PTT lösen"),
    ("Strg+L", "QSO ins Log"),
    ("Strg+,", "Einstellungen"),
    ("Strg+R", "Aufnahme starten/beenden"),
];

impl App {
    pub(crate) fn ui_menus(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.menu_button("Datei", |ui| {
            if ui.button("WAV dekodieren …").clicked() { self.pick_wav(ctx); }
            #[cfg(windows)]
            if ui.button("Sendefoto wählen …").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("Bilder", &["png", "jpg", "jpeg", "bmp"]).pick_file() { self.load_photo(&p); }
            }
            let rec = if self.recording() { "Aufnahme beenden" } else { "Aufnahme starten" };
            if ui.add(egui::Button::new(rec).shortcut_text("Strg+R")).clicked() { self.toggle_record(); }
            if ui.button("Ordner mit Bildern und Aufnahmen öffnen").clicked() { self.open_save_dir(); }
            ui.separator();
            if ui.button("Logbuch als ADIF exportieren …").clicked() { self.export_adif(); }
            ui.separator();
            if ui.button("Beenden").clicked() { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
        });
        ui.menu_button("Bearbeiten", |ui| {
            if ui.button("Neues QSO").on_hover_text("Felder leeren und zurück zu CQ").clicked() { self.new_qso(); }
            if ui.add_enabled(!self.qso.call.is_empty(), egui::Button::new("QSO ins Log").shortcut_text("Strg+L")).clicked() { self.log_qso(); }
            ui.separator();
            let clear = match self.op { Op::Rtty => "RTTY-Empfang leeren", Op::Cw => "CW-Empfang leeren", Op::Aprs => "APRS-Monitor leeren", Op::Pocsag => "Funkrufe leeren", Op::Packet => "Packet-Monitor leeren", Op::Sstv => "" };
            if !clear.is_empty() && ui.button(clear).clicked() {
                match self.op {
                    Op::Rtty => self.rtty_lines = vec![(String::new(), false)],
                    Op::Cw => self.cw_lines = vec![(String::new(), false)],
                    Op::Pocsag => self.pocsag.msgs.clear(),
                    Op::Packet => self.packet.monitor.clear(),
                    _ => self.aprs.monitor.clear(),
                }
            }
            if ui.button("Vorlagen bearbeiten").clicked() { self.tab = Tab::Templates; }
        });
        ui.menu_button("Ansicht", |ui| {
            for (t, label) in [(Tab::Operate, "Betrieb"), (Tab::Templates, "Vorlagen"), (Tab::Logbook, "Logbuch"), (Tab::Gallery, "SSTV-Galerie")] {
                if ui.radio(self.tab == t, label).clicked() { self.tab = t; }
            }
            ui.separator();
            let civ = self.rig.as_ref().is_some_and(|r| r.model.proto == cat::Proto::Civ);
            ui.add_enabled(civ, egui::Checkbox::new(&mut self.scope_want, "Spektrum vom Funkgerät"))
                .on_disabled_hover_text("Icom-Gerät mit Spektrum (IC-705, IC-7300 …) verbinden");
            ui.checkbox(&mut self.skim_on, "CW-Skimmer");
            ui.separator();
            let mut t = self.touch.unwrap_or(false);
            if ui.checkbox(&mut t, "Touch-Bedienung").changed() { self.touch = Some(t); }
            ui.checkbox(&mut self.power_save, "Energiesparen");
            ui.menu_button("Zoom", |ui| {
                for z in [0.8, 0.9, 1.0, 1.1, 1.25, 1.5] {
                    if ui.radio((self.zoom - z).abs() < 0.01, format!("{:.0} %", z * 100.0)).clicked() { self.zoom = z; ctx.set_zoom_factor(z); }
                }
            });
        });
        ui.menu_button("Funkgerät", |ui| {
            self.ui_connect_row(ui);
            ui.separator();
            ui.menu_button("Bänder", |ui| self.ui_band_list(ui));
            ui.menu_button("Mode", |ui| {
                for (sb, l) in [(cat::Sb::Usb, "USB"), (cat::Sb::Lsb, "LSB"), (cat::Sb::Cw, "CW"), (cat::Sb::Fm, "FM")] {
                    if ui.add_enabled(self.rig.is_some(), egui::Button::new(l)).clicked() { if let Some(r) = &mut self.rig { let _ = r.set_mode(sb); } }
                }
            });
            ui.separator();
            ui.menu_button("Profil laden", |ui| {
                let names: Vec<String> = self.profiles.all.keys().cloned().collect();
                if names.is_empty() { ui.label(RichText::new("noch keine Profile").weak()); }
                for n in names {
                    if ui.radio(n == self.profiles.active, &n).clicked() { self.profile_load(&n, ctx); self.profiles.name = n; }
                }
                ui.separator();
                if ui.button("Profile verwalten …").clicked() { self.set_page = Some(SetPage::Profile); }
            });
            if ui.button("Einstellungen › Transceiver …").clicked() { self.set_page = Some(SetPage::Transceiver); }
        });
        ui.menu_button("Einstellungen", |ui| {
            for (p, label) in [(SetPage::Station, "Station …"), (SetPage::Audio, "Audio …"), (SetPage::Transceiver, "Transceiver …"), (SetPage::Sstv, "SSTV …"),
                               (SetPage::Aprs, "APRS …"), (SetPage::Darstellung, "Darstellung …"), (SetPage::Profile, "Profile …")] {
                let b = egui::Button::new(label).shortcut_text(if p == SetPage::Station { "Strg+," } else { "" });
                if ui.add(b).clicked() { self.set_page = Some(p); }
            }
        });
        ui.menu_button("Hilfe", |ui| {
            if ui.button("Tastenkürzel").clicked() { self.help = Some(Help::Keys); }
            if ui.button("Selbsttest …").on_hover_text("prüft Audio, Funkgerät, WLAN, GPS, Uhr und mehr – mit Rohdaten zum Weitergeben").clicked() { self.tab = Tab::Diag; }
            if ui.button("Über Rustwave").clicked() { self.help = Some(Help::About); }
        });
    }

    /// Tastenkürzel – vor dem Zeichnen ausgewertet, damit Textfelder Strg+Enter nicht als Zeilenumbruch schlucken
    pub(crate) fn shortcuts(&mut self, ctx: &egui::Context) {
        let k = |m: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(m, key));
        let idle = self.tx.is_none() && self.cw_tx.is_none();
        // Esc nur abfangen, wenn gesendet wird – sonst schließt es Menüs und Fenster wie gewohnt
        if !idle && k(Modifiers::NONE, Key::Escape) {
            if self.cw_tx.is_some() { self.cw_stop() } else { self.stop_tx() }
        }
        for (o, key) in [(Op::Sstv, Key::F1), (Op::Rtty, Key::F2), (Op::Cw, Key::F3), (Op::Aprs, Key::F4), (Op::Pocsag, Key::F9), (Op::Packet, Key::F10)] {
            if k(Modifiers::NONE, key) && idle && self.op != o { self.switch_op(o); }
        }
        if k(Modifiers::COMMAND, Key::L) { self.log_qso(); }
        if k(Modifiers::COMMAND, Key::Comma) { self.set_page = Some(self.set_page.unwrap_or(SetPage::Station)); }
        if k(Modifiers::COMMAND, Key::R) { self.toggle_record(); }
        if self.tab != Tab::Operate { return; }
        for (i, key) in [Key::F5, Key::F6, Key::F7, Key::F8].into_iter().enumerate() {
            if k(Modifiers::NONE, key) && idle { self.send_phase(template::Phase::ALL[i]); }
        }
        if k(Modifiers::COMMAND, Key::Enter) && idle { self.send_primary(); }
    }

    /// F5–F8: Phase wählen und deren Vorlage bzw. Makro senden
    fn send_phase(&mut self, p: template::Phase) {
        let i = template::Phase::ALL.iter().position(|&x| x == p).unwrap_or(0);
        match self.op {
            Op::Sstv => {
                if !self.templates.iter().any(|t| t.phase == Some(p)) { self.log(format!("Keine Vorlage für „{}“", p.label())); return; }
                self.select_phase(p);
                self.start_tx(TxKind::Image);
            }
            Op::Rtty => { self.select_phase(p); let m = self.rtty_macros[i].clone(); self.rtty_send(m, Some(p)); }
            Op::Cw if self.cw_can_send() => { self.select_phase(p); let m = self.cw_macros[i].clone(); self.cw_send(&m, Some(p)); }
            Op::Cw => self.log("CW senden geht über ein per CAT verbundenes Icom-Gerät"),
            Op::Aprs | Op::Pocsag | Op::Packet => {}
        }
    }

    /// Strg+Enter: das Naheliegende senden
    fn send_primary(&mut self) {
        let i = template::Phase::ALL.iter().position(|&x| x == self.phase).unwrap_or(0);
        match self.op {
            Op::Sstv => self.start_tx(TxKind::Image),
            Op::Rtty => {
                let t = std::mem::take(&mut self.rtty_input);
                if t.trim().is_empty() { self.send_phase(self.phase) } else { self.rtty_send(t, None) }
            }
            Op::Cw => {
                if !self.cw_can_send() { self.send_phase(self.phase); return; }
                let t = std::mem::take(&mut self.cw_input);
                if t.trim().is_empty() { let m = self.cw_macros[i].clone(); self.cw_send(&m, Some(self.phase)) } else { self.cw_send(&t, None) }
            }
            Op::Aprs => if self.aprs.msg_text.trim().is_empty() { self.aprs_beacon() } else { self.aprs_send_msg() },
            Op::Pocsag => self.pocsag_send(),
            Op::Packet => {} // Enter im Eingabefeld sendet
        }
    }

    pub(crate) fn ui_help_window(&mut self, ctx: &egui::Context) {
        let Some(h) = self.help else { return };
        let mut open = true;
        let title = match h { Help::Keys => "Tastenkürzel", Help::About => "Über Rustwave" };
        egui::Window::new(title).open(&mut open).collapsible(false).resizable(false)
            .pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center())
            .show(ctx, |ui| match h {
                Help::Keys => {
                    egui::Grid::new("keys").num_columns(2).spacing([16.0, 6.0]).striped(true).show(ui, |ui| {
                        for (key, what) in KEYS { ui.label(RichText::new(key).monospace().strong()); ui.label(what); ui.end_row(); }
                    });
                    ui.label(RichText::new("Strg+Enter und F5–F8 wirken im Reiter Betrieb.").small().weak());
                }
                Help::About => {
                    if let Some(t) = &self.logo_tex { ui.add(egui::Image::new(t).fit_to_exact_size(egui::vec2(380.0, 97.0))); }
                    ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).strong());
                    ui.label("SSTV, RTTY, CW, APRS, POCSAG/DAPNET und Packet Radio mit CAT-Steuerung – auch drahtlos über das WLAN des IC-705.");
                    ui.add_space(4.0);
                    ui.label(RichText::new("© 2026 Roman Ewert, DL4RE").strong());
                    ui.label(RichText::new("Freie Software unter der GNU General Public License, Version 3 oder später (GPL-3.0-or-later). \
                        Ohne jede Gewährleistung.").small());
                    ui.label(RichText::new("Getestet und verifiziert mit Icom IC-705 (WLAN und USB) und IC-7300 (USB); andere Geräte der Liste sind ungetestet.").small().weak());
                    ui.label(RichText::new("Schrift DejaVu (freie Lizenz). Das Icom-Netzwerkprotokoll folgt kappanhang (MIT). Lizenztexte liegen neben der .exe.").small().weak());
                }
            });
        if !open { self.help = None; }
    }
}
