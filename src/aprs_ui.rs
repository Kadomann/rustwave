//! APRS-Oberfläche: gehörte Stationen, Monitor, Positionsbake, Nachrichten mit Bestätigung.

use crate::aprs::{self, Body, Frame};
use crate::{App, TxKind, GREEN, ORANGE};
use eframe::egui::{self, Color32, RichText};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const RETRY: Duration = Duration::from_secs(30);
const MAX_TRIES: u32 = 3;
/// Bestätigung nicht sofort senden: Digipeater wiederholen die Nachricht gerade noch
const ACK_DELAY: Duration = Duration::from_millis(2500);

pub struct Station { pub call: String, pub last: i64, pub pos: Option<(f64, f64)>, pub symbol: (char, char), pub comment: String, pub count: u32,
    /// KW-Skimmer: Tonmitte im Audio (Hz) und daraus die Frequenz (Hz), sofern das Funkgerät sie meldet
    pub tone: Option<f64>, pub rf: Option<u64>,
    /// nur übers Internet (APRS-IS) gehört; Wetterdaten lesbar
    pub is_only: bool, pub wx: Option<String>,
    /// ausgewertetes letztes Positionspaket, letzte Wetterdaten, Statustext, Weg des letzten Pakets, Rohpaket
    pub det: Option<crate::aprsinfo::Details>, pub wxd: Option<aprs::Wx>, pub status: String, pub path: String, pub raw: String,
    /// Wetterverlauf (Zeit, Luftdruck, Temperatur) für Tendenzen, Zeitpunkt des letzten Wetterberichts
    pub wx_hist: Vec<(i64, Option<f64>, Option<f64>)>, pub wx_time: Option<i64>,
    /// Spur (Zeit, Breite, Länge), benutzte Digipeater des letzten Funkpakets, Besitzer (bei Objekten), zuletzt direkt gehört
    pub track: Vec<(i64, f64, f64)>, pub digis: Vec<String>, pub owner: Option<String>, pub direct: Option<i64>,
    /// S-Meter beim letzten Funkpaket und höchster Wert (Icom-Rohwert)
    pub level: Option<u32>, pub level_max: Option<u32> }

/// eigenes Objekt (Treffpunkt, Relais, Fieldday …)
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Obj { pub name: String, pub lat: String, pub lon: String, pub sym: usize, pub comment: String,
    /// wird gerade gemeldet (sonst gelöscht bzw. noch nicht gesendet)
    pub live: bool }

/// Symbole für Objekte
pub const OBJ_SYMBOLS: [(char, char, &str); 10] = [
    ('/', ';', "Zeltlager/Portabel"), ('/', 'r', "Relais"), ('/', '#', "Digipeater"), ('/', 'E', "Augenzeuge/Veranstaltung"),
    ('/', 'h', "Krankenhaus"), ('/', '+', "Rotes Kreuz"), ('/', 'a', "Rettungswagen"), ('\\', 'N', "Navigationsboje"),
    ('/', '-', "Haus"), ('/', '_', "Wetterstation")];

pub struct Msg { pub time: i64, pub partner: String, pub text: String, pub id: Option<String>, pub out: bool, pub acked: bool, pub tries: u32, next: Option<Instant>, pub seen: bool,
    /// Weg: empfangen über bzw. zuletzt gesendet über (Funk, iGate, Internet, Funk + Internet)
    pub via: String }
/// Bulletin (an BLN…): Zeit, Absender, Kennung, Text
pub struct Bulletin { pub time: i64, pub from: String, pub to: String, pub text: String }

pub struct AprsState {
    // Einstellungen (werden gespeichert)
    pub ssid: String,
    pub lat: String,
    pub lon: String,
    pub symbol: usize,
    pub comment: String,
    pub path: String,
    /// Pfad für Kurzwelle (dort keine WIDE-Digipeater)
    pub path_hf: String,
    /// Kurzwelle: 300 Baud in USB statt 1200 Baud in FM
    pub hf: bool,
    /// KW: ganzes Durchlassband absuchen (Skimmer) statt nur um 1700 Hz
    pub skim: bool,
    /// APRS-IS: Empfangs-iGate, Internetstationen im Umkreis (km), Server
    pub igate: bool,
    /// Wege für Nachrichten: Empfang und Senden getrennt
    pub msg_rx: aprs::RxWeg,
    pub msg_tx: aprs::TxWeg,
    /// Gegenstellen, deren Nachrichten übers Internet kamen: Antworten gehen dann auch übers Internet
    is_partners: std::collections::HashSet<String>,
    pub is_show: bool,
    pub is_km: u32,
    pub is_server: String,
    is_conn: Option<crate::igate::Conn>,
    pub is_status: String,
    is_ok: bool,
    pub gated: u32,
    pub is_rx: u32,
    is_beacon: Option<Instant>,
    pub bulletins: Vec<Bulletin>,
    /// gewählte Unterhaltung
    pub conv: Option<String>,
    pub beacon_min: u32,
    /// vor jeder Bake die Position vom GPS des Funkgeräts holen
    pub gps_auto: bool,
    /// SmartBeaconing mit Tempo und Kurs vom GPS des Funkgeräts
    pub smart: bool,
    last_course: Option<f64>,
    pub last_reason: &'static str,
    // Laufzeit
    locating: bool,
    pub stations: Vec<Station>,
    pub monitor: VecDeque<(i64, String, bool)>,
    pub msgs: Vec<Msg>,
    pub msg_to: String,
    pub msg_text: String,
    /// Kartenansicht statt Liste; Maßstab (km Radius, None = automatisch); angetippte Station
    pub map: bool,
    map_km: Option<f64>,
    map_sel: Option<String>,
    msg_counter: u32,
    queue: Vec<Frame>,
    not_before: Instant,
    /// gerade in Aussendung (vom Modulator gelesen); dazu Rohrahmen von KISS/AGW
    pub sending: Vec<Frame>,
    pub sending_raw: Vec<Vec<u8>>,
    /// Rohrahmen (mit FCS) von anderen Programmen, die auf den Kanal warten
    pub kiss_q: Vec<Vec<u8>>,
    last_beacon: Option<Instant>,
    /// zuletzt eigenes Paket über einen Digipeater gehört
    digi_heard: Option<i64>,
    /// Telemetrie je Rufzeichen (Werte und Beschreibung)
    pub telem: std::collections::HashMap<String, crate::aprs_telem::Telem>,
    /// Karte: Spuren und Funkwege zeigen
    pub map_tracks: bool,
    pub map_paths: bool,
    /// SmartBeaconing-Voreinstellung (Auto, Fahrrad, zu Fuß), Bake als Mic-E, Höhe mitsenden
    pub smart_prof: usize,
    pub mice: bool,
    pub send_alt: bool,
    /// Hinweise: Rufzeichen (mit * am Ende als Platzhalter), Umkreis km, Böen km/h, Frost, Regen mm/h (0 = aus)
    pub al_calls: String,
    pub al_km: f64,
    pub al_gust: f64,
    pub al_frost: bool,
    pub al_rain: f64,
    pub alerts: Vec<(i64, String)>,
    al_fired: std::collections::HashMap<(String, &'static str), i64>,
    /// eigene Objekte, mit der Bake wiederholen; Eingabefelder für ein neues
    pub objects: Vec<Obj>,
    pub obj_repeat: bool,
    pub obj_new: Obj,
    obj_last: Option<Instant>,
    /// Abfragen (?APRSP, ?APRSD …) beantworten; letzte Antwort je Station; fällige Antworten auf allgemeine Abfragen
    pub answer_q: bool,
    q_last: std::collections::HashMap<String, Instant>,
    q_due: Option<(Instant, &'static str)>,
}

impl AprsState {
    pub fn load(get: &dyn Fn(&str) -> Option<String>) -> Self {
        let mut a = AprsState {
            ssid: String::new(), lat: String::new(), lon: String::new(), symbol: 0, comment: String::new(), path: String::new(),
            path_hf: String::new(), hf: false, skim: true, igate: false, msg_rx: aprs::RxWeg::Funk, msg_tx: aprs::TxWeg::Auto, is_partners: Default::default(), is_show: false, is_km: 100, is_server: String::new(), is_conn: None,
            is_status: String::new(), is_ok: false, gated: 0, is_rx: 0, is_beacon: None, bulletins: vec![], conv: None, beacon_min: 0, gps_auto: false, smart: false, map: false,
            last_course: None, last_reason: "", locating: false, map_km: None, map_sel: None,
            stations: vec![], monitor: VecDeque::new(), msgs: vec![], msg_to: String::new(), msg_text: String::new(),
            msg_counter: get("aprs_msgno").and_then(|s| s.parse().ok()).unwrap_or(1),
            queue: vec![], not_before: Instant::now(), sending: vec![], sending_raw: vec![], kiss_q: vec![], last_beacon: None, digi_heard: None,
            telem: Default::default(), map_tracks: true, map_paths: false, smart_prof: 0, mice: false, send_alt: true,
            al_calls: String::new(), al_km: 0.0, al_gust: 0.0, al_frost: false, al_rain: 0.0, alerts: vec![], al_fired: Default::default(),
            objects: vec![], obj_repeat: true, obj_new: Obj { name: String::new(), lat: String::new(), lon: String::new(), sym: 0, comment: String::new(), live: false },
            obj_last: None, answer_q: true, q_last: Default::default(), q_due: None,
        };
        a.apply_settings(get);
        a
    }
    /// nur die Einstellungen (beim Start und beim Laden eines Profils); gehörte Stationen bleiben
    pub fn apply_settings(&mut self, get: &dyn Fn(&str) -> Option<String>) {
        self.ssid = get("aprs_ssid").unwrap_or_default();
        self.lat = get("aprs_lat").unwrap_or_default();
        self.lon = get("aprs_lon").unwrap_or_default();
        self.symbol = get("aprs_sym").and_then(|s| s.parse().ok()).filter(|&i: &usize| i < aprs::SYMBOLS.len()).unwrap_or(0);
        self.comment = get("aprs_comment").unwrap_or_else(|| "Rustwave".into());
        self.path = get("aprs_path").unwrap_or_else(|| "WIDE1-1,WIDE2-1".into());
        self.path_hf = get("aprs_path_hf").unwrap_or_default();
        self.hf = get("aprs_hf").as_deref() == Some("1");
        self.skim = get("aprs_skim").as_deref() != Some("0");
        self.igate = get("aprs_igate").as_deref() == Some("1");
        // früher nur ein Schalter „auch übers Internet“
        let old = get("aprs_is_msgs").as_deref() == Some("1");
        let rx = get("aprs_msg_rx");
        self.msg_rx = aprs::RxWeg::ALL.iter().find(|w| Some(w.1) == rx.as_deref()).map_or(if old { aprs::RxWeg::Beides } else { aprs::RxWeg::Funk }, |w| w.0);
        let tx = get("aprs_msg_tx");
        self.msg_tx = aprs::TxWeg::ALL.iter().find(|w| Some(w.1) == tx.as_deref()).map_or(aprs::TxWeg::Auto, |w| w.0);
        self.is_show = get("aprs_is_show").as_deref() == Some("1");
        self.is_km = get("aprs_is_km").and_then(|s| s.parse().ok()).unwrap_or(100);
        self.is_server = get("aprs_is_server").filter(|s| !s.is_empty()).unwrap_or_else(|| "rotate.aprs2.net:14580".into());
        self.beacon_min = get("aprs_beacon").and_then(|s| s.parse().ok()).unwrap_or(0);
        self.gps_auto = get("aprs_gps_auto").as_deref() == Some("1");
        self.smart = get("aprs_smart").as_deref() == Some("1");
        self.map = get("aprs_map").as_deref() == Some("1");
        let b = |k: &str, d: bool| get(k).map_or(d, |v| v == "1");
        let f = |k: &str| get(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        self.map_tracks = b("aprs_tracks", true);
        self.map_paths = b("aprs_paths", false);
        self.smart_prof = get("aprs_smart_prof").and_then(|v| v.parse().ok()).filter(|&i: &usize| i < aprs::SMART_PROFILES.len()).unwrap_or(0);
        self.mice = b("aprs_mice", false);
        self.send_alt = b("aprs_alt", true);
        self.al_calls = get("aprs_al_calls").unwrap_or_default();
        self.al_km = f("aprs_al_km");
        self.al_gust = f("aprs_al_gust");
        self.al_frost = b("aprs_al_frost", false);
        self.al_rain = f("aprs_al_rain");
        self.objects = get("aprs_objects").and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
        self.obj_repeat = b("aprs_obj_repeat", true);
        self.answer_q = b("aprs_answer_q", true);
    }
    pub fn save(&self, s: &mut dyn eframe::Storage) {
        for (k, v) in [("aprs_ssid", &self.ssid), ("aprs_lat", &self.lat), ("aprs_lon", &self.lon), ("aprs_comment", &self.comment), ("aprs_path", &self.path), ("aprs_path_hf", &self.path_hf)] {
            s.set_string(k, v.clone());
        }
        s.set_string("aprs_sym", self.symbol.to_string());
        s.set_string("aprs_beacon", self.beacon_min.to_string());
        s.set_string("aprs_msgno", self.msg_counter.to_string());
        s.set_string("aprs_hf", if self.hf { "1" } else { "0" }.into());
        s.set_string("aprs_skim", if self.skim { "1" } else { "0" }.into());
        s.set_string("aprs_igate", if self.igate { "1" } else { "0" }.into());
        s.set_string("aprs_msg_rx", aprs::RxWeg::ALL.iter().find(|w| w.0 == self.msg_rx).map_or("funk", |w| w.1).into());
        s.set_string("aprs_msg_tx", aprs::TxWeg::ALL.iter().find(|w| w.0 == self.msg_tx).map_or("auto", |w| w.1).into());
        s.set_string("aprs_is_msgs", "0".into());
        s.set_string("aprs_is_show", if self.is_show { "1" } else { "0" }.into());
        s.set_string("aprs_is_km", self.is_km.to_string());
        s.set_string("aprs_is_server", self.is_server.clone());
        s.set_string("aprs_map", if self.map { "1" } else { "0" }.into());
        s.set_string("aprs_smart", if self.smart { "1" } else { "0" }.into());
        s.set_string("aprs_gps_auto", if self.gps_auto { "1" } else { "0" }.into());
        let b = |v: bool| if v { "1" } else { "0" }.to_string();
        s.set_string("aprs_tracks", b(self.map_tracks));
        s.set_string("aprs_paths", b(self.map_paths));
        s.set_string("aprs_smart_prof", self.smart_prof.to_string());
        s.set_string("aprs_mice", b(self.mice));
        s.set_string("aprs_alt", b(self.send_alt));
        s.set_string("aprs_al_calls", self.al_calls.clone());
        s.set_string("aprs_al_km", self.al_km.to_string());
        s.set_string("aprs_al_gust", self.al_gust.to_string());
        s.set_string("aprs_al_frost", b(self.al_frost));
        s.set_string("aprs_al_rain", self.al_rain.to_string());
        s.set_string("aprs_objects", serde_json::to_string(&self.objects).unwrap_or_default());
        s.set_string("aprs_obj_repeat", b(self.obj_repeat));
        s.set_string("aprs_answer_q", b(self.answer_q));
    }
}

impl AprsState {
    pub fn modem(&self) -> aprs::Modem { if self.hf { aprs::HF } else { aprs::VHF } }
}

fn now() -> i64 { chrono::Utc::now().timestamp() }
fn hhmm(t: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(t, 0).map_or(String::new(), |d| d.with_timezone(&chrono::Local).format("%H:%M").to_string())
}
fn parse_deg(s: &str) -> Option<f64> { s.trim().replace(',', ".").parse().ok() }
fn base_call(c: &str) -> &str { c.trim_end_matches('*') }

/// Standort über den Windows-Ortungsdienst (WLAN-Ortung oder angeschlossenes GPS); blockiert bis zu einigen Sekunden
#[cfg(windows)]
pub(crate) fn windows_location() -> Result<(f64, f64, f64), String> {
    use windows::Devices::Geolocation::{GeolocationAccessStatus, Geolocator};
    let e = |e: windows::core::Error| {
        let m = e.message().trim().to_string();
        format!("Ortungsdienst nicht verfügbar ({}Code 0x{:08X})", if m.is_empty() { m } else { m + ", " }, e.code().0)
    };
    // eigener Thread: WinRT dort als Multithread-Apartment anmelden
    unsafe { let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED); }
    let access = Geolocator::RequestAccessAsync().and_then(|op| op.join()).map_err(e)?;
    if access != GeolocationAccessStatus::Allowed {
        return Err("Standortzugriff ist gesperrt – Windows-Einstellungen > Datenschutz und Sicherheit > Standort: \
                    „Standortdienste“ und „Desktop-Apps Zugriff auf Ihren Standort erlauben“ einschalten".into());
    }
    let pos = Geolocator::new().and_then(|g| g.GetGeopositionAsync()).and_then(|op| op.join()).map_err(e)?;
    let c = pos.Coordinate().map_err(e)?;
    let p = c.Point().and_then(|p| p.Position()).map_err(e)?;
    Ok((p.Latitude, p.Longitude, c.Accuracy().unwrap_or(0.0)))
}
#[cfg(not(windows))]
pub(crate) fn windows_location() -> Result<(f64, f64, f64), String> { Err("Der Ortungsdienst steht nur unter Windows zur Verfügung".into()) }

impl App {
    fn aprs_my_call(&self) -> String {
        // AX.25 kennt kein „/P“ o. Ä.: der längste Teil ist das eigentliche Rufzeichen (Portabel zeigt die SSID)
        let call = self.station.call.split('/').max_by_key(|p| p.len()).unwrap_or("").to_uppercase();
        let ssid = self.aprs.ssid.trim();
        if ssid.is_empty() || ssid == "0" { call } else { format!("{call}-{ssid}") }
    }
    /// eigene Position für andere Module (Satelliten)
    pub(crate) fn aprs_pos_pub(&self) -> Option<(f64, f64)> { self.aprs_my_pos() }
    /// eigene Position: manuell eingetragen, sonst Mitte des Locators
    fn aprs_my_pos(&self) -> Option<(f64, f64)> {
        match (parse_deg(&self.aprs.lat), parse_deg(&self.aprs.lon)) {
            (Some(a), Some(o)) if a.abs() <= 90.0 && o.abs() <= 180.0 => Some((a, o)),
            _ => aprs::locator_to_latlon(&self.station.locator),
        }
    }
    fn aprs_frame(&self, info: Vec<u8>) -> Frame {
        let path = if self.aprs.hf { &self.aprs.path_hf } else { &self.aprs.path };
        let path = path.split(',').map(|p| p.trim().to_uppercase()).filter(|p| !p.is_empty()).collect();
        Frame { src: self.aprs_my_call(), dst: aprs::TOCALL.into(), path, info }
    }
    fn aprs_ready(&mut self) -> bool {
        if self.station.call.is_empty() { self.log("APRS: erst unter Einstellungen › Station das Rufzeichen eintragen"); return false; }
        true
    }
    /// GPS-Position vom Funkgerät in die Positionsfelder übernehmen
    fn aprs_rig_gps(&mut self) -> bool {
        let Some(r) = &mut self.rig else { self.log("GPS: zuerst das Funkgerät verbinden"); return false };
        match r.read_gps() {
            Ok((lat, lon)) => {
                self.aprs.lat = format!("{lat:.5}");
                self.aprs.lon = format!("{lon:.5}");
                self.log(format!("GPS-Position vom Funkgerät: {lat:.5}, {lon:.5}"));
                true
            }
            Err(e) => { self.log(format!("GPS: {e}")); false }
        }
    }
    fn aprs_windows_location(&mut self) {
        if self.aprs.locating { return; }
        self.aprs.locating = true;
        self.log("Frage den Windows-Ortungsdienst …");
        let tx = self.ev_tx.clone();
        std::thread::spawn(move || { let _ = tx.send(crate::UiEv::Location(windows_location())); });
    }
    pub(crate) fn aprs_location(&mut self, r: Result<(f64, f64, f64), String>) {
        self.aprs.locating = false;
        match r {
            Ok((lat, lon, acc)) => {
                self.aprs.lat = format!("{lat:.5}");
                self.aprs.lon = format!("{lon:.5}");
                self.log(format!("Windows-Standort: {lat:.5}, {lon:.5} (Genauigkeit etwa {acc:.0} m)"));
            }
            Err(e) => self.log(format!("Windows-Standort: {e}")),
        }
    }

    pub(crate) fn aprs_beacon(&mut self) {
        if !self.aprs_ready() { return; }
        // GPS: frische Daten aus der Hintergrundabfrage, sonst einmal direkt fragen;
        // ohne GPS-Empfang bleibt es bei der zuletzt eingetragenen Position
        let fix = self.gps.fresh(Duration::from_secs(15));
        if let Some(f) = fix.filter(|_| self.aprs.gps_auto || self.aprs.smart) {
            self.aprs.lat = format!("{:.5}", f.lat);
            self.aprs.lon = format!("{:.5}", f.lon);
        } else if self.aprs.gps_auto && self.rig.is_some() { self.aprs_rig_gps(); }
        let Some((lat, lon)) = self.aprs_my_pos() else { self.log("APRS: Position eintragen oder Locator unter „Station“ angeben"); return };
        let s = aprs::SYMBOLS[self.aprs.symbol];
        let prof = &aprs::SMART_PROFILES[self.aprs.smart_prof].1;
        // in Bewegung: Kurs und Tempo mitsenden; Höhe vom GPS
        let moving = fix.and_then(|f| f.course.zip(f.speed_kmh)).filter(|(_, v)| *v >= prof.slow_kmh);
        if let Some((c, _)) = moving { self.aprs.last_course = Some(c); }
        let alt = fix.and_then(|f| f.alt_m).filter(|_| self.aprs.send_alt);
        let f = if self.aprs.mice {
            // Mic-E: kompakt, Kurs/Tempo/Höhe binär; Status „Unterwegs“ in Fahrt, sonst „Außer Dienst“
            let status = if moving.is_some() { aprs::MICE_EN_ROUTE } else { aprs::MICE_OFF_DUTY };
            let (dst, info) = aprs::mice_encode(lat, lon, moving.map(|m| m.0), moving.map(|m| m.1), (s.0, s.1), alt, status, &self.aprs.comment);
            let mut f = self.aprs_frame(info);
            f.dst = dst;
            f
        } else {
            let mut comment = String::new();
            if let Some((c, v)) = moving { comment += &aprs::course_speed_ext(c, v); }
            comment += &self.aprs.comment;
            if let Some(a) = alt { comment += &aprs::alt_ext(a); }
            self.aprs_frame(aprs::position_info(lat, lon, (s.0, s.1), &comment))
        };
        self.aprs.queue.push(f);
        self.aprs.last_beacon = Some(Instant::now());
        if self.aprs.obj_repeat { self.aprs_objects_send(); }
    }
    pub(crate) fn aprs_send_msg(&mut self) {
        if !self.aprs_ready() { return; }
        let to = self.aprs.msg_to.trim().to_uppercase();
        let text: String = self.aprs.msg_text.trim().chars().filter(|c| !matches!(c, '|' | '~' | '{')).take(67).collect();
        if to.is_empty() || text.is_empty() { self.log("APRS: Empfänger und Text eingeben"); return; }
        let id = self.aprs.msg_counter.to_string();
        self.aprs.msg_counter = self.aprs.msg_counter % 999 + 1;
        self.aprs.msgs.push(Msg { time: now(), partner: to, text, id: Some(id), out: true, acked: false, tries: 0, next: Some(Instant::now()), seen: true, via: String::new() });
        self.aprs.msg_text.clear();
    }

    /// empfangenen Rahmen auswerten
    pub(crate) fn aprs_received(&mut self, f: Frame, tone: Option<f64>) { self.aprs_received_src(f, tone, false) }

    /// Meldung von APRS-IS (Internet)
    pub(crate) fn aprs_is_event(&mut self, e: crate::igate::IsEv) {
        match e {
            crate::igate::IsEv::Status(t, ok) => { self.aprs.is_ok = ok; if t != self.aprs.is_status { self.log(t.clone()); } self.aprs.is_status = t; }
            crate::igate::IsEv::Line(l) => if let Some(f) = aprs::parse_tnc2(&l) { self.aprs.is_rx += 1; self.aprs_received_src(f, None, true); },
        }
    }

    /// Verbindung zu APRS-IS halten (nur in der Betriebsart APRS), eigene iGate-Position alle 30 min ins Internet
    fn aprs_is_tick(&mut self, ctx: &egui::Context) {
        let call = self.aprs_my_call();
        let msg_net = self.aprs.msg_rx.internet() || matches!(self.aprs.msg_tx, aprs::TxWeg::Internet | aprs::TxWeg::Beides);
        let want = self.op == crate::Op::Aprs && (self.aprs.igate || self.aprs.is_show || msg_net) && !self.station.call.is_empty();
        if !want { if self.aprs.is_conn.take().is_some() { self.aprs.is_status.clear(); self.aprs.is_ok = false; } return; }
        let pos = self.aprs_my_pos();
        let mut login = format!("user {call} pass {} vers Rustwave {}", aprs::passcode(&call), env!("CARGO_PKG_VERSION"));
        // Filter: Stationen im Umkreis, und immer Nachrichten an mich (mit SSID und an das Grundrufzeichen)
        let mut filt = vec![];
        if let (true, Some((a, o))) = (self.aprs.is_show, pos) { filt.push(format!("r/{a:.2}/{o:.2}/{}", self.aprs.is_km)); }
        let base = call.split('-').next().unwrap_or(&call).to_string();
        filt.push(if base != call { format!("g/{call}/{base}") } else { format!("g/{call}") });
        login += &format!(" filter {}", filt.join(" "));
        let key = format!("{}|{login}", self.aprs.is_server.trim());
        if self.aprs.is_conn.as_ref().is_none_or(|c| c.key != key) {
            self.aprs.is_ok = false;
            self.aprs.is_conn = Some(crate::igate::Conn::start(self.aprs.is_server.trim().to_string(), login, self.ev_tx.clone(), Some(ctx.clone())));
        }
        if self.aprs.igate && self.aprs.is_ok && self.aprs.is_beacon.is_none_or(|t| t.elapsed() > Duration::from_secs(1800)) {
            if let (Some((a, o)), Some(c)) = (pos, &self.aprs.is_conn) {
                let info = String::from_utf8_lossy(&aprs::position_info(a, o, ('R', '&'), "Rustwave RX-iGate")).to_string();
                c.send(format!("{call}>{},TCPIP*:{info}", aprs::TOCALL));
                self.aprs.is_beacon = Some(Instant::now());
            }
        }
    }

    fn aprs_received_src(&mut self, f: Frame, tone: Option<f64>, is: bool) {
        let me = self.aprs_my_call();
        // Drittverkehr von einem iGate (Internet -> Funk): erst weiterleiten/anzeigen wie gehabt, dann das innere Paket auswerten
        let inner = if is { None } else { aprs::third_party(&f) };
        // Empfangs-iGate: auf Funk Gehörtes ins Internet
        if !is && self.aprs.igate && self.aprs.is_ok {
            if let (Some(c), Some(l)) = (&self.aprs.is_conn, aprs::igate_line(&f, &me)) { c.send(l); self.aprs.gated += 1; }
        }
        if !is { if let Some(k) = &self.kiss { k.broadcast(&f.to_bytes()); } }
        let rf = tone.and_then(|t| self.rf_of(t));
        // Skimmer: Frequenz (bzw. Tonlage) vor das Paket schreiben
        let at = match (rf, tone) { (Some(r), _) => format!("[{:.1}] ", r as f64 / 1000.0), (None, Some(t)) => format!("[{t:.0} Hz] "), _ => String::new() };
        let mon = format!("{}{at}{}", if is { "[IS] " } else { "" }, f.monitor().trim_end());
        self.aprs.monitor.push_back((now(), mon, false));
        while self.aprs.monitor.len() > 400 { self.aprs.monitor.pop_front(); }
        if let Some(inner) = inner {
            self.aprs_message(&inner, false, true);
            return;
        }
        if !me.is_empty() && f.src.eq_ignore_ascii_case(&me) {
            // unter dem eigenen Rufzeichen: meist das Echo der eigenen Aussendung über einen Digipeater.
            // Eine Nachricht an mich ist es nie – die kommt z. B. von einem Online-Dienst, der unter meinem Rufzeichen sendet.
            let to_me = matches!(aprs::parse(&f), Body::Message { ref to, .. } | Body::Ack { ref to, .. } if aprs::addressed_to_me(to, &me));
            if !to_me {
                if f.path.iter().any(|p| p.ends_with('*')) { self.aprs.digi_heard = Some(now()); }
                return;
            }
        }
        // gelöschtes Objekt („;NAME_____ _…“): aus der Liste nehmen
        if f.info.first() == Some(&b';') && f.info.get(10) == Some(&b'_') {
            let name = format!("◆ {}", String::from_utf8_lossy(&f.info[1..10]).trim());
            self.aprs.stations.retain(|s| s.call != name);
            return;
        }
        // allgemeine Abfragen an alle: nach Zufallszeit (bis 2 min) antworten
        if self.aprs.answer_q && !is && self.aprs.q_due.is_none() {
            let q = String::from_utf8_lossy(&f.info).to_string();
            let what = if q.starts_with("?APRS?") { Some("pos") } else if q.starts_with("?IGATE?") && self.aprs.igate { Some("igate") } else { None };
            if let Some(w) = what {
                let delay = 5 + (now() as u64 * 7919 + f.src.len() as u64) % 115;
                self.aprs.q_due = Some((Instant::now() + Duration::from_secs(delay), w));
                self.log(format!("APRS: allgemeine Abfrage {} von {} – Antwort in {delay} s", q.trim(), f.src));
            }
        }
        let body = aprs::parse(&f);
        // Telemetrie-Beschreibung (PARM./UNIT./EQNS./BITS.) als Nachricht an die beschriebene Station
        if let Body::Message { to, text, .. } = &body {
            let key = to.trim().to_uppercase();
            let mut t = self.aprs.telem.get(&key).cloned().unwrap_or_default();
            if crate::aprs_telem::apply_meta(&mut t, text.trim()) { self.aprs.telem.insert(key, t); return; }
        }
        // Telemetriewerte: T#… oder Base91 im Kommentar
        let info_s = String::from_utf8_lossy(&f.info).to_string();
        if let Some((seq, raw, bits)) = crate::aprs_telem::parse_t(&info_s).or_else(|| if info_s.starts_with("T#") { None } else { crate::aprs_telem::parse_b91(&info_s) }) {
            let t = self.aprs.telem.entry(f.src.to_uppercase()).or_default();
            t.seq = seq;
            t.raw = raw;
            if bits.is_some() { t.bits = bits; }
            t.time = Some(now());
        }
        let (call, pos, symbol, comment) = match &body {
            Body::Position { lat, lon, symbol, comment } => (f.src.clone(), Some((*lat, *lon)), Some(*symbol), Some(comment.clone())),
            Body::Object { name, lat, lon, symbol, comment } => (format!("◆ {name}"), Some((*lat, *lon)), Some(*symbol), Some(comment.clone())),
            Body::Status(s) => (f.src.clone(), None, None, Some(s.clone())),
            _ => (f.src.clone(), None, None, None),
        };
        let st = match self.aprs.stations.iter_mut().find(|s| s.call == call) {
            Some(s) => s,
            None => { self.aprs.stations.push(Station { call: call.clone(), last: 0, pos: None, symbol: ('/', '/'), comment: String::new(), count: 0, tone: None, rf: None, is_only: true, wx: None,
                det: None, wxd: None, status: String::new(), path: String::new(), raw: String::new(),
                wx_hist: vec![], wx_time: None, track: vec![], digis: vec![], owner: None, direct: None, level: None, level_max: None }); self.aprs.stations.last_mut().unwrap() }
        };
        if !is { st.is_only = false; }
        // Wetter: Position mit Wettersymbol oder Wetterbericht ohne Position („_MMDDhhmm…“)
        let wx = if f.info.first() == Some(&b'_') { f.info.get(9..).and_then(|w| aprs::parse_wx(&String::from_utf8_lossy(w))) }
            else if symbol.is_some_and(|s| s.1 == '_') { comment.as_deref().and_then(aprs::parse_wx) } else { None };
        if let Some(w) = wx { st.wx = Some(w.text()); st.wxd = Some(w); }
        if let Some(d) = crate::aprsinfo::details(&f) {
            if d.kind == "Status" { st.status = d.comment.clone(); }
            else {
                if let Some(w) = &d.wx {
                    let now = chrono::Utc::now().timestamp();
                    st.wx = Some(w.text());
                    st.wxd = Some(w.clone());
                    st.wx_time = Some(now);
                    // Verlauf: höchstens alle 5 min ein Punkt, 6 h zurück
                    if st.wx_hist.last().is_none_or(|h| now - h.0 >= 300) { st.wx_hist.push((now, w.hpa, w.temp_c)); }
                    st.wx_hist.retain(|h| now - h.0 <= 6 * 3600);
                }
                // Mic-E und Positionsmeldungen ohne Gerätekennung: zuletzt bekanntes Gerät behalten
                let dev = st.det.as_ref().and_then(|o| o.device.clone());
                st.det = Some(crate::aprsinfo::Details { device: d.device.clone().or(dev), ..d });
            }
        }
        st.path = if is { "APRS-IS (Internet)".into() } else {
            let used: Vec<&str> = f.path.iter().filter(|p| p.ends_with('*')).map(|p| p.trim_end_matches('*')).collect();
            if used.is_empty() { format!("direkt gehört ({})", if f.path.is_empty() { "ohne Pfad".into() } else { f.path.join(",") }) }
            else { format!("über {} (Pfad {})", used.join(", "), f.path.join(",")) }
        };
        st.raw = String::from_utf8_lossy(&f.info).to_string();
        st.last = now();
        if tone.is_some() { st.tone = tone; st.rf = rf; }
        st.count += 1;
        if let Some(p) = pos {
            st.pos = Some(p);
            // Spur: neuer Punkt, wenn mindestens 30 m bewegt; 24 h, höchstens 1000 Punkte
            let t = now();
            let moved = st.track.last().is_none_or(|l| aprs::dist_bearing((l.1, l.2), p).0 >= 0.03);
            if moved { st.track.push((t, p.0, p.1)); }
            st.track.retain(|x| t - x.0 <= 86400);
            if st.track.len() > 1000 { st.track.remove(0); }
        }
        if !is {
            if let Some(v) = self.poll.s_recent(3.0) { st.level = Some(v); st.level_max = Some(st.level_max.map_or(v, |m| m.max(v))); }
            st.digis = f.path.iter().filter(|p| p.ends_with('*')).map(|p| p.trim_end_matches('*').to_uppercase()).collect();
            if st.digis.is_empty() { st.direct = Some(now()); }
        }
        if let Body::Object { .. } = &body { st.owner = Some(f.src.clone()); }
        if let Some(s) = symbol { st.symbol = s; }
        if let Some(c) = comment { if !c.is_empty() { st.comment = c; } }
        let call_now = st.call.clone();
        self.aprs_alerts(&call_now);

        match body {
            Body::Message { to, text, .. } if to.to_uppercase().starts_with("BLN") => {
                let b = &mut self.aprs.bulletins;
                if !b.iter().any(|x| x.from == f.src && x.to == to && x.text == text) {
                    b.push(Bulletin { time: now(), from: f.src.clone(), to, text });
                    if b.len() > 100 { b.remove(0); }
                }
            }
            Body::Message { .. } | Body::Ack { .. } => self.aprs_message(&f, is, false),
            _ => {}
        }
    }

    fn aprs_is_ready(&self) -> bool { self.aprs.is_ok && self.aprs.is_conn.is_some() }

    /// Nachricht als Antwort einreihen (mit Nummer, Wiederholung und Bestätigung wie eine eigene)
    fn aprs_reply(&mut self, to: &str, text: String) {
        let id = self.aprs.msg_counter.to_string();
        self.aprs.msg_counter = self.aprs.msg_counter % 999 + 1;
        let text: String = text.chars().filter(|c| !matches!(c, '|' | '~' | '{')).take(67).collect();
        self.aprs.msgs.push(Msg { time: now(), partner: base_call(to).to_uppercase(), text, id: Some(id), out: true, acked: false, tries: 0,
            next: Some(Instant::now() + Duration::from_secs(3)), seen: true, via: String::new() });
    }

    /// gerichtete Abfrage in einer Nachricht an mich beantworten (APRS 1.0.1 Kap. 15)
    fn aprs_query(&mut self, f: &Frame) {
        if !self.aprs.answer_q { return; }
        let Body::Message { text, .. } = aprs::parse(f) else { return };
        let q = text.trim().to_uppercase();
        if !q.starts_with('?') { return; }
        let from = f.src.to_uppercase();
        if self.aprs.q_last.get(&from).is_some_and(|t| t.elapsed() < Duration::from_secs(60)) { return; }
        let t = now();
        let answer = match q.split_whitespace().next().unwrap_or("") {
            "?APRSP" | "?APRS?" => { self.aprs.last_reason = "Abfrage"; self.aprs_beacon(); "Position gesendet".to_string() }
            "?APRSS" => {
                let st = if self.aprs.comment.trim().is_empty() { format!("Rustwave {}", env!("CARGO_PKG_VERSION")) } else { self.aprs.comment.clone() };
                self.aprs_send_info(format!(">{st}").into_bytes(), true, false);
                "Status gesendet".into()
            }
            "?APRSO" => { self.aprs_objects_send(); "Objekte gesendet".into() }
            "?APRSD" => {
                let mut d: Vec<&Station> = self.aprs.stations.iter().filter(|s| s.direct.is_some_and(|x| t - x < 3600) && !s.call.starts_with('◆')).collect();
                d.sort_by_key(|s| -s.direct.unwrap_or(0));
                let mut txt = "Directs=".to_string();
                for s in d { if txt.len() + s.call.len() + 1 > 67 { break; } txt += " "; txt += &s.call; }
                self.aprs_reply(&from, txt);
                "gehörte Stationen geschickt".into()
            }
            "?APRSH" => {
                let who = q.split_whitespace().nth(1).unwrap_or("").to_string();
                let txt = match self.aprs.stations.iter().find(|s| s.call.eq_ignore_ascii_case(&who)) {
                    Some(s) => format!("{} HEARD {}x, last {} UTC{}", s.call, s.count,
                        chrono::DateTime::from_timestamp(s.last, 0).unwrap_or_default().format("%H:%M"), if s.direct.is_some() { " direct" } else { "" }),
                    None => format!("{who} not heard"),
                };
                self.aprs_reply(&from, txt);
                "Auskunft geschickt".into()
            }
            "?APRST" | "?PING?" => {
                let path = if f.path.is_empty() { "direct".to_string() } else { f.path.join(",") };
                self.aprs_reply(&from, format!("{}>{},{}", f.src, f.dst, path));
                "Weg geschickt".into()
            }
            _ => return,
        };
        self.aprs.q_last.insert(from.clone(), Instant::now());
        self.log(format!("APRS: Abfrage {q} von {from} beantwortet ({answer})"));
    }

    /// eigene aktive Objekte senden
    fn aprs_objects_send(&mut self) {
        let t = now();
        let objs: Vec<Obj> = self.aprs.objects.iter().filter(|o| o.live).cloned().collect();
        for o in objs {
            if let (Some(a), Some(b)) = (parse_deg(&o.lat), parse_deg(&o.lon)) {
                let sym = OBJ_SYMBOLS[o.sym.min(OBJ_SYMBOLS.len() - 1)];
                let f = self.aprs_frame(aprs::object_info(&o.name, true, a, b, (sym.0, sym.1), &o.comment, t));
                self.aprs.queue.push(f);
            }
        }
        self.aprs.obj_last = Some(Instant::now());
    }
    /// ein Objekt sofort senden (live) oder bei den anderen löschen
    fn aprs_object(&mut self, i: usize, live: bool) {
        let Some(o) = self.aprs.objects.get_mut(i) else { return };
        let (Some(a), Some(b)) = (parse_deg(&o.lat), parse_deg(&o.lon)) else { self.log("APRS-Objekt: Position als Dezimalgrad eintragen"); return };
        o.live = live;
        let o = o.clone();
        let sym = OBJ_SYMBOLS[o.sym.min(OBJ_SYMBOLS.len() - 1)];
        let f = self.aprs_frame(aprs::object_info(&o.name, live, a, b, (sym.0, sym.1), &o.comment, now()));
        self.aprs.queue.push(f);
        self.log(format!("APRS-Objekt {} {}", o.name, if live { "gesendet" } else { "gelöscht" }));
    }

    /// Hinweise prüfen: beobachtete Rufzeichen, Umkreis, Wetterschwellen (je Station und Anlass höchstens stündlich)
    fn aprs_alerts(&mut self, call: &str) {
        let Some(s) = self.aprs.stations.iter().find(|s| s.call == call) else { return };
        let mut hits: Vec<(&'static str, String)> = vec![];
        let watch: Vec<String> = self.aprs.al_calls.split(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .map(|x| x.trim().to_uppercase()).filter(|x| !x.is_empty()).collect();
        let cu = call.to_uppercase();
        if watch.iter().any(|w| match w.strip_suffix('*') { Some(p) => cu.starts_with(p), None => cu == *w || base_call(&cu).split('-').next() == Some(w.as_str()) }) {
            hits.push(("call", format!("{call} gehört ({})", s.path)));
        }
        if let (true, Some(me), Some(p)) = (self.aprs.al_km > 0.0, self.aprs_my_pos(), s.pos) {
            let (d, br) = aprs::dist_bearing(me, p);
            if d <= self.aprs.al_km { hits.push(("near", format!("{call} ist {d:.1} km entfernt ({})", aprs::compass(br)))); }
        }
        if let Some(w) = &s.wxd {
            if let Some(g) = w.gust_kmh.filter(|g| self.aprs.al_gust > 0.0 && *g >= self.aprs.al_gust) { hits.push(("gust", format!("{call}: Böen {g:.0} km/h"))); }
            if let Some(t) = w.temp_c.filter(|t| self.aprs.al_frost && *t <= 0.0) { hits.push(("frost", format!("{call}: Frost {t:.1} °C"))); }
            if let Some(r) = w.rain_mm.filter(|r| self.aprs.al_rain > 0.0 && *r >= self.aprs.al_rain) { hits.push(("rain", format!("{call}: Regen {r:.1} mm/h"))); }
        }
        let t = now();
        for (k, text) in hits {
            let key = (call.to_string(), k);
            if self.aprs.al_fired.get(&key).is_some_and(|x| t - x < 3600) { continue; }
            self.aprs.al_fired.insert(key, t);
            self.log(format!("APRS-Hinweis: {text}"));
            self.aprs.alerts.push((t, text));
            if self.aprs.alerts.len() > 50 { self.aprs.alerts.remove(0); }
            self.attention = true;
        }
    }

    /// Nachricht oder Bestätigung senden: auf Funk (Warteschlange) und/oder übers Internet; liefert den genutzten Weg
    fn aprs_send_info(&mut self, info: Vec<u8>, rf: bool, net: bool) -> String {
        let net_ok = net && self.aprs_is_ready();
        if net_ok {
            let line = format!("{}>{},TCPIP*:{}", self.aprs_my_call(), aprs::TOCALL, String::from_utf8_lossy(&info));
            self.aprs.monitor.push_back((now(), format!("[IS] {line}"), true));
            if let Some(c) = &self.aprs.is_conn { c.send(line); }
        }
        if rf { let f = self.aprs_frame(info); self.aprs.queue.push(f); }
        match (rf, net_ok, net) {
            (true, true, _) => "Funk + Internet", (true, false, true) => "Funk (Internet nicht verbunden)",
            (true, false, false) => "Funk", (false, true, _) => "Internet", (false, false, _) => "wartet auf Internet",
        }.into()
    }

    /// Nachricht oder Bestätigung an mich: anzeigen, bestätigen, eigene Nachrichten als bestätigt markieren.
    /// `is`: kam über APRS-IS (Bestätigung dann übers Internet), `third`: kam als Drittverkehr über ein iGate auf Funk
    fn aprs_message(&mut self, f: &Frame, is: bool, third: bool) {
        let me = self.aprs_my_call();
        if me.is_empty() { return; }
        let mark_acked = |s: &mut Self, id: &str| {
            if let Some(m) = s.aprs.msgs.iter_mut().find(|m| m.out && m.partner.eq_ignore_ascii_case(&f.src) && m.id.as_deref() == Some(id)) {
                if !m.acked { m.acked = true; s.log(format!("APRS: {} hat Nachricht {id} bestätigt", f.src)); }
            }
        };
        match aprs::parse(f) {
            // Nachricht über einen abgewählten Weg: nicht annehmen (Bestätigungen für uns zählen dagegen immer)
            Body::Message { to, .. } if aprs::addressed_to_me(&to, &me) && !self.aprs.msg_rx.accepts(is) => {}
            Body::Message { to, text, id } if aprs::addressed_to_me(&to, &me) => {
                // Reply-Ack „{MM}AA“: AA bestätigt eine unserer Nachrichten
                if let Some((_, Some(aa))) = id.as_deref().map(aprs::split_id) { mark_acked(self, aa); }
                let dup = self.aprs.msgs.iter().any(|m| !m.out && m.partner.eq_ignore_ascii_case(&f.src) && m.id == id && m.text == text);
                if !dup {
                    let via = if is { " (über das Internet)" } else if third { " (über ein iGate)" } else { "" };
                    self.log(format!("APRS-Nachricht von {}{via}: {text}", f.src));
                    let via = if is { "Internet" } else if third { "iGate" } else { "Funk" }.to_string();
                    self.aprs.msgs.push(Msg { time: now(), partner: base_call(&f.src).into(), text, id: id.clone(), out: false, acked: false, tries: 0, next: None, seen: false, via });
                    if id.is_none() { self.aprs_query(f); }
                }
                // auch bei Wiederholungen bestätigen – unsere Bestätigung ging evtl. verloren
                if is { self.aprs.is_partners.insert(f.src.to_uppercase()); } else { self.aprs.is_partners.remove(&f.src.to_uppercase()); }
                let Some(id) = id else { return };
                // Bestätigung auf dem eingestellten Weg (Automatisch: wie die Nachricht kam).
                // Auf Funk leitet ein iGate die Bestätigung bei Bedarf ins Internet weiter.
                let (rf, net) = self.aprs.msg_tx.paths(is);
                self.aprs_send_info(aprs::ack_info(&f.src, &id), rf, net);
                self.aprs.not_before = self.aprs.not_before.max(Instant::now() + ACK_DELAY);
                if !dup { self.aprs_query(f); }
            }
            Body::Ack { to, id } if aprs::addressed_to_me(&to, &me) => mark_acked(self, aprs::split_id(&id).0),
            _ => {}
        }
    }

    /// Zeitsteuerung: automatische Bake, Wiederholungen, Senden der Warteschlange
    pub(crate) fn aprs_tick(&mut self, ctx: &egui::Context) {
        self.aprs_is_tick(ctx);
        if self.op != crate::Op::Aprs { return; }
        ctx.request_repaint_after(Duration::from_secs(1));
        let path_ready = !self.use_net || self.net.is_some();
        if self.aprs.smart && !self.station.call.is_empty() {
            let since = self.aprs.last_beacon.map_or(f64::MAX, |t| t.elapsed().as_secs_f64());
            // ohne GPS (noch kein Empfang): wie im Stand alle 30 min
            let fix = self.gps.fresh(Duration::from_secs(15));
            let speed = fix.and_then(|f| f.speed_kmh).unwrap_or(0.0);
            // nach dem Verbinden bis zu 60 s auf die erste GPS-Position warten, sonst ginge die erste Bake mit der alten Position raus
            let waiting = fix.is_none() && self.gps.waiting_first(Duration::from_secs(60));
            // noch kein Kurs von einer früheren Bake: in Fahrt nach 30 s eine Bake mit Kurs senden
            let prof = &aprs::SMART_PROFILES[self.aprs.smart_prof].1;
            let first_moving = fix.is_some() && speed >= prof.slow_kmh && self.aprs.last_course.is_none() && since >= prof.min_turn_s;
            let due = if first_moving { Some("Fahrt") } else { aprs::smart_due(prof, speed, fix.and_then(|f| f.course), self.aprs.last_course, since) };
            if let Some(reason) = due.filter(|_| !waiting && path_ready) {
                self.aprs.last_reason = reason;
                self.aprs_beacon();
            }
        } else if self.aprs.beacon_min > 0 && !self.station.call.is_empty()
            && self.aprs.last_beacon.is_none_or(|t| t.elapsed() >= Duration::from_secs(self.aprs.beacon_min as u64 * 60)) {
            self.aprs_beacon();
        }
        // Objekte ohne eigene Bake alle 10 min wiederholen
        if self.aprs.obj_repeat && self.aprs.objects.iter().any(|o| o.live)
            && self.aprs.obj_last.is_none_or(|t| t.elapsed() >= Duration::from_secs(600)) && self.aprs.beacon_min == 0 && !self.aprs.smart {
            self.aprs_objects_send();
        }
        // fällige Antwort auf eine allgemeine Abfrage (?APRS? bzw. ?IGATE?)
        if let Some((t, what)) = self.aprs.q_due {
            if Instant::now() >= t {
                self.aprs.q_due = None;
                match what {
                    "pos" => { self.aprs.last_reason = "Abfrage"; self.aprs_beacon(); }
                    _ => {
                        let st = format!("<IGATE,MSG_CNT=0,LOC_CNT={}", self.aprs.stations.iter().filter(|s| s.direct.is_some_and(|d| now() - d < 1800)).count());
                        self.aprs_send_info(st.into_bytes(), true, false);
                    }
                }
            }
        }
        let due: Vec<usize> = self.aprs.msgs.iter().enumerate()
            .filter(|(_, m)| m.out && !m.acked && m.tries < MAX_TRIES && m.next.is_some_and(|t| Instant::now() >= t)).map(|(i, _)| i).collect();
        for i in due {
            let (to, text, id) = { let m = &self.aprs.msgs[i]; (m.partner.clone(), m.text.clone(), m.id.clone()) };
            let info = aprs::message_info(&to, &text, id.as_deref());
            let (rf, net) = self.aprs.msg_tx.paths(self.aprs.is_partners.contains(&to.to_uppercase()));
            // nur Internet gewählt, aber gerade keine Verbindung: warten (nicht still auf Funk ausweichen)
            if net && !rf && !self.aprs_is_ready() { continue; }
            let via = self.aprs_send_info(info, rf, net);
            let m = &mut self.aprs.msgs[i];
            m.via = via;
            m.tries += 1;
            m.next = Some(Instant::now() + RETRY);
        }
        // WLAN gewählt, aber (noch) nicht verbunden: warten statt über die Soundkarte des PCs zu senden
        let path_ready = !self.use_net || self.net.is_some();
        let busy_ch = self.rx_sh.packet_dcd.load(std::sync::atomic::Ordering::Relaxed);
        if self.tx.is_none() && path_ready && (!self.aprs.queue.is_empty() || (!self.aprs.kiss_q.is_empty() && !busy_ch)) && Instant::now() >= self.aprs.not_before {
            self.aprs.sending = std::mem::take(&mut self.aprs.queue);
            self.aprs.sending_raw = std::mem::take(&mut self.aprs.kiss_q);
            for f in self.aprs.sending.clone() { self.aprs.monitor.push_back((now(), f.monitor(), true)); }
            for b in self.aprs.sending_raw.clone() {
                let t = Frame::from_bytes(&b).map_or_else(|| "(Rahmen eines anderen Programms)".to_string(), |f| f.monitor());
                self.aprs.monitor.push_back((now(), format!("[KISS] {t}"), true));
            }
            self.start_tx(TxKind::Aprs);
        }
    }

    pub(crate) fn ui_aprs_rx(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let me = self.aprs_my_pos();
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(self.tx.is_none(), |ui| {
                for (hf, label, tip) in [(false, "UKW 1200 Bd", "FM, Bell 202 (1200/2200 Hz)"), (true, "KW 300 Bd", "USB, 1600/1800 Hz – z. B. 10,1476 MHz")] {
                    if ui.selectable_label(self.aprs.hf == hf, label).on_hover_text(tip).clicked() && self.aprs.hf != hf {
                        self.aprs.hf = hf;
                        self.log(if hf { "APRS Kurzwelle: 300 Baud, USB, Töne 1600/1800 Hz" } else { "APRS UKW: 1200 Baud, FM" });
                    }
                }
            });
            if self.aprs.hf {
                ui.checkbox(&mut self.aprs.skim, "Skimmer").on_hover_text(format!(
                    "Ganzes Durchlassband absuchen (Tonmitte {:.0}–{:.0} Hz statt nur um 1700 Hz): findet auch Stationen neben der Normalfrequenz \
                     und zeigt, auf welcher Frequenz jede gehört wurde. Filter am Funkgerät dafür breit lassen (2,4 kHz).", aprs::SKIM_RANGE.0, aprs::SKIM_RANGE.1));
            }
            ui.separator();
            ui.heading(format!("{} Stationen gehört", self.aprs.stations.len()));
            ui.add(egui::ProgressBar::new((self.level * 2.0).min(1.0)).desired_width(90.0).desired_height(8.0)).on_hover_text("Eingangspegel");
        });
        if let Some(n) = &self.net {
            let sh = &n.shared;
            let (rx, rec, lost) = (sh.audio_rx.load(std::sync::atomic::Ordering::Relaxed), sh.audio_recovered.load(std::sync::atomic::Ordering::Relaxed), sh.audio_lost.load(std::sync::atomic::Ordering::Relaxed));
            let pct = 100.0 * lost as f64 / (rx + lost).max(1) as f64;
            ui.label(RichText::new(format!("WLAN-Audio: {rx} Pakete, {rec} nachgefordert, {lost} verloren ({pct:.2} %)")).small()
                .color(if pct > 0.5 { crate::RED } else if pct > 0.05 { ORANGE } else { Color32::GRAY }))
                .on_hover_text("Jedes verlorene 20-ms-Paket kann ein APRS-Paket zerstören. Bei vielen Verlusten: näher an den Router, 5-GHz-WLAN am PC meiden (IC-705 funkt nur in 2,4 GHz) oder USB-Kabel nutzen.");
        }
        // nur Stille vom Funkgerät: meist ist die Rauschsperre zu – dann öffnen schwache Pakete sie nicht
        if let Some(t) = self.silent_since.filter(|t| t.elapsed() > Duration::from_secs(20) && self.tx.is_none()) {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("Seit {} s kommt vom Funkgerät nur Stille. Ist die Rauschsperre (SQL) zu? Für APRS die Rauschsperre ganz öffnen – \
                    der Dekoder kommt mit Rauschen zurecht, verpasst aber Pakete, die die Sperre nicht öffnen.", t.elapsed().as_secs())).small().color(ORANGE));
                let civ = self.rig.as_ref().is_some_and(|r| r.model.proto == crate::cat::Proto::Civ);
                if civ && ui.button("Rauschsperre öffnen").on_hover_text("stellt SQL am Funkgerät auf 0 (offen)").clicked() {
                    if let Some(r) = &self.rig { self.poll.cmdq.push_back((0, crate::cat::civ(&r.model, &[0x14, 0x03, 0x00, 0x00]))); }
                    self.log("Rauschsperre am Funkgerät geöffnet (SQL 0)");
                }
            });
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
        if !self.aprs.is_status.is_empty() {
            let mut t = self.aprs.is_status.clone();
            if self.aprs.igate { t += &format!(" · {} weitergeleitet", self.aprs.gated); }
            if self.aprs.is_show { t += &format!(" · {} aus dem Internet", self.aprs.is_rx); }
            ui.label(RichText::new(t).small().color(if self.aprs.is_ok { GREEN } else { ORANGE }));
        }
        ui.label(RichText::new(match me {
            Some((a, o)) => format!("Eigene Position {a:.4}, {o:.4} · Klick auf eine Station setzt sie als Empfänger einer Nachricht"),
            None => "Eigene Position unbekannt – bei der Bake eintragen oder Locator unter Einstellungen › Station angeben".into(),
        }).small().weak());
        let h = ui.ctx().content_rect().height() * 0.4;
        ui.horizontal(|ui| {
            for (m, label) in [(false, "Liste"), (true, "Karte")] {
                if ui.selectable_label(self.aprs.map == m, label).clicked() { self.aprs.map = m; }
            }
            ui.separator();
            ui.menu_button("Export", |ui| {
                if ui.button("Stationen und Spuren als GPX …").on_hover_text("für Navigationsgeräte und Karten-Apps").clicked() { self.aprs_export(false); ui.close(); }
                if ui.button("Stationen und Spuren als KML …").on_hover_text("für Google Earth").clicked() { self.aprs_export(true); ui.close(); }
            }).response.on_hover_text("Gehörte Stationen mit Position und ihre Spuren speichern");
            let n = self.aprs.alerts.len();
            if n > 0 { ui.label(RichText::new(format!("{n} Hinweis{}", if n == 1 { "" } else { "e" })).small().color(ORANGE)); }
        });
        if self.aprs.map {
            if let Some(c) = self.ui_aprs_map(ui, me, h) { if !c.starts_with('◆') { self.aprs.msg_to = c; } }
            self.ui_aprs_alerts(ui);
            ui.separator();
            self.ui_aprs_monitor(ui);
            return;
        }
        let mut pick = None;
        let mut hover_card: Option<usize> = None;
        let mut order: Vec<usize> = (0..self.aprs.stations.len()).collect();
        order.sort_by_key(|&i| -self.aprs.stations[i].last);
        // Frequenzspalte, sobald der Skimmer etwas geliefert hat
        let qrg = self.aprs.stations.iter().any(|s| s.tone.is_some());
        egui::ScrollArea::vertical().id_salt("aprs_st").max_height(h).auto_shrink([false, true]).show(ui, |ui| {
            let lvl = self.aprs.stations.iter().any(|s| s.level.is_some());
            egui::Grid::new("aprs_grid").striped(true).num_columns(6 + qrg as usize + lvl as usize).spacing([12.0, 3.0]).show(ui, |ui| {
                let mut cols: Vec<&str> = if qrg { vec!["Rufzeichen", "Frequenz", "zuletzt", "Pakete", "Entfernung", "Symbol", "Kommentar"] }
                    else { vec!["Rufzeichen", "zuletzt", "Pakete", "Entfernung", "Symbol", "Kommentar"] };
                if lvl { let k = cols.iter().position(|c| *c == "Pakete").unwrap() + 1; cols.insert(k, "Pegel"); }
                for t in cols { ui.label(RichText::new(t).strong()); }
                ui.end_row();
                for &i in &order {
                    let s = &self.aprs.stations[i];
                    let ct = if s.is_only { RichText::new(&s.call).monospace().weak() } else { RichText::new(&s.call).monospace().strong() };
                    // ganze Zeile: Fläche merken, Stationskarte erscheint sofort beim Darüberfahren (wie in der Karte)
                    let rc = ui.add(egui::Label::new(ct).sense(egui::Sense::click()));
                    if rc.clicked() { pick = Some(s.call.clone()); }
                    let mut row = rc.rect;
                    if qrg {
                        match s.tone {
                            Some(t) => {
                                // mehr als 100 Hz neben der Normallage (1700 Hz): hervorheben
                                let c = if (t - 1700.0).abs() > 100.0 { ORANGE } else { ui.visuals().text_color() };
                                let txt = s.rf.map_or(format!("{t:.0} Hz"), |r| format!("{:.1}", r as f64 / 1000.0));
                                row = row.union(ui.label(RichText::new(txt).monospace().color(c)).rect);
                            }
                            None => { row = row.union(ui.label("–").rect); }
                        }
                    }
                    row = row.union(ui.label(hhmm(s.last)).rect);
                    row = row.union(ui.label(s.count.to_string()).rect);
                    if lvl {
                        let t = s.level.map_or("–".to_string(), crate::rigpoll::s_text);
                        row = row.union(ui.label(RichText::new(t).monospace()).rect);
                    }
                    row = row.union(ui.label(match (me, s.pos) {
                        (Some(a), Some(b)) => { let (d, br) = aprs::dist_bearing(a, b); format!("{d:.0} km {}", aprs::compass(br)) }
                        (None, Some(b)) => format!("{:.3}, {:.3}", b.0, b.1),
                        _ => "–".into(),
                    }).rect);
                    row = row.union(ui.label(if s.pos.is_some() { aprs::symbol_name(s.symbol) } else { "–".into() }).rect);
                    // Kommentar ohne Kurs/Höhe/PHG-Kürzel; Mouse-over zeigt alles
                    let shown = match (&s.wx, &s.det) {
                        (Some(w), _) => RichText::new(w).color(WX_BLUE),
                        (None, Some(d)) => {
                            let mut parts = vec![];
                            if let (Some(v), true) = (d.speed_kmh, d.speed_kmh.is_some_and(|v| v >= 1.0)) {
                                parts.push(format!("{v:.0} km/h{}", d.course.map_or(String::new(), |c| format!(" {}", aprs::compass(c)))));
                            }
                            if !d.comment.is_empty() { parts.push(d.comment.clone()); } else if !s.status.is_empty() { parts.push(s.status.clone()); }
                            RichText::new(parts.join(" · "))
                        }
                        _ => RichText::new(&s.comment),
                    };
                    row = row.union(ui.add(egui::Label::new(shown).truncate()).rect);
                    ui.end_row();
                    if hover_card.is_none() && ui.rect_contains_pointer(row.expand2(egui::vec2(6.0, 2.0))) { hover_card = Some(i); }
                }
            });
        });
        if let Some(i) = hover_card {
            let s = &self.aprs.stations[i];
            egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), egui::Id::new("aprs_row_card"), egui::PopupAnchor::Pointer).show(|ui| {
                station_card(ui, s, me, self.aprs.telem.get(&s.call.to_uppercase()));
                ui.label(RichText::new(if s.is_only { "nur übers Internet (APRS-IS) gehört – Klick aufs Rufzeichen: als Empfänger übernehmen" }
                    else { "Klick aufs Rufzeichen: als Empfänger übernehmen" }).small().weak());
            });
        }
        if let Some(c) = pick { if !c.starts_with('◆') { self.aprs.msg_to = c; } }
        self.ui_aprs_alerts(ui);
        ui.separator();
        self.ui_aprs_monitor(ui);
    }

    /// eigene Objekte: anlegen, senden, löschen
    fn ui_aprs_objects(&mut self, ui: &mut egui::Ui) {
        let live = self.aprs.objects.iter().filter(|o| o.live).count();
        egui::CollapsingHeader::new(if live > 0 { format!("Objekte ({live} aktiv)") } else { "Objekte".into() }).id_salt("aprs_obj").show(ui, |ui| {
            ui.label(RichText::new("Objekte stehen für Dinge statt Stationen – Treffpunkt, Fieldday, Relais, Veranstaltung. Sie erscheinen bei anderen \
                unter ihrem Namen mit deinem Rufzeichen als Besitzer; „Löschen“ nimmt sie dort wieder heraus.").small().weak());
            let mut act: Option<(usize, bool)> = None;
            let mut del = None;
            egui::Grid::new("aprs_obj_g").num_columns(5).spacing([8.0, 4.0]).striped(true).show(ui, |ui| {
                for (i, o) in self.aprs.objects.iter().enumerate() {
                    ui.label(RichText::new(&o.name).monospace().strong().color(if o.live { GREEN } else { Color32::GRAY }));
                    ui.label(RichText::new(format!("{} · {}, {}", OBJ_SYMBOLS[o.sym.min(OBJ_SYMBOLS.len() - 1)].2, o.lat, o.lon)).small());
                    ui.add(egui::Label::new(RichText::new(&o.comment).small()).truncate());
                    ui.horizontal(|ui| {
                        if ui.small_button(if o.live { "erneut senden" } else { "senden" }).clicked() { act = Some((i, true)); }
                        if o.live && ui.small_button("löschen").on_hover_text("sendet das Objekt als gelöscht").clicked() { act = Some((i, false)); }
                        if !o.live && ui.small_button("✖").on_hover_text("aus der Liste entfernen").clicked() { del = Some(i); }
                    });
                    ui.end_row();
                }
            });
            if let Some((i, l)) = act { self.aprs_object(i, l); }
            if let Some(i) = del { self.aprs.objects.remove(i); }
            ui.add_space(4.0);
            let n = &mut self.aprs.obj_new;
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut n.name).desired_width(80.0).char_limit(9).hint_text("Name"));
                ui.add(egui::TextEdit::singleline(&mut n.lat).desired_width(70.0).hint_text("Breite"));
                ui.add(egui::TextEdit::singleline(&mut n.lon).desired_width(70.0).hint_text("Länge"));
                egui::ComboBox::from_id_salt("obj_sym").width(110.0).selected_text(OBJ_SYMBOLS[n.sym].2).show_ui(ui, |ui| {
                    for (i, s) in OBJ_SYMBOLS.iter().enumerate() { ui.selectable_value(&mut n.sym, i, s.2); }
                });
            });
            ui.add(egui::TextEdit::singleline(&mut n.comment).desired_width(220.0).char_limit(43).hint_text("Kommentar, z. B. 145.500 MHz ab 10 Uhr"));
            let ok = !n.name.trim().is_empty() && parse_deg(&n.lat).is_some_and(|a| a.abs() <= 90.0) && parse_deg(&n.lon).is_some_and(|o| o.abs() <= 180.0);
            let me = self.aprs_my_pos();
            ui.horizontal(|ui| {
                if ui.add_enabled(ok, egui::Button::new("hinzufügen")).on_hover_text("Name höchstens 9 Zeichen, Position in Dezimalgrad").clicked() {
                    let mut o = self.aprs.obj_new.clone();
                    o.name = o.name.trim().to_string();
                    o.live = false;
                    self.aprs.objects.retain(|x| x.name != o.name);
                    self.aprs.objects.push(o);
                    self.aprs.obj_new.name.clear();
                    self.aprs.obj_new.comment.clear();
                }
                if ui.small_button("hier").on_hover_text("eigene Position übernehmen").clicked() {
                    if let Some((a, o)) = me { self.aprs.obj_new.lat = format!("{a:.5}"); self.aprs.obj_new.lon = format!("{o:.5}"); }
                }
            });
            ui.checkbox(&mut self.aprs.obj_repeat, "aktive Objekte mit jeder Bake wiederholen (ohne automatische Bake alle 10 min)");
        });
    }

    /// gehörte Stationen mit Position als GPX bzw. KML speichern
    fn aprs_export(&mut self, kml: bool) {
        let items: Vec<crate::aprs_export::Item> = self.aprs.stations.iter().filter_map(|s| {
            let pos = s.pos?;
            let mut desc = vec![aprs::symbol_name(s.symbol)];
            if let Some(w) = &s.wx { desc.push(w.clone()); } else if !s.comment.is_empty() { desc.push(s.comment.clone()); }
            Some(crate::aprs_export::Item { call: &s.call, pos, last: s.last, desc: desc.join(" · "), track: &s.track })
        }).collect();
        if items.is_empty() { self.log("APRS-Export: noch keine Stationen mit Position"); return; }
        let (txt, ext) = if kml { (crate::aprs_export::kml(&items), "kml") } else { (crate::aprs_export::gpx(&items), "gpx") };
        let n = items.len();
        let path = crate::save_dir().join(format!("aprs-{}.{ext}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
        match std::fs::create_dir_all(crate::save_dir()).and_then(|_| std::fs::write(&path, txt)) {
            Ok(()) => self.log(format!("APRS: {n} Stationen gespeichert in {}", path.display())),
            Err(e) => self.log(format!("APRS-Export: {e}")),
        }
    }

    /// Hinweise: Einstellungen und zuletzt ausgelöste
    fn ui_aprs_alerts(&mut self, ui: &mut egui::Ui) {
        let n = self.aprs.alerts.len();
        egui::CollapsingHeader::new(if n > 0 { format!("Hinweise ({n})") } else { "Hinweise".into() }).id_salt("aprs_al").show(ui, |ui| {
            let a = &mut self.aprs;
            egui::Grid::new("aprs_al_g").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                ui.label("Rufzeichen");
                ui.add(egui::TextEdit::singleline(&mut a.al_calls).desired_width(220.0).hint_text("z. B. DL1ABC, DB0* (leer = aus)"))
                    .on_hover_text("Meldung, sobald eine dieser Stationen gehört wird; * am Ende steht für beliebige Zeichen, ohne SSID gilt jede SSID");
                ui.end_row();
                ui.label("Umkreis");
                ui.add(egui::DragValue::new(&mut a.al_km).range(0.0..=500.0).speed(0.5).suffix(" km")).on_hover_text("Station näher als … an deiner Position (0 = aus)");
                ui.end_row();
                ui.label("Wetter");
                ui.horizontal(|ui| {
                    ui.label("Böen ab");
                    ui.add(egui::DragValue::new(&mut a.al_gust).range(0.0..=200.0).suffix(" km/h")).on_hover_text("0 = aus");
                    ui.label("Regen ab");
                    ui.add(egui::DragValue::new(&mut a.al_rain).range(0.0..=100.0).speed(0.1).suffix(" mm/h")).on_hover_text("0 = aus");
                    ui.checkbox(&mut a.al_frost, "Frost");
                });
                ui.end_row();
            });
            ui.label(RichText::new("Je Station und Anlass höchstens einmal pro Stunde; das Fenster blinkt in der Taskleiste.").small().weak());
            if n > 0 {
                egui::ScrollArea::vertical().id_salt("aprs_al_s").max_height(90.0).stick_to_bottom(true).show(ui, |ui| {
                    for (t, text) in &a.alerts { ui.label(RichText::new(format!("{}  {text}", hhmm(*t))).small().monospace().color(ORANGE)); }
                });
                if ui.small_button("Hinweise leeren").clicked() { a.alerts.clear(); }
            }
        });
    }

    fn ui_aprs_monitor(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Monitor").strong());
            ui.menu_button("…", |ui| {
                if ui.button("Monitor leeren").clicked() { self.aprs.monitor.clear(); }
                if ui.button("WAV dekodieren …").on_hover_text("z. B. eine Aufnahme – Dateien lassen sich auch ins Fenster ziehen").clicked() { let c = ui.ctx().clone(); self.pick_wav(&c); }
            }).response.on_hover_text("weitere Funktionen");
        });
        egui::Frame::new().fill(Color32::from_gray(12)).inner_margin(6.0).corner_radius(4.0).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("aprs_mon").stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                for (t, line, out) in &self.aprs.monitor {
                    let ts = chrono::DateTime::<chrono::Utc>::from_timestamp(*t, 0).map_or(String::new(), |d| d.with_timezone(&chrono::Local).format("%H:%M:%S").to_string());
                    // Pakete haben lange Wörter ohne Leerzeichen: überall umbrechen, nicht im Blocksatz strecken
                    let mut job = egui::text::LayoutJob::simple(format!("{ts} {line}"), egui::FontId::monospace(12.0),
                        if *out { ORANGE } else { Color32::from_gray(220) }, ui.available_width());
                    job.wrap.break_anywhere = true;
                    job.justify = false;
                    ui.add(egui::Label::new(job));
                }
            });
        });
    }

    /// Karte ohne Kacheln: eigene Position in der Mitte, Ringe mit Entfernung, Norden oben.
    /// Liefert das angetippte Rufzeichen.
    fn ui_aprs_map(&mut self, ui: &mut egui::Ui, me: Option<(f64, f64)>, h: f32) -> Option<String> {
        let pts: Vec<(&Station, (f64, f64))> = self.aprs.stations.iter().filter_map(|s| s.pos.map(|p| (s, p))).collect();
        let tel = &self.aprs.telem;
        let Some(center) = me.or_else(|| pts.first().map(|p| p.1)) else {
            ui.label(RichText::new("Noch keine Position – eigene Position eintragen oder auf Stationen mit Position warten.").weak());
            return None;
        };
        // ebene Näherung um den Mittelpunkt (km)
        let to_km = |p: (f64, f64)| ((p.1 - center.1) * 111.32 * center.0.to_radians().cos(), (p.0 - center.0) * 110.57);
        let far = pts.iter().map(|p| { let (x, y) = to_km(p.1); x.hypot(y) }).fold(0.0f64, f64::max);
        let auto = (far * 1.15).max(5.0);
        let radius_km = self.aprs.map_km.unwrap_or(auto);
        let mut picked = None;
        ui.horizontal_wrapped(|ui| {
            if ui.button("−").on_hover_text("weiter weg").clicked() { self.aprs.map_km = Some(radius_km * 1.6); }
            if ui.button("+").on_hover_text("näher heran").clicked() { self.aprs.map_km = Some((radius_km / 1.6).max(0.5)); }
            if ui.selectable_label(self.aprs.map_km.is_none(), "Auto").on_hover_text("alle Stationen zeigen").clicked() { self.aprs.map_km = None; }
            ui.label(RichText::new(format!("Radius {radius_km:.0} km · Norden oben")).small().weak());
            ui.separator();
            ui.checkbox(&mut self.aprs.map_tracks, "Spuren").on_hover_text("Weg mobiler Stationen (letzte 24 h)");
            ui.checkbox(&mut self.aprs.map_paths, "Funkwege").on_hover_text("Linien von der Station über die benutzten Digipeater zu dir. \
                Die Station unter dem Mauszeiger zeigt ihren Weg immer.");
        });
        let w = ui.available_width();
        let (resp, painter) = ui.allocate_painter(egui::vec2(w, h), egui::Sense::click_and_drag());
        let r = resp.rect;
        painter.rect_filled(r, 4.0, Color32::from_gray(14));
        let c = r.center();
        let px_per_km = (r.width().min(r.height()) / 2.0 - 8.0) / radius_km as f32;
        let to_px = |p: (f64, f64)| { let (x, y) = to_km(p); egui::pos2(c.x + x as f32 * px_per_km, c.y - y as f32 * px_per_km) };
        // Ringe in runden Abständen
        let step = [0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0].into_iter()
            .find(|s| radius_km / s <= 4.0).unwrap_or(5000.0);
        let painter = painter.with_clip_rect(r);
        let mut k = step;
        while k <= radius_km * 1.5 {
            painter.circle_stroke(c, k as f32 * px_per_km, egui::Stroke::new(1.0_f32, Color32::from_gray(50)));
            painter.text(egui::pos2(c.x + 3.0, c.y - k as f32 * px_per_km + 2.0), egui::Align2::LEFT_TOP, format!("{k} km"), egui::FontId::proportional(10.0), Color32::from_gray(110));
            k += step;
        }
        painter.text(egui::pos2(r.center().x, r.top() + 2.0), egui::Align2::CENTER_TOP, "N", egui::FontId::proportional(12.0), Color32::from_gray(150));
        if me.is_some() {
            painter.line_segment([c - egui::vec2(6.0, 0.0), c + egui::vec2(6.0, 0.0)], egui::Stroke::new(2.0_f32, Color32::WHITE));
            painter.line_segment([c - egui::vec2(0.0, 6.0), c + egui::vec2(0.0, 6.0)], egui::Stroke::new(2.0_f32, Color32::WHITE));
        }
        let near = resp.hover_pos().and_then(|hp| pts.iter().map(|(s, p)| (hp.distance(to_px(*p)), *s)).filter(|(d, _)| *d < 14.0)
            .min_by(|a, b| a.0.total_cmp(&b.0)).map(|x| x.1));
        // Spuren
        if self.aprs.map_tracks {
            for (s, _) in &pts {
                if s.track.len() < 2 { continue; }
                let line: Vec<egui::Pos2> = s.track.iter().map(|t| to_px((t.1, t.2))).collect();
                let hl = near.is_some_and(|n| n.call == s.call);
                painter.add(egui::Shape::line(line, egui::Stroke::new(if hl { 2.5_f32 } else { 1.5 }, GREEN.gamma_multiply(if hl { 0.9 } else { 0.45 }))));
            }
        }
        // Funkwege: Station → benutzte Digipeater (mit bekannter Position) → ich
        let pos_of = |c: &str| self.aprs.stations.iter().find(|s| s.call.eq_ignore_ascii_case(c) || s.call.split('-').next() == Some(c)).and_then(|s| s.pos);
        for (s, p) in &pts {
            let hl = near.is_some_and(|n| n.call == s.call);
            if !(self.aprs.map_paths || hl) || s.is_only || s.call.starts_with('◆') { continue; }
            let mut chain = vec![to_px(*p)];
            chain.extend(s.digis.iter().filter_map(|d| pos_of(d)).map(to_px));
            if me.is_some() { chain.push(c); }
            if chain.len() < 2 { continue; }
            let col = if s.digis.is_empty() { WX_BLUE } else { ORANGE };
            painter.add(egui::Shape::line(chain, egui::Stroke::new(if hl { 2.0_f32 } else { 1.0 }, col.gamma_multiply(if hl { 0.9 } else { 0.3 }))));
        }
        // Stationen: Digipeater/Gateways orange, andere grün; angetippte hervorgehoben
        let tap = resp.clicked().then(|| resp.interact_pointer_pos()).flatten();
        let mut best: Option<(f32, String)> = None;
        for (s, p) in &pts {
            let q = to_px(*p);
            let infra = matches!(s.symbol.1, '#' | '&' | 'r');
            let sel = self.aprs.map_sel.as_deref() == Some(s.call.as_str());
            painter.circle_filled(q, if sel { 6.0 } else { 4.0 }, if sel { Color32::WHITE } else if infra { ORANGE } else { GREEN });
            painter.text(q + egui::vec2(6.0, -6.0), egui::Align2::LEFT_BOTTOM, &s.call, egui::FontId::monospace(11.0), Color32::from_gray(210));
            if let Some(t) = tap {
                let d = t.distance(q);
                if d < 24.0 && best.as_ref().is_none_or(|b| d < b.0) { best = Some((d, s.call.clone())); }
            }
        }
        if let Some((_, call)) = best { self.aprs.map_sel = Some(call.clone()); picked = Some(call); }
        // Mouse-over: Stationskarte der nächsten Station unter dem Zeiger
        {
            if let Some(s) = near {
                egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), resp.id.with("card"), egui::PopupAnchor::Pointer).show(|ui| station_card(ui, s, me, tel.get(&s.call.to_uppercase())));
            }
        }
        // Angaben zur gewählten Station
        if let Some(s) = self.aprs.map_sel.as_ref().and_then(|c| self.aprs.stations.iter().find(|s| &s.call == c)) {
            let dist = match (me, s.pos) { (Some(a), Some(b)) => { let (d, br) = aprs::dist_bearing(a, b); format!("{d:.1} km {}", aprs::compass(br)) } _ => String::new() };
            ui.label(RichText::new(format!("{}  {}  {}  {}  {}", s.call, hhmm(s.last), dist, aprs::symbol_name(s.symbol), s.comment)).monospace().small());
        } else {
            ui.label(RichText::new("Station antippen: zeigt Angaben und setzt sie als Empfänger einer Nachricht").small().weak());
        }
        picked
    }

    pub(crate) fn ui_aprs_tx(&mut self, ui: &mut egui::Ui) {
        let busy = self.tx.is_some();
        ui.heading("Positionsbake");
        if self.station.call.is_empty() {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Zuerst das Rufzeichen eintragen.").color(crate::RED));
                if ui.button("Einstellungen › Station …").clicked() { self.set_page = Some(crate::SetPage::Station); }
            });
        }
        let loc = aprs::locator_to_latlon(&self.station.locator);
        // feste Angaben (SSID, Symbol, Pfad, Kommentar) stehen unter Einstellungen › APRS
        ui.horizontal_wrapped(|ui| {
            let ssid = if self.aprs.ssid.trim().is_empty() { String::new() } else { format!("-{}", self.aprs.ssid.trim()) };
            let path = if self.aprs.hf { &self.aprs.path_hf } else { &self.aprs.path };
            ui.label(RichText::new(format!("{}{ssid}", if self.station.call.is_empty() { "?" } else { &self.station.call })).monospace().strong());
            ui.label(RichText::new(format!("{} · Pfad {}", aprs::SYMBOLS[self.aprs.symbol].2, if path.is_empty() { "–" } else { path }))).on_hover_text(&self.aprs.comment);
            if ui.small_button("Ändern …").on_hover_text("SSID, Symbol, Kommentar und Pfad").clicked() { self.set_page = Some(crate::SetPage::Aprs); }
        });
        egui::Grid::new("aprs_cfg").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
            ui.label("Position");
            ui.horizontal(|ui| {
                let (hl, ho) = loc.map_or(("Breite".to_string(), "Länge".to_string()), |(a, o)| (format!("{a:.4}"), format!("{o:.4}")));
                ui.add(egui::TextEdit::singleline(&mut self.aprs.lat).desired_width(80.0).hint_text(hl));
                ui.add(egui::TextEdit::singleline(&mut self.aprs.lon).desired_width(80.0).hint_text(ho));
            }).response.on_hover_text("Dezimalgrad, z. B. 52.5163 und 13.3777 (Süd/West negativ). Leer = Mitte des Locators");
            ui.end_row();
            ui.label("");
            ui.horizontal(|ui| {
                let has_rig = self.rig.as_ref().is_some_and(|r| r.model.proto == crate::cat::Proto::Civ);
                if ui.add_enabled(has_rig, egui::Button::new("📡 vom Funkgerät").small())
                    .on_hover_text("GPS-Position des Funkgeräts lesen (IC-705, IC-9700, IC-905 …; GPS muss eingeschaltet sein)")
                    .on_disabled_hover_text("Nur mit verbundenem Icom-Gerät mit GPS").clicked() { self.aprs_rig_gps(); }
                let label = if self.aprs.locating { "⏳ suche …" } else { "🖥 Windows-Standort" };
                if ui.add_enabled(!self.aprs.locating, egui::Button::new(label).small())
                    .on_hover_text("Standort über den Windows-Ortungsdienst (WLAN-Ortung oder GPS am PC)").clicked() { self.aprs_windows_location(); }
                if !self.aprs.lat.is_empty() || !self.aprs.lon.is_empty() {
                    if ui.small_button("Locator").on_hover_text("Felder leeren – dann gilt die Mitte des Locators").clicked() { self.aprs.lat.clear(); self.aprs.lon.clear(); }
                }
            });
            ui.end_row();
        });
        ui.horizontal(|ui| {
            let b = egui::Button::new(RichText::new("Bake senden").color(Color32::BLACK).strong()).fill(ORANGE);
            if ui.add_enabled(!busy, b.min_size(egui::vec2(140.0, 26.0))).clicked() { self.aprs_beacon(); }
            ui.label("automatisch");
            let sel = if self.aprs.smart { "Smart".to_string() } else if self.aprs.beacon_min == 0 { "aus".to_string() } else { format!("{} min", self.aprs.beacon_min) };
            egui::ComboBox::from_id_salt("aprs_auto").width(80.0).selected_text(sel).show_ui(ui, |ui| {
                for m in [0, 10, 20, 30, 60] {
                    if ui.selectable_label(!self.aprs.smart && self.aprs.beacon_min == m, if m == 0 { "aus".to_string() } else { format!("alle {m} min") }).clicked() {
                        self.aprs.beacon_min = m;
                        self.aprs.smart = false;
                    }
                }
                if ui.selectable_label(self.aprs.smart, "SmartBeaconing (GPS)")
                    .on_hover_text("Abstand nach Tempo und zusätzlich beim Abbiegen; Tempo und Kurs vom GPS des Funkgeräts").clicked() {
                    self.aprs.smart = true;
                }
            });
            if self.aprs.smart {
                egui::ComboBox::from_id_salt("aprs_prof").width(80.0).selected_text(aprs::SMART_PROFILES[self.aprs.smart_prof].0).show_ui(ui, |ui| {
                    for (i, (n, p)) in aprs::SMART_PROFILES.iter().enumerate() {
                        ui.selectable_value(&mut self.aprs.smart_prof, i, *n).on_hover_text(format!(
                            "Stand (unter {:.0} km/h): alle {:.0} min · ab {:.0} km/h: alle {:.0} s · dazwischen entsprechend öfter; \
                             Kurswechsel ab {:.0}° (bei langsamer Fahrt mehr), frühestens nach {:.0} s",
                            p.slow_kmh, p.slow_rate_s / 60.0, p.fast_kmh, p.fast_rate_s, p.min_turn_deg, p.min_turn_s));
                    }
                }).response.on_hover_text("Voreinstellung für SmartBeaconing");
            }
        });
        let p = self.tx.as_ref().map_or(0.0, |t| t.state.progress());
        ui.add(egui::ProgressBar::new(p).desired_height(6.0));
        self.ui_gain(ui).on_hover_text(if self.aprs.hf { "Wie bei SSB-Digimodes: so einstellen, dass die ALC gerade nicht ausschlägt" }
            else { "Bei FM bestimmt der Pegel den Hub – zu laut verzerrt, zu leise wird schlecht gelesen" });
        let mut info = vec![];
        if let Some(t) = self.aprs.last_beacon {
            info.push(format!("letzte Bake vor {} min", t.elapsed().as_secs() / 60));
            if self.aprs.smart && !self.aprs.last_reason.is_empty() { info.push(format!("Anlass: {}", self.aprs.last_reason)); }
            else if self.aprs.beacon_min > 0 {
                let left = (self.aprs.beacon_min as u64 * 60).saturating_sub(t.elapsed().as_secs());
                info.push(format!("nächste in {}:{:02}", left / 60, left % 60));
            }
        }
        // GPS-Zustand, sobald das Funkgerät welche liefert
        if let Some((f, t)) = self.gps.fix {
            let mut g = format!("GPS {:.4}, {:.4}", f.lat, f.lon);
            if let Some(v) = f.speed_kmh { g += &format!(" · {v:.0} km/h"); }
            if let Some(c) = f.course { g += &format!(" · {c:.0}°"); }
            g += &format!(" (vor {} s)", t.elapsed().as_secs());
            info.push(g);
        } else if self.aprs.smart {
            info.push(if self.gps.unsupported { "SmartBeaconing braucht ein Funkgerät mit GPS (IC-705 …)".into() }
                else if !self.gps.status.is_empty() { format!("GPS: {}", self.gps.status) } else { "warte auf GPS …".into() });
        }
        if let Some(t) = self.aprs.digi_heard { info.push(format!("✔ eigenes Paket um {} über Digipeater gehört", hhmm(t))); }
        if !info.is_empty() { ui.label(RichText::new(info.join(" · ")).small().color(if self.aprs.digi_heard.is_some() { GREEN } else { Color32::GRAY })); }

        self.ui_aprs_objects(ui);
        ui.add_space(8.0);
        ui.separator();
        // Unterhaltungen: links die Gegenstationen (neueste oben, ungelesene fett), rechts der Verlauf
        let mut partners: Vec<(String, i64, usize)> = vec![];
        for m in &self.aprs.msgs {
            let unread = (!m.out && !m.seen) as usize;
            match partners.iter_mut().find(|p| p.0.eq_ignore_ascii_case(&m.partner)) {
                Some(p) => { p.1 = p.1.max(m.time); p.2 += unread; }
                None => partners.push((m.partner.clone(), m.time, unread)),
            }
        }
        partners.sort_by_key(|p| -p.1);
        let unread_total: usize = partners.iter().map(|p| p.2).sum();
        ui.horizontal(|ui| {
            ui.heading(if unread_total > 0 { format!("Nachrichten ({unread_total} neu)") } else { "Nachrichten".into() });
            if !self.aprs.bulletins.is_empty() {
                ui.selectable_value(&mut self.aprs.conv, Some("#BLN".into()), format!("Bulletins ({})", self.aprs.bulletins.len()))
                    .on_hover_text("Rundsprüche an alle (BLN…)");
            }
            // eingestellte Wege, Klick öffnet die Einstellung
            let name = |rx: bool| if rx { aprs::RxWeg::ALL.iter().find(|w| w.0 == self.aprs.msg_rx).map_or("", |w| w.2) }
                else { aprs::TxWeg::ALL.iter().find(|w| w.0 == self.aprs.msg_tx).map_or("", |w| w.2) };
            let t = format!("Empfang: {} · Senden: {}", name(true), name(false));
            if ui.add(egui::Label::new(RichText::new(t).small().weak()).sense(egui::Sense::click())).on_hover_text("Wege für Nachrichten ändern").clicked() {
                self.set_page = Some(crate::SetPage::Aprs);
            }
        });
        if self.aprs.conv.is_none() {
            self.aprs.conv = partners.first().map(|p| p.0.clone());
            if let Some(c) = &self.aprs.conv { self.aprs.msg_to = c.clone(); }
        }
        let h = (ui.available_height() - 70.0).max(80.0);
        ui.horizontal_top(|ui| {
            // Gesprächspartner
            egui::ScrollArea::vertical().id_salt("aprs_partners").max_height(h).max_width(130.0).auto_shrink([false, false]).show(ui, |ui| ui.vertical(|ui| {
                ui.set_width(120.0);
                if ui.selectable_label(self.aprs.conv.is_none() || self.aprs.conv.as_deref() == Some(""), RichText::new("+ neu").weak()).clicked() {
                    self.aprs.conv = Some(String::new());
                    self.aprs.msg_to.clear();
                }
                for (p, _, unread) in &partners {
                    let t = if *unread > 0 { RichText::new(format!("{p} ({unread})")).strong().color(GREEN) } else { RichText::new(p) };
                    if ui.selectable_label(self.aprs.conv.as_deref() == Some(p.as_str()), t.monospace()).clicked() {
                        self.aprs.conv = Some(p.clone());
                        self.aprs.msg_to = p.clone();
                    }
                }
            }));
            // Verlauf
            let w = ui.available_width() - 14.0;
            egui::Frame::new().fill(Color32::from_gray(12)).inner_margin(6.0).corner_radius(4.0).show(ui, |ui| {
                ui.set_width(w);
                egui::ScrollArea::vertical().id_salt("aprs_msgs").stick_to_bottom(true).max_height(h).auto_shrink([false, false]).show(ui, |ui| ui.vertical(|ui| {
                    let conv = self.aprs.conv.clone().unwrap_or_default();
                    if conv == "#BLN" {
                        for b in &self.aprs.bulletins {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(format!("{} {} {}", hhmm(b.time), b.to, b.from)).monospace().color(ORANGE));
                                ui.add(egui::Label::new(RichText::new(&b.text).monospace()).wrap());
                            });
                        }
                        return;
                    }
                    if conv.is_empty() {
                        ui.add(egui::Label::new(RichText::new(if self.aprs.msgs.is_empty() { "Noch keine Nachrichten. An dich gerichtete Nachrichten werden automatisch bestätigt. \
                            Unten Empfänger und Text eingeben – oder eine Station in der Liste anklicken." } else { "Neue Unterhaltung: unten Empfänger und Text eingeben." }).weak()).wrap());
                        return;
                    }
                    let w = ui.available_width();
                    for m in self.aprs.msgs.iter_mut().filter(|m| m.partner.eq_ignore_ascii_case(&conv)) {
                        m.seen = true;
                        let status = if !m.out { String::new() } else if m.acked { "✔ bestätigt".into() }
                            else if m.tries >= MAX_TRIES && m.next.is_some_and(|t| Instant::now() >= t) { "✗ keine Bestätigung".into() }
                            else { format!("… gesendet {}/{}", m.tries, MAX_TRIES) };
                        // eigene rechts (orange), empfangene links (grün); Blasenbreite nach dem Text
                        let font = egui::TextStyle::Monospace.resolve(ui.style());
                        let info = if m.via.is_empty() { format!("{} {status}", hhmm(m.time)) } else { format!("{} · {} {status}", hhmm(m.time), m.via) };
                        let tw = ui.fonts_mut(|f| f.layout_no_wrap(m.text.clone(), font, Color32::WHITE).size().x)
                            .max(ui.fonts_mut(|f| f.layout_no_wrap(info.clone(), egui::TextStyle::Small.resolve(ui.style()), Color32::WHITE).size().x));
                        let bw = (tw + 4.0).min(w * 0.8);
                        ui.horizontal(|ui| {
                            if m.out { ui.add_space(w - bw - 16.0); }
                            egui::Frame::new().fill(if m.out { Color32::from_rgb(60, 40, 15) } else { Color32::from_rgb(20, 45, 25) })
                                .inner_margin(6.0).corner_radius(6.0).show(ui, |ui| ui.vertical(|ui| {
                                    ui.set_width(bw);
                                    ui.add(egui::Label::new(RichText::new(&m.text).monospace()).wrap());
                                    ui.label(RichText::new(info).small().weak());
                                }));
                        });
                        ui.add_space(3.0);
                    }
                }));
            });
        });
        ui.horizontal(|ui| {
            ui.label("An");
            ui.add(egui::TextEdit::singleline(&mut self.aprs.msg_to).desired_width(90.0).hint_text("DL1ABC-9").font(egui::TextStyle::Monospace));
            let r = ui.add(egui::TextEdit::singleline(&mut self.aprs.msg_text).desired_width(ui.available_width() - 80.0).hint_text("Text (max. 67 Zeichen)"));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Senden").clicked() || enter {
                let to = self.aprs.msg_to.trim().to_uppercase();
                self.aprs_send_msg();
                if !to.is_empty() { self.aprs.conv = Some(to); }
            }
        });
    }

    /// Wege für Nachrichten: Empfang und Senden getrennt
    pub(crate) fn ui_msg_ways(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Nachrichten").strong());
        egui::Grid::new("msg_ways").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Empfangen über");
            ui.horizontal(|ui| for (w, _, l) in aprs::RxWeg::ALL { ui.selectable_value(&mut self.aprs.msg_rx, w, l); })
                .response.on_hover_text("Funk: nur was auf der Frequenz ankommt (auch von iGates ausgesendete Internet-Nachrichten). \
                    Internet: Nachrichten an dich direkt von APRS-IS – kommen auch an, wenn kein iGate in der Nähe sie aussendet.");
            ui.end_row();
            ui.label("Senden über");
            ui.horizontal(|ui| for (w, _, l) in aprs::TxWeg::ALL { ui.selectable_value(&mut self.aprs.msg_tx, w, l); })
                .response.on_hover_text("Gilt für eigene Nachrichten und Bestätigungen. Automatisch: Antwort auf dem Weg, auf dem die Gegenstelle zuletzt kam \
                    (Internet-Gegenstellen übers Internet, Funkstationen auf Funk). Bei „Internet“ ohne Verbindung wartet die Nachricht.");
            ui.end_row();
        });
        if (self.aprs.msg_rx.internet() || matches!(self.aprs.msg_tx, aprs::TxWeg::Internet | aprs::TxWeg::Beides)) && self.station.call.is_empty() {
            ui.label(RichText::new("Für das Internet wird das Rufzeichen gebraucht.").small().color(ORANGE));
        }
    }

    /// Einstellungen › APRS: feste Angaben der Bake
    pub(crate) fn ui_set_aprs(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("aprs_set").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("SSID");
            ui.horizontal(|ui| {
                ui.label(RichText::new(if self.station.call.is_empty() { "?" } else { &self.station.call }).monospace().strong());
                ui.label("–");
                ui.add(egui::TextEdit::singleline(&mut self.aprs.ssid).desired_width(28.0).hint_text("–"));
            });
            ui.end_row();
            ui.label("");
            ui.label(RichText::new("leer = Feststation, 7 = Handfunkgerät, 9 = Mobil, 10 = Internet/Portabel").small().weak());
            ui.end_row();
            ui.label("Symbol");
            egui::ComboBox::from_id_salt("aprs_sym").selected_text(aprs::SYMBOLS[self.aprs.symbol].2).show_ui(ui, |ui| {
                for (i, s) in aprs::SYMBOLS.iter().enumerate() { ui.selectable_value(&mut self.aprs.symbol, i, s.2); }
            });
            ui.end_row();
            ui.label("Kommentar");
            ui.add(egui::TextEdit::singleline(&mut self.aprs.comment).desired_width(260.0));
            ui.end_row();
            ui.label("Pfad UKW");
            ui.add(egui::TextEdit::singleline(&mut self.aprs.path).desired_width(160.0)).on_hover_text("Standard: WIDE1-1,WIDE2-1");
            ui.end_row();
            ui.label("Pfad KW");
            ui.add(egui::TextEdit::singleline(&mut self.aprs.path_hf).desired_width(160.0).hint_text("leer"))
                .on_hover_text("Auf Kurzwelle gibt es keine WIDE-Digipeater: leer lassen oder GATE (für Gateways zu UKW)");
            ui.end_row();
            ui.label("GPS");
            ui.vertical(|ui| {
                ui.checkbox(&mut self.aprs.gps_auto, "vor jeder Bake GPS vom Funkgerät lesen")
                    .on_hover_text("Für Mobil- und Portabelbetrieb. Ohne GPS-Empfang wird die zuletzt eingetragene Position gesendet.");
                ui.checkbox(&mut self.aprs.send_alt, "Höhe vom GPS mitsenden").on_hover_text("hängt /A=… (Fuß) an den Kommentar bzw. kodiert sie bei Mic-E");
            });
            ui.end_row();
            ui.label("Format");
            ui.checkbox(&mut self.aprs.mice, "Bake als Mic-E (kompakt)").on_hover_text(
                "Mic-E packt Position, Kurs, Tempo und Höhe in wenige Bytes – kürzere Aussendung, wie Kenwood/Yaesu-Geräte. \
                 Die Breite steckt dann im Zielfeld; Status „Unterwegs“ in Fahrt, sonst „Außer Dienst“.");
            ui.end_row();
            ui.label("Abfragen");
            ui.checkbox(&mut self.aprs.answer_q, "beantworten (?APRSP, ?APRSD, ?APRSH, ?APRST, ?APRS? …)").on_hover_text(
                "Gerichtete Abfragen per Nachricht und allgemeine Abfragen an alle automatisch beantworten: Position, Status, Objekte, \
                 direkt gehörte Stationen, ob eine Station gehört wurde, Weg des Pakets. Je Station höchstens einmal pro Minute; \
                 allgemeine Abfragen nach einer Zufallszeit bis 2 min.");
            ui.end_row();
        });
        ui.add_space(10.0);
        self.ui_msg_ways(ui);
        ui.add_space(10.0);
        ui.label(RichText::new("Internet (APRS-IS)").strong());
        ui.checkbox(&mut self.aprs.igate, "Empfangs-iGate: auf Funk gehörte Pakete ins Internet weiterleiten")
            .on_hover_text("Damit erscheinen die Stationen, die du hörst, z. B. auf aprs.fi. Gesendet wird nichts aus dem Internet auf Funk. \
                           Pakete mit NOGATE/RFONLY/TCPIP im Pfad und Abfragen werden nicht weitergeleitet. Alle 30 min geht die eigene iGate-Position ins Internet.");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.aprs.is_show, "Stationen aus dem Internet zeigen im Umkreis von");
            ui.add(egui::DragValue::new(&mut self.aprs.is_km).range(10..=1000).suffix(" km"));
        });
        egui::Grid::new("aprs_is").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Server");
            ui.add(egui::TextEdit::singleline(&mut self.aprs.is_server).desired_width(220.0).hint_text("rotate.aprs2.net:14580"));
            ui.end_row();
            let call = self.aprs_my_call();
            ui.label("Anmeldung");
            ui.label(RichText::new(if call.is_empty() { "– Rufzeichen fehlt –".to_string() } else { format!("{call}, Passcode {}", aprs::passcode(&call)) }).monospace())
                .on_hover_text("Der Passcode wird aus dem Rufzeichen berechnet. Für ein iGate ist eine eigene SSID üblich, z. B. -10.");
            ui.end_row();
        });
        if self.aprs.is_show && self.aprs_my_pos().is_none() {
            ui.label(RichText::new("Für den Umkreis braucht es die eigene Position (bei der Bake oder Locator unter Station).").small().color(ORANGE));
        }
        ui.label(RichText::new("Läuft nur in der Betriebsart APRS und braucht Internet.").small().weak());
    }
}

// ---------------- Stationskarte (Mouse-over) ----------------

const WX_BLUE: Color32 = Color32::from_rgb(120, 200, 255);

/// Kompassrose: Fahrtrichtung (grün), Wind (blau, Pfeil zeigt wohin er weht), Richtung von dir zur Station (orange)
fn compass_rose(ui: &mut egui::Ui, course: Option<f64>, wind_from: Option<f64>, bearing: Option<f64>) {
    let size = 128.0;
    let (r, p) = ui.allocate_painter(egui::vec2(size, size), egui::Sense::hover());
    let c = r.rect.center();
    let rad = size / 2.0 - 14.0;
    let dir = |deg: f64, k: f32| { let a = (deg as f32).to_radians(); c + egui::vec2(a.sin(), -a.cos()) * rad * k };
    p.circle_filled(c, rad + 2.0, Color32::from_gray(22));
    p.circle_stroke(c, rad, egui::Stroke::new(1.0_f32, Color32::from_gray(90)));
    for k in 0..16 {
        let deg = k as f64 * 22.5;
        let inner = if k % 4 == 0 { 0.78 } else if k % 2 == 0 { 0.87 } else { 0.93 };
        p.line_segment([dir(deg, inner), dir(deg, 1.0)], egui::Stroke::new(1.0_f32, Color32::from_gray(if k % 4 == 0 { 170 } else { 90 })));
    }
    for (deg, t) in [(0.0, "N"), (90.0, "O"), (180.0, "S"), (270.0, "W")] {
        p.text(dir(deg, 1.17), egui::Align2::CENTER_CENTER, t, egui::FontId::proportional(11.0),
               if deg == 0.0 { crate::RED } else { Color32::from_gray(190) });
    }
    let arrow = |from: egui::Pos2, to: egui::Pos2, col: Color32, w: f32| {
        p.line_segment([from, to], egui::Stroke::new(w, col));
        let v = (to - from).normalized();
        let n = egui::vec2(-v.y, v.x);
        p.add(egui::Shape::convex_polygon(vec![to, to - v * 9.0 + n * 4.5, to - v * 9.0 - n * 4.5], col, egui::Stroke::NONE));
    };
    if let Some(b) = bearing {
        p.circle_filled(dir(b, 1.0), 4.5, ORANGE);
        p.line_segment([c, dir(b, 0.95)], egui::Stroke::new(1.0_f32, ORANGE.gamma_multiply(0.5)));
    }
    if let Some(w) = wind_from { arrow(dir(w, 0.95), dir(w + 180.0, 0.35), WX_BLUE, 2.5); }
    if let Some(co) = course { arrow(c, dir(co, 0.85), GREEN, 3.0); }
    p.circle_filled(c, 2.5, Color32::from_gray(200));
}

/// kleines Fenster mit allen Angaben zu einer Station
pub(crate) fn station_card(ui: &mut egui::Ui, s: &Station, me: Option<(f64, f64)>, tm: Option<&crate::aprs_telem::Telem>) {
    use crate::aprsinfo as ai;
    ui.set_max_width(460.0);
    // kompakt: Zeilenhöhe nach Text, nicht nach Bedienelementen
    ui.spacing_mut().interact_size.y = 14.0;
    ui.spacing_mut().item_spacing.y = 2.0;
    let d = s.det.clone().unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(RichText::new(&s.call).monospace().strong().size(16.0));
        if s.pos.is_some() { ui.label(RichText::new(aprs::symbol_name(s.symbol)).weak()); }
        if !d.kind.is_empty() { ui.label(RichText::new(format!("· {}", d.kind)).small().weak()); }
    });
    let moving = d.speed_kmh.is_some_and(|v| v >= 1.0);
    let wind_from = s.wxd.as_ref().and_then(|w| w.dir.filter(|_| w.wind_kmh.is_some_and(|v| v > 0.0)));
    let bearing = match (me, s.pos) { (Some(a), Some(b)) => Some(aprs::dist_bearing(a, b)), _ => None };
    ui.horizontal_top(|ui| {
        compass_rose(ui, d.course.filter(|_| moving), wind_from, bearing.map(|b| b.1));
        ui.vertical(|ui| {
            let row = |ui: &mut egui::Ui, k: &str, v: String, c: Option<Color32>| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(k).small().weak());
                    let t = RichText::new(v);
                    ui.label(match c { Some(c) => t.color(c), None => t });
                });
            };
            if let Some((la, lo)) = s.pos {
                row(ui, "Position", format!("{}  {}", ai::dm(la, 'N', 'S'), ai::dm(lo, 'O', 'W')), None);
                row(ui, "", format!("{la:.5}, {lo:.5} · Locator {}", ai::locator(la, lo)), None);
            } else {
                row(ui, "Position", "keine gemeldet".into(), None);
            }
            if let Some(a) = d.alt_m { row(ui, "Höhe", format!("{a:.0} m über NN"), None); }
            if let Some((km, br)) = bearing {
                row(ui, "Von dir", format!("{km:.1} km in Richtung {br:.0}° ({})", aprs::compass(br)), Some(ORANGE));
            }
            match (d.course, d.speed_kmh) {
                (_, Some(v)) if v < 1.0 => row(ui, "Bewegung", "steht".into(), None),
                (Some(co), Some(v)) => row(ui, "Bewegung", format!("{v:.0} km/h, Kurs {co:.0}° ({})", aprs::compass(co)), Some(GREEN)),
                (None, Some(v)) => row(ui, "Bewegung", format!("{v:.0} km/h"), Some(GREEN)),
                (Some(co), None) => row(ui, "Kurs", format!("{co:.0}° ({})", aprs::compass(co)), Some(GREEN)),
                _ => {}
            }
            if let Some(m) = d.mice { row(ui, "Status", m.to_string(), if m == "Notfall" { Some(crate::RED) } else { None }); }
            if let Some(t) = &d.time { row(ui, "Zeit im Paket", t.clone(), None); }
        });
    });
    if let Some(w) = &s.wxd { wx_card(ui, s, w); }
    if let Some(t) = tm.filter(|t| t.raw.iter().any(|v| v.is_some()) || t.bits.is_some()) { telem_card(ui, t); }
    ui.separator();
    egui::Grid::new("st_card").num_columns(2).spacing([10.0, 1.0]).show(ui, |ui| {
        let mut row = |k: &str, v: &str| { if !v.is_empty() { ui.label(RichText::new(k).small().weak()); ui.add(egui::Label::new(v).wrap()); ui.end_row(); } };
        row("Kommentar", if d.comment.is_empty() && s.det.is_none() { &s.comment } else { &d.comment });
        row("Status", &s.status);
        if let Some(p) = &d.phg { row("Station", p); }
        if let Some(r) = d.range_km { row("Reichweite", &format!("ca. {r:.0} km")); }
        if let Some(dev) = &d.device { row("Gerät", dev); }
        if let Some(o) = &s.owner { row("Gesetzt von", o); }
        if s.track.len() >= 2 {
            let km: f64 = s.track.windows(2).map(|w| aprs::dist_bearing((w[0].1, w[0].2), (w[1].1, w[1].2)).0).sum();
            row("Spur", &format!("{} Punkte seit {}, {km:.1} km", s.track.len(), hhmm(s.track[0].0)));
        }
        row("Empfangen", &format!("{}× · zuletzt {} · {}", s.count, hhmm(s.last), s.path));
        if let Some(l) = s.level {
            row("Pegel", &format!("{} beim letzten Paket, höchstens {}", crate::rigpoll::s_text(l), crate::rigpoll::s_text(s.level_max.unwrap_or(l))));
        }
        if let Some(r) = s.rf { row("Frequenz", &format!("{:.1} kHz", r as f64 / 1000.0)); }
    });
    if !s.raw.is_empty() {
        ui.label(RichText::new(&s.raw).monospace().small().weak()).on_hover_text("letztes Paket im Rohformat");
    }
    // Legende zur Rose
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(RichText::new("Rose:").small().weak());
        if moving { ui.label(RichText::new("Fahrtrichtung").small().color(GREEN)); }
        if wind_from.is_some() { ui.label(RichText::new("Wind (Pfeil zeigt, wohin er weht)").small().color(WX_BLUE)); }
        if bearing.is_some() { ui.label(RichText::new("Punkt: Richtung von dir aus").small().color(ORANGE)); }
    });
}

/// Wetterteil der Stationskarte, nach Themen gegliedert
fn wx_card(ui: &mut egui::Ui, s: &Station, w: &aprs::Wx) {
    let now = chrono::Utc::now().timestamp();
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Wetter").strong().color(WX_BLUE));
        if let Some(t) = s.wx_time {
            let age = (now - t).max(0);
            ui.label(RichText::new(if age < 90 { "gerade gemeldet".into() } else { format!("vor {} min gemeldet", age / 60) }).small().weak());
        }
    });
    // Tendenz aus dem Verlauf (mindestens 30 min Abstand), auf 3 h bzw. 1 h hochgerechnet
    let trend = |pick: fn(&(i64, Option<f64>, Option<f64>)) -> Option<f64>, per_h: f64| -> Option<f64> {
        let last = s.wx_hist.last()?;
        let first = s.wx_hist.iter().find(|h| last.0 - h.0 <= 3 * 3600 && pick(h).is_some())?;
        let dt = (last.0 - first.0) as f64 / 3600.0;
        if dt < 0.5 { return None; }
        Some((pick(last)? - pick(first)?) / dt * per_h)
    };
    let sect = |ui: &mut egui::Ui, title: &str| { ui.label(RichText::new(title).small().strong().color(Color32::from_gray(200))); ui.label(""); ui.end_row(); };
    let row = |ui: &mut egui::Ui, k: &str, v: String| { ui.label(RichText::new(k).small().weak()); ui.label(RichText::new(v).color(WX_BLUE)); ui.end_row(); };
    // zwei Spalten: links Temperatur und Luftdruck, rechts Wind, Niederschlag, Station
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| { egui::Grid::new("wx_l").num_columns(2).spacing([10.0, 1.0]).show(ui, |ui| {
        if w.temp_c.is_some() || w.hum.is_some() {
            sect(ui, "Temperatur");
            if let Some(t) = w.temp_c {
                let tr = trend(|h| h.2, 1.0).map_or(String::new(), |d| format!("  ({d:+.1} °C/h)"));
                row(ui, "Luft", format!("{t:.1} °C{tr}"));
                if let Some((f, how)) = aprs::feels_like(t, w.wind_kmh, w.hum) { row(ui, "gefühlt", format!("{f:.1} °C ({how})")); }
                if let Some(h) = w.hum { row(ui, "Taupunkt", format!("{:.1} °C", aprs::dew_point(t, h))); }
                if t <= 0.0 { row(ui, "", "Frost".into()); }
            }
            if let Some(h) = w.hum { row(ui, "Luftfeuchte", format!("{h} %")); }
        }
        if let Some(p) = w.hpa {
            sect(ui, "Luftdruck");
            row(ui, "Meereshöhe", format!("{p:.1} hPa"));
            if let Some(d) = trend(|h| h.1, 3.0) {
                let what = match d { d if d <= -3.0 => "stark fallend", d if d < -0.5 => "fallend", d if d < 0.5 => "gleichbleibend", d if d < 3.0 => "steigend", _ => "stark steigend" };
                row(ui, "Tendenz", format!("{what} ({d:+.1} hPa/3 h)"));
            }
        }
        }); });
        ui.add_space(14.0);
        ui.vertical(|ui| { egui::Grid::new("wx_r").num_columns(2).spacing([10.0, 1.0]).show(ui, |ui| {
        if w.wind_kmh.is_some() || w.gust_kmh.is_some() {
            sect(ui, "Wind");
            if let Some(v) = w.wind_kmh {
                let (b, name) = aprs::beaufort(v);
                row(ui, "Mittel", format!("{v:.0} km/h · {:.1} m/s", v / 3.6));
                row(ui, "", format!("{b} Bft · {name}"));
                if let Some(d) = w.dir.filter(|_| v > 0.0) { row(ui, "Richtung", format!("aus {} ({d:.0}°)", aprs::compass(d))); }
            }
            if let Some(g) = w.gust_kmh {
                let (b, _) = aprs::beaufort(g);
                row(ui, "Böen", format!("{g:.0} km/h · {b} Bft"));
            }
        }
        if w.rain_mm.is_some() || w.rain_24h.is_some() || w.rain_mid.is_some() || w.snow_cm.is_some() {
            sect(ui, "Niederschlag");
            if let Some(r) = w.rain_mm { row(ui, "1 h", format!("{r:.1} mm")); }
            if let Some(r) = w.rain_mid { row(ui, "seit 0 Uhr", format!("{r:.1} mm")); }
            if let Some(r) = w.rain_24h { row(ui, "24 h", format!("{r:.1} mm")); }
            if let Some(c) = w.snow_cm { row(ui, "Schnee 24 h", format!("{c:.1} cm")); }
        }
        if w.lum.is_some() || w.volt.is_some() || w.rain_raw.is_some() {
            sect(ui, "Station");
            if let Some(l) = w.lum { row(ui, "Sonne", format!("{l:.0} W/m²")); }
            if let Some(v) = w.volt { row(ui, "Spannung", format!("{v:.1} V")); }
            if let Some(r) = w.rain_raw { row(ui, "Regenzähler", r.to_string()); }
        }
        }); });
    });
    if let Some(st) = &w.station {
        ui.horizontal_wrapped(|ui| { ui.label(RichText::new("Wetterstation").small().weak()); ui.label(RichText::new(st).color(WX_BLUE)); });
    }
    // Verlauf der letzten Stunden
    let temps: Vec<(i64, f64)> = s.wx_hist.iter().filter_map(|h| h.2.map(|v| (h.0, v))).collect();
    let press: Vec<(i64, f64)> = s.wx_hist.iter().filter_map(|h| h.1.map(|v| (h.0, v))).collect();
    if temps.len() >= 3 || press.len() >= 3 {
        ui.horizontal(|ui| {
            if temps.len() >= 3 { sparkline(ui, &temps, "°C", Color32::from_rgb(255, 170, 90)); }
            if press.len() >= 3 { sparkline(ui, &press, "hPa", WX_BLUE); }
        });
    }
}

/// kleine Verlaufskurve mit Min/Max und Zeitspanne
fn sparkline(ui: &mut egui::Ui, pts: &[(i64, f64)], unit: &str, col: Color32) {
    let (w, h) = (200.0, 46.0);
    let (r, p) = ui.allocate_painter(egui::vec2(w, h), egui::Sense::hover());
    let rect = r.rect;
    p.rect_filled(rect, 3.0, Color32::from_gray(20));
    let (t0, t1) = (pts[0].0, pts.last().unwrap().0.max(pts[0].0 + 1));
    let (lo, hi) = pts.iter().fold((f64::MAX, f64::MIN), |a, x| (a.0.min(x.1), a.1.max(x.1)));
    let span = (hi - lo).max(if unit == "hPa" { 2.0 } else { 1.0 });
    let mid = (hi + lo) / 2.0;
    let plot = rect.shrink2(egui::vec2(4.0, 12.0));
    let to = |t: i64, v: f64| egui::pos2(plot.left() + (t - t0) as f32 / (t1 - t0) as f32 * plot.width(),
                                          plot.center().y - ((v - mid) / span) as f32 * plot.height());
    let line: Vec<egui::Pos2> = pts.iter().map(|x| to(x.0, x.1)).collect();
    p.add(egui::Shape::line(line, egui::Stroke::new(1.5_f32, col)));
    let f = egui::FontId::proportional(10.0);
    let dig = if unit == "hPa" { 1 } else { 1 };
    p.text(rect.left_top() + egui::vec2(3.0, 1.0), egui::Align2::LEFT_TOP, format!("{hi:.dig$} {unit}"), f.clone(), Color32::from_gray(150));
    p.text(rect.left_bottom() + egui::vec2(3.0, -1.0), egui::Align2::LEFT_BOTTOM, format!("{lo:.dig$}"), f.clone(), Color32::from_gray(150));
    let hours = (t1 - t0) as f64 / 3600.0;
    p.text(rect.right_bottom() + egui::vec2(-3.0, -1.0), egui::Align2::RIGHT_BOTTOM,
           if hours >= 1.0 { format!("{hours:.1} h") } else { format!("{:.0} min", hours * 60.0) }, f, Color32::from_gray(150));
}

/// Telemetrie der Station (Werte nach PARM/UNIT/EQNS umgerechnet)
fn telem_card(ui: &mut egui::Ui, t: &crate::aprs_telem::Telem) {
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Telemetrie").strong().color(Color32::from_rgb(200, 160, 255)));
        let mut sub = vec![];
        if !t.title.is_empty() { sub.push(t.title.clone()); }
        if let Some(n) = t.seq { sub.push(format!("Nr. {n}")); }
        if let Some(x) = t.time { sub.push(format!("um {}", hhmm(x))); }
        ui.label(RichText::new(sub.join(" · ")).small().weak());
    });
    let rows = t.rows();
    egui::Grid::new("tm_card").num_columns(4).spacing([10.0, 1.0]).show(ui, |ui| {
        for (k, (name, val)) in rows.iter().enumerate() {
            ui.label(RichText::new(name).small().weak());
            ui.label(RichText::new(val).color(Color32::from_rgb(200, 160, 255)));
            if k % 2 == 1 { ui.end_row(); }
        }
    });
    if t.names.iter().all(|n| n.is_empty()) {
        ui.label(RichText::new("Bedeutung der Kanäle noch unbekannt (kommt per PARM/UNIT/EQNS-Nachricht der Station)").small().weak());
    }
}
