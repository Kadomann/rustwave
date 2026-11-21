//! Packet-Radio-Oberfläche: Monitor aller Rahmen, Terminal für eine Verbindung (Mailbox, Knoten, DX-Cluster,
//! andere Station), Unproto-Text, Kanalzugriff mit Belegt-Erkennung, Frequenz-Schnellwahl.

use crate::ax25::{self, Frame, Kind, LinkEv, LinkParams, Speed, State};
use crate::{cat, App, TxKind, GREEN, ORANGE};
use eframe::egui::{self, Color32, RichText};
use std::collections::VecDeque;
use std::time::Instant;

/// Packet-Frequenzen (kHz, Mode, Geschwindigkeit, Bezeichnung, Hinweis). Örtliche Digipeater und Benutzereinstiege
/// stehen in den Relaislisten; Kurzwelle mit Tönen 1600/1800 Hz.
pub const PACKET_LIST: [(f64, cat::Sb, Speed, &str, &str); 13] = [
    (144925.0, cat::Sb::Fm, Speed::B1200, "144,925", "2 m, Digitalbereich des Bandplans (IARU R1 144,800–144,990 MHz)"),
    (144950.0, cat::Sb::Fm, Speed::B1200, "144,950", "2 m, Digitalbereich des Bandplans"),
    (70325.0, cat::Sb::Fm, Speed::B1200, "70,325", "4 m Packet (IARU R1, wo 4 m freigegeben ist)"),
    (70487.5, cat::Sb::Fm, Speed::B1200, "70,4875", "4 m Packet (IARU R1)"),
    (433625.0, cat::Sb::Fm, Speed::B1200, "433,625", "70 cm Packet (u. a. Großbritannien)"),
    (433650.0, cat::Sb::Fm, Speed::B1200, "433,650", "70 cm Packet (u. a. Großbritannien)"),
    (438025.0, cat::Sb::Fm, Speed::B9600, "438,025", "70 cm, IARU-R1-Packetbereich 438,025–438,175 MHz, oft 9600 Bd"),
    (438100.0, cat::Sb::Fm, Speed::B9600, "438,100", "70 cm, IARU-R1-Packetbereich, oft 9600 Bd"),
    (438175.0, cat::Sb::Fm, Speed::B9600, "438,175", "70 cm, IARU-R1-Packetbereich, oft 9600 Bd"),
    (14105.0, cat::Sb::Lsb, Speed::B300, "14,105 LSB", "Net105 (20 m), Gespräche und Knoten"),
    (10144.0, cat::Sb::Lsb, Speed::B300, "10,144 LSB", "Net30 (30 m)"),
    (7104.0, cat::Sb::Lsb, Speed::B300, "7,104 LSB", "Net40 (40 m)"),
    (3598.7, cat::Sb::Lsb, Speed::B300, "3,5987 LSB", "Net80 (80 m)"),
];
const MAX_MON: usize = 600;
const MAX_TERM: usize = 2000;

/// Monitorzeile: Zeit, Text, Absender, Art (0 = fremd, 1 = an/von mir empfangen, 2 = selbst gesendet)
pub struct MonLine { pub time: String, pub text: String, pub src: String, pub kind: u8 }
/// Terminalzeile: Text und Art (0 = empfangen, 1 = selbst gesendet, 2 = Hinweis)
pub struct TermLine { pub text: String, pub kind: u8 }

pub struct PacketState {
    // Einstellungen (werden gespeichert)
    pub speed: Speed,
    /// SSID für Packet (leer = ohne)
    pub ssid: String,
    pub p: LinkParams,
    pub txdelay_ms: f64,
    /// Begrüßung für eingehende Verbindungen (leer = keine); Makros wie %m, %o
    pub ctext: String,
    /// Ziel einer Verbindung: "DB0XYZ" oder "DB0XYZ via DB0AAA DB0BBB"
    pub target: String,
    /// Unproto-Text ohne Verbindung an dieses Ziel über diese Digipeater
    pub un_to: String,
    pub un_via: String,
    // Laufzeit
    pub link: ax25::Link,
    pub term: Vec<TermLine>,
    pub input: String,
    pub monitor: VecDeque<MonLine>,
    pub mine_only: bool,
    /// Unproto-Rahmen, die auf den Kanal warten
    pub ui_queue: Vec<Frame>,
    /// gerade gesendete Rahmen
    pub sending: Vec<Vec<u8>>,
    /// Rahmen (mit FCS) von anderen Programmen über KISS/AGW
    pub kiss_q: Vec<Vec<u8>>,
    /// Kanalzugriff: frühester Sendezeitpunkt nach freiem Kanal
    access_at: Option<f64>,
    /// die laufende Verbindung habe ich aufgebaut (sonst: eingehend)
    outgoing: bool,
    clock: Instant,
    rng: u64,
}

fn parse_target(t: &str) -> Option<(String, Vec<String>)> {
    let up = t.trim().to_uppercase().replace(',', " ");
    let mut parts = up.split_whitespace().filter(|w| *w != "VIA" && *w != "V");
    let dst = parts.next()?.to_string();
    let via: Vec<String> = parts.map(|s| s.to_string()).collect();
    (ax25::valid_call(&dst) && via.len() <= 8 && via.iter().all(|d| ax25::valid_call(d))).then_some((dst, via))
}

impl PacketState {
    pub fn load(get: &dyn Fn(&str) -> Option<String>) -> Self {
        let mut s = PacketState {
            speed: Speed::B1200, ssid: String::new(), p: LinkParams::default(), txdelay_ms: 300.0, ctext: String::new(),
            target: String::new(), un_to: "CQ".into(), un_via: String::new(),
            link: ax25::Link::new("NOCALL", LinkParams::default()), term: vec![], input: String::new(), monitor: VecDeque::new(),
            mine_only: false, ui_queue: vec![], sending: vec![], kiss_q: vec![], access_at: None, outgoing: false, clock: Instant::now(), rng: 0x9E37_79B9_7F4A_7C15,
        };
        s.apply_settings(get);
        s
    }
    pub fn apply_settings(&mut self, get: &dyn Fn(&str) -> Option<String>) {
        let d = LinkParams::default();
        let num = |k: &str| get(k).and_then(|v| v.parse::<f64>().ok());
        self.speed = get("pk_speed").and_then(|v| Speed::from_key(&v)).unwrap_or(Speed::B1200);
        self.ssid = get("pk_ssid").unwrap_or_default();
        self.p = LinkParams {
            paclen: num("pk_paclen").map_or(d.paclen, |v| (v as usize).clamp(16, 256)),
            maxframe: num("pk_maxframe").map_or(d.maxframe, |v| (v as usize).clamp(1, 7)),
            frack: num("pk_frack").map_or(d.frack, |v| v.clamp(1.0, 30.0)),
            retries: num("pk_retries").map_or(d.retries, |v| (v as u32).clamp(1, 30)),
            resp: d.resp,
            accept: get("pk_accept").as_deref() != Some("0"),
        };
        self.txdelay_ms = num("pk_txd").unwrap_or(300.0).clamp(10.0, 1000.0);
        self.ctext = get("pk_ctext").unwrap_or_else(|| "Hallo, hier ist %m (%o, %a) mit Rustwave. Bitte schreib einfach.".into());
        self.target = get("pk_target").unwrap_or_default();
        self.un_to = get("pk_un_to").filter(|s| !s.is_empty()).unwrap_or_else(|| "CQ".into());
        self.un_via = get("pk_un_via").unwrap_or_default();
    }
    pub fn save(&self, s: &mut dyn eframe::Storage) {
        s.set_string("pk_speed", self.speed.key().into());
        s.set_string("pk_ssid", self.ssid.clone());
        s.set_string("pk_paclen", self.p.paclen.to_string());
        s.set_string("pk_maxframe", self.p.maxframe.to_string());
        s.set_string("pk_frack", self.p.frack.to_string());
        s.set_string("pk_retries", self.p.retries.to_string());
        s.set_string("pk_accept", if self.p.accept { "1" } else { "0" }.into());
        s.set_string("pk_txd", self.txdelay_ms.to_string());
        s.set_string("pk_ctext", self.ctext.clone());
        s.set_string("pk_target", self.target.clone());
        s.set_string("pk_un_to", self.un_to.clone());
        s.set_string("pk_un_via", self.un_via.clone());
    }
    pub fn now(&self) -> f64 { self.clock.elapsed().as_secs_f64() }
    fn rand(&mut self) -> f64 {
        self.rng ^= self.rng << 13; self.rng ^= self.rng >> 7; self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }
    /// läuft eine Verbindung (oder ihr Auf-/Abbau)?
    pub fn active(&self) -> bool { self.link.state != State::Disconnected }
    fn term_push(&mut self, text: &str, kind: u8) {
        for (i, part) in text.split('\n').enumerate() {
            let cont = i == 0 && self.term.last().is_some_and(|l| l.kind == kind && kind != 2);
            if cont { self.term.last_mut().unwrap().text.push_str(part); } else { self.term.push(TermLine { text: part.to_string(), kind }); }
        }
        let n = self.term.len();
        if n > MAX_TERM { self.term.drain(..n - MAX_TERM); }
    }
    fn info(&mut self, t: &str) {
        let now = chrono::Local::now().format("%H:%M:%S");
        self.term.push(TermLine { text: format!("*** {now} {t}"), kind: 2 });
        self.term.push(TermLine { text: String::new(), kind: 0 });
    }
    fn mon(&mut self, f: &Frame, kind: u8) {
        self.monitor.push_back(MonLine { time: chrono::Local::now().format("%H:%M:%S").to_string(), text: f.monitor(), src: f.src.clone(), kind });
        while self.monitor.len() > MAX_MON { self.monitor.pop_front(); }
    }
}

impl App {
    /// eigenes Packet-Rufzeichen (mit SSID)
    pub(crate) fn packet_call(&self) -> String {
        let c = self.station.call.trim().to_uppercase();
        let s = self.packet_ssid();
        if s.is_empty() || s == "0" { c } else { format!("{c}-{s}") }
    }
    fn packet_ssid(&self) -> String { self.packet.ssid.trim().trim_start_matches('-').to_string() }

    /// empfangener Rahmen (live = vom Funkgerät, sonst aus einer WAV-Datei)
    pub(crate) fn packet_received(&mut self, bytes: Vec<u8>, live: bool) {
        if live { if let Some(k) = &self.kiss { k.broadcast(&bytes); } }
        let Some(f) = Frame::from_bytes(&bytes) else { return };
        let me = ax25::norm_call(&self.packet_call());
        let mine = ax25::norm_call(&f.dst) == me || ax25::norm_call(&f.src) == me;
        self.packet.mon(&f, mine as u8);
        if live && !self.station.call.is_empty() {
            let now = self.packet.now();
            self.packet.link.receive(&f, now);
            self.packet_events();
        }
    }

    fn packet_events(&mut self) {
        let evs: Vec<LinkEv> = self.packet.link.events.drain(..).collect();
        for e in evs {
            match e {
                LinkEv::Connected(peer) => {
                    let incoming = !self.packet.outgoing;
                    self.packet.info(&format!("Verbunden mit {peer}"));
                    self.log(format!("Packet: verbunden mit {peer}"));
                    if incoming && !self.packet.ctext.trim().is_empty() {
                        let t = crate::template::expand(&self.packet.ctext, &self.macros(false));
                        let now = self.packet.now();
                        self.packet.link.send(format!("{}\r", t.replace('\n', "\r")).as_bytes(), now);
                        self.packet.term_push(&format!("{t}\n"), 1);
                    }
                }
                LinkEv::Disconnected(why) => {
                    self.packet.outgoing = false;
                    self.packet.info(&format!("Verbindung beendet: {why}"));
                    self.log(format!("Packet: {why}"));
                }
                LinkEv::Data(d) => self.packet.term_push(&ax25::text_of(&d), 0),
            }
        }
    }

    /// Zeitgeber der Verbindung und Kanalzugriff (nur senden, wenn der Kanal frei ist)
    pub(crate) fn packet_tick(&mut self, ctx: &egui::Context) {
        let me = self.packet_call();
        if !self.packet.active() && !me.is_empty() { self.packet.link.me = ax25::norm_call(&me); self.packet.link.p = self.packet.p; }
        if self.op != crate::Op::Packet && !self.packet.active() { return; }
        let now = self.packet.now();
        self.packet.link.tick(now);
        self.packet_events();
        let want = self.packet.link.has_out() || !self.packet.ui_queue.is_empty() || !self.packet.kiss_q.is_empty();
        if self.packet.active() || want { ctx.request_repaint_after(std::time::Duration::from_millis(150)); }
        if !want || self.tx.is_some() || self.cw_tx.is_some() { return; }
        if self.rx_sh.packet_dcd.load(std::sync::atomic::Ordering::Relaxed) { self.packet.access_at = None; return; }
        // freier Kanal: nach kurzer Zufallswartezeit senden (p-persistent, Zeitschlitz 100 ms, p = 0,25)
        let at = match self.packet.access_at {
            Some(t) => t,
            None => { let t = now + 0.1 * (self.packet.rand() / 0.25).floor().min(6.0); self.packet.access_at = Some(t); t }
        };
        if now < at { return; }
        self.packet.access_at = None;
        let mut frames = self.packet.link.take_out();
        frames.append(&mut self.packet.ui_queue);
        for f in &frames { self.packet.mon(f, 2); }
        self.packet.sending = frames.iter().map(|f| f.to_bytes()).collect();
        for b in std::mem::take(&mut self.packet.kiss_q) {
            if let Some(f) = Frame::from_bytes(&b) { self.packet.mon(&f, 2); }
            self.packet.sending.push(b);
        }
        self.start_tx(TxKind::Packet);
    }
    /// Aussendung fertig: ab jetzt läuft die Wartezeit auf die Antwort
    pub(crate) fn packet_sent(&mut self) {
        let now = self.packet.now();
        self.packet.link.tx_done(now);
        self.packet.sending.clear();
    }

    fn packet_connect(&mut self) {
        if self.station.call.is_empty() { self.log("Packet: zuerst unter Einstellungen › Station das Rufzeichen eintragen"); return; }
        match parse_target(&self.packet.target) {
            Some((dst, via)) => {
                let now = self.packet.now();
                self.packet.link.me = ax25::norm_call(&self.packet_call());
                self.packet.link.p = self.packet.p;
                self.packet.link.connect(&dst, &via, now);
                self.packet.outgoing = true;
                self.packet.info(&format!("Verbinde mit {dst}{} …", if via.is_empty() { String::new() } else { format!(" über {}", via.join(", ")) }));
            }
            None => self.log("Packet: Ziel als Rufzeichen eingeben, z. B. „DB0XYZ“ oder „DB0XYZ via DB0AAA“"),
        }
    }
    fn packet_input(&mut self) {
        let line = std::mem::take(&mut self.packet.input);
        let now = self.packet.now();
        if self.packet.link.state == State::Connected {
            let t = crate::template::expand(&line, &self.macros(false));
            self.packet.link.send(format!("{t}\r").as_bytes(), now);
            self.packet.term_push(&format!("{t}\n"), 1);
        } else {
            self.packet_unproto(&line);
        }
    }
    /// Text ohne Verbindung als UI-Rahmen senden
    fn packet_unproto(&mut self, text: &str) {
        if self.station.call.is_empty() { self.log("Packet: zuerst unter Einstellungen › Station das Rufzeichen eintragen"); return; }
        let to = self.packet.un_to.trim().to_uppercase();
        let via: Vec<String> = self.packet.un_via.to_uppercase().replace(',', " ").split_whitespace().map(|s| s.to_string()).collect();
        if !ax25::valid_call(&to) || !via.iter().all(|d| ax25::valid_call(d)) { self.log("Packet: Unproto-Ziel bzw. Digipeater prüfen"); return; }
        let t = crate::template::expand(text, &self.macros(false));
        let mut f = Frame::new(&self.packet_call(), &to, &via, true, Kind::Ui, false);
        f.info = format!("{t}\r").into_bytes();
        self.packet.ui_queue.push(f);
        self.packet.term_push(&format!("[Unproto an {to}] {t}\n"), 1);
    }

    /// Schnellwahl der Packet-Frequenzen
    pub(crate) fn ui_packet_bands(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Packet-Radio-Frequenzen").strong());
        egui::Grid::new("pk_f").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
            for (k, sb, sp, label, tip) in PACKET_LIST {
                if ui.add_enabled(self.rig.is_some(), egui::Button::new(RichText::new(format!("{label} {}", if sb == cat::Sb::Fm { "FM" } else { "" })).monospace()))
                    .on_hover_text(format!("{tip} – {}", sp.label())).clicked() {
                    if !self.packet.active() { self.packet.speed = sp; }
                    self.tune_to(k, Some(sb));
                    ui.close();
                }
                ui.label(RichText::new(format!("{} · {tip}", sp.label())).small().weak());
                ui.end_row();
            }
        });
        ui.label(RichText::new("Digipeater und Benutzereinstiege in deiner Nähe stehen in den Relaislisten.").small().weak());
    }

    pub(crate) fn ui_packet_rx(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let busy = self.rx_sh.packet_dcd.load(std::sync::atomic::Ordering::Relaxed);
        ui.horizontal_wrapped(|ui| {
            ui.heading("Packet Radio");
            ui.add_enabled_ui(self.tx.is_none() && !self.packet.active(), |ui| {
                for (s, label, tip) in Speed::ALL {
                    if ui.selectable_label(self.packet.speed == s, label).on_hover_text(tip).clicked() { self.packet.speed = s; }
                }
            });
            ui.label(RichText::new(if busy { "● belegt" } else { "○ frei" }).small().color(if busy { ORANGE } else { Color32::from_gray(110) }))
                .on_hover_text("Kanalbelegung: gesendet wird nur, wenn der Kanal frei ist");
            ui.add(egui::ProgressBar::new((self.level * 2.0).min(1.0)).desired_width(80.0).desired_height(8.0)).on_hover_text("Eingangspegel");
            ui.menu_button("…", |ui| {
                if ui.button("Monitor leeren").clicked() { self.packet.monitor.clear(); }
                if ui.button("WAV dekodieren …").clicked() { self.pick_wav(ctx); }
            }).response.on_hover_text("weitere Funktionen");
        });
        if self.packet.speed == Speed::B9600 {
            ui.label(RichText::new("9600 Bd braucht flaches Audio: den 9600-Bd-Datenanschluss oder die ZF-Ausgabe des Funkgeräts mit „ZF-Empfang“ \
                (Audio-Menü oben), und eine Soundkarte bzw. WLAN-Audio mit 44,1 oder 48 kHz.").small().color(ORANGE));
        }
        if let Some(t) = self.silent_since.filter(|t| t.elapsed() > std::time::Duration::from_secs(20) && self.tx.is_none()) {
            ui.label(RichText::new(format!("Seit {} s kommt vom Funkgerät nur Stille – ist die Rauschsperre zu? Für Packet die Rauschsperre öffnen.", t.elapsed().as_secs())).small().color(ORANGE));
        }
        if self.packet.speed != Speed::B300 { self.ui_if_hint(ui); }
        ui.horizontal(|ui| {
            ui.label(RichText::new("Monitor").strong());
            ui.checkbox(&mut self.packet.mine_only, "nur an/von mir");
        });
        let h = (ui.ctx().content_rect().height() * 0.55).max(180.0);
        let mut pick = None;
        egui::Frame::new().fill(Color32::from_gray(12)).inner_margin(6.0).corner_radius(4.0).show(ui, |ui| {
            ui.set_min_height(h);
            egui::ScrollArea::vertical().id_salt("pk_mon").stick_to_bottom(true).max_height(h).auto_shrink([false, false]).show(ui, |ui| {
                if self.packet.monitor.is_empty() {
                    ui.label(RichText::new("Noch nichts gehört. Frequenz über die Frequenzanzeige oben wählen und Audio starten.").weak());
                }
                for l in self.packet.monitor.iter().filter(|l| !self.packet.mine_only || l.kind > 0) {
                    let c = match l.kind { 2 => ORANGE, 1 => GREEN, _ => Color32::from_gray(215) };
                    let r = ui.add(egui::Label::new(RichText::new(format!("{} {}", l.time, l.text)).monospace().size(12.0).color(c)).wrap().sense(egui::Sense::click()));
                    if r.on_hover_text("Klick: Absender als Verbindungsziel übernehmen").clicked() { pick = Some(l.src.clone()); }
                }
            });
        });
        if let Some(s) = pick { if !self.packet.active() { self.packet.target = s; } }
    }

    pub(crate) fn ui_packet_tx(&mut self, ui: &mut egui::Ui) {
        let st = self.packet.link.state;
        ui.horizontal_wrapped(|ui| {
            ui.label("Ziel");
            ui.add_enabled(st == State::Disconnected, egui::TextEdit::singleline(&mut self.packet.target).desired_width(200.0)
                .hint_text("DB0XYZ via DB0AAA")).on_hover_text("Rufzeichen der Mailbox, des Knotens oder der Station, optional „via“ und Digipeater");
            match st {
                State::Disconnected => {
                    let ok = parse_target(&self.packet.target).is_some();
                    if ui.add_enabled(ok, egui::Button::new(RichText::new("Verbinden").color(Color32::BLACK).strong()).fill(ORANGE)).clicked() { self.packet_connect(); }
                }
                _ => {
                    if ui.add(egui::Button::new(RichText::new("Trennen").color(Color32::WHITE).strong()).fill(crate::RED)).clicked() {
                        let now = self.packet.now();
                        self.packet.link.disconnect(now);
                        self.packet_events();
                    }
                }
            }
        });
        let (vs, vr, un, retry) = self.packet.link.status();
        let (txt, col) = match st {
            State::Disconnected => (format!("nicht verbunden · ich bin {}", if self.station.call.is_empty() { "– (Rufzeichen fehlt)".into() } else { self.packet_call() }), Color32::from_gray(150)),
            State::Connecting => (format!("verbinde mit {} … Versuch {}", self.packet.link.peer, retry + 1), ORANGE),
            State::Connected => (format!("verbunden mit {}{} · gesendet {vs}, empfangen {vr}, offen {un}{}", self.packet.link.peer,
                if self.packet.link.via.is_empty() { String::new() } else { format!(" über {}", self.packet.link.via.join(",")) },
                if retry > 0 { format!(" · Wiederholung {retry}") } else { String::new() }), GREEN),
            State::Disconnecting => ("trenne …".into(), ORANGE),
        };
        ui.label(RichText::new(txt).small().color(col));
        let h = (ui.ctx().content_rect().height() * 0.38).max(150.0);
        egui::Frame::new().fill(Color32::from_gray(12)).inner_margin(6.0).corner_radius(4.0).show(ui, |ui| {
            ui.set_min_height(h);
            egui::ScrollArea::vertical().id_salt("pk_term").stick_to_bottom(true).max_height(h).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for l in &self.packet.term {
                    let c = match l.kind { 1 => ORANGE, 2 => Color32::from_gray(140), _ => Color32::from_gray(230) };
                    ui.add(egui::Label::new(RichText::new(if l.text.is_empty() { " " } else { &l.text }).monospace().color(c)).wrap());
                }
            });
        });
        let target = if st == State::Connected { format!("an {}", self.packet.link.peer) } else { format!("Unproto an {}", self.packet.un_to) };
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.packet.input).desired_width(ui.available_width() - 90.0)
                .font(egui::TextStyle::Monospace).hint_text(format!("Text {target} – Enter sendet")));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Senden").clicked() || enter) && !self.packet.input.trim().is_empty() {
                self.packet_input();
                r.request_focus();
            }
        });
        let pending = self.packet.link.pending();
        if pending > 0 || self.tx.as_ref().is_some_and(|t| t.kind == TxKind::Packet) {
            ui.label(RichText::new(format!("{pending} Bytes warten auf Bestätigung{}", if self.tx.is_some() { " · sende …" } else { "" })).small().weak());
        }
        self.ui_gain(ui).on_hover_text("bei FM bestimmt der Pegel den Hub: 1200 Bd etwa ±3 kHz, 9600 Bd etwa ±3 kHz am Datenanschluss");
        egui::CollapsingHeader::new("Einstellungen (TNC-Parameter)").id_salt("pk_set").show(ui, |ui| {
            egui::Grid::new("pk_set_g").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("SSID");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.packet.ssid).desired_width(30.0)).on_hover_text("0–15; leer = ohne SSID");
                    ui.label(RichText::new(format!("ergibt {}", self.packet_call())).small().weak());
                });
                ui.end_row();
                ui.label("PACLEN");
                ui.add(egui::DragValue::new(&mut self.packet.p.paclen).range(16..=256)).on_hover_text("Bytes je Rahmen: 1200 Bd 128–256, Kurzwelle 40–60");
                ui.end_row();
                ui.label("MAXFRAME");
                ui.add(egui::DragValue::new(&mut self.packet.p.maxframe).range(1..=7)).on_hover_text("Rahmen ohne Bestätigung am Stück: 1200 Bd 2–4, Kurzwelle 1");
                ui.end_row();
                ui.label("FRACK");
                ui.add(egui::DragValue::new(&mut self.packet.p.frack).range(1.0..=30.0).speed(0.1).suffix(" s")).on_hover_text("Wartezeit auf eine Antwort, je Digipeater mal 2 dazu; Kurzwelle 5–8 s");
                ui.end_row();
                ui.label("RETRIES");
                ui.add(egui::DragValue::new(&mut self.packet.p.retries).range(1..=30)).on_hover_text("Wiederholungen, bevor die Verbindung als verloren gilt");
                ui.end_row();
                ui.label("TXDELAY");
                ui.add(egui::DragValue::new(&mut self.packet.txdelay_ms).range(10.0..=1000.0).speed(5.0).suffix(" ms")).on_hover_text("Vorlauf nach dem Hochtasten, bis das Funkgerät sendet und die Gegenstelle eingerastet ist");
                ui.end_row();
                ui.label("Eingehend");
                ui.checkbox(&mut self.packet.p.accept, "Verbindungen annehmen");
                ui.end_row();
                ui.label("Begrüßung");
                ui.add(egui::TextEdit::singleline(&mut self.packet.ctext).desired_width(260.0)).on_hover_text("wird bei eingehenden Verbindungen gesendet; Makros wie %m, %o, %a, %l");
                ui.end_row();
                ui.label("Unproto");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.packet.un_to).desired_width(70.0)).on_hover_text("Ziel für Text ohne Verbindung, z. B. CQ, ID oder QST");
                    ui.label("via");
                    ui.add(egui::TextEdit::singleline(&mut self.packet.un_via).desired_width(140.0).hint_text("Digipeater"));
                });
                ui.end_row();
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ziel_lesen() {
        assert_eq!(parse_target("db0xyz"), Some(("DB0XYZ".into(), vec![])));
        assert_eq!(parse_target("DB0XYZ-7 via DB0AAA, DB0BBB-2"), Some(("DB0XYZ-7".into(), vec!["DB0AAA".into(), "DB0BBB-2".into()])));
        assert_eq!(parse_target("DB0XYZ v DB0AAA"), Some(("DB0XYZ".into(), vec!["DB0AAA".into()])));
        assert_eq!(parse_target(""), None);
        assert_eq!(parse_target("DB0XYZ-16"), None);
        assert_eq!(parse_target("ZUVIELEZEICHEN"), None);
    }
}

// ---------------- TNC für andere Programme (KISS/AGW) ----------------

impl App {
    /// Server nach den Einstellungen starten bzw. beenden
    pub(crate) fn kiss_tick(&mut self) {
        let key = (self.kiss_port, self.agw_port, self.kiss_lan);
        if key == self.kiss_key && (self.kiss.is_some() || (key.0 == 0 && key.1 == 0)) { return; }
        if key == self.kiss_key && self.kiss.is_none() { return; }
        self.kiss_key = key;
        self.kiss = None; // alten Server erst schließen (Ports frei)
        if key.0 == 0 && key.1 == 0 { return; }
        let name = format!("Rustwave {}", env!("CARGO_PKG_VERSION"));
        let srv = crate::kiss::Server::start(key.0, key.1, key.2, name, self.ev_tx.clone());
        self.log(format!("TNC für andere Programme: {}", srv.status));
        self.kiss = Some(srv);
    }

    /// Rahmen eines anderen Programms senden – über den APRS- bzw. Packet-Reiter
    pub(crate) fn kiss_tx(&mut self, b: Vec<u8>) {
        match self.op {
            crate::Op::Packet => self.packet.kiss_q.push(b),
            crate::Op::Aprs => self.aprs.kiss_q.push(b),
            _ => self.log("KISS/AGW: Rahmen verworfen – zum Senden den APRS- oder Packet-Reiter öffnen"),
        }
    }

    /// Einstellungen › TNC
    pub(crate) fn ui_set_tnc(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Andere Programme (Winlink Express, YAAC, APRSIS32, Xastir, Outpost, BPQ32 …) können Rustwave als \
            Software-TNC nutzen – auch über das WLAN des IC-705. Rustwave gibt alle empfangenen Rahmen weiter und sendet die der Programme \
            über das Modem des gerade offenen Reiters: APRS (1200 Bd FM bzw. 300 Bd KW) oder Packet (300/1200/9600 Bd).").small().weak());
        ui.add_space(6.0);
        egui::Grid::new("tnc_set").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            let mut on = self.kiss_port != 0;
            ui.checkbox(&mut on, "KISS über TCP").on_hover_text("Wie Dire Wolf: Port 8001. Verbindungen (Winlink, BBS) macht das Programm selbst.");
            if on && self.kiss_port == 0 { self.kiss_port = 8001; } else if !on { self.kiss_port = 0; }
            ui.add_enabled(on, egui::DragValue::new(&mut self.kiss_port).range(1024..=65535).prefix("Port "));
            ui.end_row();
            let mut on = self.agw_port != 0;
            ui.checkbox(&mut on, "AGW (AGWPE)").on_hover_text("Port 8000. Rustwave kann hier Monitor, Rohrahmen und Unproto – \
                für Verbindungen im Programm KISS wählen.");
            if on && self.agw_port == 0 { self.agw_port = 8000; } else if !on { self.agw_port = 0; }
            ui.add_enabled(on, egui::DragValue::new(&mut self.agw_port).range(1024..=65535).prefix("Port "));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut self.kiss_lan, "auch aus dem Netzwerk erreichbar").on_hover_text(
                "Sonst nur Programme auf diesem PC (127.0.0.1). Im Netzwerk kann jeder Rechner über deine Station senden – nur im eigenen Heimnetz einschalten.");
            ui.end_row();
        });
        if let Some(k) = &self.kiss {
            ui.add_space(6.0);
            ui.label(RichText::new(format!("{} · {} Programm(e) verbunden · {} Rahmen weitergegeben · {} zum Senden erhalten",
                k.status, k.clients(), k.n_rx.load(std::sync::atomic::Ordering::Relaxed), k.n_tx.load(std::sync::atomic::Ordering::Relaxed))).small().color(crate::GREEN));
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Beispiele: Winlink Express › Packet WL2K › TNC-Typ „KISS“, Verbindung TCP 127.0.0.1:8001 (TNC-Initialisierung leer). \
            YAAC › Ports › KISS over TCP. APRSIS32 › Configure › Ports › KISS (TCP).").small().weak());
    }
}
