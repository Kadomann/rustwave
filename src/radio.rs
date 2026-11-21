//! Funkgerät: verbinden (USB oder WLAN/LAN), trennen, automatisch neu verbinden, Frequenz.

use super::*;

impl App {
    pub(crate) fn connect(&mut self) {
        if self.rig.is_some() || self.net.is_some() { self.disconnect(); return; }
        let Some(i) = self.rig_idx else { self.log("Kein Gerät gewählt – PTT per VOX"); return };
        if self.use_net {
            if cat::RIGS[i].proto != cat::Proto::Civ { self.log("WLAN/LAN gibt es nur für Icom-Geräte"); return; }
            let host = if self.net_ap {
                // Access Point: der PC muss im WLAN des Funkgeräts sein, sonst kommt nie eine Antwort
                if apmode::pc_in_ap_net().is_none() {
                    self.log("Der PC ist nicht im WLAN des IC-705 – „PC mit IC-705-WLAN verbinden“ drücken oder in Windows das WLAN des Funkgeräts wählen");
                    return;
                }
                apmode::AP_IP.to_string()
            } else {
                if self.net_host.trim().is_empty() { self.log("IP-Adresse des Funkgeräts eintragen"); return; }
                self.net_host.trim().to_string()
            };
            let (h, u, p, tx) = (host.clone(), self.net_user.clone(), self.net_pass.clone(), self.ev_tx.clone());
            std::thread::spawn(move || { let _ = tx.send(UiEv::NetConnected(icomnet::connect(&h, &u, &p))); });
            self.net_connecting = true;
            self.log(format!("Verbinde mit {host} …"));
            return;
        }
        let mut model = cat::RIGS[i].clone();
        model.baud = self.baud;
        model.ptt = self.ptt;
        match cat::Rig::open(&self.port, model) {
            Ok(r) => { self.log(format!("{} an {} verbunden ({} Bd)", r.model.name, self.port, self.baud)); self.rig = Some(r); self.gps.reset(); self.poll = Default::default(); self.read_freq(); }
            Err(e) => self.log(format!("CAT-Fehler: {e}")),
        }
    }
    pub(crate) fn disconnect(&mut self) {
        self.if_restore();
        self.rig = None; // sendet PTT aus (bei WLAN über den noch laufenden Netzwerk-Thread)
        if let Some(n) = self.net.take() {
            n.shared.set_audio_sink(None);
            drop(n); // meldet sich beim Funkgerät ab
            self.rx_info = "Audio starten – der Modus wird am VIS-Code automatisch erkannt".into();
        }
        self.freq_shown = None;
        self.log("Funkgerät getrennt");
    }
    /// Anmeldung fertig: CI-V und Audio über das Netzwerk einrichten
    pub(crate) fn net_connected(&mut self, r: Result<icomnet::NetRadio, String>, ctx: &egui::Context) {
        self.net_connecting = false;
        let n = match r {
            Ok(n) => n,
            Err(e) => { self.log(format!("WLAN-Verbindung fehlgeschlagen: {e}")); return; }
        };
        let Some(i) = self.rig_idx else { return }; // Verbindung wird beim Verwerfen getrennt
        self.input = None; // Soundkarte nicht mehr nötig
        let (stx, srx) = mpsc::channel();
        n.shared.set_audio_sink(Some(stx));
        let (tx, sh, c) = (self.ev_tx.clone(), self.rx_sh.clone(), ctx.clone());
        std::thread::spawn(move || rx_thread(icomnet::RATE as f64, srx, tx, sh, c));
        self.rig = Some(cat::Rig::open_net(n.shared.clone(), cat::RIGS[i].clone()));
        self.poll = Default::default();
        self.gps.reset();
        self.log(format!("{} über WLAN/LAN verbunden – CI-V und Audio laufen über das Netzwerk", n.name));
        self.rx_info = "Warte auf VIS – Modus wird automatisch erkannt".into();
        self.net = Some(n);
        self.read_freq();
    }
    /// Verbindungsabbruch erkennen
    pub(crate) fn poll_net(&mut self) {
        let err = self.net.as_ref().and_then(|n| n.shared.error.lock().unwrap().clone());
        if let Some(e) = err {
            if let Some(t) = &self.tx { t.state.stop.store(true, Relaxed); }
            self.disconnect();
            self.log(format!("WLAN-Verbindung getrennt: {e} – neuer Versuch in 5 s"));
            self.reconnect = Some((Instant::now() + Duration::from_secs(5), 0));
        }
        // automatisch neu verbinden: 5, 10, 20, dann alle 30 s
        if let Some((at, n)) = self.reconnect {
            if self.net.is_some() { self.reconnect = None; self.log("Wieder verbunden"); return; }
            if !self.net_connecting && Instant::now() >= at {
                self.log(format!("Verbinde neu (Versuch {}) …", n + 1));
                self.connect();
                let wait = [5u64, 10, 20, 30][(n as usize + 1).min(3)];
                self.reconnect = Some((Instant::now() + Duration::from_secs(wait), n + 1));
            }
        }
    }
    pub(crate) fn read_freq(&mut self) {
        let Some(rig) = &mut self.rig else { return };
        match rig.read_freq() {
            Ok(Some(f)) => { self.freq_shown = Some(f); self.freq_khz = format!("{:.1}", f as f64 / 1000.0); }
            Ok(None) => self.log("Keine Antwort – Baudrate / CI-V-Adresse / Echo am Gerät prüfen"),
            Err(e) => self.log(format!("CAT-Fehler: {e}")),
        }
    }
    /// Frequenz setzen; `sb` = Some(..) stellt zusätzlich die Betriebsart ein
    pub(crate) fn tune_to(&mut self, khz: f64, sb: Option<cat::Sb>) {
        let Some(rig) = &mut self.rig else { return };
        let r = rig.set_freq((khz * 1000.0).round() as u64)
            .and_then(|_| { std::thread::sleep(Duration::from_millis(50)); match sb { Some(m) => rig.set_mode(m), None => Ok(()) } });
        if let Err(e) = r { self.log(format!("CAT-Fehler: {e}")); return; }
        // gleich übernehmen (sonst merkt sich die Betriebsart bis zur nächsten Abfrage die alte Sendeart)
        if let Some(m) = sb { self.poll.mode = Some((match m { cat::Sb::Usb => "USB", cat::Sb::Lsb => "LSB", cat::Sb::Fm => "FM", cat::Sb::Cw => "CW" }, Some(m))); }
        std::thread::sleep(Duration::from_millis(100));
        self.read_freq();
    }

    /// je Bild: CAT-Antworten auswerten und die nächste Abfrage schicken (Frequenz, Betriebsart, Leistung, GPS)
    pub(crate) fn cat_tick(&mut self, ctx: &egui::Context) {
        let Some(rig) = &mut self.rig else { return };
        if matches!(rig.model.proto, cat::Proto::None | cat::Proto::Yaesu5) { return; }
        let now = Instant::now();
        let model = rig.model.clone();
        // beim Senden Messwerte statt Frequenz abfragen; zu Beginn alte Werte verwerfen
        let tx = self.tx.is_some() || self.cw_tx.is_some() || self.tuning.is_some();
        if tx && !self.poll.tx { self.poll.meters = Default::default(); }
        self.poll.tx = tx;
        let bytes = rig.take_rx();
        let evs = self.poll.feed(&model, &bytes, now);
        if let Some(q) = self.poll.next_query(&model, now) {
            if let Err(e) = rig.send_raw(&q) { self.log(format!("CAT: {e}")); }
        }
        for e in evs {
            if let rigpoll::Ev::Freq(f) = e { self.on_freq(f); }
        }
        // Frequenz/Betriebsart der aktuellen Betriebsart merken
        if let (Some(f), Some((_, sb))) = (self.freq_shown, self.poll.mode) {
            self.freq_mem.insert(self.op, (f, sb));
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    /// neue Frequenz vom Gerät (Abfrage oder Knopf)
    fn on_freq(&mut self, f: u64) {
        if self.freq_shown != Some(f) {
            self.freq_shown = Some(f);
            self.freq_khz = format!("{:.1}", f as f64 / 1000.0);
        }
    }

    /// Sendeleistung (Icom in %, sonst Watt): Regler schickt an das Gerät, Anzeige folgt dem Regler am Gerät
    pub(crate) fn ui_power(&mut self, ui: &mut egui::Ui) {
        let Some(rig) = &self.rig else { return };
        if !self.poll.supported(rigpoll::Kind::Power) || matches!(rig.model.proto, cat::Proto::None | cat::Proto::Yaesu5) { return; }
        let (max, unit) = cat::power_scale(&rig.model);
        let watts = cat::max_watts(&rig.model);
        let Some(mut v) = self.poll.power else { ui.label(RichText::new("Leistung: lese …").small().weak()); return };
        ui.horizontal(|ui| {
            ui.label("Leistung");
            // ohne step_by: sonst rundet der Regler einen vom Gerät gelesenen Wert und schickt ihn zurück
            let r = ui.add(egui::Slider::new(&mut v, 0.0..=max)
                .custom_formatter(move |v, _| if unit == "W" && max <= 10.0 { format!("{v:.1} W") } else { format!("{v:.0} {unit}") }))
                .on_hover_text(if unit == "%" { format!("Sendeleistung wie im Menü RF POWER – 100 % ≈ {watts:.0} W bei voller Versorgung. Folgt auch dem Regler am Gerät.") }
                    else { "Sendeleistung des Funkgeräts – folgt auch dem Regler am Gerät".to_string() });
            let by_user = r.dragged() || r.drag_stopped() || r.clicked() || r.has_focus();
            if by_user && (r.changed() || r.drag_stopped()) {
                let now = Instant::now();
                self.poll.power = Some(v);
                self.poll.power_hold = Some(now + Duration::from_millis(1500));
                // beim Ziehen höchstens alle 150 ms senden, am Ende immer
                if r.drag_stopped() || self.power_sent.is_none_or(|t| now.duration_since(t) > Duration::from_millis(150)) {
                    self.power_sent = Some(now);
                    let Some(rig) = &mut self.rig else { return };
                    let b = cat::set_power(&rig.model, v);
                    if let Err(e) = rig.send_raw(&b) { self.log(format!("CAT: {e}")); }
                }
            }
        });
    }

    /// Messwerte beim Senden (Icom): Ausgangsleistung, SWR, ALC – bleiben nach dem Senden noch kurz (blass) stehen
    pub(crate) fn ui_meters(&mut self, ui: &mut egui::Ui) {
        let Some(rig) = &self.rig else { return };
        // im Empfang: S-Meter
        if !self.poll.tx {
            if let Some(v) = self.poll.s_now() {
                let c = if v > 120 { GREEN } else { ui.visuals().text_color() };
                ui.label(RichText::new(format!("{:<6}", crate::rigpoll::s_text(v))).monospace().color(c))
                    .on_hover_text("S-Meter des Funkgeräts (Rustwave merkt sich den Wert bei jedem dekodierten Paket)");
                ui.add(egui::ProgressBar::new((v as f32 / 241.0).clamp(0.0, 1.0)).desired_width(50.0).desired_height(10.0).fill(c.gamma_multiply(0.8)));
                ui.ctx().request_repaint_after(Duration::from_millis(300));
            }
        }
        let m = self.poll.meters;
        let Some(at) = m.at else { return };
        let live = self.poll.tx;
        if !live && at.elapsed() > Duration::from_secs(15) { return; }
        let dim = |c: Color32| if live { c } else { c.gamma_multiply(0.5) };
        let weak = ui.visuals().weak_text_color();
        if let Some(p) = m.po_pct {
            let w = p / 100.0 * cat::max_watts(&rig.model);
            ui.label(RichText::new(if w < 10.0 { format!("Po {w:.1} W") } else { format!("Po {w:.0} W") }).monospace().color(dim(ui.visuals().text_color())))
                .on_hover_text(format!("Ausgangsleistung laut Messinstrument des Funkgeräts: {p:.0} %{}", if live { "" } else { " (zuletzt gesendet)" }));
        }
        if let Some(s) = m.swr {
            let c = if s < 1.5 { GREEN } else if s < 2.5 { ORANGE } else { RED };
            ui.label(RichText::new(if s >= 3.0 { format!("SWR >{:.0}", s.floor()) } else { format!("SWR {s:.1}") }).monospace().strong().color(dim(c)))
                .on_hover_text("Stehwellenverhältnis – über 2,5 Antenne bzw. Tuner prüfen");
        }
        if let Some(a) = m.alc_pct {
            let c = if a > 100.0 { RED } else { GREEN };
            ui.label(RichText::new("ALC").monospace().color(weak));
            ui.add(egui::ProgressBar::new((a / 100.0).clamp(0.0, 1.0)).desired_width(50.0).desired_height(10.0).fill(dim(c)))
                .on_hover_text(format!("ALC {a:.0} % des Regelbereichs – bei Digitalbetrieb soll sie möglichst wenig ausschlagen; Sendepegel verringern, wenn sie deutlich anzeigt"));
        }
        if live { ui.ctx().request_repaint_after(Duration::from_millis(200)); }
    }
}
