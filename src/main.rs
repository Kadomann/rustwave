#![cfg_attr(windows, windows_subsystem = "windows")]
//! Rustwave – SSTV, RTTY, CW, APRS, POCSAG und Packet Radio mit CAT-Steuerung (früher „SSTV Station“).

mod aprs;
mod pocsag;
mod pocsag_ui;
mod ax25;
mod ifdemod;
mod ifswitch;
mod dapnet;
mod dapnet_tx;
mod aprsinfo;
mod aprs_telem;
mod aprs_export;
mod kiss;
mod voice;
mod sat;
mod packet_ui;
mod audio;
mod cat;
mod sstv;
mod editor;
mod icomnet;
mod logbook;
mod rtty;
mod rtty_ui;
mod aprs_ui;
mod cw;
mod cw_ui;
mod touchkbd;
mod apmode;
mod gps;
mod fskid;
mod profiles;
mod diag;
mod rx;
mod radio;
mod rigpoll;
mod scope;
mod skimmer;
mod ui_operate;
mod ui_settings;
mod ui_logbook;
mod menu;
mod rigbar;
mod rigctl;
mod igate;
mod gallery;
use rx::*;
mod template;

use eframe::egui::{self, Color32, ColorImage, RichText, TextureHandle, TextureOptions};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering::Relaxed};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const QUICK_KHZ: [u32; 6] = [3735, 7165, 14230, 14233, 21340, 28680];
const RTTY_KHZ: [u32; 6] = [3585, 7043, 10140, 14085, 21085, 28085];
/// APRS-Frequenzen (kHz, Seitenband, Kurzwelle 300 Bd?): UKW Europa/USA/ISS; KW nach george-smart.co.uk/aprs/aprs-on-hf
/// (Töne 1600/1800 Hz; USB und LSB gehen beide, weil NRZI nur Tonwechsel auswertet); 10 m auch mit 1200 Bd
const APRS_LIST: [(f64, cat::Sb, bool); 14] = [
    (144800.0, cat::Sb::Fm, false), (144390.0, cat::Sb::Fm, false), (145825.0, cat::Sb::Fm, false),
    (29250.0, cat::Sb::Fm, false), (28137.1, cat::Sb::Usb, false),
    (7045.2, cat::Sb::Usb, true), (7048.6, cat::Sb::Lsb, true), (7047.2, cat::Sb::Lsb, true), (7104.0, cat::Sb::Lsb, true),
    (10147.6, cat::Sb::Usb, true), (14102.3, cat::Sb::Usb, true), (14103.0, cat::Sb::Lsb, true), (14105.0, cat::Sb::Lsb, true),
    (28140.0, cat::Sb::Lsb, true),
];
const WF_W: usize = 512;
const WF_H: usize = 110;
const WF_MAX_HZ: f32 = 3000.0;
const PREROLL_S: f64 = 0.2; // Stille nach PTT, bis das Relais umgeschaltet hat
const TUNE_S: f64 = 30.0;

// ---------------- Empfangs-Thread ----------------
enum UiEv {
    Start(&'static sstv::Mode),
    Line(usize, Vec<u8>),
    End(&'static sstv::Mode, sstv::Img, bool),
    Wf(Vec<u8>, f32),
    Log(String),
    /// dekodierter RTTY-Text
    Rtty(String),
    /// empfangener Funkruf (POCSAG)
    Pocsag(pocsag::Msg),
    /// Packet-Radio-Rahmen (Bytes mit FCS); true = live vom Funkgerät, false = aus einer WAV-Datei
    Packet(Vec<u8>, bool),
    /// Rahmen (mit FCS) von einem Programm über KISS/AGW zum Senden
    KissTx(Vec<u8>),
    /// APRS-Rahmen; beim KW-Skimmer mit Tonmitte (Hz)
    Aprs(aprs::Frame, Option<f64>),
    /// Meldung der APRS-IS-Verbindung
    Is(igate::IsEv),
    /// dekodierter Morse-Text
    Cw(String),
    /// Stand des CW-Skimmers
    Skim(Vec<skimmer::SkimInfo>),
    /// Rufzeichen aus einem FSKID (nach einem SSTV-Bild)
    FskId(String),
    /// Ergebnis einer Selbsttest-Prüfung, die im Hintergrund lief
    DiagLine((diag::Lvl, String)),
    /// Standort vom Windows-Ortungsdienst (Breite, Länge, Genauigkeit in m)
    Location(Result<(f64, f64, f64), String>),
    /// Ergebnis der (im Hintergrund laufenden) WLAN/LAN-Anmeldung
    NetConnected(Result<icomnet::NetRadio, String>),
    /// PC hat sich mit dem WLAN des IC-705 (Access Point) verbunden: eigene Adresse
    ApJoined(Result<std::net::Ipv4Addr, String>),
}
const CTL_RESET: u8 = 1;
const CTL_ABORT: u8 = 2;
fn test_pattern(w: u32, h: u32) -> image::RgbImage {
    let bars = [[255, 255, 255], [255, 255, 0], [0, 255, 255], [0, 255, 0], [255, 0, 255], [255, 0, 0], [0, 0, 255], [0, 0, 0]];
    image::RgbImage::from_fn(w, h, |x, y| {
        if y < h * 7 / 10 { image::Rgb(bars[(x * 8 / w) as usize]) } else { let v = (x * 255 / (w - 1)) as u8; image::Rgb([v, v, v]) }
    })
}
/// Ordner für empfangene Bilder, Aufnahmen und Selbsttest-Berichte (Windows: Bilder\Rustwave)
fn save_dir() -> PathBuf { pictures_dir().join("Rustwave") }
fn pictures_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
    home.join("Pictures")
}
/// Datenordner für Vorlagen, Logbuch und Profile (Windows: %APPDATA%\Rustwave)
fn data_dir() -> PathBuf { data_dir_named("Rustwave", "rustwave") }
fn data_dir_named(win: &str, nix: &str) -> PathBuf {
    match std::env::var_os("APPDATA") {
        Some(a) => PathBuf::from(a).join(win),
        None => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".config").join(nix),
    }
}
/// Name, unter dem eframe die Einstellungen speichert (Windows: %APPDATA%\Rustwave\data)
const APP_ID: &str = "Rustwave";

/// Beim ersten Start unter dem neuen Namen: Daten aus „SSTV Station“ übernehmen (kopieren, das Alte bleibt liegen).
/// Erst der ganze Datenordner (unter Windows liegen dort auch die Einstellungen), dann ggf. eframes Speicherordner.
fn migrate_old_data() -> Option<String> {
    fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            let t = to.join(e.file_name());
            if e.file_type()?.is_dir() { copy_dir(&e.path(), &t)?; } else { std::fs::copy(e.path(), t)?; }
        }
        Ok(())
    }
    let pairs = [(Some(data_dir_named("SSTV Station", "sstv-station")), Some(data_dir())),
                 (Some(pictures_dir().join("SSTV")), Some(save_dir())),
                 (eframe::storage_dir("SSTV Station"), eframe::storage_dir(APP_ID))];
    let mut done = vec![];
    for (old, new) in pairs {
        let (Some(old), Some(new)) = (old, new) else { continue };
        if old.is_dir() && !new.exists() {
            match copy_dir(&old, &new) {
                Ok(()) => done.push(format!("{} → {}", old.display(), new.display())),
                Err(e) => done.push(format!("Übernahme aus {} fehlgeschlagen: {e}", old.display())),
            }
        }
    }
    (!done.is_empty()).then(|| format!("Daten von „SSTV Station“ übernommen: {}", done.join(", ")))
}
fn open_path(p: &Path) {
    #[cfg(windows)] let _ = std::process::Command::new("explorer").arg(p).spawn();
    #[cfg(not(windows))] let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}
/// Bildfläche 5:4, so breit wie die Spalte, aber höchstens `frac` der Fensterhöhe
fn pic_size(ui: &egui::Ui, w: f32, frac: f32) -> egui::Vec2 {
    let screen = ui.ctx().content_rect().height();
    // niedrige Bildschirme (z. B. 1280×800 bei 150 %): Bilder kleiner, damit „Senden“ ohne Rollen sichtbar bleibt
    let frac = if screen < 700.0 { frac * 0.7 } else { frac };
    let h = (w * 0.8).min(screen * frac);
    egui::vec2(h / 0.8, h)
}
fn heat(v: u8) -> Color32 {
    let t = v as f32 / 255.0;
    Color32::from_rgb((400.0 * t * t).min(255.0) as u8, (300.0 * t).min(255.0) as u8, (80.0 + 300.0 * t * (1.0 - t)).min(255.0) as u8)
}
fn to_color_image(img: &image::RgbImage) -> ColorImage {
    ColorImage::from_rgb([img.width() as usize, img.height() as usize], img.as_raw())
}
fn set_tex(ctx: &egui::Context, slot: &mut Option<TextureHandle>, name: &str, img: ColorImage) {
    match slot { Some(t) => t.set(img, TextureOptions::LINEAR), None => *slot = Some(ctx.load_texture(name, img, TextureOptions::LINEAR)) }
}
fn hash_of(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}
const ORANGE: Color32 = Color32::from_rgb(0xf5, 0xa5, 0x24);
const RED: Color32 = Color32::from_rgb(0xf8, 0x51, 0x49);
const GREEN: Color32 = Color32::from_rgb(0x3f, 0xb9, 0x50);

// ---------------- App ----------------
#[derive(PartialEq, Clone, Copy)]
enum TxKind { Image, Tune, Rtty, Aprs, Pocsag, Packet }

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
enum Op { Sstv, Rtty, Aprs, Cw, Pocsag, Packet }
struct TxRun { state: Arc<audio::PlayState>, _out: Option<audio::Output>, kind: TxKind, done_at: Option<Instant>, phase: Option<template::Phase>, stopped: bool }

#[derive(PartialEq, Clone, Copy)]
enum Tab { Operate, Templates, Logbook, Gallery, Diag }

/// Seiten des Einstellungsfensters
#[derive(PartialEq, Clone, Copy)]
enum SetPage { Station, Audio, Transceiver, Sstv, Aprs, Tnc, Darstellung, Profile }

/// kleine Hilfefenster
#[derive(PartialEq, Clone, Copy)]
enum Help { Keys, About }

#[derive(Default, Clone)]
struct Station { call: String, name: String, qth: String, locator: String }

struct GalleryItem { tex: TextureHandle, label: String, path: PathBuf }

struct App {
    // Einstellungen (werden gespeichert)
    in_dev: String,
    out_dev: String,
    rig_idx: Option<usize>,
    port: String,
    baud: u32,
    ptt: cat::Ptt,
    tx_mode: usize,
    /// Sendepegel je Betriebsart: SSTV, RTTY, APRS UKW, APRS Kurzwelle, CW, POCSAG, Packet
    gains: [f32; 7],
    freq_khz: String,
    station: Station,
    photo_path: String,
    tab: Tab,
    /// Verbindung über Icom-Netzwerkprotokoll (WLAN/LAN) statt USB/seriell
    use_net: bool,
    op: Op,
    rtty: rtty::Params,
    /// Makros je Phase (CQ, Antwort, Rapport, 73)
    rtty_macros: [String; 4],
    /// Empfangsfenster: (Zeile, gesendet?)
    rtty_lines: Vec<(String, bool)>,
    cw: cw::Params,
    cw_lines: Vec<(String, bool)>,
    cw_macros: [String; 4],
    cw_input: String,
    cw_wpm: u32,
    cw_tx: Option<cw_ui::CwTx>,
    /// FSKID nach jedem Bild senden
    fskid_tx: bool,
    /// gemittelte Wasserfallzeile (für „stärkstes Signal“)
    wf_avg: Vec<f32>,
    /// Darstellung: große Bedienelemente für Finger (None = automatisch bei erster Berührung), Zoom,
    /// im schmalen Fenster angezeigte Hälfte (false = Empfang)
    touch: Option<bool>,
    touch_applied: Option<bool>,
    zoom: f32,
    pane_tx: bool,
    /// zuletzt gezeichnet im schmalen Layout
    narrow: bool,
    /// Energiesparen: Wasserfall langsamer (für Akkubetrieb)
    power_save: bool,
    base_style: Option<egui::Style>,
    rtty_input: String,
    /// nächster RTTY-Sendetext und dessen Phase
    rtty_out: Option<(String, Option<template::Phase>)>,
    aprs: aprs_ui::AprsState,
    pocsag: pocsag_ui::PocsagState,
    packet: packet_ui::PacketState,
    /// Funkgerät liefert die 12-kHz-ZF statt NF: selbst FM-demodulieren
    if_rx: bool,
    /// ZF am Funkgerät je Betriebsart automatisch umschalten (Icom per CI-V); zuletzt gesendeter Zustand, Zeitpunkt
    if_auto: bool,
    if_sent: Option<bool>,
    if_switched: Option<Instant>,
    /// Fenster in der Taskleiste aufblinken lassen (APRS-Hinweis o. Ä.)
    attention: bool,
    /// TNC für andere Programme: KISS- und AGW-Port (0 = aus), auch im Netzwerk, laufender Server
    kiss_port: u16,
    agw_port: u16,
    kiss_lan: bool,
    kiss: Option<kiss::Server>,
    kiss_key: (u16, u16, bool),
    /// Sprechfunk: Mikrofon, Lautsprecher, Mikrofonpegel, Mithören (an/Lautstärke), Taste schaltet um statt halten
    mic_dev: String,
    spk_dev: String,
    mic_gain: f32,
    mon_on: bool,
    mon_vol: f32,
    ptt_toggle: bool,
    talk: Option<voice::Talk>,
    mon_out: Option<(voice::RingOut, std::sync::Arc<voice::Ring>, String)>,
    /// Sprechtaste (Leertaste) wird gehalten bzw. Schaltfläche gedrückt
    talk_btn: bool,
    talk_release: Option<Instant>,
    /// Satelliten und ISS
    sat: sat::SatState,
    net_host: String,
    net_user: String,
    net_pass: String,
    /// IC-705 als Access Point (direkte WLAN-Verbindung, Funkgerät immer 192.168.59.1)
    net_ap: bool,
    ap_ssid: String,
    ap_pass: String,
    ap_joining: bool,
    /// nach einem Abbruch der WLAN-Verbindung: nächster Versuch und Zahl der Versuche
    reconnect: Option<(Instant, u32)>,
    gps: gps::GpsState,
    /// laufende CAT-Abfrage (Frequenz, Betriebsart, Leistung, Spektrum)
    poll: rigpoll::RigPoll,
    /// zuletzt benutzte Frequenz und Betriebsart je Programm-Betriebsart
    freq_mem: HashMap<Op, (u64, Option<cat::Sb>)>,
    power_sent: Option<Instant>,
    /// Bandspektrum vom Funkgerät anzeigen
    scope_want: bool,
    scope_seen: u64,
    scope_wf: ColorImage,
    scope_tex: Option<TextureHandle>,
    scope_dirty: bool,
    scope_v: scope::ScopeView,
    /// Antennentuner stimmt ab (seit)
    tuning: Option<Instant>,
    low_volt_warned: bool,
    gal: gallery::Gallery,
    last_rx_img: Option<(PathBuf, Instant)>,
    /// empfangene Bilder zusätzlich hierhin kopieren (z. B. OneDrive-Ordner)
    copy_dir: String,
    /// Breite der rechten Gruppe der Funkgerät-Leiste (vom letzten Bild)
    right_w: f32,
    /// Breiten der Gruppen in der Funkgerät-Leiste (Leistung, Messwerte) vom letzten Bild
    bar_ws: [f32; 2],
    /// höchste Bildrate bei laufenden Daten (Bilder/s)
    fps: u32,
    /// Logo (klein für die Menüleiste, Schriftzug für „Über“)
    icon_tex: Option<TextureHandle>,
    logo_tex: Option<TextureHandle>,
    /// CW-Skimmer an und letzter Stand
    skim_on: bool,
    skim: Vec<skimmer::SkimInfo>,
    profiles: profiles::Profiles,
    diag: diag::Diag,
    /// seit dem Start eine Berührung des Bildschirms gesehen
    touch_seen: bool,
    /// eigene Adresse im Funkgeräte-WLAN (zuletzt geprüft)
    ap_pc: (Option<std::net::Ipv4Addr>, Option<Instant>),
    // Audio / CAT
    in_names: Vec<String>,
    out_names: Vec<String>,
    ports: Vec<String>,
    input: Option<audio::Input>,
    rx_sh: Arc<RxShared>,
    ev_tx: mpsc::Sender<UiEv>,
    ev_rx: mpsc::Receiver<UiEv>,
    rig: Option<cat::Rig>,
    net: Option<icomnet::NetRadio>,
    net_connecting: bool,
    freq_shown: Option<u64>,
    // Empfang
    rx_img: ColorImage,
    rx_tex: Option<TextureHandle>,
    rx_dirty: bool,
    rx_info: String,
    rx_mode: Option<&'static sstv::Mode>,
    wf: ColorImage,
    wf_tex: Option<TextureHandle>,
    level: f32,
    /// seit wann vom Funkgerät nur Stille kommt (Rauschsperre zu?)
    silent_since: Option<Instant>,
    gallery: Vec<GalleryItem>,
    gallery_sel: Option<usize>,
    save_dir: PathBuf,
    // Vorlagen und Sendebild
    templates: Vec<template::Template>,
    tpl_path: PathBuf,
    tpl_ver: u64,
    tpl_save_due: Option<Instant>,
    sel_tpl: usize,
    phase: template::Phase,
    photo: image::RgbImage,
    photo_ver: u64,
    received: Option<image::RgbImage>,
    received_ver: u64,
    files: HashMap<String, image::RgbImage>,
    tx_img: sstv::Img,
    tx_tex: Option<TextureHandle>,
    tx_key: u64,
    quads: Vec<Option<template::Quad>>,
    thumbs: Vec<TextureHandle>,
    thumbs_key: u64,
    tx: Option<TxRun>,
    // QSO und Logbuch
    qso: logbook::Qso,
    known: Option<logbook::Known>,
    logbook: logbook::Log,
    logged: bool,
    log_filter: String,
    log_undo: Option<(usize, logbook::Qso)>,
    // Editor
    ed: editor::EdState,
    status: String,
    /// offenes Einstellungsfenster (Seite) und Hilfefenster
    set_page: Option<SetPage>,
    help: Option<Help>,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let get = |k: &str| cc.storage.and_then(|s| s.get_string(k));
        let (ev_tx, ev_rx) = mpsc::channel();
        let dir = data_dir();
        let tpl_path = dir.join("vorlagen.json");
        let templates = std::fs::read_to_string(&tpl_path).ok()
            .and_then(|s| serde_json::from_str::<Vec<template::Template>>(&s).ok())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(template::defaults);
        let photo_path = get("photo").unwrap_or_default();
        let photo = image::open(&photo_path).map(|i| i.to_rgb8()).unwrap_or_else(|_| test_pattern(320, 256));
        let mut app = App {
            // Einstellungen: Platzhalter, gefüllt von apply_settings (gleicher Code für Programmstart und Profile)
            in_dev: String::new(), out_dev: String::new(), rig_idx: None, port: String::new(), baud: 19200, ptt: cat::Ptt::Cat,
            tx_mode: 0, gains: [0.5; 7], freq_khz: String::new(), station: Station::default(),
            photo_path,
            tab: Tab::Operate,
            use_net: false, op: Op::Sstv, rtty: rtty::Params::default(),
            rtty_macros: Default::default(),
            rtty_lines: vec![(String::new(), false)],
            cw: cw::Params::default(),
            cw_lines: vec![(String::new(), false)],
            cw_macros: Default::default(),
            cw_input: String::new(),
            cw_wpm: 20,
            cw_tx: None,
            fskid_tx: true,
            wf_avg: vec![0.0; WF_W],
            touch: None,
            touch_applied: None,
            zoom: 1.0,
            pane_tx: false,
            narrow: false,
            power_save: false,
            base_style: None,
            rtty_input: String::new(),
            rtty_out: None,
            aprs: aprs_ui::AprsState::load(&get),
            pocsag: pocsag_ui::PocsagState::load(&get),
            packet: packet_ui::PacketState::load(&get),
            if_rx: false, if_auto: true, if_sent: None, if_switched: None, attention: false, kiss_port: 0, agw_port: 0, kiss_lan: false, kiss: None, kiss_key: (0, 0, false), mic_dev: String::new(), spk_dev: String::new(), mic_gain: 1.0, mon_on: false, mon_vol: 0.8, ptt_toggle: false, talk: None, mon_out: None, talk_btn: false, talk_release: None, sat: sat::SatState::load(&get),
            net_host: String::new(), net_user: String::new(), net_pass: String::new(),
            net_ap: false, ap_ssid: String::new(), ap_pass: String::new(),
            ap_joining: false,
            reconnect: None,
            gps: Default::default(),
            poll: Default::default(),
            freq_mem: HashMap::new(),
            power_sent: None,
            scope_want: false, scope_seen: 0, scope_wf: ColorImage::new([1, 1], vec![Color32::BLACK]), scope_tex: None, scope_dirty: false, scope_v: Default::default(), tuning: None, low_volt_warned: false, gal: Default::default(), last_rx_img: None, copy_dir: String::new(), right_w: 300.0, bar_ws: [0.0; 2], fps: 15, icon_tex: None, logo_tex: None,
            skim_on: true, skim: vec![],
            diag: Default::default(),
            touch_seen: false,
            profiles: profiles::Profiles::load(dir.join("profile.json"), get("profile").unwrap_or_default()),
            ap_pc: (None, None),
            in_names: audio::input_names(),
            out_names: audio::output_names(),
            ports: vec![],
            input: None,
            rx_sh: Arc::default(),
            ev_tx, ev_rx,
            rig: None, net: None, net_connecting: false, freq_shown: None,
            rx_img: ColorImage::filled([320, 256], Color32::BLACK),
            rx_tex: None, rx_dirty: true,
            rx_info: "Audio starten – der Modus wird am VIS-Code automatisch erkannt".into(),
            rx_mode: None,
            wf: ColorImage::filled([WF_W, WF_H], Color32::BLACK),
            wf_tex: None, level: 0.0, silent_since: None,
            gallery: vec![], gallery_sel: None,
            save_dir: save_dir(),
            templates, tpl_path, tpl_ver: 0, tpl_save_due: None,
            sel_tpl: 0, phase: template::Phase::Cq,
            photo, photo_ver: 0, received: None, received_ver: 0, files: HashMap::new(),
            tx_img: sstv::Img::new(320, 256), tx_tex: None, tx_key: 0, quads: vec![],
            thumbs: vec![], thumbs_key: 0,
            tx: None,
            qso: logbook::Qso::default(), known: None,
            logbook: logbook::Log::load(dir.join("logbuch.json")),
            logged: false, log_filter: String::new(), log_undo: None,
            ed: editor::EdState::default(),
            status: String::new(),
            set_page: None, help: None,
        };
        app.apply_settings(&get);
        app.select_phase(template::Phase::Cq);
        app.refresh_ports();
        // erster Start: gleich die Einstellungen zeigen
        if app.station.call.is_empty() {
            app.set_page = Some(SetPage::Station);
            app.status = "Willkommen! Trag unter Einstellungen › Station dein Rufzeichen ein – es erscheint über %m in den Vorlagen.".into();
        }
        // Zustand der letzten Sitzung wiederherstellen: Audio starten, CAT verbinden
        if get("audio_on").as_deref() == Some("1") { app.start_audio(&cc.egui_ctx); }
        if get("cat_on").as_deref() == Some("1") { app.connect(); }
        // Datei als Argument ("Öffnen mit" / auf die .exe ziehen): WAV dekodieren, Bild als Sendefoto
        for a in std::env::args_os().skip(1).map(PathBuf::from) {
            if a.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")) { app.spawn_decode(a, &cc.egui_ctx); } else { app.load_photo(&a); }
        }
        app
    }

    fn log(&mut self, s: impl Into<String>) {
        self.status = format!("{}  {}", chrono::Local::now().format("%H:%M:%S"), s.into());
    }
    fn refresh_ports(&mut self) {
        self.ports = serialport::available_ports().map(|v| v.into_iter().map(|p| p.port_name).collect()).unwrap_or_default();
        if self.port.is_empty() { if let Some(p) = self.ports.first() { self.port = p.clone(); } }
    }

    // ---- Vorlagen, Phasen, Sendebild ----
    fn select_phase(&mut self, p: template::Phase) {
        self.phase = p;
        if let Some(i) = self.templates.iter().position(|t| t.phase == Some(p)) { self.sel_tpl = i; }
    }
    fn select_template(&mut self, i: usize) {
        self.sel_tpl = i.min(self.templates.len().saturating_sub(1));
        if let Some(p) = self.templates[self.sel_tpl].phase { self.phase = p; }
    }
    /// nach jeder Änderung im Editor: neu rendern und bald speichern
    fn tpl_changed(&mut self) {
        self.tpl_ver += 1;
        self.tpl_save_due = Some(Instant::now() + Duration::from_millis(800));
    }
    fn save_templates(&mut self) {
        let r = std::fs::create_dir_all(data_dir()).and_then(|_| std::fs::write(&self.tpl_path, serde_json::to_string_pretty(&self.templates).unwrap()));
        if let Err(e) = r { self.log(format!("Vorlagen nicht gespeichert: {e}")); }
        self.tpl_save_due = None;
    }
    fn load_photo(&mut self, path: &Path) {
        match image::open(path) {
            Ok(i) => { self.photo = i.to_rgb8(); self.photo_ver += 1; self.photo_path = path.to_string_lossy().into(); }
            Err(e) => self.log(format!("Bild: {e}")),
        }
    }
    fn freq_hz(&self) -> Option<u64> {
        self.freq_shown.or_else(|| self.freq_khz.replace(',', ".").trim().parse::<f64>().ok().map(|k| (k * 1000.0).round() as u64))
    }
    /// Makrowerte; `sample` füllt leere QSO-Felder mit Beispielen (Editor-Vorschau)
    fn macros(&self, sample: bool) -> HashMap<char, String> {
        let q = &self.qso;
        let pick = |v: &str, ex: &str| if v.is_empty() && sample { ex.to_string() } else { v.to_string() };
        let st = &self.station;
        let now = chrono::Utc::now();
        let f = self.freq_hz();
        HashMap::from([
            ('c', pick(&q.call, "DL1ABC")), ('n', pick(&q.name, "Hans")), ('q', pick(&q.qth, "Berlin")),
            ('r', pick(&q.rsv_sent, "595")), ('s', pick(&q.rsv_rcvd, "595")), ('x', q.note.clone()),
            ('m', pick(&st.call, "DK0XY")), ('o', st.name.clone()), ('a', st.qth.clone()), ('l', st.locator.clone()),
            ('g', template::greeting(chrono::Timelike::hour(&chrono::Local::now())).into()),
            ('t', now.format("%H:%M").to_string()), ('d', now.format("%Y-%m-%d").to_string()),
            ('f', f.map_or(String::new(), |f| format!("{:.3}", f as f64 / 1e6))),
            ('b', f.map_or(String::new(), |f| logbook::band(f).to_string())),
            ('v', sstv::MODES[self.tx_mode].name.into()),
            ('w', self.known.as_ref().map_or("0".into(), |k| k.count.to_string())),
        ])
    }
    fn ensure_files(&mut self) {
        for t in &self.templates {
            for l in &t.layers {
                if let template::Kind::Picture { source: template::Source::File, path, .. } = &l.kind {
                    if !path.is_empty() && !self.files.contains_key(path) {
                        if let Ok(img) = image::open(path) { self.files.insert(path.clone(), img.to_rgb8()); }
                    }
                }
            }
        }
    }
    fn render_key(&self, sample: bool, extra: &str) -> u64 {
        let mut m: Vec<_> = self.macros(sample).into_iter().collect();
        m.sort();
        hash_of(&format!("{:?}|{}|{}|{}|{}|{}|{}", m, self.tpl_ver, self.photo_ver, self.received_ver, self.templates.len(), self.files.len(), extra))
    }
    /// Sendebild (und Thumbnails der Ablage) nur bei Änderungen neu rendern
    fn update_renders(&mut self, ctx: &egui::Context) {
        self.ensure_files();
        let sample = self.tab == Tab::Templates && self.ed.sample;
        let key = self.render_key(sample, &format!("{}|{}", self.sel_tpl, self.tx_mode));
        if key != self.tx_key {
            let m = &sstv::MODES[self.tx_mode];
            let env = template::Env { macros: self.macros(sample), photo: Some(&self.photo), received: self.received.as_ref(), files: &self.files };
            let (img, quads) = template::render(&self.templates[self.sel_tpl], m.w as u32, m.h as u32, &env);
            set_tex(ctx, &mut self.tx_tex, "tx", to_color_image(&img));
            self.tx_img = sstv::Img { w: m.w, h: m.h, data: img.into_raw() };
            self.quads = quads;
            self.tx_key = key;
        }
        let key = self.render_key(false, "thumbs");
        if key != self.thumbs_key {
            let env = template::Env { macros: self.macros(false), photo: Some(&self.photo), received: self.received.as_ref(), files: &self.files };
            self.thumbs = self.templates.iter().enumerate().map(|(i, t)| {
                ctx.load_texture(format!("thumb{i}"), to_color_image(&template::render(t, 160, 128, &env).0), TextureOptions::LINEAR)
            }).collect();
            self.thumbs_key = key;
        }
    }

    // ---- QSO ----
    fn call_changed(&mut self) {
        self.qso.call = logbook::normalize_call(&self.qso.call);
        if self.qso.start == 0 && !self.qso.call.is_empty() { self.qso.start = gps::now_utc(&self.gps); }
        self.known = self.logbook.lookup(&self.qso.call);
        if let Some(k) = &self.known { // Vorausfüllen, ohne Eingaben zu überschreiben
            if self.qso.name.is_empty() { self.qso.name = k.name.clone(); }
            if self.qso.qth.is_empty() { self.qso.qth = k.qth.clone(); }
        }
        // Rufzeichen eingetragen = jemand hat geantwortet (oder ich antworte): weiter zur Antwort
        if self.phase == template::Phase::Cq && !self.qso.call.is_empty() { self.select_phase(template::Phase::Answer); }
    }
    fn new_qso(&mut self) {
        self.qso = logbook::Qso::default();
        self.known = None;
        self.logged = false;
        self.select_phase(template::Phase::Cq);
    }
    /// gespeicherte Einstellungen übernehmen (beim Start und beim Laden eines Profils)
    fn apply_settings(&mut self, get: &dyn Fn(&str) -> Option<String>) {
        let num = |k: &str| get(k).and_then(|s| s.parse::<f64>().ok());
        self.in_dev = get("in_dev").unwrap_or_default();
        self.out_dev = get("out_dev").unwrap_or_default();
        self.rig_idx = get("rig").and_then(|s| s.parse().ok()).filter(|&i: &usize| i < cat::RIGS.len());
        self.port = get("port").unwrap_or_default();
        self.baud = get("baud").and_then(|s| s.parse().ok()).unwrap_or(self.rig_idx.map_or(19200, |i| cat::RIGS[i].baud));
        self.ptt = get("ptt").and_then(|s| cat::Ptt::ALL.into_iter().find(|p| p.label() == s)).unwrap_or(cat::Ptt::Cat);
        self.tx_mode = get("tx_mode").and_then(|s| s.parse().ok()).filter(|&i: &usize| i < sstv::MODES.len()).unwrap_or(0);
        let old = num("gain").unwrap_or(0.5) as f32; // früher ein gemeinsamer Pegel
        self.gains = std::array::from_fn(|i| num(&format!("gain{i}")).map_or(old, |v| v as f32));
        self.freq_khz = get("freq").unwrap_or_else(|| "14230".into());
        self.station = Station { call: get("my_call").unwrap_or_default(), name: get("my_name").unwrap_or_default(),
            qth: get("my_qth").unwrap_or_default(), locator: get("my_loc").unwrap_or_default() };
        self.use_net = get("use_net").as_deref() == Some("1");
        self.op = match get("op").as_deref() { Some("RTTY") => Op::Rtty, Some("APRS") => Op::Aprs, Some("CW") => Op::Cw, Some("POCSAG") => Op::Pocsag, Some("PACKET") => Op::Packet, _ => Op::Sstv };
        let d = rtty::Params::default();
        self.rtty = rtty::Params { center: num("rtty_center").unwrap_or(d.center), shift: num("rtty_shift").unwrap_or(d.shift),
            baud: num("rtty_baud").unwrap_or(d.baud), reverse: get("rtty_rev").as_deref() == Some("1"),
            squelch: num("rtty_sq").map_or(d.squelch, |v| v as f32) };
        self.rtty_macros = std::array::from_fn(|i| get(&format!("rtty_m{i}")).unwrap_or_else(|| rtty_ui::DEFAULT_MACROS[i].to_string()));
        let c = cw::Params::default();
        self.cw = cw::Params { pitch: num("cw_pitch").unwrap_or(c.pitch), squelch: num("cw_sq").unwrap_or(c.squelch) };
        self.cw_macros = std::array::from_fn(|i| get(&format!("cw_m{i}")).unwrap_or_else(|| cw_ui::DEFAULT_MACROS[i].to_string()));
        self.cw_wpm = get("cw_wpm").and_then(|s| s.parse().ok()).filter(|w| (6..=48).contains(w)).unwrap_or(20);
        self.fskid_tx = get("fskid_tx").as_deref() != Some("0");
        self.copy_dir = get("copy_dir").unwrap_or_default();
        self.fps = get("fps").and_then(|s| s.parse().ok()).unwrap_or(15);
        self.touch = get("touch").map(|v| v == "1");
        self.zoom = num("zoom").map(|z| z as f32).filter(|z| (0.6..=2.0).contains(z)).unwrap_or(1.0);
        self.power_save = get("power_save").as_deref() == Some("1");
        self.scope_want = get("scope").as_deref() == Some("1");
        self.scope_v.band = get("scope_band").as_deref() != Some("0");
        if let Some(s) = get("scope_span").and_then(|s| s.parse().ok()).filter(|s| cat::SCOPE_SPANS.contains(s)) { self.scope_v.half_span = s; }
        self.skim_on = get("cw_skim").as_deref() != Some("0");
        self.net_host = get("net_host").unwrap_or_default();
        self.net_user = get("net_user").unwrap_or_default();
        self.net_pass = get("net_pass").unwrap_or_default();
        self.net_ap = get("net_ap").as_deref() == Some("1");
        self.ap_ssid = get("ap_ssid").unwrap_or_default();
        self.ap_pass = get("ap_pass").unwrap_or_default();
        self.aprs.apply_settings(get);
        self.pocsag.apply_settings(get);
        self.packet.apply_settings(get);
        self.if_rx = get("if_rx").as_deref() == Some("1");
        self.if_auto = get("if_auto").as_deref() != Some("0");
        self.kiss_port = get("kiss_port").and_then(|v| v.parse().ok()).unwrap_or(0);
        self.agw_port = get("agw_port").and_then(|v| v.parse().ok()).unwrap_or(0);
        self.kiss_lan = get("kiss_lan").as_deref() == Some("1");
        self.mic_dev = get("mic_dev").unwrap_or_default();
        self.spk_dev = get("spk_dev").unwrap_or_default();
        self.mic_gain = get("mic_gain").and_then(|v| v.parse().ok()).unwrap_or(1.0);
        self.mon_vol = get("mon_vol").and_then(|v| v.parse().ok()).unwrap_or(0.8);
        self.ptt_toggle = get("ptt_toggle").as_deref() == Some("1");
        self.freq_mem.clear();
        for op in [Op::Sstv, Op::Rtty, Op::Aprs, Op::Cw, Op::Pocsag, Op::Packet] {
            let Some(v) = get(&format!("mem_{op:?}")) else { continue };
            let (f, m) = v.split_once(',').unwrap_or((&v, "-"));
            let sb = [cat::Sb::Usb, cat::Sb::Lsb, cat::Sb::Fm, cat::Sb::Cw].into_iter().find(|x| format!("{x:?}") == m);
            if let Ok(f) = f.parse() { self.freq_mem.insert(op, (f, sb)); }
        }
    }

    /// Programm-Betriebsart wechseln; mit verbundenem Funkgerät die dort zuletzt benutzte Frequenz einstellen
    fn switch_op(&mut self, o: Op) {
        self.op = o;
        self.tab = Tab::Operate;
        let label = match o { Op::Sstv => "SSTV", Op::Rtty => "RTTY", Op::Cw => "CW", Op::Aprs => "APRS", Op::Pocsag => "POCSAG", Op::Packet => "Packet Radio" };
        match self.freq_mem.get(&o).copied() {
            Some((f, sb)) if self.rig.is_some() && self.freq_shown != Some(f) => {
                self.log(format!("Betriebsart {label} – Funkgerät auf {:.4} MHz{}", f as f64 / 1e6, sb.map_or(String::new(), |m| format!(" {m:?}").to_uppercase())));
                self.tune_to(f as f64 / 1000.0, sb);
            }
            _ => self.log(format!("Betriebsart {label}")),
        }
    }

    fn gain_slot(&self) -> usize {
        match self.op { Op::Sstv => 0, Op::Rtty => 1, Op::Aprs => if self.aprs.hf { 3 } else { 2 }, Op::Cw => 4, Op::Pocsag => 5, Op::Packet => 6 }
    }
    /// Aufnahme läuft?
    fn recording(&self) -> bool { self.rx_sh.rec_path.lock().unwrap().is_some() }
    /// Aufnahme starten bzw. beenden: schneidet das empfangene Audio als WAV mit (zum Prüfen und Weitergeben)
    fn toggle_record(&mut self) {
        if self.recording() {
            let p = self.rx_sh.rec_path.lock().unwrap().take();
            self.log(format!("Aufnahme gespeichert: {}", p.map_or(String::new(), |p| p.display().to_string())));
            return;
        }
        if self.input.is_none() && self.net.is_none() { self.log("Aufnahme: zuerst Audio starten oder per WLAN verbinden"); return; }
        let name = format!("aufnahme-{}-{}.wav", match self.op { Op::Sstv => "sstv", Op::Rtty => "rtty", Op::Cw => "cw", Op::Aprs => "aprs", Op::Pocsag => "pocsag", Op::Packet => "packet" },
            chrono::Local::now().format("%Y%m%d-%H%M%S"));
        *self.rx_sh.rec_path.lock().unwrap() = Some(save_dir().join(name));
        self.log("Aufnahme läuft …");
    }
    /// WAV-Datei wählen und dekodieren (Datei › WAV dekodieren, ⋯-Menüs)
    fn pick_wav(&mut self, ctx: &egui::Context) {
        #[cfg(windows)]
        if let Some(p) = rfd::FileDialog::new().add_filter("WAV", &["wav"]).pick_file() { self.spawn_decode(p, ctx); }
        #[cfg(not(windows))]
        { let _ = ctx; self.log("WAV-Datei ins Fenster ziehen, um sie zu dekodieren"); }
    }
    fn open_save_dir(&self) {
        let _ = std::fs::create_dir_all(save_dir());
        open_path(&save_dir());
    }
    /// im schmalen Fenster ist gerade die Sende-Hälfte zu sehen (Empfang samt Wasserfall verdeckt)
    fn pane_tx_hidden_rx(&self) -> bool { self.pane_tx && self.narrow }
    /// Frequenz des stärksten Signals im gemittelten Wasserfall zwischen lo und hi (Hz)
    fn wf_peak(&self, lo: f32, hi: f32) -> Option<f64> {
        let bin = |f: f32| ((f / WF_MAX_HZ * WF_W as f32) as usize).min(WF_W - 1);
        (bin(lo)..=bin(hi)).max_by(|&a, &b| self.wf_avg[a].total_cmp(&self.wf_avg[b]))
            .map(|b| ((b as f32 + 0.5) / WF_W as f32 * WF_MAX_HZ) as f64)
    }
    fn gain(&self) -> f32 { self.gains[self.gain_slot()] }
    /// Sendepegel-Regler; jede Betriebsart (APRS UKW/KW getrennt) merkt sich ihren eigenen Wert
    fn ui_gain(&mut self, ui: &mut egui::Ui) -> egui::Response {
        let i = self.gain_slot();
        let r = ui.horizontal(|ui| {
            ui.label("Sendepegel");
            if ui.add(egui::Slider::new(&mut self.gains[i], 0.0..=1.0).custom_formatter(|v, _| format!("{:.0} %", v * 100.0))).changed() {
                if let Some(t) = &self.tx { t.state.set_gain(self.gains[i]); }
            }
        }).response.on_hover_text("Wird für jede Betriebsart getrennt gespeichert");
        r
    }
    fn log_qso(&mut self) {
        if self.qso.call.is_empty() { self.log("Kein Rufzeichen – nichts geloggt"); return; }
        if self.qso.start == 0 { self.qso.start = gps::now_utc(&self.gps); }
        self.qso.end = gps::now_utc(&self.gps);
        self.qso.freq_hz = self.freq_hz();
        match self.op {
            Op::Sstv => { self.qso.mode = "SSTV".into(); self.qso.sstv_mode = sstv::MODES[self.tx_mode].name.into(); }
            Op::Rtty => { self.qso.mode = "RTTY".into(); self.qso.sstv_mode = format!("{} Bd / {} Hz", self.rtty.baud, self.rtty.shift); }
            Op::Aprs | Op::Pocsag | Op::Packet => return, // APRS-Pakete, Funkrufe und Packet-Verbindungen sind keine QSOs
            Op::Cw => {
                self.qso.mode = "CW".into();
                self.qso.sstv_mode = format!("{:.0} WpM", f32::from_bits(self.rx_sh.cw_wpm.load(Relaxed)));
            }
        }
        self.logbook.upsert(self.qso.clone());
        match self.logbook.save() {
            Ok(()) => { self.logged = true; self.log(format!("QSO mit {} geloggt", self.qso.call)); }
            Err(e) => self.log(format!("Logbuch nicht gespeichert: {e}")),
        }
        self.known = self.logbook.lookup(&self.qso.call);
    }

    // ---- Audio ----
    fn start_audio(&mut self, ctx: &egui::Context) {
        if self.net.is_some() { self.log("Audio läuft über WLAN/LAN"); return; }
        self.input = None; // alten Stream (und damit den alten Empfangs-Thread) beenden
        let (stx, srx) = mpsc::channel();
        match audio::start_input(&self.in_dev, stx) {
            Ok(inp) => {
                let (fs, tx, sh, c) = (inp.fs, self.ev_tx.clone(), self.rx_sh.clone(), ctx.clone());
                std::thread::spawn(move || rx_thread(fs, srx, tx, sh, c));
                self.log(format!("Audio läuft ({} Hz)", inp.fs));
                self.rx_info = "Warte auf VIS – Modus wird automatisch erkannt".into();
                self.input = Some(inp);
            }
            Err(e) => self.log(format!("Audio-Fehler: {e}")),
        }
    }

    fn handle_events(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.ev_rx.try_recv() {
            match ev {
                UiEv::Start(m) => {
                    self.rx_img = ColorImage::filled([m.w, m.h], Color32::BLACK);
                    self.rx_mode = Some(m);
                    self.rx_info = format!("Empfange {} ({}×{}, {:.0} s)", m.name, m.w, m.h, m.seconds());
                    self.rx_dirty = true;
                }
                UiEv::Line(y, rgb) => {
                    let w = self.rx_img.size[0];
                    if (y + 1) * w <= self.rx_img.pixels.len() {
                        for (x, p) in rgb.chunks(3).enumerate() { self.rx_img.pixels[y * w + x] = Color32::from_rgb(p[0], p[1], p[2]); }
                    }
                    if let Some(m) = self.rx_mode { self.rx_info = format!("Empfange {} · Zeile {}/{}", m.name, y + 1, m.h); }
                    self.rx_dirty = true;
                }
                UiEv::End(m, img, complete) => {
                    self.rx_info = format!("{} {} · warte auf VIS", m.name, if complete { "fertig" } else { "abgebrochen" });
                    self.rx_mode = None;
                    self.store_rx(ctx, m, img);
                }
                UiEv::Wf(row, level) if row.is_empty() => { self.track_silence(level); self.level = level; }
                UiEv::Wf(row, level) => {
                    self.track_silence(level);
                    self.wf.pixels.copy_within(0..WF_W * (WF_H - 1), WF_W);
                    for (x, v) in row.iter().enumerate() { self.wf.pixels[x] = heat(*v); self.wf_avg[x] += (*v as f32 - self.wf_avg[x]) * 0.1; }
                    for f in [1200.0, 1500.0, 1900.0, 2300.0] { self.wf.pixels[(f / WF_MAX_HZ * WF_W as f32) as usize] = Color32::from_gray(90); }
                    self.level = level;
                }
                UiEv::Log(s) => self.log(s),
                UiEv::KissTx(b) => self.kiss_tx(b),
                UiEv::Rtty(t) => self.rtty_rx(&t, false),
                UiEv::Aprs(f, tone) => self.aprs_received(f, tone),
                UiEv::Pocsag(m) => self.pocsag_received(m),
                UiEv::Packet(b, live) => self.packet_received(b, live),
                UiEv::Is(e) => self.aprs_is_event(e),
                UiEv::Location(r) => self.aprs_location(r),
                UiEv::Cw(t) => self.cw_rx(&t),
                UiEv::Skim(v) => self.skim = v,
                UiEv::DiagLine(l) => { self.diag.waiting_location = false; self.diag.lines.push(l); }
                UiEv::FskId(c) => {
                    self.gallery_fskid(&c);
                    // wie MMSSTV: Rufzeichen in die QSO-Leiste, wenn dort noch keins steht
                    if self.qso.call.trim().is_empty() {
                        self.qso.call = c.clone();
                        self.call_changed();
                        self.logged = false;
                        self.log(format!("FSKID: {c} – in die QSO-Leiste übernommen"));
                    } else { self.log(format!("FSKID: {c}")); }
                }
                UiEv::NetConnected(r) => self.net_connected(r, ctx),
                UiEv::ApJoined(r) => {
                    self.ap_joining = false;
                    match r {
                        Ok(a) => { self.ap_pc = (Some(a), Some(Instant::now())); self.log(format!("Im WLAN des IC-705 (PC: {a})")); self.connect(); }
                        Err(e) => self.log(format!("WLAN des IC-705: {e}")),
                    }
                }
            }
        }
    }

    /// Empfangenes Bild speichern, in die Galerie und als "Bild im Bild" bereitstellen
    fn store_rx(&mut self, ctx: &egui::Context, m: &sstv::Mode, img: sstv::Img) {
        let name = format!("sstv_{}_{}.png", chrono::Local::now().format("%Y%m%d_%H%M%S"), m.name.replace(' ', ""));
        let path = self.save_dir.join(&name);
        let res = std::fs::create_dir_all(&self.save_dir).map_err(|e| e.to_string()).and_then(|_| {
            image::save_buffer(&path, &img.data, img.w as u32, img.h as u32, image::ExtendedColorType::Rgb8).map_err(|e| e.to_string())
        });
        match res {
            Ok(()) => { self.log(format!("gespeichert: {}", path.display())); self.gallery_stored(&path, m.name); }
            Err(e) => self.log(format!("Speichern fehlgeschlagen: {e}")),
        }
        let tex = ctx.load_texture(&name, ColorImage::from_rgb([img.w, img.h], &img.data), TextureOptions::LINEAR);
        self.gallery.insert(0, GalleryItem { tex, label: format!("{} {}", chrono::Local::now().format("%H:%M"), m.name), path });
        self.gallery.truncate(30);
        self.gallery_sel = Some(0);
        self.received = image::RgbImage::from_raw(img.w as u32, img.h as u32, img.data);
        self.received_ver += 1;
    }
    fn spawn_decode(&mut self, p: PathBuf, ctx: &egui::Context) {
        self.log(format!("Dekodiere {} …", p.display()));
        let (tx, c) = (self.ev_tx.clone(), ctx.clone());
        let mode = match self.op {
            Op::Sstv => FileMode::Sstv,
            Op::Rtty => FileMode::Rtty(self.rtty),
            Op::Cw => FileMode::Cw(self.cw),
            Op::Aprs if self.aprs.hf && self.aprs.skim => FileMode::AprsSkim,
            Op::Aprs => FileMode::Aprs(self.aprs.modem()),
            Op::Pocsag => FileMode::Pocsag(self.pocsag.p),
            Op::Packet => FileMode::Packet(self.packet.speed),
        };
        let if_rx = self.if_rx;
        std::thread::spawn(move || decode_file(p, tx, Some(c), mode, if_rx));
    }

    // ---- Senden ----
    fn start_tx(&mut self, kind: TxKind) {
        if self.tx.is_some() { return; }
        let r = (|| -> Result<(Arc<audio::PlayState>, Option<audio::Output>), String> {
            let fs = if self.net.is_some() { icomnet::RATE as f64 } else { audio::output_rate(&self.out_dev)? };
            let mut s = vec![0f32; (PREROLL_S * fs) as usize];
            match kind {
                TxKind::Image => {
                    s.extend(sstv::encode(&sstv::MODES[self.tx_mode], &self.tx_img, fs, 1.0));
                    // Rufzeichen als FSKID wie MMSSTV (dort im Empfänger automatisch übernommen)
                    if self.fskid_tx && !self.station.call.is_empty() { s.extend(fskid::encode(&self.station.call, fs, 1.0)); }
                }
                TxKind::Tune => s.extend((0..(TUNE_S * fs) as usize).map(|i| (std::f64::consts::TAU * 1750.0 * i as f64 / fs).sin() as f32)),
                TxKind::Rtty => s.extend(rtty::encode(&self.rtty_out.as_ref().map_or(String::new(), |o| o.0.clone()), &self.rtty, fs)),
                TxKind::Aprs => {
                    let mut raw: Vec<Vec<u8>> = self.aprs.sending.iter().map(|f| f.to_bytes()).collect();
                    raw.extend(self.aprs.sending_raw.iter().cloned());
                    s.extend(aprs::modulate_raw(&raw, fs, 300.0, self.aprs.modem()))
                }
                TxKind::Packet => s.extend(ax25::modulate(&self.packet.sending, fs, self.packet.txdelay_ms, self.packet.speed)),
                TxKind::Pocsag => s.extend(pocsag::modulate(&self.pocsag.sending, self.pocsag.sending_cs, self.pocsag.sending_baud, fs, self.pocsag.tx_invert)),
            }
            if let Some(rig) = &mut self.rig { rig.ptt(true).map_err(|e| format!("PTT: {e}"))?; }
            let g = self.gain();
            if let Some(n) = &self.net { return Ok((n.shared.play(s, g), None)); }
            audio::play(&self.out_dev, s, g).map(|o| (o.state.clone(), Some(o)))
                .inspect_err(|_| { if let Some(r) = &mut self.rig { let _ = r.ptt(false); } })
        })();
        match r {
            Ok((state, out)) => {
                self.rx_sh.paused.store(true, Relaxed);
                let phase = match kind {
                    TxKind::Image => self.templates[self.sel_tpl].phase,
                    TxKind::Rtty => self.rtty_out.as_ref().and_then(|o| o.1),
                    TxKind::Tune | TxKind::Aprs | TxKind::Pocsag | TxKind::Packet => None,
                };
                if kind == TxKind::Rtty { if let Some((t, _)) = self.rtty_out.clone() { self.rtty_echo(&t); } }
                self.tx = Some(TxRun { state, _out: out, kind, done_at: None, phase, stopped: false });
                self.log(match kind {
                    TxKind::Image => format!("Sende „{}“ in {} …", self.templates[self.sel_tpl].name, sstv::MODES[self.tx_mode].name),
                    TxKind::Rtty => format!("Sende RTTY ({} Bd, {} Hz Shift) …", self.rtty.baud, self.rtty.shift),
                    TxKind::Aprs => format!("Sende {} APRS-Paket(e) …", self.aprs.sending.len()),
                    TxKind::Packet => format!("Packet: sende {} Rahmen ({}) …", self.packet.sending.len(), self.packet.speed.label()),
                    TxKind::Pocsag => match self.pocsag.sending.len() {
                        1 => format!("Sende Funkruf an RIC {} ({} Bd) …", self.pocsag.sending[0].addr, self.pocsag.sending_baud),
                        n => format!("Sende {n} DAPNET-Rufe ({} Bd) …", self.pocsag.sending_baud),
                    },
                    TxKind::Tune => format!("Abstimmton 1750 Hz – Sendepegel jetzt einstellen (stoppt nach {TUNE_S:.0} s)"),
                });
            }
            Err(e) => self.log(format!("TX-Fehler: {e}")),
        }
    }
    /// Abstand zwischen zwei Bildern bei laufenden Daten (Wasserfall, Spektrum, Senden): Bildrate aus den Einstellungen
    pub(crate) fn frame(&self) -> Duration { Duration::from_millis(1000 / if self.power_save { 8 } else { self.fps.clamp(5, 30) } as u64) }
    /// im Takt der Bildrate neu zeichnen. egui zieht von der Wartezeit die erwartete Bildzeit (meist 1/60 s) ab –
    /// die wird hier wieder addiert, sonst würden aus 15 Bildern/s gut 20
    pub(crate) fn repaint_tick(&self, ctx: &egui::Context) {
        let dt = ctx.input(|i| i.predicted_dt).clamp(0.0, 0.1);
        ctx.request_repaint_after(self.frame() + Duration::from_secs_f32(dt));
    }
    /// Stille vom Funkgerät mitzählen (Pegel praktisch null, z. B. bei geschlossener Rauschsperre über WLAN)
    fn track_silence(&mut self, level: f32) {
        if level < 1e-4 { self.silent_since.get_or_insert(Instant::now()); } else { self.silent_since = None; }
    }
    fn stop_tx(&mut self) {
        if let Some(t) = &mut self.tx { t.state.stop.store(true, Relaxed); t.stopped = true; }
    }
    /// nach Ende der Wiedergabe kurz warten, PTT lösen, Empfang aufnehmen und zur nächsten Phase gehen
    fn poll_tx(&mut self, ctx: &egui::Context) {
        if self.tx.is_some() { self.repaint_tick(ctx); }
        let Some(t) = &mut self.tx else { return };
        if !t.state.done.load(Relaxed) { return; }
        let at = *t.done_at.get_or_insert(Instant::now());
        // PTT erst lösen, wenn das Funkgerät alles ausgespielt hat: über WLAN/LAN puffert es das Audio
        // (sonst fehlt das Ende – bei APRS die Prüfsumme, das Paket ist verloren)
        let tail = if self.net.is_some() { 150 + icomnet::TX_BUFFER_MS as u64 } else { 150 };
        if at.elapsed() < Duration::from_millis(tail) { return; }
        let (kind, phase, stopped) = (t.kind, t.phase, t.stopped);
        self.tx = None;
        if let Some(rig) = &mut self.rig { if let Err(e) = rig.ptt(false) { self.log(format!("PTT-Fehler: {e}")); } }
        self.rx_sh.ctl.store(CTL_RESET, Relaxed); // Eigenecho verwerfen
        self.rx_sh.paused.store(false, Relaxed);
        if kind == TxKind::Tune { self.log("Abstimmton beendet"); return; }
        if stopped { self.log("Senden abgebrochen"); return; }
        if kind == TxKind::Aprs { self.log("APRS gesendet"); return; }
        if kind == TxKind::Pocsag { self.pocsag_sent(); self.log("Funkruf gesendet"); return; }
        if kind == TxKind::Packet { self.packet_sent(); return; }
        self.log(if kind == TxKind::Rtty { "Text gesendet" } else { "Bild gesendet" });
        match phase {
            Some(template::Phase::Final) if !self.qso.call.is_empty() => self.log_qso(),
            Some(p) => self.select_phase(p.next()),
            None => {}
        }
    }

    // ---- CAT ----
}

impl App {
    fn update_inner(&mut self, ctx: &egui::Context) {
        *self.rx_sh.rtty.lock().unwrap() = (self.op == Op::Rtty).then_some(self.rtty);
        self.rx_sh.aprs.store(self.op == Op::Aprs, Relaxed);
        *self.rx_sh.pocsag.lock().unwrap() = (self.op == Op::Pocsag).then_some(self.pocsag.p);
        self.rx_sh.if_rx.store(self.if_rx, Relaxed);
        *self.rx_sh.packet.lock().unwrap() = (self.op == Op::Packet || self.packet.active()).then_some(self.packet.speed);
        // Wasserfall nur berechnen, wenn er zu sehen ist (APRS hat keinen; Fenster minimiert -> nichts zeichnen)
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        let wf_visible = self.tab == Tab::Operate && !matches!(self.op, Op::Aprs | Op::Pocsag | Op::Packet) && !minimized
            && !self.pane_tx_hidden_rx();
        self.rx_sh.wf_rate.store(if !wf_visible { 0 } else if self.power_save { 10 } else { 25 }, Relaxed);
        *self.rx_sh.cw.lock().unwrap() = (self.op == Op::Cw).then_some(self.cw);
        self.rx_sh.skim.store(self.op == Op::Cw && self.skim_on, Relaxed);
        self.rx_sh.aprs_hf.store(self.aprs.hf, Relaxed);
        self.rx_sh.aprs_skim.store(self.aprs.hf && self.aprs.skim, Relaxed);
        let smart = self.op == Op::Aprs && self.aprs.smart;
        self.cat_tick(ctx);
        self.if_tick();
        self.dapnet_tick(ctx);
        self.kiss_tick();
        self.voice_tick(ctx);
        self.sat_tick(ctx);
        if std::mem::take(&mut self.attention) {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
        }
        self.scope_tick();
        self.tune_tick();
        self.gps_tick(Duration::from_secs(if smart { 5 } else { 30 }));
        if self.gps.fix.is_some() || smart { ctx.request_repaint_after(Duration::from_secs(1)); }
        self.aprs_tick(ctx);
        self.cw_tick(ctx);
        self.packet_tick(ctx);
        self.diag_tick();
        self.handle_events(ctx);
        self.poll_net();
        self.poll_tx(ctx);
        // laufende Daten (Audio, WLAN): im festen Takt der eingestellten Bildrate neu zeichnen – ein Taktgeber statt vieler
        if self.input.is_some() || self.net.is_some() { self.repaint_tick(ctx); }
        if self.net_connecting || self.ap_joining || self.reconnect.is_some() { ctx.request_repaint_after(Duration::from_millis(500)); }
        if self.tpl_save_due.is_some_and(|t| Instant::now() >= t) { self.save_templates(); }
        if self.tpl_save_due.is_some() { ctx.request_repaint_after(Duration::from_millis(300)); }
        // Drag & Drop: WAV -> dekodieren; Bild -> Sendefoto bzw. im Editor neue Bildebene
        for f in ctx.input(|i| i.raw.dropped_files.clone()) {
            if let Some(p) = f.path {
                let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                if ext == "wav" { self.spawn_decode(p, ctx); }
                else if self.tab == Tab::Templates { self.add_file_layer(p.to_string_lossy().into()); }
                else { self.load_photo(&p); }
            }
        }
        self.update_renders(ctx);

        // Berührung erkannt: einmalig große Bedienelemente einschalten (wenn nicht ausdrücklich abgewählt)
        if ctx.input(|i| i.any_touches()) { self.touch_seen = true; }
        if self.touch.is_none() && ctx.input(|i| i.any_touches()) {
            self.touch = Some(true);
            self.log("Touch erkannt – große Bedienelemente eingeschaltet (abschaltbar unter Einstellungen > Darstellung)");
        }
        self.apply_touch(ctx);
        if (ctx.zoom_factor() - self.zoom).abs() > 0.001 && !ctx.input(|i| i.pointer.any_down()) { ctx.set_zoom_factor(self.zoom); }
        self.shortcuts(ctx);
        // sehr schmal: Empfang und Senden umschaltbar statt nebeneinander
        let narrow = ctx.content_rect().width() < 780.0;
        self.narrow = narrow;

        self.ui_top(ctx);
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add(egui::Label::new(RichText::new(&self.status).monospace().weak()).truncate());
        });
        self.ui_settings_window(ctx);
        self.ui_sat_window(ctx);
        self.ui_help_window(ctx);
        match self.tab {
            Tab::Operate => {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if !matches!(self.op, Op::Aprs | Op::Pocsag | Op::Packet) { self.ui_qso_bar(ui); ui.separator(); } // APRS, POCSAG und Packet brauchen keine QSO-Leiste
                    self.ui_scope(ui, ctx);
                    if narrow {
                        let second = match self.op { Op::Aprs => "Bake & Nachrichten", Op::Pocsag => "Funkruf senden", Op::Packet => "Verbindung", _ => "Senden" };
                        ui.horizontal(|ui| {
                            for (tx, label) in [(false, "Empfang"), (true, second)] {
                                if ui.selectable_label(self.pane_tx == tx, RichText::new(label).size(15.0).strong()).clicked() { self.pane_tx = tx; }
                            }
                        });
                        ui.separator();
                        egui::ScrollArea::vertical().id_salt("pane").auto_shrink([false, false]).show(ui, |ui| {
                            match (self.op, self.pane_tx) {
                                (Op::Sstv, false) => self.ui_rx(ui, ctx),
                                (Op::Sstv, true) => self.ui_tx(ui),
                                (Op::Rtty, false) => self.ui_rtty_rx(ui, ctx),
                                (Op::Rtty, true) => self.ui_rtty_tx(ui),
                                (Op::Aprs, false) => self.ui_aprs_rx(ui, ctx),
                                (Op::Aprs, true) => self.ui_aprs_tx(ui),
                                (Op::Cw, false) => self.ui_cw_rx(ui, ctx),
                                (Op::Cw, true) => self.ui_cw_tx(ui),
                                (Op::Pocsag, false) => self.ui_pocsag_rx(ui, ctx),
                                (Op::Pocsag, true) => self.ui_pocsag_tx(ui),
                                (Op::Packet, false) => self.ui_packet_rx(ui, ctx),
                                (Op::Packet, true) => self.ui_packet_tx(ui),
                            }
                        });
                    } else {
                        // überall gleich: links Empfang, rechts Senden bzw. QSO
                        let mut two = |ui: &mut egui::Ui| ui.columns(2, |cols| match self.op {
                            Op::Sstv => { self.ui_rx(&mut cols[0], ctx); self.ui_tx(&mut cols[1]); }
                            Op::Rtty => { self.ui_rtty_rx(&mut cols[0], ctx); self.ui_rtty_tx(&mut cols[1]); }
                            Op::Aprs => { self.ui_aprs_rx(&mut cols[0], ctx); self.ui_aprs_tx(&mut cols[1]); }
                            Op::Cw => { self.ui_cw_rx(&mut cols[0], ctx); self.ui_cw_tx(&mut cols[1]); }
                            Op::Pocsag => { self.ui_pocsag_rx(&mut cols[0], ctx); self.ui_pocsag_tx(&mut cols[1]); }
                            Op::Packet => { self.ui_packet_rx(&mut cols[0], ctx); self.ui_packet_tx(&mut cols[1]); }
                        });
                        // rollbar, damit bei wenig Höhe (oder großen Touch-Knöpfen) unten nichts abgeschnitten wird
                        egui::ScrollArea::vertical().id_salt("main").auto_shrink([false, false]).show(ui, |ui| two(ui));
                    }
                });
            }
            Tab::Diag => {
                egui::CentralPanel::default().show(ctx, |ui| self.ui_diag(ui, ctx));
            }
            Tab::Templates => self.ui_editor(ctx),
            Tab::Logbook => { egui::CentralPanel::default().show(ctx, |ui| self.ui_logbook(ui)); }
            Tab::Gallery => self.ui_gallery(ctx),
        }
    }

}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_inner(ctx);
    }

    fn save(&mut self, s: &mut dyn eframe::Storage) {
        for (k, v) in [("in_dev", &self.in_dev), ("out_dev", &self.out_dev), ("port", &self.port), ("freq", &self.freq_khz),
                       ("my_call", &self.station.call), ("my_name", &self.station.name), ("my_qth", &self.station.qth),
                       ("my_loc", &self.station.locator), ("photo", &self.photo_path),
                       ("net_host", &self.net_host), ("net_user", &self.net_user), ("net_pass", &self.net_pass),
                       ("ap_ssid", &self.ap_ssid), ("ap_pass", &self.ap_pass)] {
            s.set_string(k, v.clone());
        }
        s.set_string("rig", self.rig_idx.map_or(String::new(), |i| i.to_string()));
        s.set_string("baud", self.baud.to_string());
        s.set_string("ptt", self.ptt.label().into());
        s.set_string("tx_mode", self.tx_mode.to_string());
        for (i, g) in self.gains.iter().enumerate() { s.set_string(&format!("gain{i}"), g.to_string()); }
        s.set_string("audio_on", if self.input.is_some() { "1" } else { "0" }.into());
        s.set_string("cat_on", if self.rig.is_some() || self.net_connecting { "1" } else { "0" }.into());
        s.set_string("use_net", if self.use_net { "1" } else { "0" }.into());
        s.set_string("net_ap", if self.net_ap { "1" } else { "0" }.into());
        s.set_string("op", match self.op { Op::Rtty => "RTTY", Op::Aprs => "APRS", Op::Cw => "CW", Op::Sstv => "SSTV", Op::Pocsag => "POCSAG", Op::Packet => "PACKET" }.into());
        s.set_string("cw_pitch", self.cw.pitch.to_string());
        s.set_string("power_save", if self.power_save { "1" } else { "0" }.into());
        s.set_string("scope", if self.scope_want { "1" } else { "0" }.into());
        s.set_string("scope_band", if self.scope_v.band { "1" } else { "0" }.into());
        s.set_string("scope_span", self.scope_v.half_span.to_string());
        s.set_string("cw_skim", if self.skim_on { "1" } else { "0" }.into());
        if let Some(t) = self.touch { s.set_string("touch", if t { "1" } else { "0" }.into()); }
        s.set_string("zoom", self.zoom.to_string());
        s.set_string("cw_sq", self.cw.squelch.to_string());
        self.aprs.save(s);
        self.pocsag.save(s);
        self.packet.save(s);
        self.sat.save(s);
        s.set_string("if_rx", if self.if_rx { "1" } else { "0" }.into());
        s.set_string("if_auto", if self.if_auto { "1" } else { "0" }.into());
        s.set_string("kiss_port", self.kiss_port.to_string());
        s.set_string("agw_port", self.agw_port.to_string());
        s.set_string("kiss_lan", if self.kiss_lan { "1" } else { "0" }.into());
        s.set_string("mic_dev", self.mic_dev.clone());
        s.set_string("spk_dev", self.spk_dev.clone());
        s.set_string("mic_gain", self.mic_gain.to_string());
        s.set_string("mon_vol", self.mon_vol.to_string());
        s.set_string("ptt_toggle", if self.ptt_toggle { "1" } else { "0" }.into());
        s.set_string("rtty_center", self.rtty.center.to_string());
        s.set_string("rtty_shift", self.rtty.shift.to_string());
        s.set_string("rtty_baud", self.rtty.baud.to_string());
        s.set_string("rtty_rev", if self.rtty.reverse { "1" } else { "0" }.into());
        s.set_string("rtty_sq", self.rtty.squelch.to_string());
        for (i, m) in self.rtty_macros.iter().enumerate() { s.set_string(&format!("rtty_m{i}"), m.clone()); }
        for (i, m) in self.cw_macros.iter().enumerate() { s.set_string(&format!("cw_m{i}"), m.clone()); }
        s.set_string("cw_wpm", self.cw_wpm.to_string());
        s.set_string("profile", self.profiles.active.clone());
        for (op, (f, sb)) in &self.freq_mem {
            s.set_string(&format!("mem_{op:?}"), format!("{f},{}", sb.map_or("-".to_string(), |m| format!("{m:?}"))));
        }
        s.set_string("fskid_tx", if self.fskid_tx { "1" } else { "0" }.into());
        s.set_string("copy_dir", self.copy_dir.clone());
        s.set_string("fps", self.fps.to_string());
        if self.tpl_save_due.is_some() { self.save_templates(); }
    }

    /// öfter als die voreingestellten 30 s speichern, damit nach hartem Beenden nichts verloren geht
    fn auto_save_interval(&self) -> Duration { Duration::from_secs(3) }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_tx();
        if let Some(r) = &mut self.rig { let _ = r.ptt(false); } // PTT sicher lösen
        self.if_restore();
        self.rig = None;
        self.net = None; // beim Funkgerät abmelden
        if self.tpl_save_due.is_some() { self.save_templates(); }
    }
}

fn main() -> eframe::Result {
    // Hilfsaufruf mit Administratorrechten: PC-Uhr stellen und beenden
    #[cfg(windows)]
    {
        let args: Vec<String> = std::env::args().collect();
        if let Some(i) = args.iter().position(|a| a == "--uhr-stellen") {
            let ok = args.get(i + 1).and_then(|s| s.parse().ok()).is_some_and(|o| rigctl::clock::set_offset(o).is_ok());
            std::process::exit(if ok { 0 } else { 1 });
        }
    }
    // Ohne Konsole gingen Panics verloren: in eine Datei im Temp-Ordner schreiben
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let path = std::env::temp_dir().join("rustwave-absturz.txt");
        let _ = std::fs::write(&path, format!("{}\n{}", info, std::backtrace::Backtrace::force_capture()));
        default_hook(info);
    }));
    let opts = eframe::NativeOptions {
        // beim ersten Start maximiert (passt so auch auf kleine Bildschirme wie 1280×800); danach merkt sich eframe die Fenstergröße
        viewport: egui::ViewportBuilder::default().with_inner_size([1360.0, 900.0]).with_min_inner_size([480.0, 360.0])
            .with_maximized(true).with_title("Rustwave")
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/logo/icon-256.png")).unwrap_or_default()),
        ..Default::default()
    };
    let migrated = migrate_old_data();
    eframe::run_native(APP_ID, opts, Box::new(move |cc| {
        cc.egui_ctx.set_theme(egui::ThemePreference::Dark); // unabhängig vom System-Theme
        let mut app = App::new(cc);
        if let Some(m) = migrated { app.log(m); }
        Ok(Box::new(app))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WAV-Pfad: Datei schreiben, wie die GUI dekodieren, Ergebnis prüfen
    #[test]
    fn wav_dekodieren() {
        let m = &sstv::MODES[5]; // Robot 36
        let img = sstv::Img { w: m.w, h: m.h, data: test_pattern(m.w as u32, m.h as u32).into_raw() };
        let fs = 44100;
        let sig = sstv::encode(m, &img, fs as f64, 0.5);
        let path = std::env::temp_dir().join("sstv_test.wav");
        let mut w = hound::WavWriter::create(&path, hound::WavSpec { channels: 2, sample_rate: fs, bits_per_sample: 16, sample_format: hound::SampleFormat::Int }).unwrap();
        for v in std::iter::repeat_n(0.0, 22050).chain(sig).chain(std::iter::repeat_n(0.0, 22050)) {
            w.write_sample((v * 32767.0) as i16).unwrap(); w.write_sample(0i16).unwrap(); // Stereo, nur links belegt
        }
        w.finalize().unwrap();
        let (tx, rx) = mpsc::channel();
        decode_file(path, tx, None, FileMode::Sstv, false);
        let end = rx.try_iter().find_map(|e| if let UiEv::End(mm, out, c) = e { Some((mm, out, c)) } else { None }).expect("kein Bild");
        assert!(std::ptr::eq(end.0, m) && end.2);
        let err: f64 = img.data.iter().zip(&end.1.data).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>() / img.data.len() as f64;
        assert!(err < 6.0, "mittlere Abweichung {err}");
    }

    /// SSTV-Bild mit FSKID: Bild und Rufzeichen werden erkannt
    #[test]
    fn sstv_mit_fskid() {
        let m = &sstv::MODES[5]; // Robot 36
        let img = sstv::Img { w: m.w, h: m.h, data: test_pattern(m.w as u32, m.h as u32).into_raw() };
        let fs = 48000.0;
        let mut sig = sstv::encode(m, &img, fs, 0.5);
        sig.extend(fskid::encode("DL1ABC/P", fs, 0.5));
        let path = std::env::temp_dir().join("sstv_fskid_test.wav");
        let mut w = hound::WavWriter::create(&path, hound::WavSpec { channels: 1, sample_rate: 48000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int }).unwrap();
        for v in std::iter::repeat_n(0.0, 24000).chain(sig) { w.write_sample((v * 32767.0) as i16).unwrap(); }
        w.finalize().unwrap();
        let (tx, rx) = mpsc::channel();
        decode_file(path, tx, None, FileMode::Sstv, false);
        let ev: Vec<UiEv> = rx.try_iter().collect();
        assert!(ev.iter().any(|e| matches!(e, UiEv::End(_, _, true))), "Bild");
        assert!(ev.iter().any(|e| matches!(e, UiEv::FskId(c) if c == "DL1ABC/P")), "FSKID");
    }

    /// APRS-Aufnahme (WAV) wie über „WAV dekodieren“
    #[test]
    fn aprs_wav_dekodieren() {
        let f = aprs::Frame { src: "DL1ABC-9".into(), dst: aprs::TOCALL.into(), path: vec!["WIDE1-1".into()], info: b">Test".to_vec() };
        let x = aprs::modulate(&[f.clone(), f.clone()], 48000.0, 300.0, aprs::VHF);
        let path = std::env::temp_dir().join("aprs_test.wav");
        let mut w = hound::WavWriter::create(&path, hound::WavSpec { channels: 1, sample_rate: 48000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int }).unwrap();
        for v in x { w.write_sample((v * 16000.0) as i16).unwrap(); }
        w.finalize().unwrap();
        let (tx, rx) = mpsc::channel();
        decode_file(path, tx, None, FileMode::Aprs(aprs::VHF), false);
        let frames: Vec<aprs::Frame> = rx.try_iter().filter_map(|e| if let UiEv::Aprs(f, _) = e { Some(f) } else { None }).collect();
        assert_eq!(frames, vec![f]); // zweimal gesendet, gleiche Rahmen innerhalb von 3 s nur einmal gemeldet
    }

    /// POCSAG-Aufnahme (WAV) wie über „WAV dekodieren“: Ruf gesendet und gelesen
    #[test]
    fn pocsag_wav_dekodieren() {
        let pg = pocsag::Page { addr: 123456, func: 3, content: pocsag::Content::Alpha("Test DAPNET de DL1ABC".into()) };
        let x = pocsag::modulate(&[pg], pocsag::Charset::Us, 1200, 44100.0, false);
        let path = std::env::temp_dir().join("pocsag_test.wav");
        let mut w = hound::WavWriter::create(&path, hound::WavSpec { channels: 1, sample_rate: 44100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int }).unwrap();
        for v in std::iter::repeat_n(0.0, 4410).chain(x) { w.write_sample((v * 12000.0) as i16).unwrap(); }
        w.finalize().unwrap();
        let (tx, rx) = mpsc::channel();
        decode_file(path, tx, None, FileMode::Pocsag(pocsag::Params::default()), false);
        let msgs: Vec<pocsag::Msg> = rx.try_iter().filter_map(|e| if let UiEv::Pocsag(m) = e { Some(m) } else { None }).collect();
        assert_eq!(msgs.len(), 1);
        assert_eq!((msgs[0].addr, msgs[0].func), (Some(123456), Some(3)));
        assert_eq!(msgs[0].clean_text(), "Test DAPNET de DL1ABC");
    }

    /// ZF-Aufnahme wie über „WAV dekodieren“ mit eingeschaltetem ZF-Empfang:
    /// ZF_WAV=/pfad.wav cargo test --release zf_wav_dekodieren -- --ignored --nocapture
    #[test]
    #[ignore]
    fn zf_wav_dekodieren() {
        let (tx, rx) = mpsc::channel();
        decode_file(PathBuf::from(std::env::var("ZF_WAV").unwrap()), tx, None, FileMode::Pocsag(pocsag::Params::default()), true);
        for e in rx.try_iter() {
            match e { UiEv::Pocsag(m) => println!("RUF {}", m.multimon_line()), UiEv::Log(l) => println!("LOG {l}"), _ => {} }
        }
    }

    /// Live-Weg wie über WLAN (20-ms-Pakete) durch den Empfangs-Thread:
    /// LIVE_WAV=/pfad.wav LIVE_IF=1 cargo test --release live_wlan -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_wlan() {
        let mut rd = hound::WavReader::open(std::env::var("LIVE_WAV").unwrap()).unwrap();
        let fs = rd.spec().sample_rate as f64;
        let x: Vec<f32> = rd.samples::<i16>().map(|v| v.unwrap() as f32 / 32768.0).collect();
        let sh: Arc<RxShared> = Arc::default();
        sh.if_rx.store(std::env::var("LIVE_IF").is_ok(), Relaxed);
        *sh.pocsag.lock().unwrap() = Some(pocsag::Params::default());
        let (atx, arx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let sh2 = sh.clone();
        let th = std::thread::spawn(move || rx_thread(fs, arx, etx, sh2, egui::Context::default()));
        let (mut seen, mut clip, mut levels) = (vec![], vec![], vec![]);
        for (k, c) in x.chunks(960).enumerate() {
            atx.send(c.to_vec()).unwrap();
            if k % 250 == 249 { std::thread::sleep(Duration::from_millis(30)); seen.push(sh.if_seen.load(Relaxed)); clip.push(f32::from_bits(sh.clip.load(Relaxed))); }
        }
        drop(atx);
        th.join().unwrap();
        let mut n = 0;
        for e in erx.try_iter() {
            match e { UiEv::Pocsag(m) => { n += 1; println!("RUF {}", m.multimon_line()); } UiEv::Wf(_, l) => levels.push(l), _ => {} }
        }
        let lv = levels.iter().sum::<f32>() / levels.len().max(1) as f32;
        println!("ERGEBNIS: {n} Rufe, ZF erkannt {}/{}, Übersteuerung max {:.2} ‰, Pegel mittel {lv:.3}", seen.iter().filter(|&&b| b).count(), seen.len(),
            clip.iter().cloned().fold(0.0f32, f32::max));
    }

    /// APRS-Datei zählen: APRS_WAV=/pfad.wav [APRS_IF=1] cargo test --release aprs_zaehlen -- --ignored --nocapture
    #[test]
    #[ignore]
    fn aprs_zaehlen() {
        let (tx, rx) = mpsc::channel();
        decode_file(PathBuf::from(std::env::var("APRS_WAV").unwrap()), tx, None, FileMode::Aprs(aprs::VHF), std::env::var("APRS_IF").is_ok());
        println!("APRS-PAKETE {}", rx.try_iter().filter(|e| matches!(e, UiEv::Aprs(..))).count());
    }

    /// Hilfsmittel für Vergleichstests mit anderen Dekodern:
    /// SSTV_WAV=/pfad.wav SSTV_MODE="Robot 36" cargo test --release export_wav -- --ignored
    #[test]
    #[ignore]
    fn export_wav() {
        let name = std::env::var("SSTV_MODE").unwrap_or("Robot 36".into());
        let m = sstv::MODES.iter().find(|m| m.name == name).expect("Modus");
        let img = sstv::Img { w: m.w, h: m.h, data: test_pattern(m.w as u32, m.h as u32).into_raw() };
        let sig = sstv::encode(m, &img, 48000.0, 0.5);
        let mut w = hound::WavWriter::create(std::env::var("SSTV_WAV").unwrap(), hound::WavSpec { channels: 1, sample_rate: 48000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int }).unwrap();
        for v in std::iter::repeat_n(0.0, 24000).chain(sig).chain(std::iter::repeat_n(0.0, 24000)) { w.write_sample((v * 32767.0) as i16).unwrap(); }
        w.finalize().unwrap();
    }
}
