//! Icom-Netzwerkprotokoll (wie RS-BA1), z. B. IC-705 über WLAN oder IC-9700/IC-7610 über LAN:
//! CI-V und Audio (16 Bit PCM, 48 kHz, mono) über UDP.
//!
//! Ablauf und Paketaufbau nach kappanhang (MIT-Lizenz, © 2020 Norbert Varga HA2NON,
//! Akos Marton ES1AKOS); Feldlagen mit den Paketstrukturen von wfview abgeglichen.
//! Fehlende Audiopakete vom Funkgerät werden neu angefordert und in die richtige Reihenfolge
//! gebracht (wichtig für APRS: schon ein verlorenes 20-ms-Paket zerstört einen Rahmen);
//! Neuübertragungswünsche des Funkgeräts für unsere Pakete werden bedient.

use crate::audio::PlayState;
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

pub const CONTROL_PORT: u16 = 50001;
pub const CIV_PORT: u16 = 50002;
pub const AUDIO_PORT: u16 = 50003;
pub const RATE: u32 = 48000;
const FRAME: usize = 960; // 20 ms bei 48 kHz
/// Sendepuffer im Funkgerät: es spielt unser Audio erst nach dieser Zeit aus
pub const TX_BUFFER_MS: u16 = 300;
const PING_EVERY: Duration = Duration::from_millis(500);
const TOKEN_EVERY: Duration = Duration::from_secs(60);

/// Kodierung von Benutzername/Passwort (Tabelle für ASCII 32..=126)
const PASSCODE: [u8; 95] = [
    0x47, 0x5d, 0x4c, 0x42, 0x66, 0x20, 0x23, 0x46, 0x4e, 0x57, 0x45, 0x3d, 0x67, 0x76, 0x60, 0x41, 0x62, 0x39, 0x59, 0x2d,
    0x68, 0x7e, 0x7c, 0x65, 0x7d, 0x49, 0x29, 0x72, 0x73, 0x78, 0x21, 0x6e, 0x5a, 0x5e, 0x4a, 0x3e, 0x71, 0x2c, 0x2a, 0x54,
    0x3c, 0x3a, 0x63, 0x4f, 0x43, 0x75, 0x27, 0x79, 0x5b, 0x35, 0x70, 0x48, 0x6b, 0x56, 0x6f, 0x34, 0x32, 0x6c, 0x30, 0x61,
    0x6d, 0x7b, 0x2f, 0x4b, 0x64, 0x38, 0x2b, 0x2e, 0x50, 0x40, 0x3f, 0x55, 0x33, 0x37, 0x25, 0x77, 0x24, 0x26, 0x74, 0x6a,
    0x28, 0x53, 0x4d, 0x69, 0x22, 0x5c, 0x44, 0x31, 0x36, 0x58, 0x3b, 0x7a, 0x51, 0x5f, 0x52,
];

pub fn passcode(s: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, b) in s.bytes().take(16).enumerate() {
        let mut p = b as usize + i;
        if p > 126 { p = 32 + p % 127; }
        if p >= 32 { out[i] = PASSCODE[p - 32]; }
    }
    out
}

fn put_be32(p: &mut [u8], v: u32) { p.copy_from_slice(&v.to_be_bytes()); }
fn rand16() -> u16 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() as u16 ^ (d.as_secs() as u16)) }

/// Ein UDP-Kanal (Steuerung, CI-V oder Audio) mit Sitzungs-IDs, Laufnummern und Sendepuffer
struct Stream {
    sock: UdpSocket,
    local: u32,
    remote: u32,
    seq: u16,
    sent: VecDeque<(u16, Vec<u8>)>,
    ping_seq: u16,
    ping_inner: u16,
    last_ping: Instant,
    last_idle: Instant,
    last_tracked: Instant,
}

impl Stream {
    fn open(ip: IpAddr, port: u16) -> Result<Stream, String> {
        let sock = UdpSocket::bind(("0.0.0.0", 0)).map_err(|e| e.to_string())?;
        sock.connect(SocketAddr::new(ip, port)).map_err(|e| e.to_string())?;
        sock.set_nonblocking(true).map_err(|e| e.to_string())?;
        let la = sock.local_addr().map_err(|e| e.to_string())?;
        // eigene Sitzungs-ID aus lokaler IP und Port (wie kappanhang)
        let ip4 = match la.ip() {
            IpAddr::V4(v) => u32::from_be_bytes(v.octets()),
            IpAddr::V6(_) => 0x7f000001,
        };
        let now = Instant::now();
        Ok(Stream { sock, local: (ip4 << 16) | la.port() as u32, remote: 0, seq: 1, sent: VecDeque::new(),
            ping_seq: 1, ping_inner: 0x8304, last_ping: now, last_idle: now, last_tracked: now - Duration::from_secs(10) })
    }

    fn header(&self, len: usize, typ: u8, seq: u16) -> Vec<u8> {
        let mut p = vec![0u8; len];
        p[..4].copy_from_slice(&(len as u32).to_le_bytes());
        p[4] = typ;
        p[6..8].copy_from_slice(&seq.to_le_bytes());
        put_be32(&mut p[8..12], self.local);
        put_be32(&mut p[12..16], self.remote);
        p
    }
    fn send(&self, p: &[u8]) { let _ = self.sock.send(p); }
    fn send_twice(&self, p: &[u8]) { self.send(p); self.send(p); }
    /// Paket mit fortlaufender Nummer senden und für Neuübertragungen aufheben
    fn send_tracked(&mut self, mut p: Vec<u8>) {
        p[6..8].copy_from_slice(&self.seq.to_le_bytes());
        self.send(&p);
        if !(p.len() == 16 && p[4] == 0) { self.last_tracked = Instant::now(); }
        self.sent.push_back((self.seq, p));
        while self.sent.len() > 500 { self.sent.pop_front(); }
        self.seq = self.seq.wrapping_add(1);
    }
    fn recv(&self) -> Option<Vec<u8>> {
        let mut b = [0u8; 2048];
        self.sock.recv(&mut b).ok().map(|n| b[..n].to_vec())
    }

    /// "Bist du da?" / "Bereit?" – liefert die Sitzungs-ID des Funkgeräts
    fn handshake(&mut self) -> Result<(), String> {
        self.send_twice(&self.header(16, 0x03, 0));
        let r = self.wait(Duration::from_secs(2), |r| r.len() == 16 && r[4] == 0x04)
            .ok_or("Funkgerät antwortet nicht – IP-Adresse, WLAN und Netzwerkeinstellungen am Gerät prüfen")?;
        self.remote = u32::from_be_bytes([r[8], r[9], r[10], r[11]]);
        self.send_twice(&self.header(16, 0x06, 1));
        self.wait(Duration::from_secs(2), |r| r.len() == 16 && r[4] == 0x06).ok_or("Funkgerät meldet sich nicht bereit")?;
        Ok(())
    }
    fn wait(&mut self, t: Duration, pred: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
        let t0 = Instant::now();
        while t0.elapsed() < t {
            while let Some(r) = self.recv() {
                if self.handle_common(&r) { continue; }
                if pred(&r) { return Some(r); }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        None
    }

    /// Ping-Anfragen beantworten, Neuübertragungen bedienen; true = Paket erledigt
    fn handle_common(&mut self, r: &[u8]) -> bool {
        if r.len() == 21 && r[4] == 0x07 {
            if r[16] == 0 { // Anfrage des Funkgeräts -> Antwort mit gleicher ID
                let mut p = self.header(21, 0x07, u16::from_le_bytes([r[6], r[7]]));
                p[16] = 1;
                p[17..21].copy_from_slice(&r[17..21]);
                self.send(&p);
            }
            return true;
        }
        if r.len() >= 16 && r[4] == 0x01 && r[5] == 0 {
            let mut want: Vec<u16> = vec![];
            if r.len() == 16 { want.push(u16::from_le_bytes([r[6], r[7]])); } else {
                for c in r[16..].chunks_exact(4) {
                    let (a, b) = (u16::from_le_bytes([c[0], c[1]]), u16::from_le_bytes([c[2], c[3]]));
                    let mut s = a;
                    for _ in 0..64 { want.push(s); if s == b { break; } s = s.wrapping_add(1); }
                }
            }
            for s in want {
                match self.sent.iter().find(|(q, _)| *q == s) {
                    Some((_, p)) => { let p = p.clone(); self.send_twice(&p); }
                    None => { let p = self.header(16, 0x00, s); self.send_twice(&p); } // Leerpaket mit der Nummer
                }
            }
            return true;
        }
        r.len() == 16 && r[4] == 0x00 // Leerpaket
    }

    fn tick(&mut self, idle: bool) {
        if self.last_ping.elapsed() >= PING_EVERY {
            let mut p = self.header(21, 0x07, self.ping_seq);
            p[17] = rand16() as u8;
            p[18..20].copy_from_slice(&self.ping_inner.to_le_bytes());
            p[20] = 0x06;
            self.send(&p);
            self.ping_seq = self.ping_seq.wrapping_add(1);
            self.ping_inner = self.ping_inner.wrapping_add(1);
            self.last_ping = Instant::now();
        }
        if idle {
            let every = if self.last_tracked.elapsed() < Duration::from_secs(1) { 100 } else { 1000 };
            if self.last_idle.elapsed() >= Duration::from_millis(every) {
                let p = self.header(16, 0x00, 0);
                self.send_tracked(p);
                self.last_idle = Instant::now();
            }
        }
    }
}

/// Beim Verwerfen abmelden – auch wenn die Anmeldung scheitert, damit das Funkgerät
/// (das nur eine Sitzung zulässt) nicht blockiert bleibt
impl Drop for Stream {
    fn drop(&mut self) { if self.remote != 0 { self.send_twice(&self.header(16, 0x05, 0)); } }
}

/// Zwischen App und Netzwerk-Thread geteilter Zustand
#[derive(Default)]
pub struct Shared {
    civ_out: Mutex<Vec<Vec<u8>>>,
    civ_in: Mutex<Vec<u8>>,
    audio_in: Mutex<Option<mpsc::Sender<Vec<f32>>>>,
    play: Mutex<Option<(Vec<f32>, Arc<PlayState>)>>,
    /// Sprechfunk: laufender Mikrofon-Ring (wird mit 48 kHz in 20-ms-Paketen gesendet)
    pub live: Mutex<Option<Arc<crate::voice::Ring>>>,
    stop: AtomicBool,
    /// gesetzt, wenn die Verbindung abgerissen ist
    pub error: Mutex<Option<String>>,
    /// Empfangsstatistik Audio: Pakete, davon neu angefordert und gerettet, endgültig verloren
    pub audio_rx: AtomicU64,
    pub audio_recovered: AtomicU64,
    pub audio_lost: AtomicU64,
}
impl Shared {
    pub fn send_civ(&self, frame: &[u8]) { self.civ_out.lock().unwrap().push(frame.to_vec()); }
    pub fn take_civ(&self) -> Vec<u8> { std::mem::take(&mut *self.civ_in.lock().unwrap()) }
    pub fn clear_civ(&self) { self.civ_in.lock().unwrap().clear(); }
    pub fn set_audio_sink(&self, tx: Option<mpsc::Sender<Vec<f32>>>) { *self.audio_in.lock().unwrap() = tx; }
    /// Mono-Samples (48 kHz, volle Aussteuerung) senden; Pegel/Fortschritt/Stopp über den PlayState
    pub fn play(&self, samples: Vec<f32>, gain: f32) -> Arc<PlayState> {
        let st = PlayState::new(samples.len(), gain);
        *self.play.lock().unwrap() = Some((samples, st.clone()));
        st
    }
}

pub struct NetRadio {
    pub shared: Arc<Shared>,
    pub name: String,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for NetRadio {
    fn drop(&mut self) {
        self.shared.stop.store(true, Relaxed);
        if let Some(t) = self.thread.take() { let _ = t.join(); }
    }
}

struct Session { token: [u8; 6], inner: u16 }
impl Session {
    fn token_packet(&mut self, c: &Stream, kind: u8) -> Vec<u8> {
        let mut p = c.header(0x40, 0x00, 0);
        p[19] = 0x30;
        p[20] = 0x01;
        p[21] = kind;
        p[23] = self.inner as u8;
        p[24] = (self.inner >> 8) as u8;
        p[26..32].copy_from_slice(&self.token);
        self.inner = self.inner.wrapping_add(1);
        p
    }
}

fn cstr(b: &[u8]) -> String { String::from_utf8_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]).to_string() }

/// Anmelden und alle drei Kanäle öffnen (blockiert einige Sekunden)
pub fn connect(host: &str, user: &str, pass: &str) -> Result<NetRadio, String> {
    let ip = (host.trim(), CONTROL_PORT).to_socket_addrs().map_err(|e| format!("{host}: {e}"))?
        .find(|a| a.is_ipv4()).ok_or_else(|| format!("{host}: keine IPv4-Adresse"))?.ip();
    let mut c = Stream::open(ip, CONTROL_PORT)?;
    c.handshake()?;

    // Anmeldung
    let mut s = Session { token: [0; 6], inner: 0 };
    let mut p = c.header(0x80, 0x00, 0);
    p[19] = 0x70;
    p[20] = 0x01;
    p[23] = s.inner as u8;
    p[24] = (s.inner >> 8) as u8;
    let start_id = rand16() | 1;
    p[26..28].copy_from_slice(&start_id.to_le_bytes());
    p[0x40..0x50].copy_from_slice(&passcode(user));
    p[0x50..0x60].copy_from_slice(&passcode(pass));
    p[0x60..0x67].copy_from_slice(b"icom-pc");
    s.inner += 1;
    c.send_tracked(p);
    let r = c.wait(Duration::from_secs(3), |r| r.len() == 0x60 && r[0] == 0x60).ok_or("keine Antwort auf die Anmeldung")?;
    if r[0x30..0x34] == [0xff, 0xff, 0xff, 0xfe] { return Err("Benutzername oder Passwort falsch (am Gerät: Netzwerk > Netzwerkbenutzer)".into()); }
    s.token.copy_from_slice(&r[26..32]);
    let p = s.token_packet(&c, 0x02);
    c.send_tracked(p);
    let p = s.token_packet(&c, 0x05);
    c.send_tracked(p);

    // Fähigkeiten abwarten, dann CI-V- und Audiokanal anfordern
    let (mut guid, mut name, mut auth_ok, mut requested) = (None::<[u8; 16]>, String::new(), false, false);
    let t0 = Instant::now();
    loop {
        if t0.elapsed() > Duration::from_secs(6) { return Err("Funkgerät gibt CI-V/Audio nicht frei (Zeitüberschreitung)".into()); }
        c.tick(true);
        let Some(r) = c.recv() else { std::thread::sleep(Duration::from_millis(2)); continue };
        if c.handle_common(&r) { continue; }
        match (r.len(), r[0]) {
            (0xa8, 0xa8) => { let mut g = [0u8; 16]; g.copy_from_slice(&r[0x42..0x52]); guid = Some(g); name = cstr(&r[0x52..0x72]); }
            (0x40, 0x40) => { if r[21] == 0x05 { auth_ok = true; } }
            (0x50, 0x50) => {
                if r[0x30..0x33] == [0xff, 0xff, 0xff] { return Err("Anmeldung abgelehnt – Funkgerät neu starten".into()); }
            }
            (0x90, 0x90) if requested && r[0x60] == 1 => {
                c.remote = u32::from_be_bytes([r[8], r[9], r[10], r[11]]);
                s.token.copy_from_slice(&r[26..32]);
                break;
            }
            _ => {}
        }
        if let (Some(g), true, false) = (guid, auth_ok, requested) {
            let mut p = c.header(0x90, 0x00, 0);
            p[19] = 0x80;
            p[20] = 0x01;
            p[21] = 0x03;
            p[23] = s.inner as u8;
            p[24] = (s.inner >> 8) as u8;
            p[26..32].copy_from_slice(&s.token);
            p[0x20..0x30].copy_from_slice(&g);
            let n = name.as_bytes();
            p[0x40..0x40 + n.len().min(32)].copy_from_slice(&n[..n.len().min(32)]);
            p[0x60..0x70].copy_from_slice(&passcode(user));
            p[0x70] = 1; // RX an
            p[0x71] = 1; // TX an
            p[0x72] = 0x04; // RX: PCM 16 Bit mono
            p[0x73] = 0x04; // TX: PCM 16 Bit mono
            put_be32(&mut p[0x74..0x78], RATE);
            put_be32(&mut p[0x78..0x7c], RATE);
            put_be32(&mut p[0x7c..0x80], CIV_PORT as u32);
            put_be32(&mut p[0x80..0x84], AUDIO_PORT as u32);
            put_be32(&mut p[0x84..0x88], TX_BUFFER_MS as u32);
            p[0x88] = 1;
            s.inner += 1;
            c.send_tracked(p);
            requested = true;
        }
    }

    // CI-V-Kanal
    let mut civ = Stream::open(ip, CIV_PORT)?;
    civ.handshake()?;
    let mut civ_seq: u16 = 0;
    let mut p = civ.header(0x16, 0x00, 0);
    p[16] = 0xc0;
    p[17] = 0x01;
    p[19..21].copy_from_slice(&civ_seq.to_be_bytes());
    p[21] = 0x05; // öffnen
    civ.send_tracked(p);
    civ_seq += 1;

    // Audiokanal
    let mut au = Stream::open(ip, AUDIO_PORT)?;
    au.handshake()?;

    let shared = Arc::new(Shared::default());
    let sh = shared.clone();
    let thread = std::thread::spawn(move || run(c, civ, au, s, civ_seq, sh));
    Ok(NetRadio { shared, name: if name.is_empty() { "Icom".into() } else { name }, thread: Some(thread) })
}

/// Ordnet Audiopakete nach ihrer Folgenummer, meldet Lücken zum Neuanfordern und füllt nach
/// `WAIT` endgültig fehlende Pakete mit Stille (so bleibt die Zeitachse für die Dekoder erhalten).
struct Jitter { next: Option<u16>, pending: HashMap<u16, (Instant, Vec<f32>, bool)>, requested: Vec<u16>, last_len: usize }
const WAIT: Duration = Duration::from_millis(150);

impl Jitter {
    fn new() -> Self { Jitter { next: None, pending: HashMap::new(), requested: vec![], last_len: 682 } }
    /// Paket einordnen; liefert Folgenummern, die neu angefordert werden sollen
    fn put(&mut self, seq: u16, x: Vec<f32>, now: Instant) -> Vec<u16> {
        let next = *self.next.get_or_insert(seq);
        let d = seq.wrapping_sub(next) as i16;
        if d < 0 { return vec![]; } // Doppel oder zu spät
        if d > 500 { self.pending.clear(); self.requested.clear(); self.next = Some(seq); } // Neustart der Zählung
        if self.pending.contains_key(&seq) { return vec![]; } // doppelt (Neuübertragung kommt oft zweimal)
        let late = self.requested.contains(&seq);
        if late { self.requested.retain(|&r| r != seq); }
        self.pending.insert(seq, (now, x, late));
        let next = self.next.unwrap();
        let mut ask = vec![];
        let mut s = next;
        while s != seq {
            if !self.pending.contains_key(&s) && !self.requested.contains(&s) { self.requested.push(s); ask.push(s); }
            s = s.wrapping_add(1);
        }
        ask
    }
    /// fertige Audioblöcke in richtiger Reihenfolge; (Daten, nachgeliefert?, Stille für verlorenes Paket?)
    fn take(&mut self, now: Instant, out: &mut Vec<(Vec<f32>, bool)>) {
        while let Some(next) = self.next {
            if let Some((_, x, late)) = self.pending.remove(&next) {
                self.last_len = x.len();
                out.push((x, late));
                self.next = Some(next.wrapping_add(1));
            } else if self.pending.values().any(|(t, _, _)| now.duration_since(*t) >= WAIT) {
                self.requested.retain(|&r| r != next);
                out.push((vec![], true)); // verloren
                self.next = Some(next.wrapping_add(1));
            } else { break; }
        }
    }
}

fn run(mut c: Stream, mut civ: Stream, mut au: Stream, mut s: Session, mut civ_seq: u16, sh: Arc<Shared>) {
    let mut last_token = Instant::now();
    let mut audio_seq: u16 = 0;
    let mut next_frame = Instant::now();
    let mut last_audio_rx = Instant::now();
    let mut jitter = Jitter::new();
    let mut live_phase = 0.0f64;
    let fail = |m: &str| { *sh.error.lock().unwrap() = Some(m.to_string()); };
    loop {
        let stopping = sh.stop.load(Relaxed);
        // Steuerkanal
        while let Some(r) = c.recv() {
            if c.handle_common(&r) { continue; }
            if r.len() == 0x50 && r[0] == 0x50 && r[0x30..0x33] == [0, 0, 0] && r[0x40] == 1 { fail("Funkgerät hat die Verbindung getrennt"); }
        }
        c.tick(true);
        if last_token.elapsed() >= TOKEN_EVERY {
            let p = s.token_packet(&c, 0x05);
            c.send_tracked(p);
            last_token = Instant::now();
        }
        // CI-V: empfangen und senden
        while let Some(r) = civ.recv() {
            if civ.handle_common(&r) { continue; }
            // Länge im Kopf (32 Bit) und Nutzlänge (16 Bit) – Spektrumdaten sind länger als 255 Byte
            let ok = r.len() >= 22 && r[16] == 0xc1
                && u32::from_le_bytes([r[0], r[1], r[2], r[3]]) as usize == r.len()
                && u16::from_le_bytes([r[17], r[18]]) as usize + 0x15 == r.len();
            if ok {
                let mut b = sh.civ_in.lock().unwrap();
                b.extend_from_slice(&r[21..]);
                let n = b.len();
                if n > 4096 { b.drain(..n - 4096); }
            }
        }
        for f in std::mem::take(&mut *sh.civ_out.lock().unwrap()) {
            let mut p = civ.header(0x15 + f.len(), 0x00, 0);
            p[16] = 0xc1;
            p[17] = f.len() as u8;
            p[19..21].copy_from_slice(&civ_seq.to_be_bytes());
            p[21..].copy_from_slice(&f);
            civ.send_tracked(p);
            civ_seq = civ_seq.wrapping_add(1);
        }
        civ.tick(true);
        // Audio empfangen
        while let Some(r) = au.recv() {
            if au.handle_common(&r) { continue; }
            if r.len() >= 0x20 && r[4] != 0x01 && u32::from_le_bytes([r[0], r[1], r[2], r[3]]) as usize == r.len() {
                last_audio_rx = Instant::now();
                let x: Vec<f32> = r[24..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
                for m in jitter.put(u16::from_le_bytes([r[6], r[7]]), x, last_audio_rx) {
                    let p = au.header(16, 0x01, m); // Neuübertragung anfordern
                    au.send_twice(&p);
                }
            }
        }
        {
            let mut ready = vec![];
            jitter.take(Instant::now(), &mut ready);
            let sink = sh.audio_in.lock().unwrap();
            for (x, flag) in ready {
                if x.is_empty() {
                    sh.audio_lost.fetch_add(1, Relaxed);
                    if let Some(tx) = &*sink { let _ = tx.send(vec![0.0; jitter.last_len]); }
                    continue;
                }
                sh.audio_rx.fetch_add(1, Relaxed);
                if flag { sh.audio_recovered.fetch_add(1, Relaxed); }
                if let Some(tx) = &*sink { let _ = tx.send(x); }
            }
        }
        // Audio senden, im 20-ms-Takt
        {
            let mut play = sh.play.lock().unwrap();
            if let Some((samples, st)) = &mut *play {
                let mut pos = st.pos.load(Relaxed);
                if pos == 0 && next_frame < Instant::now() - Duration::from_millis(40) { next_frame = Instant::now(); }
                while Instant::now() >= next_frame && pos < samples.len() && !st.stop.load(Relaxed) {
                    let g = st.gain();
                    let end = (pos + FRAME).min(samples.len());
                    let mut pcm: Vec<u8> = samples[pos..end].iter().flat_map(|v| (((v * g).clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
                    pcm.resize(FRAME * 2, 0);
                    for part in pcm.chunks(1364) {
                        let mut p = au.header(0x18 + part.len(), 0x00, 0);
                        p[16] = 0x80;
                        p[18..20].copy_from_slice(&audio_seq.to_be_bytes());
                        p[22..24].copy_from_slice(&(part.len() as u16).to_be_bytes());
                        p[24..].copy_from_slice(part);
                        au.send_tracked(p);
                        audio_seq = audio_seq.wrapping_add(1);
                    }
                    pos = end;
                    next_frame += Duration::from_millis(20);
                }
                st.pos.store(pos, Relaxed);
                if pos >= samples.len() || st.stop.load(Relaxed) { st.done.store(true, Relaxed); *play = None; }
            }
        }
        // Sprechfunk: Mikrofon im 20-ms-Takt (nur, wenn keine Datei-Aussendung läuft)
        let live = sh.live.lock().unwrap().clone();
        if let (Some(ring), true) = (live, sh.play.lock().unwrap().is_none()) {
            let now = Instant::now();
            if next_frame < now - Duration::from_millis(60) { next_frame = now; }
            while Instant::now() >= next_frame {
                let x = ring.pull(FRAME, RATE as f64, &mut live_phase);
                let pcm: Vec<u8> = x.iter().flat_map(|v| ((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
                for part in pcm.chunks(1364) {
                    let mut p = au.header(0x18 + part.len(), 0x00, 0);
                    p[16] = 0x80;
                    p[18..20].copy_from_slice(&audio_seq.to_be_bytes());
                    p[22..24].copy_from_slice(&(part.len() as u16).to_be_bytes());
                    p[24..].copy_from_slice(part);
                    au.send_tracked(p);
                    audio_seq = audio_seq.wrapping_add(1);
                }
                next_frame += Duration::from_millis(20);
            }
        } else { live_phase = 0.0; }
        au.tick(false);
        if last_audio_rx.elapsed() > Duration::from_secs(5) && sh.error.lock().unwrap().is_none() {
            fail("Kein Audio vom Funkgerät seit 5 s – WLAN-Verbindung prüfen");
        }
        if stopping { break; }
        std::thread::sleep(Duration::from_millis(2));
    }
    // sauber abmelden: CI-V schließen, Token zurückgeben, Kanäle trennen
    let mut p = civ.header(0x16, 0x00, 0);
    p[16] = 0xc0;
    p[17] = 0x01;
    p[19..21].copy_from_slice(&civ_seq.to_be_bytes());
    civ.send_tracked(p);
    let p = s.token_packet(&c, 0x01);
    c.send_tracked(p);
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_millis(300) { // Neuübertragungswünsche noch bedienen
        for st in [&mut c, &mut civ, &mut au] { while let Some(r) = st.recv() { st.handle_common(&r); } }
        std::thread::sleep(Duration::from_millis(5));
    }
    // Trennen übernimmt Drop der drei Kanäle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_ordnet_fordert_an_und_fuellt_luecken() {
        let t0 = Instant::now();
        let mut j = Jitter::new();
        let pk = |v: f32| vec![v; 4];
        let mut out = vec![];
        assert!(j.put(10, pk(1.0), t0).is_empty());
        assert_eq!(j.put(13, pk(4.0), t0), vec![11, 12]); // Lücke -> anfordern
        assert!(j.put(12, pk(3.0), t0).is_empty());       // vertauscht
        j.take(t0, &mut out);
        assert_eq!(out.len(), 1);                          // 11 fehlt noch: warten
        assert!(j.put(11, pk(2.0), t0).is_empty());       // Neuübertragung kommt
        assert!(j.put(11, pk(2.0), t0).is_empty());       // … und gleich noch einmal: darf die Markierung nicht löschen
        j.take(t0, &mut out);
        assert_eq!(out.iter().map(|o| o.0[0]).collect::<Vec<_>>(), vec![1.0, 2.0, 3.0, 4.0]);
        assert!(out[1].1 && out[2].1 && !out[3].1);         // 11 und 12 waren angefordert
        // 14 kommt nie: nach WAIT als Stille
        out.clear();
        j.put(15, pk(6.0), t0);
        j.take(t0 + WAIT, &mut out);
        assert_eq!(out.len(), 2);
        assert!(out[0].0.is_empty() && out[1].0[0] == 6.0);
        assert!(j.put(9, pk(0.0), t0).is_empty());        // alt: verworfen
        // Überlauf der Folgenummer
        let mut j = Jitter::new();
        j.put(65535, pk(1.0), t0);
        assert!(j.put(0, pk(2.0), t0).is_empty());
        out.clear(); j.take(t0, &mut out);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn passcode_wie_referenz() {
        // erste Zeichen: 'a' (97) -> 0x38, 'b'+1 (99) -> 0x2e, Rest 0
        assert_eq!(&passcode("ab")[..3], &[0x38, 0x2e, 0x00]);
        // Überlauf über 126 läuft ab 32 weiter: '~' (126) + 1 -> 32 + 127 % 127 = 32 -> 0x47
        assert_eq!(passcode(" ~")[1], 0x47);
        assert_eq!(passcode("12345678901234567890").len(), 16);
    }

    /// Ende-zu-Ende gegen tools/fake_ic705.py (läuft lokal auf 50001-50003):
    /// SSTV_FAKE_DIR=/ordner cargo test --release gegen_simuliertes -- --ignored --nocapture
    /// Im Ordner: rx.wav (Robot 36, 48 kHz), der Server schreibt tx.wav und report.json
    #[test]
    #[ignore]
    fn gegen_simuliertes_ic705() {
        use crate::{cat, sstv};
        let dir = std::path::PathBuf::from(std::env::var("SSTV_FAKE_DIR").unwrap());
        let pattern = crate::test_pattern(320, 240).into_raw();
        let mean_err = |a: &[u8]| a.iter().zip(&pattern).map(|(x, y)| (*x as f64 - *y as f64).abs()).sum::<f64>() / a.len() as f64;

        // 1. falsches Passwort
        let e = connect("127.0.0.1", "test", "falsch").err().expect("Anmeldung hätte scheitern müssen");
        println!("falsches Passwort: {e}");
        assert!(e.contains("Passwort"));

        // 2. Anmeldung und CI-V (das Gerät verwirft jedes 7. CI-V-Paket und fordert es neu an)
        let radio = connect("127.0.0.1", "test", "geheim").expect("Anmeldung");
        assert_eq!(radio.name, "IC-705");
        let model = cat::RIGS.iter().find(|r| r.name == "Icom IC-705").unwrap().clone();
        let mut rig = cat::Rig::open_net(radio.shared.clone(), model);
        rig.set_freq(14_230_000).unwrap();
        rig.set_mode(cat::Sb::Usb).unwrap();
        for i in 0..10 {
            assert_eq!(rig.read_freq().unwrap(), Some(14_230_000), "Lesen #{i}");
        }

        // 3. Empfang: Robot-36-Bild aus dem WLAN-Audio
        let (tx, rx) = mpsc::channel();
        radio.shared.set_audio_sink(Some(tx));
        let mut dec = sstv::Decoder::new(RATE as f64);
        let t0 = Instant::now();
        let mut got = None;
        while got.is_none() && t0.elapsed() < Duration::from_secs(90) {
            if let Ok(x) = rx.recv_timeout(Duration::from_millis(200)) {
                dec.push(&x);
                for e in dec.events.drain(..) {
                    if let sstv::Event::End { mode, img, complete } = e { got = Some((mode.name, img, complete)); }
                }
            }
        }
        radio.shared.set_audio_sink(None);
        let (mode, img, complete) = got.expect("kein Bild über WLAN empfangen");
        println!("Empfang: {mode}, komplett {complete}, mittlere Abweichung {:.2}", mean_err(&img.data));
        assert!(mode == "Robot 36" && complete && mean_err(&img.data) < 6.0);

        // 4. Senden: PTT an, Bild als Audio, PTT aus
        let m = sstv::MODES.iter().find(|m| m.name == "Robot 36").unwrap();
        let src = sstv::Img { w: m.w, h: m.h, data: pattern.clone() };
        let mut samples = vec![0f32; (RATE / 5) as usize];
        samples.extend(sstv::encode(m, &src, RATE as f64, 1.0));
        rig.ptt(true).unwrap();
        let st = radio.shared.play(samples, 0.5);
        let t0 = Instant::now();
        while !st.done.load(Relaxed) { std::thread::sleep(Duration::from_millis(50)); }
        println!("Senden dauerte {:.1} s für {:.1} s Audio", t0.elapsed().as_secs_f64(), st.len as f64 / RATE as f64);
        std::thread::sleep(Duration::from_millis(300));
        rig.ptt(false).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        drop(rig);
        drop(radio);
        std::thread::sleep(Duration::from_secs(2));

        // 5. Bericht des Funkgeräts und dessen Aufzeichnung prüfen
        let rep: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("report.json")).unwrap()).unwrap();
        println!("Bericht: {}", serde_json::to_string(&rep).unwrap().chars().take(600).collect::<String>());
        assert_eq!(rep["login"], "ok");
        assert_eq!(rep["civ_open"], false, "CI-V beim Trennen geschlossen");
        let ci = &rep["conninfo"];
        assert!(ci["guid_ok"].as_bool().unwrap() && ci["user_ok"].as_bool().unwrap());
        assert_eq!((ci["rxcodec"].as_u64(), ci["rxsample"].as_u64(), ci["civport"].as_u64(), ci["audioport"].as_u64()),
                   (Some(4), Some(48000), Some(50002), Some(50003)));
        assert!(rep["retransmit_requested"].as_u64().unwrap() > 0);
        assert_eq!(rep["retransmit_requested"], rep["retransmit_received"], "alle verworfenen Pakete neu gesendet");
        assert!(rep["ping_replies"].as_u64().unwrap() > 10);
        assert_eq!(rep["ptt"], false);
        assert!(rep["disconnects"].as_u64().unwrap() >= 3);
        let mut r = hound::WavReader::open(dir.join("tx.wav")).unwrap();
        let x: Vec<f32> = r.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();
        println!("Aufzeichnung des Funkgeräts: {:.1} s", x.len() as f64 / 48000.0);
        let mut dec = sstv::Decoder::new(48000.0);
        dec.push(&x);
        dec.push(&vec![0.0; 96000]);
        dec.abort();
        let out = dec.events.drain(..).find_map(|e| if let sstv::Event::End { img, complete, .. } = e { Some((img, complete)) } else { None })
            .expect("gesendetes Bild nicht dekodierbar");
        println!("Gesendet und gegendekodiert: komplett {}, mittlere Abweichung {:.2}", out.1, mean_err(&out.0.data));
        assert!(out.1 && mean_err(&out.0.data) < 6.0);
    }
}
