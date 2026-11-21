//! POCSAG-Oberfläche (Funkruf, z. B. DAPNET): empfangene Rufe als Liste oder Monitor im Stil von multimon-ng,
//! Dekoder-Einstellungen wie die multimon-ng-Schalter, Frequenz-Schnellwahl und Senden eines Funkrufs.

use crate::dapnet;
use crate::pocsag::{self, Charset, Content, Format, Page, Polarity};
use crate::{cat, App, TxKind, GREEN, ORANGE};
use eframe::egui::{self, Color32, RichText};
use std::io::Write;

/// POCSAG-Frequenzen im Amateurfunk (kHz, Bezeichnung, Hinweis) – nach der DAPNET-Frequenzliste (hampager.de).
/// DAPNET sendet mit 1200 Baud.
pub const POCSAG_LIST: [(f64, &str, &str); 11] = [
    (439987.5, "439,9875", "DAPNET-Standard: DL, HB9, PA, ON, F, G, OK, SP, HA, LA, CT, S5, LZ, RU, USA, Kanada"),
    (438025.0, "438,025", "Österreich (DAPNET in OE)"),
    (439930.0, "439,930", "Österreich, laut Bandplan ebenfalls möglich"),
    (433625.0, "433,625", "Irland"),
    (144862.5, "144,8625", "Luxemburg, Spanien, Grenzregion DL/LX"),
    (144937.5, "144,9375", "Italien"),
    (144512.5, "144,5125", "Italien"),
    (144975.0, "144,975", "Russland"),
    (144280.0, "144,280", "China"),
    (144390.0, "144,390", "Thailand"),
    (144412.5, "144,4125", "Thailand"),
];
const MAX_MSGS: usize = 500;

/// eine Zeile der Liste: Uhrzeit, Nachricht, selbst gesendet?
pub struct RxMsg { pub time: String, pub m: pocsag::Msg, pub tx: bool }

/// Art des zu sendenden Rufs
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TxKindP { Alpha, Numeric, Tone }

pub struct PocsagState {
    // Einstellungen (werden gespeichert)
    pub p: pocsag::Params,
    /// eigene RICs (durch Komma getrennt): werden grün hervorgehoben und lassen sich filtern
    pub my_rics: String,
    pub tx_ric: String,
    pub tx_func: u8,
    pub tx_kind: TxKindP,
    pub tx_baud: u32,
    pub tx_text: String,
    pub tx_invert: bool,
    /// eigenes Rufzeichen an alphanumerische Rufe anhängen (Stationskennung)
    pub tx_call: bool,
    /// Mitschrift in eine Textdatei je Tag
    pub logfile: bool,
    // Laufzeit
    pub msgs: Vec<RxMsg>,
    pub filter: String,
    pub only_mine: bool,
    /// Monitor (Zeilen wie multimon-ng) statt Liste
    pub monitor: bool,
    pub sending: Vec<Page>,
    /// seit wann der Dekoder läuft (für die Diagnose-Hinweise)
    pub since: Option<std::time::Instant>,
    /// Eingabefeld für die Skyper-RICs
    pub sky_text: String,
    /// Baudrate und Zeichensatz der laufenden Aussendung (eigener Ruf bzw. DAPNET-Rufe)
    pub sending_baud: u32,
    pub sending_cs: pocsag::Charset,
    /// DAPNET-Sender: Rufzeichen und AuthKey des Senders, Kern, RIC-Filter
    pub dn_call: String,
    pub dn_auth: String,
    pub dn_server: String,
    pub dn_white: String,
    pub dn_link: Option<crate::dapnet_tx::Link>,
    pub dn_host: Option<String>,
    pub dn_state: String,
    pub dn_slots: Option<[bool; 16]>,
    pub dn_queue: std::collections::VecDeque<crate::dapnet_tx::Call>,
    pub dn_got: u32,
    pub dn_sent: u32,
    pub dn_tx: bool,
    /// nach misslungenem Senden (Audio/PTT) erst später wieder versuchen
    pub dn_retry: Option<std::time::Instant>,
    /// Ruf ins Netz über hampager.de: Anmeldung, Empfänger, Sendergruppen, Text, laufende Anfrage
    pub api_user: String,
    pub api_pass: String,
    pub api_url: String,
    pub api_to: String,
    pub api_groups: String,
    pub api_text: String,
    pub api_busy: Option<std::sync::mpsc::Receiver<Result<u16, String>>>,
    pub api_msg: Option<(bool, String)>,
    /// gehörte DAPNET-Sender (aus der Senderkennung auf RIC 8) und der gerade laufende Durchgang
    pub dn_heard: Vec<HeardTx>,
    pub dn_cur: Option<(usize, std::time::Instant)>,
    /// DAPNET-Ansicht statt Liste/Monitor
    pub dapnet: bool,
    /// Zeitsignale (RIC 200/208/216/224, optional 2504) in Liste und Monitor ausblenden
    pub hide_time: bool,
    /// Dienst-RICs (Senderkennung, Skyper-Rubriknamen) ausblenden
    pub hide_svc: bool,
    /// eigene Region (Sendergruppen) für die Rubrik-Bezeichnungen, z. B. „dl-all, dl-by“
    pub region: String,
    /// empfangene Rubrik-Nachrichten
    pub news: Vec<dapnet::News>,
    /// gewählte Rubrik in der DAPNET-Ansicht (None = alle) und Suche in der Rubrikliste
    pub rub_sel: Option<u8>,
}

impl PocsagState {
    pub fn load(get: &dyn Fn(&str) -> Option<String>) -> Self {
        let mut s = PocsagState {
            p: pocsag::Params::default(), my_rics: String::new(), tx_ric: String::new(), tx_func: 3, tx_kind: TxKindP::Alpha,
            tx_baud: 1200, tx_text: String::new(), tx_invert: false, tx_call: true, logfile: false,
            msgs: vec![], filter: String::new(), only_mine: false, monitor: false, sending: vec![], since: None, sky_text: String::new(),
            sending_baud: 1200, sending_cs: pocsag::Charset::Us,
            dn_call: String::new(), dn_auth: String::new(), dn_server: crate::dapnet_tx::DEFAULT_SERVER.into(), dn_white: String::new(),
            dn_link: None, dn_host: None, dn_state: String::new(), dn_slots: None, dn_queue: Default::default(), dn_got: 0, dn_sent: 0, dn_tx: false, dn_retry: None,
            api_user: String::new(), api_pass: String::new(), api_url: crate::dapnet_tx::DEFAULT_API.into(), api_to: String::new(),
            api_groups: "dl-all".into(), api_text: String::new(), api_busy: None, api_msg: None,
            dn_heard: vec![], dn_cur: None,
            dapnet: false, hide_time: true, hide_svc: true, region: String::new(), news: vec![], rub_sel: None,
        };
        s.apply_settings(get);
        s
    }
    pub fn apply_settings(&mut self, get: &dyn Fn(&str) -> Option<String>) {
        let flag = |k: &str, d: bool| get(k).map_or(d, |v| v == "1");
        let d = pocsag::Params::default();
        let rates: Vec<bool> = get("poc_rates").map(|v| v.split(',').map(|x| x == "1").collect()).unwrap_or_default();
        self.p = pocsag::Params {
            rates: if rates.len() == 3 { [rates[0], rates[1], rates[2]] } else { d.rates },
            format: get("poc_fmt").and_then(|v| Format::from_key(&v)).unwrap_or(d.format),
            charset: get("poc_cs").and_then(|v| Charset::ALL.iter().find(|c| c.1 == v).map(|c| c.0)).unwrap_or(d.charset),
            ecc: get("poc_ecc").and_then(|v| v.parse().ok()).filter(|&e: &u8| e <= 2).unwrap_or(d.ecc),
            polarity: get("poc_pol").and_then(|v| Polarity::ALL.iter().find(|p| p.1 == v).map(|p| p.0)).unwrap_or(d.polarity),
            prune_empty: flag("poc_e", d.prune_empty),
            heuristic: flag("poc_u", d.heuristic),
            partial: flag("poc_p", d.partial),
            skyper: get("poc_sky").map(|v| parse_rics(&v)).unwrap_or(d.skyper),
        };
        self.sky_text = sky_str(&self.p.skyper);
        self.my_rics = get("poc_rics").unwrap_or_default();
        self.hide_time = flag("poc_hide_time", true);
        let gs = |k: &str, d: &str| get(k).filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string());
        self.dn_call = gs("dn_call", ""); self.dn_auth = gs("dn_auth", ""); self.dn_server = gs("dn_server", crate::dapnet_tx::DEFAULT_SERVER);
        self.dn_white = gs("dn_white", "");
        self.api_user = gs("api_user", ""); self.api_pass = gs("api_pass", ""); self.api_url = gs("api_url", crate::dapnet_tx::DEFAULT_API);
        self.api_to = gs("api_to", ""); self.api_groups = gs("api_groups", "dl-all");
        self.hide_svc = flag("poc_hide_svc", true);
        self.dapnet = get("poc_view").as_deref() == Some("dapnet");
        self.monitor = get("poc_view").as_deref() == Some("monitor");
        self.region = get("poc_region").unwrap_or_else(|| "dl-all".into());
        self.tx_ric = get("poc_ric").unwrap_or_default();
        self.tx_func = get("poc_func").and_then(|v| v.parse().ok()).filter(|&f: &u8| f <= 3).unwrap_or(3);
        self.tx_kind = match get("poc_kind").as_deref() { Some("num") => TxKindP::Numeric, Some("ton") => TxKindP::Tone, _ => TxKindP::Alpha };
        self.tx_baud = get("poc_baud").and_then(|v| v.parse().ok()).filter(|b| pocsag::BAUDS.contains(b)).unwrap_or(1200);
        self.tx_text = get("poc_text").unwrap_or_default();
        self.tx_invert = flag("poc_inv", false);
        self.tx_call = flag("poc_call", true);
        self.logfile = flag("poc_log", false);
    }
    pub fn save(&self, s: &mut dyn eframe::Storage) {
        let b = |v: bool| if v { "1" } else { "0" }.to_string();
        s.set_string("poc_rates", self.p.rates.iter().map(|&r| b(r)).collect::<Vec<_>>().join(","));
        s.set_string("poc_fmt", self.p.format.key().into());
        s.set_string("poc_cs", self.p.charset.label().into());
        s.set_string("poc_ecc", self.p.ecc.to_string());
        s.set_string("poc_pol", self.p.polarity.label().into());
        s.set_string("poc_e", b(self.p.prune_empty));
        s.set_string("poc_u", b(self.p.heuristic));
        s.set_string("poc_p", b(self.p.partial));
        s.set_string("poc_sky", sky_str(&self.p.skyper));
        s.set_string("poc_rics", self.my_rics.clone());
        s.set_string("poc_hide_time", b(self.hide_time));
        for (k, v) in [("dn_call", &self.dn_call), ("dn_auth", &self.dn_auth), ("dn_server", &self.dn_server), ("dn_white", &self.dn_white),
                       ("api_user", &self.api_user), ("api_pass", &self.api_pass), ("api_url", &self.api_url), ("api_to", &self.api_to), ("api_groups", &self.api_groups)] {
            s.set_string(k, v.clone());
        }
        s.set_string("poc_hide_svc", b(self.hide_svc));
        s.set_string("poc_view", if self.dapnet { "dapnet" } else if self.monitor { "monitor" } else { "list" }.into());
        s.set_string("poc_region", self.region.clone());
        s.set_string("poc_ric", self.tx_ric.clone());
        s.set_string("poc_func", self.tx_func.to_string());
        s.set_string("poc_kind", match self.tx_kind { TxKindP::Numeric => "num", TxKindP::Tone => "ton", TxKindP::Alpha => "alpha" }.into());
        s.set_string("poc_baud", self.tx_baud.to_string());
        s.set_string("poc_text", self.tx_text.clone());
        s.set_string("poc_inv", b(self.tx_invert));
        s.set_string("poc_call", b(self.tx_call));
        s.set_string("poc_log", b(self.logfile));
    }
    fn mine(&self) -> Vec<u32> { self.my_rics.split([',', ' ', ';']).filter_map(|x| x.trim().parse().ok()).collect() }
    /// entsprechende multimon-ng-Befehlszeile (zur Orientierung für Umsteiger)
    pub fn multimon_cmd(&self) -> String {
        let mut c = String::from("multimon-ng");
        for (b, on) in pocsag::BAUDS.iter().zip(self.p.rates) { if on { c += &format!(" -a POCSAG{b}"); } }
        match self.p.format {
            Format::Standard => {}
            Format::Auto => c += " -f auto",
            Format::Numeric => c += " -f numeric",
            Format::Alpha => c += " -f alpha",
            Format::Skyper => c += " -f skyper",
        }
        if self.p.ecc != 2 { c += &format!(" -b {}", self.p.ecc); }
        if self.p.charset != Charset::Us { c += &format!(" -C {}", self.p.charset.label()); }
        match self.p.polarity { Polarity::Auto => {}, Polarity::Normal => c += " -P normal", Polarity::Inverted => c += " -P inverted" }
        if self.p.prune_empty { c += " -e"; }
        if self.p.heuristic { c += " -u"; }
        if self.p.partial { c += " -p"; }
        c
    }
    /// Funkruf aus den Eingaben; Fehlertext, wenn etwas nicht passt
    pub fn page(&self, my_call: &str) -> Result<Page, String> {
        let addr: u32 = self.tx_ric.trim().parse().map_err(|_| "RIC als Zahl eingeben (0–2097151)".to_string())?;
        if addr > pocsag::MAX_RIC { return Err(format!("RIC höchstens {}", pocsag::MAX_RIC)); }
        let text = self.tx_text.trim();
        let content = match self.tx_kind {
            TxKindP::Tone => Content::Tone,
            TxKindP::Numeric => {
                if let Some(c) = text.chars().find(|c| !"0123456789 -.U[]u".contains(*c)) {
                    return Err(format!("„{c}“ geht numerisch nicht – erlaubt sind 0–9, Leerzeichen, - . U [ ]"));
                }
                Content::Numeric(text.into())
            }
            TxKindP::Alpha => {
                let mut t = text.to_string();
                if self.tx_call && !my_call.is_empty() && !t.to_uppercase().contains(&my_call.to_uppercase()) {
                    t = if t.is_empty() { my_call.to_string() } else { format!("{t} de {my_call}") };
                }
                if t.is_empty() { return Err("Text eingeben".into()); }
                Content::Alpha(t)
            }
        };
        Ok(Page { addr, func: self.tx_func, content })
    }
}

/// ein gehörter DAPNET-Sender
#[derive(Clone, Debug, Default)]
pub struct HeardTx {
    pub call: String,
    pub idents: u32,
    pub first: i64,
    pub last: i64,
    /// Rufe im selben Durchgang nach der Kennung (geschätzt) und davon fehlerfrei empfangene
    pub calls: u32,
    pub clean: u32,
    /// Kennungen ohne Bitfehler
    pub ident_clean: u32,
    /// S-Meter bei der letzten Kennung, höchster und Summe/Zahl für den Mittelwert (in dB über S0)
    pub s_last: Option<u32>,
    pub s_max: Option<u32>,
    pub s_sum_db: f32,
    pub s_n: u32,
}

/// Funktionsbits 0–3; auf Pagern meist als A–D bezeichnet
fn func_label(f: u8) -> String { format!("{f} ({})", (b'A' + f) as char) }

impl App {
    /// empfangener Funkruf
    pub(crate) fn pocsag_received(&mut self, m: pocsag::Msg) {
        let now = chrono::Local::now();
        if self.pocsag.logfile {
            let path = crate::save_dir().join(format!("pocsag-{}.txt", now.format("%Y%m%d")));
            let r = std::fs::create_dir_all(crate::save_dir()).and_then(|_| {
                let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
                writeln!(f, "{} {}", now.format("%Y-%m-%d %H:%M:%S"), m.multimon_line())
            });
            if let Err(e) = r { self.log(format!("POCSAG-Mitschrift: {e}")); }
        }
        let time = now.format("%H:%M:%S").to_string();
        dapnet::add_news(&mut self.pocsag.news, &time, &m);
        self.dapnet_heard(&m);
        let mine = m.addr.is_some_and(|a| self.pocsag.mine().contains(&a));
        if mine { self.log(format!("Funkruf an deine RIC {}: {}", m.addr.unwrap_or(0), m.clean_text())); }
        self.pocsag.msgs.push(RxMsg { time: now.format("%H:%M:%S").to_string(), m, tx: false });
        let n = self.pocsag.msgs.len();
        if n > MAX_MSGS { self.pocsag.msgs.drain(..n - MAX_MSGS); }
    }

    /// Senderkennung (RIC 8) auswerten und folgende Rufe desselben Durchgangs dem Sender zuordnen
    fn dapnet_heard(&mut self, m: &pocsag::Msg) {
        let p = &mut self.pocsag;
        let t = chrono::Utc::now().timestamp();
        let clean = m.corrected() == 0;
        if m.addr == Some(8) && m.kind != pocsag::Kind::Tone {
            let call = m.clean_text().trim().to_lowercase();
            if call.is_empty() || call.len() > 12 || !call.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') { return; }
            let i = match p.dn_heard.iter().position(|h| h.call == call) {
                Some(i) => i,
                None => { p.dn_heard.push(HeardTx { call, first: t, ..Default::default() }); p.dn_heard.len() - 1 }
            };
            let lv = self.poll.s_recent(2.0);
            let h = &mut p.dn_heard[i];
            if let Some(v) = lv {
                h.s_last = Some(v);
                h.s_max = Some(h.s_max.map_or(v, |m| m.max(v)));
                h.s_sum_db += crate::rigpoll::s_db(v);
                h.s_n += 1;
            }
            h.idents += 1;
            h.last = t;
            if clean { h.ident_clean += 1; }
            p.dn_cur = Some((i, std::time::Instant::now()));
            return;
        }
        // gleicher Durchgang (geschätzt): bis ein Zeitschlitz (6,4 s) nach der Kennung
        if let Some((i, at)) = p.dn_cur {
            if at.elapsed().as_secs_f64() <= 6.4 {
                let h = &mut p.dn_heard[i];
                h.calls += 1;
                if clean { h.clean += 1; }
                h.last = t;
            } else {
                p.dn_cur = None;
            }
        }
    }

    pub(crate) fn pocsag_send(&mut self) {
        match self.pocsag.page(&self.station.call) {
            Ok(pg) => {
                self.pocsag.sending = vec![pg];
                self.pocsag.sending_baud = self.pocsag.tx_baud;
                self.pocsag.sending_cs = self.pocsag.p.charset;
                self.pocsag.dn_tx = false;
                self.start_tx(TxKind::Pocsag);
            }
            Err(e) => self.log(format!("POCSAG: {e}")),
        }
    }
    /// nach dem Senden: eigenen Ruf in die Liste
    pub(crate) fn pocsag_sent(&mut self) {
        let baud = self.pocsag.sending_baud;
        if std::mem::take(&mut self.pocsag.dn_tx) { self.pocsag.dn_sent += self.pocsag.sending.len() as u32; }
        for pg in std::mem::take(&mut self.pocsag.sending) {
            let (kind, text) = match pg.content {
                Content::Tone => (pocsag::Kind::Tone, String::new()),
                Content::Numeric(t) => (pocsag::Kind::Numeric, t),
                Content::Alpha(t) => (pocsag::Kind::Alpha, t),
            };
            let m = pocsag::Msg::sent(baud, pg.addr, pg.func, kind, text);
            self.pocsag.msgs.push(RxMsg { time: chrono::Local::now().format("%H:%M:%S").to_string(), m, tx: true });
        }
    }

    /// Schnellwahl der POCSAG-Frequenzen (FM)
    pub(crate) fn ui_pocsag_bands(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("POCSAG-Frequenzen (FM, DAPNET 1200 Bd)").strong());
        egui::Grid::new("poc_f").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
            for (k, label, tip) in POCSAG_LIST {
                if ui.add_enabled(self.rig.is_some(), egui::Button::new(RichText::new(format!("{label} FM")).monospace()))
                    .on_hover_text(tip).clicked() {
                    self.tune_to(k, Some(cat::Sb::Fm));
                    ui.close();
                }
                ui.label(RichText::new(tip).small().weak());
                ui.end_row();
            }
        });
    }

    pub(crate) fn ui_pocsag_rx(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let sync = self.rx_sh.pocsag_sync.load(std::sync::atomic::Ordering::Relaxed);
        ui.horizontal_wrapped(|ui| {
            ui.heading("Empfang POCSAG");
            for (i, b) in pocsag::BAUDS.iter().enumerate() {
                if !self.pocsag.p.rates[i] { continue; }
                let on = sync & (1 << i) != 0;
                ui.label(RichText::new(format!("{b}")).monospace().small().color(if on { GREEN } else { Color32::from_gray(90) }))
                    .on_hover_text(if on { "im Takt: Sync-Wort erkannt" } else { "kein Signal mit dieser Baudrate" });
            }
            ui.add(egui::ProgressBar::new((self.level * 2.0).min(1.0)).desired_width(90.0).desired_height(8.0)).on_hover_text("Eingangspegel");
            ui.menu_button("…", |ui| {
                if ui.button("Liste leeren").clicked() { self.pocsag.msgs.clear(); }
                if ui.button("WAV dekodieren …").clicked() { self.pick_wav(ctx); }
                if ui.button("Ordner mit Mitschriften öffnen").clicked() { self.open_save_dir(); }
            }).response.on_hover_text("weitere Funktionen");
        });
        // nur Stille vom Funkgerät: meist ist die Rauschsperre zu
        if let Some(t) = self.silent_since.filter(|t| t.elapsed() > std::time::Duration::from_secs(20) && self.tx.is_none()) {
            ui.label(RichText::new(format!("Seit {} s kommt vom Funkgerät nur Stille – ist die Rauschsperre (SQL) zu? Für POCSAG die Rauschsperre öffnen; \
                Rauschen erzeugt keine Fehlmeldungen.", t.elapsed().as_secs())).small().color(ORANGE));
        }
        self.ui_pocsag_diag(ui);
        ui.horizontal_wrapped(|ui| {
            for (v, label, tip) in [(0, "Liste", "Zeit, Baudrate, RIC, Funktion und Text"), (1, "Monitor", "Zeilen wie die Ausgabe von multimon-ng"),
                                    (2, "DAPNET", "Rubriken (Wetter, DX, Relais, Warnungen …) zusammengefasst, Rubrikliste und Dienst-RICs")] {
                let cur = if self.pocsag.dapnet { 2 } else { self.pocsag.monitor as i32 };
                if ui.selectable_label(cur == v, label).on_hover_text(tip).clicked() { self.pocsag.dapnet = v == 2; self.pocsag.monitor = v == 1; }
            }
            ui.separator();
            if !self.pocsag.dapnet {
                ui.add(egui::TextEdit::singleline(&mut self.pocsag.filter).desired_width(130.0).hint_text("Filter: RIC oder Text"));
                ui.checkbox(&mut self.pocsag.only_mine, "nur eigene RICs");
            }
            ui.checkbox(&mut self.pocsag.hide_time, "Zeitsignale ausblenden").on_hover_text(
                dapnet::TIME_RICS.iter().map(|(r, d)| format!("RIC {r}: {d}")).chain([format!("RIC {}: Zeitsignal numerisch (UTC)", dapnet::TIME_NUM_RIC)])
                    .collect::<Vec<_>>().join("\n"));
            ui.checkbox(&mut self.pocsag.hide_svc, "Dienst-RICs ausblenden").on_hover_text(
                dapnet::SERVICE_RICS.iter().map(|(r, d)| format!("RIC {r}: {d}")).collect::<Vec<_>>().join("\n"));
        });
        if self.pocsag.dapnet { self.ui_dapnet(ui); } else { self.ui_pocsag_list(ui); }

        egui::CollapsingHeader::new("Dekoder (wie multimon-ng)").id_salt("poc_set").show(ui, |ui| {
            let p = &mut self.pocsag.p;
            ui.horizontal_wrapped(|ui| {
                ui.label("Baudraten");
                for (i, b) in pocsag::BAUDS.iter().enumerate() {
                    ui.checkbox(&mut p.rates[i], format!("POCSAG{b}"));
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Ausgabe");
                egui::ComboBox::from_id_salt("poc_fmt").selected_text(p.format.label()).show_ui(ui, |ui| {
                    for (f, l, tip) in Format::ALL { ui.selectable_value(&mut p.format, f, l).on_hover_text(tip); }
                }).response.on_hover_text(Format::ALL.iter().find(|f| f.0 == p.format).map_or("", |f| f.2));
                ui.label("Zeichensatz");
                egui::ComboBox::from_id_salt("poc_cs").width(50.0).selected_text(p.charset.label()).show_ui(ui, |ui| {
                    for (c, l) in Charset::ALL { ui.selectable_value(&mut p.charset, c, l); }
                }).response.on_hover_text("nationale Zeichen statt [ \\ ] { | } ~ – DE: Ä Ö Ü ä ö ü ß");
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Fehlerkorrektur");
                egui::ComboBox::from_id_salt("poc_ecc").width(60.0).selected_text(format!("{} Bit", p.ecc)).show_ui(ui, |ui| {
                    for e in 0..=2u8 { ui.selectable_value(&mut p.ecc, e, format!("{e} Bit")); }
                }).response.on_hover_text("BCH-Korrektur je Codewort (-b): 2 korrigiert am meisten, 0 nur fehlerfreie Wörter");
                ui.label("Polarität");
                egui::ComboBox::from_id_salt("poc_pol").width(90.0).selected_text(p.polarity.label()).show_ui(ui, |ui| {
                    for (v, l) in Polarity::ALL { ui.selectable_value(&mut p.polarity, v, l); }
                }).response.on_hover_text("automatisch erkennt beides – je nach Funkgerät ist das Signal invertiert (-P)");
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut p.prune_empty, "leere ausblenden").on_hover_text("Rufe ohne Text (nur Ton) nicht anzeigen (-e)");
                ui.checkbox(&mut p.heuristic, "Unwahrscheinliches verwerfen").on_hover_text("Inhalte, die weder wie Ziffern noch wie Text aussehen, nicht anzeigen (-u)");
                ui.checkbox(&mut p.partial, "Teilempfang zeigen").on_hover_text("auch Rufe, bei denen der Takt mittendrin verloren ging (-p), mit … markiert");
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Skyper-RICs");
                let r = ui.add(egui::TextEdit::singleline(&mut self.pocsag.sky_text).desired_width(110.0).hint_text("4520"))
                    .on_hover_text("Alphanumerische Rufe an diese RICs werden im Standard-Format als Skyper (Zeichen um eins verschoben) gelesen \
                                    und der zweistellige Kopf abgetrennt. DAPNET verschlüsselt so nur die Skyper-Rubriken auf RIC 4520 (0x11A8); \
                                    dieselben Inhalte gehen unverschlüsselt an andere RICs. Leer = aus.");
                if r.changed() { self.pocsag.p.skyper = parse_rics(&self.pocsag.sky_text); }
                if r.lost_focus() { self.pocsag.sky_text = sky_str(&self.pocsag.p.skyper); }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Eigene RICs");
                ui.add(egui::TextEdit::singleline(&mut self.pocsag.my_rics).desired_width(160.0).hint_text("z. B. 123456, 2504"))
                    .on_hover_text("werden grün hervorgehoben und in der Statuszeile gemeldet");
                ui.checkbox(&mut self.pocsag.logfile, "Mitschrift in Datei")
                    .on_hover_text(format!("jede Zeile wie bei multimon-ng mit Datum in {}", crate::save_dir().join("pocsag-JJJJMMTT.txt").display()));
            });
            self.ui_if_switch(ui);
            ui.label(RichText::new(format!("entspricht: {}", self.pocsag.multimon_cmd())).monospace().small().weak());
            ui.label(RichText::new("Am besten das Audio vom Daten- bzw. 9600-Bd-Anschluss nehmen (ohne Deemphasis). Audio vom Lautsprecher geht auch: \
                Der Dekoder probiert dafür parallel einen Deemphasis- und Hochpass-Ausgleich.").small().weak());
        });
    }

    /// Diagnose: was der Dekoder hört – Präambeln, Sync, Codewörter – und ein Hinweis, woran es liegen kann
    fn ui_pocsag_diag(&mut self, ui: &mut egui::Ui) {
        let running = self.input.is_some() || self.net.is_some();
        if !running { self.pocsag.since = None; return; }
        let since = *self.pocsag.since.get_or_insert_with(std::time::Instant::now);
        let st = *self.rx_sh.pocsag_stats.lock().unwrap();
        let on: Vec<(u32, pocsag::Stats)> = pocsag::BAUDS.iter().zip(st).zip(self.pocsag.p.rates).filter(|(_, r)| *r).map(|((b, s), _)| (*b, s)).collect();
        let heard = on.iter().any(|(_, s)| s.preambles > 0 || s.syncs > 0);
        let msgs: u32 = on.iter().map(|(_, s)| s.msgs).sum();
        let words: u32 = on.iter().map(|(_, s)| s.ok + s.fixed + s.bad).sum();
        let bad: u32 = on.iter().map(|(_, s)| s.fixed + s.bad).sum();
        let mins = since.elapsed().as_secs() / 60;
        egui::CollapsingHeader::new(RichText::new(format!("Diagnose · läuft seit {mins} min · {}", if !heard { "noch kein POCSAG-Signal erkannt".to_string() }
                else if msgs == 0 { "Signal erkannt, aber nichts lesbar".to_string() } else { format!("{msgs} Rufe gelesen") })).small())
            .id_salt("poc_diag").show(ui, |ui| {
            egui::Grid::new("poc_diag_g").num_columns(6).spacing([12.0, 2.0]).show(ui, |ui| {
                for h in ["Baud", "Präambeln", "Sync", "Wörter gut", "korrigiert", "defekt"] { ui.label(RichText::new(h).small().strong()); }
                ui.end_row();
                for (b, s) in &on {
                    for v in [b.to_string(), s.preambles.to_string(), s.syncs.to_string(), s.ok.to_string(), s.fixed.to_string(), s.bad.to_string()] {
                        ui.label(RichText::new(v).small().monospace());
                    }
                    ui.end_row();
                }
            });
            ui.label(RichText::new("Präambel = ein POCSAG-Sender wurde gehört (Bitfolge 1010…). Sync = Anfang eines Datenblocks erkannt. \
                Die Wörter zeigen, wie sauber das Signal ankommt.").small().weak());
        });
        let hint = if !heard && since.elapsed().as_secs() > 300 {
            Some("In 5 Minuten kein POCSAG-Signal erkannt. Prüfen: Frequenz 439,9875 MHz FM, Rauschsperre offen, NF-Pegel (Balken oben bewegt sich bei Rauschen), \
                  ist ein DAPNET-Sender in Reichweite? (Senderkarte auf hampager.de). Ein POCSAG-Ruf ist als kurzes, raues Schnarren zu hören.")
        } else if heard && msgs == 0 && words > 20 && bad * 3 > words {
            Some("POCSAG-Signale kommen an, sind aber zu verzerrt zum Lesen. Der Entzerrer liest starke Sender auch über den NF-Weg \
                  (Deemphasis, Hochpass gegen CTCSS); schwächere gehen dort im Rauschen unter. Besser: Audio vom Daten- bzw. 9600-Bd-Anschluss \
                  bzw. Datenmodus (FM-D/DATA) mit breitem Filter, Tonsquelch/CTCSS aus. Eine kurze Aufnahme (⏺) hilft bei der Fehlersuche.")
        } else if heard && msgs == 0 && words == 0 && since.elapsed().as_secs() > 120 {
            Some("Präambeln erkannt, aber kein Sync-Wort: Das Signal ist zu schwach oder zu verzerrt. Audio vom Datenanschluss bzw. im Datenmodus (FM-D) versuchen.")
        } else { None };
        if f32::from_bits(self.rx_sh.clip.load(std::sync::atomic::Ordering::Relaxed)) > 0.5 {
            ui.label(RichText::new(format!("Eingang übersteuert ({:.1} ‰ der Samples stoßen an). Am Funkgerät den Ausgangspegel zurücknehmen: \
                SET > Connectors > {} AF/IF Output > {} Output Level.", f32::from_bits(self.rx_sh.clip.load(std::sync::atomic::Ordering::Relaxed)),
                if self.net.is_some() { "WLAN" } else { "USB" }, if self.if_rx { "IF" } else { "AF" })).small().color(ORANGE));
        }
        if let Some(h) = hint { ui.label(RichText::new(h).small().color(ORANGE)); }
        self.ui_if_hint(ui);
    }

    /// Hinweis, wenn ZF-Schalter und tatsächliches Signal nicht zusammenpassen; sonst Empfehlung
    pub(crate) fn ui_if_hint(&mut self, ui: &mut egui::Ui) {
        if self.input.is_none() && self.net.is_none() { return; }
        // kurz nach dem Umschalten passt die Messung noch nicht
        if self.if_switched.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(5)) { return; }
        let seen = self.rx_sh.if_seen.load(std::sync::atomic::Ordering::Relaxed);
        if self.if_managed() {
            if seen != self.if_rx {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("ZF wird automatisch geführt, das Signal sieht aber nach {} aus.", if seen { "ZF" } else { "NF" })).small().color(ORANGE));
                    if ui.small_button("Automatik aus, Signal folgen").clicked() { self.if_auto = false; self.if_rx = seen; }
                });
            }
            return;
        }
        if seen && !self.if_rx {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Das Funkgerät liefert offenbar ZF (12 kHz) statt NF.").small().color(ORANGE));
                if ui.small_button("ZF-Empfang einschalten").clicked() { self.if_rx = true; }
            });
        } else if !seen && self.if_rx {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("ZF-Empfang ist an, aber das Signal sieht nach NF aus – am Gerät Output Select = IF wählen oder hier ausschalten.").small().color(ORANGE));
                if ui.small_button("ZF-Empfang aus").clicked() { self.if_rx = false; }
            });
        } else if !self.if_rx && !self.if_managed() {
            ui.label(RichText::new("Tipp: Am besten liest POCSAG über die ZF-Ausgabe des Funkgeräts (Icom: SET > Connectors > WLAN/USB AF/IF Output > \
                Output Select = IF), dann hier „ZF-Empfang“ einschalten – ohne Deemphasis und Bass-Beschneidung des NF-Wegs.").small().weak());
        }
    }

    /// Liste bzw. Monitor der empfangenen Rufe
    fn ui_pocsag_list(&mut self, ui: &mut egui::Ui) {
        let mine = self.pocsag.mine();
        let f = self.pocsag.filter.trim().to_lowercase();
        let only_mine = self.pocsag.only_mine;
        let (hide_t, hide_s) = (self.pocsag.hide_time, self.pocsag.hide_svc);
        let region = dapnet::region_tokens(&self.pocsag.region);
        let show = |r: &&RxMsg| {
            !(!r.tx && dapnet::hidden(r.m.addr, hide_t, hide_s))
            && (!only_mine || r.m.addr.is_some_and(|a| mine.contains(&a)))
                && (f.is_empty() || r.m.addr.is_some_and(|a| a.to_string().contains(&f)) || r.m.text.to_lowercase().contains(&f))
        };
        let h = (ui.ctx().content_rect().height() * 0.42).max(160.0);
        let mut pick = None;
        egui::Frame::new().fill(Color32::from_gray(12)).inner_margin(6.0).corner_radius(4.0).show(ui, |ui| {
            ui.set_min_height(h);
            egui::ScrollArea::vertical().id_salt("poc_list").stick_to_bottom(true).max_height(h).auto_shrink([false, false]).show(ui, |ui| {
                let rows: Vec<&RxMsg> = self.pocsag.msgs.iter().filter(show).collect();
                if rows.is_empty() {
                    ui.label(RichText::new("Noch keine Funkrufe. Funkgerät auf eine POCSAG-Frequenz (FM) stellen – Schnellwahl über die Frequenzanzeige oben – \
                        und Audio starten.").weak());
                }
                if self.pocsag.monitor {
                    for r in rows {
                        let c = if r.tx { ORANGE } else if r.m.addr.is_some_and(|a| mine.contains(&a)) { GREEN } else { Color32::from_gray(225) };
                        ui.label(RichText::new(format!("{} {}", r.time, r.m.multimon_line())).monospace().color(c));
                    }
                } else {
                    if !rows.is_empty() { egui::Grid::new("poc_grid").num_columns(5).striped(true).spacing([10.0, 3.0]).show(ui, |ui| {
                        for h in ["Zeit", "Bd", "RIC", "Fkt", "Text"] { ui.label(RichText::new(h).small().strong()); }
                        ui.end_row();
                        for r in rows {
                            let own = r.m.addr.is_some_and(|a| mine.contains(&a));
                            let c = if r.tx { ORANGE } else if own { GREEN } else { Color32::from_gray(225) };
                            ui.label(RichText::new(&r.time).monospace().small().color(c));
                            ui.label(RichText::new(r.m.baud.to_string()).monospace().small().color(c));
                            match r.m.addr {
                                Some(a) => {
                                    if ui.add(egui::Label::new(RichText::new(a.to_string()).monospace().color(c).underline()).sense(egui::Sense::click()))
                                        .on_hover_text(match dapnet::ric_info(a, &region) { Some(i) => format!("{i}\n\nKlick: als Empfänger zum Senden übernehmen"), None => "als Empfänger zum Senden übernehmen".into() }).clicked() { pick = Some((a, r.m.func.unwrap_or(3))); }
                                }
                                None => { ui.label(RichText::new("–").monospace().color(c)); }
                            }
                            ui.label(RichText::new(r.m.func.map_or("–".into(), |f| ((b'A' + f) as char).to_string())).monospace().color(c))
                                .on_hover_text(r.m.func.map_or(String::new(), |f| format!("Funktionsbits {f}")));
                            let text = match r.m.kind {
                                pocsag::Kind::Tone => "(nur Ton)".to_string(),
                                pocsag::Kind::Skyper => match r.m.skyper_split() {
                                    Some((h, t)) => format!("[{}] {t}", h.chars().map(|c| format!("{:02X}", c as u32)).collect::<Vec<_>>().join(" ")),
                                    None => format!("[Skyper] {}", r.m.clean_text()),
                                },
                                _ => r.m.clean_text(),
                            };
                            let text = if r.m.complete { text } else { format!("{text} …") };
                            ui.add(egui::Label::new(RichText::new(text).monospace().color(c)).wrap())
                                .on_hover_text(format!("{}{}", r.m.kind.label(), if r.m.complete { "" } else { " – nur teilweise empfangen" }));
                            ui.end_row();
                        }
                    }); }
                }
            });
        });
        if let Some((a, f)) = pick { self.pocsag.tx_ric = a.to_string(); self.pocsag.tx_func = f; }
    }

    pub(crate) fn ui_pocsag_tx(&mut self, ui: &mut egui::Ui) {
        let busy = self.tx.is_some();
        ui.heading("Funkruf senden");
        ui.add_enabled_ui(!busy, |ui| {
            egui::Grid::new("poc_tx").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("RIC");
                ui.add(egui::TextEdit::singleline(&mut self.pocsag.tx_ric).desired_width(110.0).hint_text("z. B. 123456"))
                    .on_hover_text("Rufnummer (Adresse) des Pagers, 0–2097151; ein Klick auf eine RIC in der Liste übernimmt sie");
                ui.end_row();
                ui.label("Funktion");
                ui.horizontal(|ui| {
                    for f in 0..4u8 {
                        ui.selectable_value(&mut self.pocsag.tx_func, f, func_label(f));
                    }
                }).response.on_hover_text("Funktionsbits (Ruftöne A–D). Üblich: 0 für numerische, 3 für Textrufe (DAPNET)");
                ui.end_row();
                ui.label("Art");
                ui.horizontal(|ui| {
                    let mut set = None;
                    for (k, l) in [(TxKindP::Alpha, "Text"), (TxKindP::Numeric, "Numerisch"), (TxKindP::Tone, "Nur Ton")] {
                        if ui.selectable_label(self.pocsag.tx_kind == k, l).clicked() { set = Some(k); }
                    }
                    if let Some(k) = set {
                        // Funktion passend vorschlagen (wie Standard-Auswertung in multimon-ng: 0 = numerisch)
                        if k == TxKindP::Numeric { self.pocsag.tx_func = 0; } else if self.pocsag.tx_kind == TxKindP::Numeric { self.pocsag.tx_func = 3; }
                        self.pocsag.tx_kind = k;
                    }
                });
                ui.end_row();
                ui.label("Baudrate");
                ui.horizontal(|ui| {
                    for b in pocsag::BAUDS { ui.selectable_value(&mut self.pocsag.tx_baud, b, b.to_string()); }
                }).response.on_hover_text("DAPNET und die meisten Pager im Amateurfunk: 1200 Baud");
                ui.end_row();
            });
            if self.pocsag.tx_kind != TxKindP::Tone {
                ui.add(egui::TextEdit::multiline(&mut self.pocsag.tx_text).desired_rows(3).desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace).hint_text(if self.pocsag.tx_kind == TxKindP::Numeric { "Ziffern, z. B. 0815 4711" } else { "Text des Funkrufs" }));
            }
            ui.horizontal_wrapped(|ui| {
                if self.pocsag.tx_kind == TxKindP::Alpha {
                    ui.checkbox(&mut self.pocsag.tx_call, "Rufzeichen anhängen").on_hover_text("hängt „de <eigenes Rufzeichen>“ an, damit die Aussendung gekennzeichnet ist");
                }
                ui.checkbox(&mut self.pocsag.tx_invert, "invertiert").on_hover_text("Polarität umdrehen – nötig, wenn der Pager mit normaler Polarität nichts empfängt");
            });
        });
        let page = self.pocsag.page(&self.station.call);
        match &page {
            Ok(pg) => {
                let chars = match &pg.content { Content::Alpha(t) | Content::Numeric(t) => t.chars().count(), Content::Tone => 0 };
                let secs = pocsag::seconds(std::slice::from_ref(pg), self.pocsag.p.charset, self.pocsag.tx_baud);
                ui.label(RichText::new(format!("{chars} Zeichen · Dauer {secs:.1} s{}", if chars > 80 { " · viele Pager zeigen nur 80 Zeichen" } else { "" })).small().weak());
            }
            Err(e) => { ui.label(RichText::new(e).small().color(ORANGE)); }
        }
        ui.horizontal(|ui| {
            let kind = self.tx.as_ref().map(|t| t.kind);
            let send = if kind == Some(TxKind::Pocsag) { egui::Button::new(RichText::new("Stopp").color(Color32::WHITE).strong()).fill(crate::RED) } else {
                egui::Button::new(RichText::new("Funkruf senden").color(Color32::BLACK).strong()).fill(ORANGE)
            };
            let can = kind == Some(TxKind::Pocsag) || (kind.is_none() && page.is_ok());
            if ui.add_enabled_ui(can, |ui| ui.add_sized([200.0, 26.0], send)).inner.clicked() {
                if busy { self.stop_tx() } else { self.pocsag_send() }
            }
        });
        let pr = self.tx.as_ref().map_or(0.0, |t| t.state.progress());
        ui.add(egui::ProgressBar::new(pr).desired_height(6.0));
        self.ui_gain(ui).on_hover_text("bestimmt bei FM den Hub: für POCSAG etwa ±4,5 kHz (eher weniger als bei Sprache)");
        ui.add_space(6.0);
        ui.label(RichText::new("Gesendet wird das POCSAG-Basisband (NRZ) für den Modulationseingang in FM. Sauber geht das über den Daten-/9600-Anschluss \
            oder FM-D (ohne Preemphasis). Auf der DAPNET-Frequenz senden die Sender des Netzes in Zeitschlitzen – eigene Rufe dort stören sie; \
            zum Testen eines eigenen Pagers besser eine freie Frequenz wählen.").small().weak());
        self.ui_dapnet_tx(ui);
    }
}

/// „4520, 1234“ → bis zu vier RICs (Rest 0)
pub(crate) fn parse_rics(v: &str) -> [u32; 4] {
    let mut r = [0u32; 4];
    for (i, x) in v.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).filter_map(|x| x.trim().parse::<u32>().ok()).filter(|&x| x > 0 && x < 1 << 21).take(4).enumerate() { r[i] = x; }
    r
}
pub(crate) fn sky_str(r: &[u32; 4]) -> String { r.iter().filter(|&&x| x != 0).map(|x| x.to_string()).collect::<Vec<_>>().join(", ") }

impl App {
    /// DAPNET-Ansicht: Rubrik-Nachrichten (beide Wege zusammengeführt) und übrige Rufe, Zeit-/Dienst-RICs nach Schalter
    pub(crate) fn ui_dapnet(&mut self, ui: &mut egui::Ui) {
        let st = &mut self.pocsag;
        let region = dapnet::region_tokens(&st.region);
        // Auswahl: None = alles, Some(0) = nur Rubriken, Some(n) = Rubrik n
        ui.horizontal_wrapped(|ui| {
            ui.label("Region");
            ui.add(egui::TextEdit::singleline(&mut st.region).desired_width(110.0).hint_text("dl-all"))
                .on_hover_text("Sendergruppen für die Rubrik-Bezeichnungen, z. B. „dl-all“, „dl-by“, „oe-all“, „hb-all“. \
                                Die Nummern sind je Land unterschiedlich belegt (11 = Berlin in DL, Wien in OE). Bezeichnungen nach der Rubrikliste von pi-star.de.");
            ui.separator();
            ui.label("Zeigen");
            let mut seen: Vec<u8> = st.news.iter().map(|n| n.num).filter(|&n| n > 0).collect();
            seen.sort_unstable();
            seen.dedup();
            let name = |sel: Option<u8>| match sel { None => "alles".to_string(), Some(0) => "nur Rubriken".into(), Some(n) => format!("{n} {}", dapnet::label(n, &region)) };
            egui::ComboBox::from_id_salt("dap_sel").width(200.0).height(420.0).selected_text(name(st.rub_sel)).show_ui(ui, |ui| {
                ui.set_min_width(240.0);
                for v in [None, Some(0)].into_iter().chain(seen.into_iter().map(Some)) {
                    ui.selectable_value(&mut st.rub_sel, v, name(v));
                }
            });
            if ui.small_button("leeren").clicked() { st.news.clear(); }
        });
        let (ht, hs) = (st.hide_time, st.hide_svc);
        let h = (ui.ctx().content_rect().height() * 0.5).max(160.0);
        egui::Frame::new().fill(Color32::from_gray(12)).inner_margin(6.0).corner_radius(4.0).show(ui, |ui| {
            ui.set_min_height(h);
            egui::ScrollArea::vertical().id_salt("dap_news").stick_to_bottom(true).max_height(h).auto_shrink([false, false]).show(ui, |ui| {
                let rows: Vec<&dapnet::News> = st.news.iter().filter(|n| match st.rub_sel { None => true, Some(0) => n.num > 0, Some(s) => n.num == s })
                    .filter(|n| n.num > 0 || !dapnet::hidden(n.via.first().copied(), ht, hs)).collect();
                if rows.is_empty() {
                    ui.label(RichText::new("Noch nichts empfangen. Rubriken kommen im Klartext an RIC 1000 + Rubrik (AlphaPoc/Swissphone) und \
                        für Skyper an RIC 4520; Rustwave legt beide Wege zusammen.").weak());
                    return;
                }
                let mut pick = None;
                egui::Grid::new("dap_grid").num_columns(4).striped(true).spacing([10.0, 3.0]).show(ui, |ui| {
                    for h in ["Zeit", "Rubrik / RIC", "Nr", "Text"] { ui.label(RichText::new(h).small().strong()); }
                    ui.end_row();
                    for n in rows {
                        let c = Color32::from_gray(225);
                        ui.label(RichText::new(&n.time).monospace().small().color(c));
                        let tip = n.via.iter().map(|&v| match dapnet::ric_info(v, &region) { Some(i) => format!("RIC {v}: {i}"), None => format!("RIC {v}") })
                            .collect::<Vec<_>>().join("\n");
                        if n.num > 0 {
                            let r = ui.add(egui::Label::new(RichText::new(format!("{:>2} {}", n.num, dapnet::label(n.num, &region))).monospace().color(GREEN))
                                .sense(egui::Sense::click()));
                            if r.on_hover_text(format!("{tip}\n\nKlick: nur diese Rubrik")).clicked() { pick = Some(n.num); }
                        } else {
                            ui.label(RichText::new(n.via.first().map_or(String::new(), |r| r.to_string())).monospace().color(c)).on_hover_text(tip);
                        }
                        ui.label(RichText::new(n.slot.map_or("–".into(), |s| s.to_string())).monospace().small().color(c))
                            .on_hover_text("Platz der Nachricht in der Rubrik (aus dem Skyper-Kopf; Skyper zeigt bis zu 10 je Rubrik)");
                        ui.add(egui::Label::new(RichText::new(&n.text).monospace().color(c)).wrap());
                        ui.end_row();
                    }
                });
                if pick.is_some() { st.rub_sel = pick; }
            });
        });
        self.ui_dapnet_heard(ui);
    }

    /// gehörte DAPNET-Sender
    fn ui_dapnet_heard(&mut self, ui: &mut egui::Ui) {
        let n = self.pocsag.dn_heard.len();
        egui::CollapsingHeader::new(format!("Gehörte DAPNET-Sender ({n})")).id_salt("dn_heard").default_open(n > 0).show(ui, |ui| {
            if n == 0 {
                ui.label(RichText::new("Noch keine Senderkennung empfangen. Jeder DAPNET-Sender schickt regelmäßig sein Rufzeichen an RIC 8.").small().weak());
                return;
            }
            let now = chrono::Utc::now().timestamp();
            let mut rows: Vec<&HeardTx> = self.pocsag.dn_heard.iter().collect();
            rows.sort_by_key(|h| -h.last);
            egui::Grid::new("dn_heard_g").num_columns(6).striped(true).spacing([14.0, 3.0]).show(ui, |ui| {
                for h in ["Sender", "Kennungen", "zuletzt", "Pegel", "Rufe danach", "fehlerfrei"] { ui.label(RichText::new(h).small().strong()); }
                ui.end_row();
                for h in rows {
                    let age = now - h.last;
                    let c = if age < 600 { GREEN } else if age < 3600 { Color32::from_gray(210) } else { Color32::GRAY };
                    ui.label(RichText::new(&h.call).monospace().strong().color(c))
                        .on_hover_text(format!("zuerst gehört {}", chrono::DateTime::from_timestamp(h.first, 0).map(|d| d.with_timezone(&chrono::Local).format("%H:%M").to_string()).unwrap_or_default()));
                    ui.label(h.idents.to_string());
                    ui.label(if age < 90 { "gerade".to_string() } else if age < 3600 { format!("vor {} min", age / 60) } else { format!("vor {} h", age / 3600) });
                    match h.s_last {
                        Some(v) => {
                            let avg = h.s_sum_db / h.s_n.max(1) as f32;
                            let avg_raw = if avg <= 54.0 { avg / 54.0 * 120.0 } else { 120.0 + (avg - 54.0) / 60.0 * 121.0 };
                            ui.label(RichText::new(crate::rigpoll::s_text(v)).monospace()).on_hover_text(format!(
                                "S-Meter bei der letzten Kennung; Mittel {}, höchstens {}", crate::rigpoll::s_text(avg_raw.round() as u32),
                                crate::rigpoll::s_text(h.s_max.unwrap_or(v))));
                        }
                        None => { ui.label("–").on_hover_text("Pegel braucht ein per CAT verbundenes Icom-Gerät"); }
                    }
                    ui.label(h.calls.to_string()).on_hover_text("Rufe im selben Durchgang direkt nach der Kennung (geschätzt: bis ein Zeitschlitz danach)");
                    let tot = h.calls + h.idents;
                    let ok = h.clean + h.ident_clean;
                    ui.label(format!("{:.0} %", 100.0 * ok as f64 / tot.max(1) as f64))
                        .on_hover_text("Anteil der Rufe dieses Senders ohne korrigierte Bitfehler – ein Maß für die Empfangsqualität");
                    ui.end_row();
                }
            });
            if ui.small_button("Liste leeren").clicked() { self.pocsag.dn_heard.clear(); self.pocsag.dn_cur = None; }
        });
    }
}

// ---------------- DAPNET: Sender und Ruf ins Netz ----------------

/// Rufe, die länger warten, sind überholt (Zeitsignale, Wetter …)
const DN_MAX_AGE_S: u64 = 600;

impl App {
    /// Zeitpunkt für die Zeitschlitze: PC-Uhr, falls bekannt um die GPS-Abweichung korrigiert
    fn dn_now(&self) -> f64 {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        t + self.gps.clock_offset.unwrap_or(0) as f64
    }

    /// je Bild: Ereignisse vom Kern, Warteschlange in den erlaubten Zeitschlitzen senden, REST-Antwort abholen
    pub(crate) fn dapnet_tick(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.pocsag.api_busy {
            if let Ok(r) = rx.try_recv() {
                self.pocsag.api_busy = None;
                self.pocsag.api_msg = Some(match r {
                    Ok(_) => (true, "Ruf an DAPNET übergeben – die Sender der Gruppen strahlen ihn in ihren Zeitschlitzen aus".into()),
                    Err(e) => (false, e),
                });
            } else { ctx.request_repaint_after(std::time::Duration::from_millis(200)); }
        }
        let Some(link) = &self.pocsag.dn_link else { return };
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        let evs: Vec<_> = link.rx.try_iter().collect();
        let white: Vec<u32> = self.pocsag.dn_white.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).filter_map(|x| x.trim().parse().ok()).collect();
        for e in evs {
            use crate::dapnet_tx::Event as E;
            match e {
                E::Connected(h) => { self.log(format!("DAPNET: verbunden mit {h}")); self.pocsag.dn_state = "angemeldet, warte auf Zeitschlitze".into(); self.pocsag.dn_host = Some(h); }
                E::Disconnected(why) => {
                    if self.pocsag.dn_host.is_some() { self.log(format!("DAPNET: getrennt – {why}")); }
                    self.pocsag.dn_host = None;
                    self.pocsag.dn_state = format!("keine Verbindung: {why} – neuer Versuch in 5 s");
                }
                E::Slots(s) => { self.pocsag.dn_slots = Some(s); self.pocsag.dn_state = "bereit".into(); }
                E::Log(t) => self.log(format!("DAPNET: {t}")),
                E::Call(c) => {
                    self.pocsag.dn_got += 1;
                    if white.is_empty() || white.contains(&c.page.addr) {
                        self.pocsag.dn_queue.push_back(c);
                        if self.pocsag.dn_queue.len() > 200 { self.pocsag.dn_queue.pop_front(); }
                    }
                }
            }
        }
        self.pocsag.dn_queue.retain(|c| c.got.elapsed().as_secs() < DN_MAX_AGE_S);
        // senden: nur im POCSAG-Reiter, wenn nichts anderes läuft und ein erlaubter Schlitz genug Zeit lässt
        let (Some(slots), true, true) = (self.pocsag.dn_slots, self.tx.is_none(), self.op == crate::Op::Pocsag) else { return };
        if self.pocsag.dn_retry.is_some_and(|r| r.elapsed().as_secs() < 10) { return; }
        let Some(first) = self.pocsag.dn_queue.front() else { return };
        let t = self.dn_now();
        let budget = crate::dapnet_tx::budget_s(&slots, t);
        // Vorlauf (PTT), Nachlauf (WLAN-Puffer) und etwas Reserve
        let extra = 0.2 + if self.net.is_some() { 0.45 } else { 0.15 } + 0.3;
        let baud = first.baud;
        let cs = Charset::Us; // der Kern liefert fertigen 7-Bit-Text
        let mut take = 0;
        while take < self.pocsag.dn_queue.len() && self.pocsag.dn_queue[take].baud == baud {
            let pages: Vec<Page> = self.pocsag.dn_queue.iter().take(take + 1).map(|c| c.page.clone()).collect();
            if pocsag::seconds(&pages, cs, baud) + extra > budget { break; }
            take += 1;
        }
        if take == 0 {
            // passt selbst in fünf freie Schlitze nicht: verwerfen, sonst blockiert er die Schlange
            let full = crate::dapnet_tx::SLOT_S * 5.0;
            if pocsag::seconds(&[first.page.clone()], cs, baud) + extra > full {
                let c = self.pocsag.dn_queue.pop_front();
                self.log(format!("DAPNET: Ruf an RIC {} zu lang für die Zeitschlitze – verworfen", c.map_or(0, |c| c.page.addr)));
            }
            return;
        }
        self.pocsag.sending = self.pocsag.dn_queue.drain(..take).map(|c| c.page).collect();
        self.pocsag.sending_baud = baud;
        self.pocsag.sending_cs = cs;
        self.pocsag.dn_tx = true;
        self.start_tx(TxKind::Pocsag);
        if self.tx.is_none() {
            // Senden ging nicht (Audio/PTT): Rufe zurück in die Schlange
            let back = std::mem::take(&mut self.pocsag.sending);
            for p in back.into_iter().rev() { self.pocsag.dn_queue.push_front(crate::dapnet_tx::Call { page: p, baud, got: std::time::Instant::now() }); }
            self.pocsag.dn_tx = false;
            self.pocsag.dn_retry = Some(std::time::Instant::now());
            self.log("DAPNET: Senden nicht möglich (Audio-Ausgang/PTT prüfen) – neuer Versuch in 10 s");
        }
    }

    fn dapnet_connect(&mut self) {
        let p = &mut self.pocsag;
        p.dn_link = Some(crate::dapnet_tx::Link::start(&p.dn_server, &p.dn_call, &p.dn_auth));
        p.dn_state = "verbinde …".into();
        p.dn_slots = None;
        p.dn_got = 0;
        p.dn_sent = 0;
    }
    fn dapnet_disconnect(&mut self) {
        let p = &mut self.pocsag;
        p.dn_link = None;
        p.dn_host = None;
        p.dn_slots = None;
        p.dn_queue.clear();
        p.dn_state.clear();
        self.log("DAPNET-Sender getrennt");
    }

    /// Einstellungen und Zustand von DAPNET (Ruf ins Netz, eigener Sender)
    pub(crate) fn ui_dapnet_tx(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        egui::CollapsingHeader::new(RichText::new("Ruf über DAPNET (Internet)").strong()).id_salt("dn_api").show(ui, |ui| {
            let p = &mut self.pocsag;
            ui.label(RichText::new("Gibt einen Textruf über hampager.de ins Netz; ausgestrahlt wird er von den DAPNET-Sendern der gewählten Gruppen. \
                Nötig ist ein Benutzerkonto bei hampager.de.").small().weak());
            egui::Grid::new("dn_api_g").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("Benutzer"); ui.add(egui::TextEdit::singleline(&mut p.api_user).desired_width(140.0).hint_text("Rufzeichen")); ui.end_row();
                ui.label("Passwort"); ui.add(egui::TextEdit::singleline(&mut p.api_pass).password(true).desired_width(140.0)); ui.end_row();
                ui.label("An"); ui.add(egui::TextEdit::singleline(&mut p.api_to).desired_width(220.0).hint_text("Rufzeichen, mit Komma getrennt"))
                    .on_hover_text("DAPNET-Rufzeichen der Empfänger (deren Pager-RICs kennt DAPNET)"); ui.end_row();
                ui.label("Sendergruppen"); ui.add(egui::TextEdit::singleline(&mut p.api_groups).desired_width(220.0).hint_text("dl-all"))
                    .on_hover_text("z. B. dl-all, dl-by, oe-all – wo der Ruf ausgestrahlt wird"); ui.end_row();
            });
            ui.add(egui::TextEdit::singleline(&mut p.api_text).desired_width(f32::INFINITY).char_limit(80).hint_text("Text (höchstens 80 Zeichen)"));
            let list = |s: &str| s.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect::<Vec<_>>();
            let (to, groups) = (list(&p.api_to), list(&p.api_groups));
            ui.horizontal_wrapped(|ui| {
                let ok = p.api_busy.is_none() && !p.api_user.is_empty() && !p.api_pass.is_empty() && !to.is_empty() && !groups.is_empty() && !p.api_text.trim().is_empty();
                if ui.add_enabled(ok, egui::Button::new(if p.api_busy.is_some() { "sende …" } else { "Ruf ins Netz geben" })).clicked() {
                    let body = crate::dapnet_tx::call_json(p.api_text.trim(), &to, &groups, false);
                    let (url, user, pass) = (p.api_url.clone(), p.api_user.trim().to_lowercase(), p.api_pass.clone());
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || { let _ = tx.send(crate::dapnet_tx::api_send(&url, &user, &pass, &body)); });
                    p.api_busy = Some(rx);
                    p.api_msg = None;
                }
                if let Some((ok, m)) = &p.api_msg { ui.label(RichText::new(m).small().color(if *ok { GREEN } else { ORANGE })); }
            });
        });

        egui::CollapsingHeader::new(RichText::new("Als DAPNET-Sender betreiben").strong()).id_salt("dn_tx").show(ui, |ui| {
            let connected = self.pocsag.dn_link.is_some();
            ui.label(RichText::new("Wie UniPager oder Pi-Star: Rustwave meldet sich beim DAPNET-Kern als Sender an, nimmt die Rufe für deine \
                Sendergruppen entgegen und strahlt sie in den zugeteilten Zeitschlitzen auf 439,9875 MHz aus. Dafür braucht es einen eigenen \
                Sender-Zugang (Ticket bei hampager.de, „New Transmitter“) – Rufzeichen und AuthKey des Senders, nicht das Benutzerkonto.").small().weak());
            ui.add_enabled_ui(!connected, |ui| {
                let p = &mut self.pocsag;
                egui::Grid::new("dn_tx_g").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                    ui.label("Sender"); ui.add(egui::TextEdit::singleline(&mut p.dn_call).desired_width(140.0).hint_text("Rufzeichen des Senders")); ui.end_row();
                    ui.label("AuthKey"); ui.add(egui::TextEdit::singleline(&mut p.dn_auth).password(true).desired_width(140.0)); ui.end_row();
                    ui.label("Kern"); ui.add(egui::TextEdit::singleline(&mut p.dn_server).desired_width(220.0))
                        .on_hover_text(format!("Host:Port des DAPNET-Kerns. Bei der Voreinstellung probiert Rustwave nacheinander auch {} (HAMNET).",
                            crate::dapnet_tx::FALLBACK.join(", "))); ui.end_row();
                    ui.label("Nur RICs"); ui.add(egui::TextEdit::singleline(&mut p.dn_white).desired_width(220.0).hint_text("leer = alle"))
                        .on_hover_text("Wie die POCSAG-Whitelist bei Pi-Star: nur Rufe an diese RICs aussenden, z. B. die eigene. \
                                        Dann fallen auch Zeitsignale und Rubriken weg, wenn ihre RICs fehlen."); ui.end_row();
                });
            });
            ui.horizontal_wrapped(|ui| {
                if connected {
                    if ui.button("Trennen").clicked() { self.dapnet_disconnect(); }
                } else {
                    let ok = !self.pocsag.dn_call.trim().is_empty() && !self.pocsag.dn_auth.trim().is_empty();
                    if ui.add_enabled(ok, egui::Button::new(RichText::new("Verbinden").strong())).clicked() { self.dapnet_connect(); }
                }
                let p = &self.pocsag;
                if connected {
                    let c = if p.dn_host.is_some() { GREEN } else { ORANGE };
                    ui.label(RichText::new(&p.dn_state).small().color(c));
                }
            });
            if connected {
                let p = &self.pocsag;
                ui.label(RichText::new(format!("{} · empfangen {} · gesendet {} · wartend {}",
                    p.dn_host.as_deref().unwrap_or("–"), p.dn_got, p.dn_sent, p.dn_queue.len())).small());
                // Zeitschlitze 0–F, laufender umrandet
                let t = self.dn_now();
                let cur = crate::dapnet_tx::slot_at(t);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for i in 0..16 {
                        let on = p.dn_slots.is_some_and(|s| s[i]);
                        let (r, pt) = ui.allocate_painter(egui::vec2(16.0, 16.0), egui::Sense::hover());
                        pt.rect_filled(r.rect, 2.0, if on { GREEN.gamma_multiply(0.7) } else { Color32::from_gray(45) });
                        if i == cur { pt.rect_stroke(r.rect, 2.0, egui::Stroke::new(2.0_f32, Color32::WHITE), egui::StrokeKind::Inside); }
                        pt.text(r.rect.center(), egui::Align2::CENTER_CENTER, format!("{i:X}"), egui::FontId::monospace(10.0),
                                if on { Color32::BLACK } else { Color32::from_gray(140) });
                    }
                    let hint = match p.dn_slots {
                        None => "noch keine Zeitschlitze vom Kern".to_string(),
                        Some(s) => match crate::dapnet_tx::wait_s(&s, t) {
                            Some(w) if w <= 0.0 => format!("frei, noch {:.1} s", crate::dapnet_tx::budget_s(&s, t)),
                            Some(w) => format!("nächster Schlitz in {w:.0} s"),
                            None => "keine Schlitze zugeteilt".into(),
                        },
                    };
                    ui.label(RichText::new(hint).small().weak());
                }).response.on_hover_text("Zeitschlitze à 6,4 s; grün = deinem Sender zugeteilt, weiß umrandet = jetzt. \
                    Die Zeit kommt von der PC-Uhr (bei GPS um deren Abweichung korrigiert) – sie sollte auf die Sekunde stimmen.");
                if self.op != crate::Op::Pocsag {
                    ui.label(RichText::new("Gesendet wird nur, solange der POCSAG-Reiter offen ist.").small().color(ORANGE));
                }
                if self.gps.clock_offset.is_some_and(|o| o.abs() >= 1) {
                    ui.label(RichText::new("Die PC-Uhr weicht von der GPS-Zeit ab – Rustwave rechnet das heraus, besser die Uhr stellen.").small().color(ORANGE));
                }
            }
            ui.label(RichText::new("Hinweise: Funkgerät auf 439,9875 MHz FM (Daten/FM-D, ohne Preemphasis), Hub über den Sendepegel etwa ±4,5 kHz. \
                Ein Sender ohne Aufsicht gilt in Deutschland als automatisch arbeitende Station und braucht eine eigene Zuteilung; \
                im Beisein des Betreibers mit eigenem Rufzeichen und kleiner Leistung (persönlicher Sender) – vorher mit dem DAPNET-Team und \
                den Regeln deines Landes klären.").small().weak());
        });
    }
}
