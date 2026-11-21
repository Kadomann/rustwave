//! Einstellungen (Station, Audio, Transceiver, Access Point, Darstellung) und Touch-Stil.

use super::*;

impl App {
    /// Einstellungsfenster: links die Seiten, rechts der Inhalt (schmal: Seiten oben)
    pub(crate) fn ui_settings_window(&mut self, ctx: &egui::Context) {
        let Some(mut page) = self.set_page else { return };
        let mut open = true;
        let mut done = false;
        let r = ctx.content_rect();
        let page_h = (r.height() - 220.0).clamp(200.0, 440.0);
        egui::Window::new("Einstellungen").open(&mut open).collapsible(false).resizable(true)
            .pivot(egui::Align2::CENTER_CENTER).default_pos(r.center())
            .default_width(680.0f32.min(r.width() - 20.0))
            .max_width(r.width() - 20.0).max_height(r.height() - 40.0)
            .show(ctx, |ui| {
                let pages = [(SetPage::Station, "Station"), (SetPage::Audio, "Audio"), (SetPage::Transceiver, "Transceiver"), (SetPage::Sstv, "SSTV"),
                             (SetPage::Aprs, "APRS"), (SetPage::Tnc, "TNC"), (SetPage::Darstellung, "Darstellung"), (SetPage::Profile, "Profile")];
                let title = pages.iter().find(|p| p.0 == page).map_or("", |p| p.1);
                let mut body = |ui: &mut egui::Ui, page: SetPage| {
                    ui.heading(title);
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().id_salt("set_page").auto_shrink([false, false]).max_height(page_h).show(ui, |ui| {
                        match page {
                            SetPage::Station => self.ui_set_station(ui),
                            SetPage::Audio => self.ui_set_audio(ui, ctx),
                            SetPage::Transceiver => self.ui_set_transceiver(ui),
                            SetPage::Sstv => self.ui_set_sstv(ui),
                            SetPage::Aprs => self.ui_set_aprs(ui),
                            SetPage::Tnc => self.ui_set_tnc(ui),
                            SetPage::Darstellung => self.ui_set_display(ui, ctx),
                            SetPage::Profile => self.ui_profiles(ui, ctx),
                        }
                    });
                    ui.separator();
                    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 30.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new("Fertig").strong()).clicked() { done = true; }
                        ui.label(RichText::new("Änderungen gelten sofort und werden gespeichert.").small().weak());
                    });
                };
                if ui.available_width() > 560.0 {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(130.0);
                            for (p, label) in pages {
                                if ui.add_sized([130.0, 0.0], egui::Button::selectable(page == p, RichText::new(label).size(15.0))).clicked() { page = p; }
                            }
                        });
                        ui.add_space(12.0);
                        ui.vertical(|ui| body(ui, page));
                    });
                } else {
                    ui.horizontal_wrapped(|ui| {
                        for (p, label) in pages { if ui.selectable_label(page == p, RichText::new(label).size(15.0)).clicked() { page = p; } }
                    });
                    ui.separator();
                    body(ui, page);
                }
            });
        self.set_page = (open && !done).then_some(page);
    }

    fn ui_set_station(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("station").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            for (label, v, hint, tip) in [("Rufzeichen", &mut self.station.call, "DK0XY", "in Vorlagen und Makros: %m"), ("Name", &mut self.station.name, "", "%o"),
                                     ("QTH", &mut self.station.qth, "", "%a"), ("Locator", &mut self.station.locator, "JO62QM", "%l – auch Mitte der APRS-Position, wenn keine eingetragen ist")] {
                ui.label(label).on_hover_text(tip);
                ui.add(egui::TextEdit::singleline(v).desired_width(180.0).hint_text(hint));
                ui.end_row();
            }
        });
        self.station.call = self.station.call.to_uppercase();
        if !self.station.call.is_empty() && self.status.contains("Willkommen") { self.status.clear(); }
        ui.label(RichText::new("In Vorlagen und Makros: %m Rufzeichen, %o Name, %a QTH, %l Locator.").small().weak());
    }

    fn ui_set_audio(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.net.is_some() {
            ui.label(RichText::new("Empfang und Senden laufen über WLAN/LAN (48 kHz). Die Soundkarte wird nicht gebraucht.").color(GREEN));
        }
        let combo = |ui: &mut egui::Ui, id: &str, val: &mut String, names: &[String]| {
            egui::ComboBox::from_id_salt(id).width(ui.available_width().min(360.0)).selected_text(if val.is_empty() { "Standard" } else { val.as_str() })
                .show_ui(ui, |ui| {
                    ui.selectable_value(val, String::new(), "Standard");
                    for n in names { ui.selectable_value(val, n.clone(), n); }
                }).response
        };
        ui.add_enabled_ui(self.net.is_none(), |ui| {
            ui.label("Eingang (vom Funkgerät)");
            let old_in = self.in_dev.clone();
            combo(ui, "in", &mut self.in_dev, &self.in_names.clone());
            ui.label("Ausgang (zum Funkgerät)");
            combo(ui, "out", &mut self.out_dev, &self.out_names.clone());
            ui.horizontal(|ui| {
                let label = if self.input.is_some() { "Audio neu starten" } else { "Audio starten" };
                if ui.button(label).clicked() || (self.input.is_some() && old_in != self.in_dev) { self.start_audio(ctx); }
                if ui.button("⟳").on_hover_text("Geräteliste neu laden").clicked() {
                    self.in_names = audio::input_names(); self.out_names = audio::output_names();
                }
            });
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Den Sendepegel stellst du je Betriebsart direkt beim Senden ein.").small().weak());
        self.ui_set_voice(ui);
    }

    fn ui_set_transceiver(&mut self, ui: &mut egui::Ui) {
        let connected = self.rig.is_some() || self.net.is_some() || self.net_connecting;
        if connected { ui.label(RichText::new("Zum Ändern zuerst trennen.").small().color(ORANGE)); }
        ui.add_enabled_ui(!connected, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.use_net, false, "USB / seriell");
                ui.selectable_value(&mut self.use_net, true, "WLAN / LAN");
            });
            let sel = self.rig_idx.map_or("Kein CAT (VOX)", |i| cat::RIGS[i].name);
            let mut idx = self.rig_idx;
            egui::ComboBox::from_id_salt("rig").width(ui.available_width().min(300.0)).selected_text(sel).show_ui(ui, |ui| {
                ui.selectable_value(&mut idx, None, "Kein CAT (VOX)");
                for (i, r) in cat::RIGS.iter().enumerate() { ui.selectable_value(&mut idx, Some(i), r.name); }
            });
            if idx != self.rig_idx {
                self.rig_idx = idx;
                if let Some(i) = idx { self.baud = cat::RIGS[i].baud; self.ptt = cat::RIGS[i].ptt; }
            }
            if self.use_net {
                let locked = self.net.is_some() || self.net_connecting;
                ui.add_enabled_ui(!locked, |ui| ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut self.net_ap, false, "Heimnetz").on_hover_text("Funkgerät und PC im selben (W)LAN, z. B. über den Router");
                    ui.selectable_value(&mut self.net_ap, true, "Direkt (Access Point)").on_hover_text("IC-705 spannt sein eigenes WLAN auf – ohne Router, z. B. portabel");
                }));
                if self.net_ap { self.ui_ap(ui, locked); }
                egui::Grid::new("net").num_columns(2).show(ui, |ui| {
                    if !self.net_ap {
                        ui.label("IP-Adresse");
                        ui.add(egui::TextEdit::singleline(&mut self.net_host).desired_width(150.0).hint_text("192.168.1.50"));
                        ui.end_row();
                    }
                    ui.label(if self.net_ap { "Netzw.-Benutzer" } else { "Benutzer" }).on_hover_text("Netzwerkbenutzer aus MENU > SET > WLAN Set > Remote Settings");
                    ui.add(egui::TextEdit::singleline(&mut self.net_user).desired_width(150.0));
                    ui.end_row();
                    ui.label(if self.net_ap { "Benutzer-Passw." } else { "Passwort" });
                    ui.add(egui::TextEdit::singleline(&mut self.net_pass).desired_width(150.0).password(true));
                    ui.end_row();
                });
                ui.label(RichText::new("IC-705: MENU > SET > WLAN Set > Remote Settings: Network Control ON, Netzwerkbenutzer anlegen. MENU > SET > Connectors > MOD Input: DATA OFF MOD (für USB) bzw. DATA MOD (für USB-D) = WLAN.").small().weak());
                return;
            }
            ui.horizontal(|ui| {
                ui.label("Port");
                // frei editierbar (falls Windows einen Port nicht meldet), Liste als Vorschlag
                ui.add(egui::TextEdit::singleline(&mut self.port).desired_width(70.0).hint_text("COM3"));
                egui::ComboBox::from_id_salt("port").width(16.0).selected_text("").show_ui(ui, |ui| {
                    if self.ports.is_empty() { ui.label("keine Ports gefunden"); }
                    for p in self.ports.clone() { ui.selectable_value(&mut self.port, p.clone(), p); }
                });
                if ui.button("⟳").on_hover_text("Ports neu suchen").clicked() { self.refresh_ports(); }
            });
            ui.horizontal(|ui| {
                ui.label("Baud");
                ui.add(egui::DragValue::new(&mut self.baud).range(1200..=921600));
                ui.label("PTT");
                egui::ComboBox::from_id_salt("ptt").selected_text(self.ptt.label()).show_ui(ui, |ui| {
                    for p in cat::Ptt::ALL { ui.selectable_value(&mut self.ptt, p, p.label()); }
                });
            });
            ui.label(RichText::new("IC-7300: SET > Connectors > DATA OFF MOD / DATA MOD = USB, USB SEND = OFF. ALC beim Abstimmton beobachten.").small().weak());
        });
        ui.add_space(6.0);
        self.ui_connect_row(ui);
        if self.rig.as_ref().is_some_and(|r| r.model.proto == cat::Proto::Civ) || (self.rig.is_none() && self.rig_idx.is_some_and(|i| cat::RIGS[i].proto == cat::Proto::Civ)) {
            ui.checkbox(&mut self.scope_want, "Spektrum vom Funkgerät anzeigen")
                .on_hover_text("Bandausschnitt des Icom-Spektrums oberhalb von Empfang/Senden; Klick stimmt ab. Am besten über WLAN oder mit 115200 Bd.");
        }
        ui.label(RichText::new("Frequenz, Betriebsart, Bänder und Sendeleistung stellst du in der Funkgerät-Leiste oben ein. \
            Für sofortiges Mitlaufen mit dem Abstimmknopf am Gerät CI-V Transceive einschalten.").small().weak());
    }

    /// Verbinden/Trennen samt Wiederverbinden-Hinweis (Einstellungen und Menü der Funkgerät-Leiste)
    pub(crate) fn ui_connect_row(&mut self, ui: &mut egui::Ui) {
        let connected = self.rig.is_some() || self.net.is_some() || self.net_connecting;
        if let Some((at, _)) = self.reconnect {
            ui.horizontal(|ui| {
                let left = at.saturating_duration_since(Instant::now()).as_secs();
                ui.label(RichText::new(if self.net_connecting { "Verbinde neu …".to_string() } else { format!("Verbindung verloren – neuer Versuch in {left} s") }).small().color(ORANGE));
                if ui.small_button("Abbrechen").clicked() { self.reconnect = None; }
            });
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
        ui.horizontal(|ui| {
            let label = if self.net_connecting { "Verbinde …" } else if connected { "Trennen" } else { "Verbinden" };
            if ui.add_enabled(!self.net_connecting, egui::Button::new(label)).clicked() { self.reconnect = None; self.connect(); }
            let has_cat = self.rig.as_ref().is_some_and(|r| r.model.proto != cat::Proto::None);
            if ui.add_enabled(has_cat, egui::Button::new("Frequenz lesen")).clicked() { self.read_freq(); }
        });
    }

    fn ui_set_sstv(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Empfangene Bilder").strong());
        ui.label(RichText::new(format!("werden gespeichert in {} (Übersicht im Reiter Galerie)", self.save_dir.display())).small().weak());
        ui.horizontal(|ui| {
            ui.label("zusätzlich kopieren nach");
            ui.add(egui::TextEdit::singleline(&mut self.copy_dir).desired_width(260.0).hint_text("leer = nicht kopieren"));
            #[cfg(windows)]
            if ui.button("…").on_hover_text("Ordner wählen").clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() { self.copy_dir = p.display().to_string(); }
            }
        });
        ui.label(RichText::new("Zum automatischen Hochladen einen Ordner nehmen, den OneDrive, Dropbox o. Ä. abgleicht, \
            oder eine Netzwerkfreigabe, z. B. für die Bilderseite deines Clubs.").small().weak());
    }

    fn ui_set_display(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut t = self.touch.unwrap_or(false);
        if ui.checkbox(&mut t, "Touch-Bedienung (große Knöpfe, Menü hinter ☰)").changed() { self.touch = Some(t); }
        ui.label(RichText::new("Schaltet sich bei der ersten Berührung des Bildschirms von selbst ein.").small().weak());
        ui.horizontal(|ui| {
            ui.label("Bildrate");
            for (f, l) in [(8, "sparsam (8/s)"), (15, "normal (15/s)"), (25, "flüssig (25/s)")] {
                ui.selectable_value(&mut self.fps, f, l);
            }
        }).response.on_hover_text("Wie oft Wasserfall, Spektrum und Anzeigen bei laufendem Empfang neu gezeichnet werden. \
            Weniger Bilder = deutlich weniger CPU-Last. Die Dekoder arbeiten unabhängig davon immer mit voller Genauigkeit.");
        ui.checkbox(&mut self.power_save, "Energiesparen (Akkubetrieb)")
            .on_hover_text("Bildrate 8/s und Wasserfall mit 10 statt 25 Zeilen pro Sekunde – Dekoder arbeiten unverändert");
        ui.horizontal(|ui| {
            ui.label("Zoom");
            let r = ui.add(egui::Slider::new(&mut self.zoom, 0.7..=1.6).step_by(0.05).custom_formatter(|v, _| format!("{:.0} %", v * 100.0)));
            if r.drag_stopped() || (r.changed() && !r.dragged()) { ctx.set_zoom_factor(self.zoom); }
        });
    }

    /// IC-705 als Access Point: WLAN-Zugangsdaten, Verbinden des PCs, Prüfung der Adresse
    pub(crate) fn ui_ap(&mut self, ui: &mut egui::Ui, locked: bool) {
        // alle 2 s nachsehen, ob der PC im Funkgeräte-WLAN ist
        if self.ap_pc.1.is_none_or(|t| t.elapsed() > Duration::from_secs(2)) {
            self.ap_pc = (apmode::pc_in_ap_net(), Some(Instant::now()));
            ui.ctx().request_repaint_after(Duration::from_secs(2));
        }
        egui::Grid::new("ap").num_columns(2).show(ui, |ui| {
            ui.label("Funkgerät");
            ui.label(RichText::new(apmode::AP_IP).monospace()).on_hover_text("im Access-Point-Betrieb fest vorgegeben");
            ui.end_row();
            ui.label("WLAN-Name");
            ui.add_enabled(!locked, egui::TextEdit::singleline(&mut self.ap_ssid).desired_width(150.0).hint_text("SSID am IC-705"));
            ui.end_row();
            ui.label("WLAN-Passwort");
            ui.add_enabled(!locked, egui::TextEdit::singleline(&mut self.ap_pass).desired_width(150.0).password(true));
            ui.end_row();
        });
        match self.ap_pc.0 {
            Some(a) => { ui.label(RichText::new(format!("✔ PC ist im WLAN des IC-705 ({a})")).small().color(GREEN)); }
            None => {
                ui.label(RichText::new("PC ist noch nicht im WLAN des IC-705").small().color(ORANGE));
                let label = if self.ap_joining { "verbinde WLAN …" } else { "PC mit IC-705-WLAN verbinden" };
                if ui.add_enabled(!self.ap_joining && !locked && !self.ap_ssid.trim().is_empty(), egui::Button::new(label))
                    .on_hover_text("legt in Windows ein WLAN-Profil an, verbindet und danach das Funkgerät")
                    .on_disabled_hover_text("WLAN-Name eintragen").clicked() {
                    self.ap_joining = true;
                    self.log(format!("Verbinde den PC mit dem WLAN „{}“ …", self.ap_ssid.trim()));
                    let (ssid, pass, tx) = (self.ap_ssid.trim().to_string(), self.ap_pass.clone(), self.ev_tx.clone());
                    std::thread::spawn(move || { let _ = tx.send(UiEv::ApJoined(apmode::join_wlan(&ssid, &pass))); });
                }
            }
        }
        ui.label(RichText::new("IC-705: MENU > SET > WLAN Set: WLAN OFF › Connection Type: Access Point › Connection Settings (Access Point): \
            SSID und Passwort festlegen › WLAN ON. Nur ein Gerät kann verbunden sein; der PC hat währenddessen über WLAN kein Internet.").small().weak());
    }

    /// Stil für Finger: größere Klickflächen, Abstände, Schrift und Rollbalken
    pub(crate) fn apply_touch(&mut self, ctx: &egui::Context) {
        let touch = self.touch.unwrap_or(false);
        if self.touch_applied == Some(touch) { return; }
        let base = self.base_style.get_or_insert_with(|| (*ctx.style()).clone()).clone();
        let mut st = base;
        if touch {
            let sp = &mut st.spacing;
            sp.interact_size = egui::vec2(40.0, 32.0);
            sp.button_padding = egui::vec2(10.0, 6.0);
            sp.item_spacing = egui::vec2(10.0, 8.0);
            sp.slider_width = 170.0;
            sp.icon_width = 24.0;
            sp.icon_width_inner = 14.0;
            sp.icon_spacing = 8.0;
            sp.scroll = egui::style::ScrollStyle::solid();
            sp.scroll.bar_width = 16.0;
            for (k, f) in st.text_styles.iter_mut() {
                if *k == egui::TextStyle::Small { f.size = f.size.max(12.0); } // Hinweistexte sonst kaum lesbar
            }
        }
        ctx.set_style(st);
        self.touch_applied = Some(touch);
    }

}
