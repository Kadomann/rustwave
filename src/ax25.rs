//! Packet Radio: AX.25-Rahmen aller Arten (I, S, U), Verbindungsprotokoll AX.25 2.0 (modulo 8) für
//! Verbindungen zu Mailboxen, Knoten und anderen Stationen, und das 9600-Baud-Modem nach G3RUH.
//! 1200 Baud (Bell 202) und 300 Baud (Kurzwelle) kommen aus dem AFSK-Modem in `aprs`.

use crate::aprs;
use std::collections::VecDeque;
use std::f64::consts::TAU;

// ---------------- Rahmen ----------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Informationsrahmen mit Sendefolge- und Empfangsfolgenummer
    I { ns: u8, nr: u8 },
    Rr(u8),
    Rnr(u8),
    Rej(u8),
    Sabm,
    Sabme,
    Ua,
    Disc,
    Dm,
    Frmr,
    Ui,
    Other(u8),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub dst: String,
    pub src: String,
    /// Digipeater mit „schon wiederholt“-Bit
    pub digis: Vec<(String, bool)>,
    /// Some(true) = Kommando, Some(false) = Antwort, None = AX.25 1.0
    pub cmd: Option<bool>,
    pub kind: Kind,
    /// Poll-/Final-Bit
    pub pf: bool,
    pub pid: Option<u8>,
    pub info: Vec<u8>,
}

/// "DL1ABC-7" -> ("DL1ABC", 7)
pub fn split_call(c: &str) -> (String, u8) {
    let c = c.trim().trim_end_matches('*').to_uppercase();
    match c.split_once('-') {
        Some((a, s)) => (a.to_string(), s.parse().unwrap_or(0)),
        None => (c, 0),
    }
}
/// Rufzeichen vergleichbar machen ("dl1abc-0" == "DL1ABC")
pub fn norm_call(c: &str) -> String {
    let (call, ssid) = split_call(c);
    if ssid == 0 { call } else { format!("{call}-{ssid}") }
}
/// gültiges Rufzeichen für ein Adressfeld (1–6 Zeichen A–Z/0–9, SSID 0–15)
pub fn valid_call(c: &str) -> bool {
    let c = c.trim().to_uppercase();
    let (call, ssid) = match c.split_once('-') { Some((a, s)) => (a.to_string(), s.parse::<u8>().ok()), None => (c.clone(), Some(0)) };
    (1..=6).contains(&call.len()) && call.chars().all(|ch| ch.is_ascii_alphanumeric()) && ssid.is_some_and(|s| s <= 15)
}
fn addr(c: &str, last: bool, top: bool) -> [u8; 7] {
    let (call, ssid) = split_call(c);
    let mut a = [b' ' << 1; 7];
    for (i, ch) in call.bytes().take(6).enumerate() { a[i] = ch << 1; }
    a[6] = (top as u8) << 7 | 0x60 | ((ssid & 0x0f) << 1) | last as u8;
    a
}

impl Kind {
    fn control(self, pf: bool) -> u8 {
        let p = (pf as u8) << 4;
        match self {
            Kind::I { ns, nr } => (nr & 7) << 5 | p | (ns & 7) << 1,
            Kind::Rr(nr) => (nr & 7) << 5 | p | 0x01,
            Kind::Rnr(nr) => (nr & 7) << 5 | p | 0x05,
            Kind::Rej(nr) => (nr & 7) << 5 | p | 0x09,
            Kind::Sabm => 0x2F | p,
            Kind::Sabme => 0x6F | p,
            Kind::Ua => 0x63 | p,
            Kind::Disc => 0x43 | p,
            Kind::Dm => 0x0F | p,
            Kind::Frmr => 0x87 | p,
            Kind::Ui => 0x03 | p,
            Kind::Other(c) => c,
        }
    }
    fn from_control(c: u8) -> (Kind, bool) {
        let pf = c & 0x10 != 0;
        let nr = c >> 5;
        if c & 1 == 0 { return (Kind::I { ns: (c >> 1) & 7, nr }, pf); }
        if c & 3 == 1 {
            return (match (c >> 2) & 3 { 0 => Kind::Rr(nr), 1 => Kind::Rnr(nr), 2 => Kind::Rej(nr), _ => Kind::Other(c) }, pf);
        }
        (match c & 0xEF {
            0x2F => Kind::Sabm, 0x6F => Kind::Sabme, 0x63 => Kind::Ua, 0x43 => Kind::Disc, 0x0F => Kind::Dm,
            0x87 => Kind::Frmr, 0x03 => Kind::Ui, _ => Kind::Other(c),
        }, pf)
    }
    fn has_info(self) -> bool { matches!(self, Kind::I { .. } | Kind::Ui) }
}

impl Frame {
    /// Rahmen ohne Info von `src` an `dst` über `via`
    pub fn new(src: &str, dst: &str, via: &[String], cmd: bool, kind: Kind, pf: bool) -> Frame {
        Frame { dst: norm_call(dst), src: norm_call(src), digis: via.iter().map(|d| (norm_call(d), false)).collect(),
                cmd: Some(cmd), kind, pf, pid: kind.has_info().then_some(0xF0), info: vec![] }
    }
    /// Bytes mit FCS, ohne Flags
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = vec![];
        let (cd, cs) = match self.cmd { Some(true) => (true, false), Some(false) => (false, true), None => (false, false) };
        b.extend(addr(&self.dst, false, cd));
        b.extend(addr(&self.src, self.digis.is_empty(), cs));
        for (i, (d, h)) in self.digis.iter().enumerate() { b.extend(addr(d, i + 1 == self.digis.len(), *h)); }
        b.push(self.kind.control(self.pf));
        if self.kind.has_info() {
            b.push(self.pid.unwrap_or(0xF0));
            b.extend(&self.info);
        }
        let c = aprs::crc(&b);
        b.extend([c as u8, (c >> 8) as u8]);
        b
    }
    /// Rahmen aus Bytes mit FCS (Prüfsumme wird geprüft)
    pub fn from_bytes(b: &[u8]) -> Option<Frame> {
        if b.len() < 17 || aprs::crc(&b[..b.len() - 2]) != u16::from_le_bytes([b[b.len() - 2], b[b.len() - 1]]) { return None; }
        let b = &b[..b.len() - 2];
        let mut calls: Vec<(String, bool)> = vec![];
        let mut i = 0;
        loop {
            if i + 7 > b.len() || calls.len() > 10 { return None; }
            let a = &b[i..i + 7];
            if a[..6].iter().any(|&c| c & 1 != 0) { return None; }
            let call: String = a[..6].iter().map(|c| (c >> 1) as char).collect::<String>().trim().to_string();
            if call.is_empty() || !call.chars().all(|c| c.is_ascii_alphanumeric()) { return None; }
            let ssid = (a[6] >> 1) & 0x0f;
            calls.push((if ssid > 0 { format!("{call}-{ssid}") } else { call }, a[6] & 0x80 != 0));
            i += 7;
            if a[6] & 1 != 0 { break; }
        }
        if calls.len() < 2 || i >= b.len() { return None; }
        let (kind, pf) = Kind::from_control(b[i]);
        let (pid, info) = if kind.has_info() {
            if i + 1 >= b.len() { return None; }
            (Some(b[i + 1]), b[i + 2..].to_vec())
        } else { (None, b[i + 1..].to_vec()) };
        let cmd = match (calls[0].1, calls[1].1) { (true, false) => Some(true), (false, true) => Some(false), _ => None };
        Some(Frame { dst: calls[0].0.clone(), src: calls[1].0.clone(), digis: calls[2..].to_vec(), cmd, kind, pf, pid, info })
    }
    /// alle Digipeater haben den Rahmen schon wiederholt (bzw. es gibt keine)
    pub fn path_done(&self) -> bool { self.digis.iter().all(|d| d.1) }
    /// Kurzform des Steuerfelds, z. B. "I s2 r3 P", "RR r1 F", "SABM P"
    pub fn ctl_text(&self) -> String {
        let (name, n) = match self.kind {
            Kind::I { ns, nr } => (format!("I s{ns} r{nr}"), None),
            Kind::Rr(nr) => ("RR".into(), Some(nr)),
            Kind::Rnr(nr) => ("RNR".into(), Some(nr)),
            Kind::Rej(nr) => ("REJ".into(), Some(nr)),
            Kind::Sabm => ("SABM".into(), None), Kind::Sabme => ("SABME".into(), None), Kind::Ua => ("UA".into(), None),
            Kind::Disc => ("DISC".into(), None), Kind::Dm => ("DM".into(), None), Kind::Frmr => ("FRMR".into(), None),
            Kind::Ui => ("UI".into(), None), Kind::Other(c) => (format!("?{c:02X}"), None),
        };
        let mut s = name;
        if let Some(n) = n { s += &format!(" r{n}"); }
        if self.pf { s += if self.cmd == Some(false) { " F" } else { " P" }; }
        match self.cmd { Some(true) => s += " C", Some(false) => s += " R", None => {} }
        s
    }
    /// Monitorzeile wie bei einem TNC: SRC>DST,DIGI* [ctl]: Text
    pub fn monitor(&self) -> String {
        let mut s = format!("{}>{}", self.src, self.dst);
        for (d, h) in &self.digis { s += &format!(",{d}{}", if *h { "*" } else { "" }); }
        s += &format!(" [{}]", self.ctl_text());
        if self.kind.has_info() && !self.info.is_empty() { s += ": "; s += &text_of(&self.info).replace('\n', " ¶ "); }
        s
    }
}

/// Bytes als Text: UTF-8, sonst Latin-1; CR (Zeilenende im Packet Radio) wird zu \n
pub fn text_of(b: &[u8]) -> String {
    let t = match std::str::from_utf8(b) { Ok(s) => s.to_string(), Err(_) => b.iter().map(|&c| c as char).collect() };
    t.replace("\r\n", "\n").replace('\r', "\n").chars().filter(|&c| c == '\n' || c == '\t' || !c.is_control()).collect()
}

// ---------------- Verbindung (AX.25 2.0, modulo 8) ----------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { Disconnected, Connecting, Connected, Disconnecting }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkParams {
    /// höchstens so viele Bytes Info je Rahmen
    pub paclen: usize,
    /// höchstens so viele unbestätigte Rahmen (k)
    pub maxframe: usize,
    /// Wartezeit auf eine Antwort ohne Digipeater (s); je Digipeater kommt das Doppelte hinzu
    pub frack: f64,
    /// Wiederholungen, bevor die Verbindung als verloren gilt (N2)
    pub retries: u32,
    /// Bestätigung verzögern, um sie mit eigenen Daten zu verbinden (s)
    pub resp: f64,
    /// eingehende Verbindungen annehmen
    pub accept: bool,
}
impl Default for LinkParams {
    fn default() -> Self { LinkParams { paclen: 128, maxframe: 4, frack: 3.0, retries: 10, resp: 0.5, accept: true } }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LinkEv {
    Connected(String),
    /// Verbindung beendet: Grund
    Disconnected(String),
    Data(Vec<u8>),
}

/// eine Verbindung (wie ein TNC im Terminalbetrieb: eine Verbindung zur Zeit)
pub struct Link {
    pub me: String,
    pub peer: String,
    /// Digipeater zur Gegenstelle
    pub via: Vec<String>,
    pub state: State,
    pub p: LinkParams,
    vs: u8,
    vr: u8,
    va: u8,
    /// gesendete, noch unbestätigte Info-Felder (N(S), Info)
    unacked: VecDeque<(u8, Vec<u8>)>,
    sendq: VecDeque<u8>,
    t1: Option<f64>,
    retry: u32,
    ack_due: Option<f64>,
    rej_sent: bool,
    peer_busy: bool,
    out: Vec<Frame>,
    pub events: Vec<LinkEv>,
}

/// Abstand a..b modulo 8
fn d8(a: u8, b: u8) -> u8 { (b + 8 - a) & 7 }

impl Link {
    pub fn new(me: &str, p: LinkParams) -> Self {
        Link { me: norm_call(me), peer: String::new(), via: vec![], state: State::Disconnected, p, vs: 0, vr: 0, va: 0,
               unacked: VecDeque::new(), sendq: VecDeque::new(), t1: None, retry: 0, ack_due: None, rej_sent: false,
               peer_busy: false, out: vec![], events: vec![] }
    }
    fn t1_len(&self) -> f64 { self.p.frack * (1.0 + 2.0 * self.via.len() as f64) }
    fn arm(&mut self, now: f64) { self.t1 = Some(now + self.t1_len()); }
    fn reset_seq(&mut self) {
        self.vs = 0; self.vr = 0; self.va = 0;
        self.unacked.clear(); self.rej_sent = false; self.peer_busy = false; self.ack_due = None; self.retry = 0;
    }
    fn frame(&self, cmd: bool, kind: Kind, pf: bool) -> Frame { Frame::new(&self.me, &self.peer, &self.via, cmd, kind, pf) }
    fn ended(&mut self, why: &str) {
        self.state = State::Disconnected;
        self.t1 = None;
        self.sendq.clear();
        self.unacked.clear();
        self.ack_due = None;
        self.events.push(LinkEv::Disconnected(why.into()));
    }
    /// unbestätigte Daten und Sendepuffer (Bytes)
    pub fn pending(&self) -> usize { self.sendq.len() + self.unacked.iter().map(|u| u.1.len()).sum::<usize>() }
    /// Rahmen, die jetzt gesendet werden sollen
    pub fn take_out(&mut self) -> Vec<Frame> { std::mem::take(&mut self.out) }
    pub fn has_out(&self) -> bool { !self.out.is_empty() }
    /// Aussendung beendet: ab jetzt auf die Antwort warten (T1 neu starten)
    pub fn tx_done(&mut self, now: f64) { if self.t1.is_some() { self.arm(now); } }

    /// Verbindung aufbauen
    pub fn connect(&mut self, peer: &str, via: &[String], now: f64) {
        self.peer = norm_call(peer);
        self.via = via.iter().map(|d| norm_call(d)).collect();
        self.reset_seq();
        self.sendq.clear();
        self.state = State::Connecting;
        self.out.push(self.frame(true, Kind::Sabm, true));
        self.arm(now);
    }
    /// Verbindung abbauen (bei Connecting sofort aufgeben)
    pub fn disconnect(&mut self, now: f64) {
        match self.state {
            State::Connected => {
                self.state = State::Disconnecting;
                self.retry = 0;
                self.sendq.clear();
                self.out.push(self.frame(true, Kind::Disc, true));
                self.arm(now);
            }
            State::Connecting => self.ended("Verbindungsaufbau abgebrochen"),
            _ => {}
        }
    }
    /// Daten senden (werden in Rahmen zu höchstens PACLEN Bytes geteilt)
    pub fn send(&mut self, data: &[u8], now: f64) {
        if self.state != State::Connected { return; }
        self.sendq.extend(data);
        self.pump(now);
    }
    fn pump(&mut self, now: f64) {
        if self.state != State::Connected { return; }
        while !self.peer_busy && !self.sendq.is_empty() && self.unacked.len() < self.p.maxframe.clamp(1, 7) {
            let n = self.sendq.len().min(self.p.paclen.max(1));
            let info: Vec<u8> = self.sendq.drain(..n).collect();
            let mut f = self.frame(true, Kind::I { ns: self.vs, nr: self.vr }, false);
            f.info = info.clone();
            self.out.push(f);
            self.unacked.push_back((self.vs, info));
            self.vs = (self.vs + 1) & 7;
            self.ack_due = None; // Bestätigung steckt im I-Rahmen
            if self.t1.is_none() { self.arm(now); }
        }
    }
    /// Bestätigung N(R) verarbeiten; false = ungültig
    fn ack(&mut self, nr: u8, now: f64) -> bool {
        if d8(self.va, nr) > d8(self.va, self.vs) { return false; }
        let mut progressed = false;
        while self.va != nr {
            self.unacked.pop_front();
            self.va = (self.va + 1) & 7;
            progressed = true;
        }
        if self.unacked.is_empty() { if self.state == State::Connected { self.t1 = None; } self.retry = 0; }
        else if progressed { self.arm(now); self.retry = 0; }
        true
    }
    /// unbestätigte Rahmen ab N(R) erneut senden, den letzten mit Poll
    fn resend(&mut self, poll: bool, now: f64) {
        let n = self.unacked.len();
        let frames: Vec<Frame> = self.unacked.iter().enumerate().map(|(i, (ns, info))| {
            let mut f = self.frame(true, Kind::I { ns: *ns, nr: self.vr }, poll && i + 1 == n);
            f.info = info.clone();
            f
        }).collect();
        self.out.extend(frames);
        if n > 0 { self.ack_due = None; self.arm(now); }
    }

    /// empfangenen Rahmen verarbeiten (nur solche an mich, vollständig über alle Digipeater)
    pub fn receive(&mut self, f: &Frame, now: f64) {
        if norm_call(&f.dst) != self.me || !f.path_done() { return; }
        let from = norm_call(&f.src);
        let reply_via: Vec<String> = f.digis.iter().rev().map(|d| d.0.clone()).collect();
        let ours = from == self.peer && self.state != State::Disconnected;
        // fremde Station, während eine Verbindung läuft: ablehnen
        if !ours && self.state != State::Disconnected {
            if matches!(f.kind, Kind::Sabm | Kind::Sabme | Kind::I { .. } | Kind::Disc) {
                self.out.push(Frame::new(&self.me, &from, &reply_via, false, Kind::Dm, f.pf));
            }
            return;
        }
        match f.kind {
            Kind::Sabme => {
                // nur modulo 8: DM, dann versucht die Gegenstelle SABM
                self.out.push(Frame::new(&self.me, &from, &reply_via, false, Kind::Dm, f.pf));
            }
            Kind::Sabm => {
                if self.state == State::Disconnected && !self.p.accept {
                    self.out.push(Frame::new(&self.me, &from, &reply_via, false, Kind::Dm, f.pf));
                    return;
                }
                let fresh = self.state != State::Connected;
                if self.state == State::Disconnected { self.peer = from.clone(); self.via = reply_via; self.sendq.clear(); }
                self.reset_seq();
                self.t1 = None;
                self.out.push(self.frame(false, Kind::Ua, f.pf));
                self.state = State::Connected;
                if fresh { self.events.push(LinkEv::Connected(self.peer.clone())); }
            }
            Kind::Ua => match self.state {
                State::Connecting => {
                    self.state = State::Connected;
                    self.t1 = None;
                    self.retry = 0;
                    self.events.push(LinkEv::Connected(self.peer.clone()));
                    self.pump(now);
                }
                State::Disconnecting => self.ended("getrennt"),
                _ => {}
            },
            Kind::Dm => match self.state {
                State::Connecting => self.ended("Verbindung abgelehnt (DM) – Station besetzt oder nicht bereit"),
                State::Connected => self.ended("Gegenstelle hat die Verbindung beendet (DM)"),
                State::Disconnecting => self.ended("getrennt"),
                State::Disconnected => {}
            },
            Kind::Disc => {
                if self.state == State::Disconnected {
                    self.out.push(Frame::new(&self.me, &from, &reply_via, false, Kind::Dm, f.pf));
                } else {
                    self.out.push(self.frame(false, Kind::Ua, f.pf));
                    self.ended("Gegenstelle hat getrennt");
                }
            }
            Kind::Frmr => if self.state != State::Disconnected { self.ended("Protokollfehler (FRMR)") },
            Kind::I { ns, nr } => {
                if self.state == State::Disconnected {
                    self.out.push(Frame::new(&self.me, &from, &reply_via, false, Kind::Dm, f.pf));
                    return;
                }
                if self.state != State::Connected { return; }
                self.ack(nr, now);
                if ns == self.vr {
                    self.vr = (self.vr + 1) & 7;
                    self.rej_sent = false;
                    if !f.info.is_empty() { self.events.push(LinkEv::Data(f.info.clone())); }
                    if f.pf { self.out.push(self.frame(false, Kind::Rr(self.vr), true)); self.ack_due = None; }
                    else if self.ack_due.is_none() { self.ack_due = Some(now + self.p.resp); }
                } else if !self.rej_sent {
                    self.rej_sent = true;
                    self.out.push(self.frame(false, Kind::Rej(self.vr), f.pf));
                    self.ack_due = None;
                } else if f.pf {
                    self.out.push(self.frame(false, Kind::Rr(self.vr), true));
                }
                self.pump(now);
            }
            Kind::Rr(nr) | Kind::Rnr(nr) | Kind::Rej(nr) => {
                if self.state != State::Connected { return; }
                self.peer_busy = matches!(f.kind, Kind::Rnr(_));
                self.ack(nr, now);
                let is_cmd = f.cmd != Some(false);
                if is_cmd && f.pf {
                    self.out.push(self.frame(false, Kind::Rr(self.vr), true)); // wir sind nie belegt
                    self.ack_due = None;
                }
                // REJ oder Antwort auf unseren Poll: Fehlendes ab N(R) wiederholen
                if matches!(f.kind, Kind::Rej(_)) || (!is_cmd && f.pf) { self.resend(false, now); }
                self.pump(now);
            }
            Kind::Ui | Kind::Other(_) => {}
        }
    }

    /// Zeitgeber: verzögerte Bestätigung senden, Wiederholungen nach Ablauf von T1
    pub fn tick(&mut self, now: f64) {
        if let Some(t) = self.ack_due {
            if now >= t && self.state == State::Connected {
                self.out.push(self.frame(false, Kind::Rr(self.vr), false));
                self.ack_due = None;
            }
        }
        let Some(t) = self.t1 else { return };
        if now < t || self.has_out() { return; }
        self.retry += 1;
        if self.retry > self.p.retries {
            match self.state {
                State::Connecting => self.ended("keine Antwort – Station nicht erreichbar"),
                State::Connected => {
                    self.out.push(self.frame(true, Kind::Disc, false));
                    self.ended("Verbindung verloren (keine Antwort mehr)");
                }
                State::Disconnecting => self.ended("getrennt (ohne Bestätigung)"),
                State::Disconnected => self.t1 = None,
            }
            return;
        }
        match self.state {
            State::Connecting => { self.out.push(self.frame(true, Kind::Sabm, true)); self.arm(now); }
            State::Disconnecting => { self.out.push(self.frame(true, Kind::Disc, true)); self.arm(now); }
            State::Connected => {
                if self.unacked.is_empty() { self.out.push(self.frame(true, Kind::Rr(self.vr), true)); self.arm(now); }
                else { self.resend(true, now); }
            }
            State::Disconnected => self.t1 = None,
        }
    }
    /// Stand für die Anzeige: (V(S), V(R), unbestätigte Rahmen, Wiederholung)
    pub fn status(&self) -> (u8, u8, usize, u32) { (self.vs, self.vr, self.unacked.len(), self.retry) }
}

// ---------------- 9600 Baud nach G3RUH ----------------

pub const G3RUH_BAUD: f64 = 9600.0;

/// Basisband für den 9600-Bd-Datenanschluss: HDLC, NRZI, Scrambler 1 + x^12 + x^17, gefiltertes NRZ
pub fn modulate_9600(frames: &[Vec<u8>], fs: f64, txdelay_ms: f64) -> Vec<f32> {
    let bits = aprs::hdlc_bits(frames, G3RUH_BAUD, txdelay_ms);
    let (mut level, mut sr) = (false, 0u32);
    let line: Vec<bool> = bits.iter().map(|&b| {
        if b == 0 { level = !level; } // NRZI
        let y = (level as u32) ^ ((sr >> 11) & 1) ^ ((sr >> 16) & 1);
        sr = (sr << 1) | y;
        y == 1
    }).collect();
    let spb = fs / G3RUH_BAUD;
    let n = (line.len() as f64 * spb).ceil() as usize;
    let raw: Vec<f64> = (0..n).map(|i| if line[((i as f64 / spb) as usize).min(line.len() - 1)] { 1.0 } else { -1.0 }).collect();
    // Tiefpass (gefenstertes sinc, Grenze 0,6 × Baudrate) gegen Nachbarkanalstörungen
    let fc = 0.6 * G3RUH_BAUD / fs;
    let half = (2.0 * spb).ceil() as i64;
    let taps: Vec<f64> = (-half..=half).map(|k| {
        let x = k as f64;
        let sinc = if k == 0 { 2.0 * fc } else { (TAU * fc * x).sin() / (std::f64::consts::PI * x) };
        sinc * (0.54 + 0.46 * (std::f64::consts::PI * x / (half as f64 + 1.0)).cos())
    }).collect();
    let g: f64 = taps.iter().sum();
    (0..n).map(|i| {
        let mut a = 0.0;
        for (j, t) in taps.iter().enumerate() {
            let k = i as i64 + j as i64 - half;
            if k >= 0 && (k as usize) < n { a += t * raw[k as usize]; }
        }
        (0.8 * a / g) as f32
    }).collect()
}

/// ein Entscheider des 9600-Bd-Empfängers: Schwelle, Taktrückgewinnung, Entwürfeln, NRZI, HDLC
struct Slice9600 { thr: f64, phase: f64, last: bool, sr: u32, hdlc: aprs::Slicer }


/// Empfänger für 9600 Bd G3RUH: Tiefpass, Gleichanteil weg, mehrere Entscheider mit versetzten Schwellen
/// (wie Direwolf), je Entscheider eigene Taktrückgewinnung, Entwürfeln, NRZI und HDLC
pub struct Rx9600 {
    lp: [crate::sstv::Biquad; 2],
    dc: f64,
    dc_a: f64,
    amp: f64,
    amp_a: f64,
    prev: f64,
    step: f64,
    gain: f64,
    slices: Vec<Slice9600>,
    n: u64,
    fs: f64,
    recent: Vec<(u64, Vec<u8>)>,
}
/// Schwellen relativ zur Amplitude
const THRESH_9600: [f64; 5] = [-0.3, -0.15, 0.0, 0.15, 0.3];
impl Rx9600 {
    pub fn new(fs: f64) -> Self { Self::with(fs, LP_9600, PLL_9600) }
    fn with(fs: f64, lpf: f64, gain: f64) -> Self {
        use crate::sstv::Biquad;
        let fc = (lpf * G3RUH_BAUD).min(fs * 0.45);
        Rx9600 { lp: [Biquad::lowpass(fs, fc, 0.5412), Biquad::lowpass(fs, fc, 1.3066)], dc: 0.0, dc_a: 1.0 - (-TAU * 20.0 / fs).exp(),
                 amp: 1e-3, amp_a: 1.0 - (-TAU * 5.0 / fs).exp(), prev: 0.0, step: G3RUH_BAUD / fs, gain,
                 slices: THRESH_9600.iter().map(|&thr| Slice9600 { thr, phase: 0.0, last: false, sr: 0, hdlc: aprs::Slicer::new(1.0) }).collect(),
                 n: 0, fs, recent: vec![] }
    }
    pub fn push_raw(&mut self, x: &[f32], dedup_s: f64) -> Vec<Vec<u8>> {
        let mut out = vec![];
        for &v in x {
            self.dc += (v as f64 - self.dc) * self.dc_a;
            let y0 = self.lp[0].run(v as f64 - self.dc);
            let y = self.lp[1].run(y0);
            self.amp += (y.abs() - self.amp) * self.amp_a;
            let prev = self.prev;
            self.prev = y;
            for sl in &mut self.slices {
                let t = sl.thr * self.amp;
                let level = y > t;
                let before = sl.phase;
                sl.phase += self.step;
                if level != sl.last {
                    // Zeitpunkt des Nulldurchgangs zwischen den Samples (linear) – genauer bei nur 4–5 Samples je Bit
                    let frac = ((t - prev) / (y - prev)).clamp(0.0, 1.0);
                    let at = before + frac * self.step;
                    let at = if at >= 1.0 { at - 1.0 } else { at };
                    let e = if at < 0.5 { at } else { at - 1.0 };
                    sl.phase -= e * self.gain;
                    sl.last = level;
                }
                // in der Bitmitte abtasten (zwischen den Samples interpoliert)
                if before < 0.5 && sl.phase >= 0.5 {
                    let frac = ((0.5 - before) / (sl.phase - before)).clamp(0.0, 1.0);
                    let b = (prev + (y - prev) * frac > t) as u32;
                    let d = b ^ ((sl.sr >> 11) & 1) ^ ((sl.sr >> 16) & 1); // Entwürfeln
                    sl.sr = (sl.sr << 1) | b;
                    if let Some(f) = sl.hdlc.bit(d == 1) { out.push(f); }
                }
                if sl.phase >= 1.0 { sl.phase -= 1.0; }
            }
        }
        self.n += x.len() as u64;
        let (n, win) = (self.n, (dedup_s * self.fs) as u64);
        self.recent.retain(|(t, _)| n - t < win);
        out.into_iter().filter(|b| {
            if self.recent.iter().any(|(_, r)| r == b) { return false; }
            self.recent.push((n, b.clone()));
            true
        }).collect()
    }
    pub fn busy(&self) -> bool { self.slices.iter().any(|s| s.hdlc.carrier) }
}
const LP_9600: f64 = 0.65;
const PLL_9600: f64 = 0.1;

/// Modem für Packet Radio
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed { B1200, B300, B9600 }
impl Speed {
    pub const ALL: [(Speed, &'static str, &'static str); 3] = [
        (Speed::B1200, "1200 Bd FM", "AFSK 1200/2200 Hz (Bell 202) – der Standard auf 2 m und 70 cm"),
        (Speed::B300, "300 Bd KW", "AFSK 1600/1800 Hz in USB/LSB, z. B. Net105 auf 14,105 MHz LSB"),
        (Speed::B9600, "9600 Bd G3RUH", "Basisband über den 9600-Bd-Datenanschluss (FM), oft an Benutzereinstiegen auf 70 cm"),
    ];
    pub fn label(self) -> &'static str { Self::ALL.iter().find(|s| s.0 == self).map_or("", |s| s.1) }
    pub fn key(self) -> &'static str { match self { Speed::B1200 => "1200", Speed::B300 => "300", Speed::B9600 => "9600" } }
    pub fn from_key(k: &str) -> Option<Speed> { Self::ALL.iter().map(|s| s.0).find(|s| s.key() == k) }
}

/// Rahmen als Audio für die gewählte Geschwindigkeit
pub fn modulate(frames: &[Vec<u8>], fs: f64, txdelay_ms: f64, s: Speed) -> Vec<f32> {
    match s {
        Speed::B1200 => aprs::modulate_raw(frames, fs, txdelay_ms, aprs::VHF),
        Speed::B300 => aprs::modulate_raw(frames, fs, txdelay_ms, aprs::HF),
        Speed::B9600 => modulate_9600(frames, fs, txdelay_ms),
    }
}

/// Empfänger für die gewählte Geschwindigkeit
pub enum Receiver { Afsk(aprs::Receiver), G3ruh(Rx9600) }
impl Receiver {
    pub fn new(fs: f64, s: Speed) -> Self {
        match s {
            Speed::B1200 => Receiver::Afsk(aprs::Receiver::new(fs, aprs::VHF)),
            Speed::B300 => Receiver::Afsk(aprs::Receiver::new(fs, aprs::HF)),
            Speed::B9600 => Receiver::G3ruh(Rx9600::new(fs)),
        }
    }
    /// Rahmen mit gültiger Prüfsumme; Wiederholungen derselben Bytes erst nach 0,5 s wieder (z. B. erneut gesendete Rahmen)
    pub fn push(&mut self, x: &[f32]) -> Vec<Vec<u8>> {
        match self { Receiver::Afsk(r) => r.push_raw(x, 0.5), Receiver::G3ruh(r) => r.push_raw(x, 0.5) }
    }
    pub fn busy(&self) -> bool { match self { Receiver::Afsk(r) => r.busy(), Receiver::G3ruh(r) => r.busy() } }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn u(&mut self) -> f64 { self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (self.0 >> 11) as f64 / (1u64 << 53) as f64 }
        fn gauss(&mut self) -> f64 { (-2.0 * (self.u() + 1e-12).ln()).sqrt() * (TAU * self.u()).cos() }
    }

    fn sample_frames() -> Vec<Frame> {
        let via = vec!["DB0AAA".to_string(), "DB0BBB-2".to_string()];
        let mut v = vec![];
        for (cmd, kind, pf) in [(true, Kind::Sabm, true), (false, Kind::Ua, true), (true, Kind::I { ns: 5, nr: 3 }, false),
                                (false, Kind::Rr(6), true), (false, Kind::Rej(2), false), (false, Kind::Rnr(7), false),
                                (true, Kind::Disc, true), (false, Kind::Dm, false), (true, Kind::Ui, false)] {
            let mut f = Frame::new("DL1ABC-7", "DB0XYZ", &via, cmd, kind, pf);
            if kind.has_info() { f.info = b"Hallo Welt\r".to_vec(); }
            v.push(f);
        }
        v
    }

    #[test]
    fn rahmen_hin_und_zurueck() {
        for f in sample_frames() {
            let b = f.to_bytes();
            assert_eq!(Frame::from_bytes(&b).as_ref(), Some(&f), "{}", f.monitor());
        }
        let mut b = sample_frames()[2].to_bytes();
        b[10] ^= 1;
        assert!(Frame::from_bytes(&b).is_none(), "Prüfsumme");
        assert_eq!(sample_frames()[3].monitor(), "DL1ABC-7>DB0XYZ,DB0AAA,DB0BBB-2 [RR r6 F R]");
        assert_eq!(sample_frames()[2].monitor(), "DL1ABC-7>DB0XYZ,DB0AAA,DB0BBB-2 [I s5 r3 C]: Hallo Welt ¶ ");
    }

    /// zwei Stationen über einen Kanal, der Rahmen verliert: Daten kommen vollständig und in Reihenfolge an
    fn run_link(loss: f64, seed: u64, maxframe: usize) {
        let p = LinkParams { paclen: 64, maxframe, frack: 3.0, retries: 30, resp: 0.4, accept: true };
        let (mut a, mut b) = (Link::new("DL1ABC", p), Link::new("DB0XYZ-1", p));
        let mut rng = Rng(seed);
        let text_a: Vec<u8> = (0..1500).map(|i| b'A' + (i % 26) as u8).collect();
        let text_b: Vec<u8> = (0..900).map(|i| b'0' + (i % 10) as u8).collect();
        let (mut got_a, mut got_b) = (vec![], vec![]);
        let mut t = 0.0;
        a.connect("DB0XYZ-1", &[], t);
        let (mut sent, mut closing) = (false, false);
        while t < 2000.0 {
            // Kanal: was einer sendet, hört der andere (mit Verlust); dann je 0,1 s Zeit
            for dir in 0..2 {
                let (x, y) = if dir == 0 { (&mut a, &mut b) } else { (&mut b, &mut a) };
                let fr = x.take_out();
                if !fr.is_empty() {
                    for f in fr {
                        let bytes = f.to_bytes();
                        if rng.u() >= loss { y.receive(&Frame::from_bytes(&bytes).unwrap(), t); }
                    }
                    x.tx_done(t);
                }
            }
            for e in a.events.drain(..) { if let LinkEv::Data(d) = e { got_a.extend(d); } }
            for e in b.events.drain(..) { if let LinkEv::Data(d) = e { got_b.extend(d); } }
            if a.state == State::Connected && b.state == State::Connected && !sent {
                a.send(&text_a, t); b.send(&text_b, t); sent = true;
            }
            if sent && !closing && got_b.len() == text_a.len() && got_a.len() == text_b.len() && a.pending() == 0 && b.pending() == 0 {
                a.disconnect(t); closing = true;
            }
            if closing && a.state == State::Disconnected && b.state == State::Disconnected { break; }
            t += 0.1;
            a.tick(t); b.tick(t);
        }
        assert_eq!(got_b, text_a, "A→B, Verlust {loss}");
        assert_eq!(got_a, text_b, "B→A, Verlust {loss}");
        assert_eq!((a.state, b.state), (State::Disconnected, State::Disconnected), "Abbau, Verlust {loss}");
    }

    #[test]
    fn verbindung_ohne_und_mit_verlusten() {
        for (loss, seed, k) in [(0.0, 1, 4), (0.1, 2, 4), (0.3, 3, 4), (0.3, 4, 1), (0.4, 5, 7)] { run_link(loss, seed, k); }
    }

    #[test]
    fn abgelehnt_und_unerreichbar() {
        let p = LinkParams { retries: 3, ..LinkParams::default() };
        let mut a = Link::new("DL1ABC", p);
        let mut b = Link::new("DB0XYZ", LinkParams { accept: false, ..p });
        a.connect("DB0XYZ", &[], 0.0);
        for f in a.take_out() { b.receive(&f, 0.0); }
        for f in b.take_out() { assert_eq!(f.kind, Kind::Dm); a.receive(&f, 0.1); }
        assert!(matches!(a.events.last(), Some(LinkEv::Disconnected(w)) if w.contains("abgelehnt")));
        // niemand antwortet: nach N2 Wiederholungen aufgeben
        let mut c = Link::new("DL1ABC", p);
        c.connect("DL9XX", &["DB0AAA".into()], 0.0);
        let mut t = 0.0;
        let mut sabm = 0;
        while c.state != State::Disconnected && t < 200.0 {
            let n = c.take_out().len();
            if n > 0 { sabm += n; c.tx_done(t); }
            t += 0.5;
            c.tick(t);
        }
        assert_eq!(sabm, 4);
        assert!(t >= 4.0 * 9.0, "T1 mit Digipeater auf 9 s verlängert: {t}");
    }

    #[test]
    fn digipeater_pfad() {
        let mut b = Link::new("DB0XYZ", LinkParams::default());
        // über DB0AAA gehört, aber noch nicht wiederholt: nicht für mich
        let mut f = Frame::new("DL1ABC", "DB0XYZ", &["DB0AAA".into(), "DB0BBB".into()], true, Kind::Sabm, true);
        b.receive(&f, 0.0);
        assert!(b.take_out().is_empty());
        f.digis.iter_mut().for_each(|d| d.1 = true);
        b.receive(&f, 0.0);
        let ua = b.take_out();
        assert_eq!(ua[0].kind, Kind::Ua);
        assert_eq!(ua[0].digis, vec![("DB0BBB".into(), false), ("DB0AAA".into(), false)], "Rückweg umgekehrt");
        assert_eq!(b.state, State::Connected);
    }

    #[test]
    fn modem_9600_rundlauf() {
        for &fs in &[48000.0, 44100.0] {
            let frames: Vec<Vec<u8>> = sample_frames().iter().map(|f| f.to_bytes()).collect();
            let x = modulate_9600(&frames, fs, 100.0);
            let mut r = Rng(9);
            let y: Vec<f32> = std::iter::repeat_n(0.0, 2000).chain(x).chain(std::iter::repeat_n(0.0, 2000))
                .map(|v| 0.4 * v + (0.06 * r.gauss()) as f32).collect();
            let mut rx = Rx9600::new(fs);
            let mut got = vec![];
            for c in y.chunks(1024) { got.extend(rx.push_raw(c, 0.0)); }
            // gleiche Bytes kommen nur einmal je Aussendung; alle Rahmen müssen da sein
            for f in &frames { assert!(got.contains(f), "{fs} Hz: {:?}", Frame::from_bytes(f).map(|f| f.monitor())); }
        }
    }

    #[test]
    fn afsk_kurze_rahmen() {
        // RR (17 Bytes, ohne PID) muss durch den AFSK-Weg gehen
        for s in [Speed::B1200, Speed::B300] {
            let frames: Vec<Vec<u8>> = sample_frames().iter().map(|f| f.to_bytes()).collect();
            let x = modulate(&frames, 48000.0, 200.0, s);
            let mut rx = Receiver::new(48000.0, s);
            let got: Vec<Vec<u8>> = x.chunks(4800).flat_map(|c| rx.push(c)).collect();
            for f in &frames { assert!(got.contains(f), "{s:?}: {:?}", Frame::from_bytes(f).map(|f| f.monitor())); }
        }
    }

    /// Verbindung über das echte Modem (Audio mit Rauschen) in allen drei Geschwindigkeiten
    #[test]
    fn verbindung_ueber_modem() {
        let fs = 48000.0;
        for sp in [Speed::B1200, Speed::B9600, Speed::B300] {
            let p = LinkParams { paclen: if sp == Speed::B300 { 40 } else { 128 }, maxframe: 2, ..LinkParams::default() };
            let (mut a, mut b) = (Link::new("DL1ABC", p), Link::new("DB0XYZ", p));
            let (mut ra, mut rb) = (Receiver::new(fs, sp), Receiver::new(fs, sp));
            let mut r = Rng(5);
            let text: Vec<u8> = b"Packet Radio mit Rustwave: Verbindung, Daten, Bestaetigung und Abbau.\r".repeat(3);
            let mut got = vec![];
            let mut t = 0.0;
            a.connect("DB0XYZ", &[], t);
            let mut sent = false;
            while t < 600.0 && !(sent && a.state == State::Disconnected && b.state == State::Disconnected) {
                for dir in 0..2 {
                    let (x, rx, y) = if dir == 0 { (&mut a, &mut rb, &mut b) } else { (&mut b, &mut ra, &mut a) };
                    let fr: Vec<Vec<u8>> = x.take_out().iter().map(|f| f.to_bytes()).collect();
                    if fr.is_empty() { continue; }
                    let audio = modulate(&fr, fs, 150.0, sp);
                    t += audio.len() as f64 / fs;
                    let noisy: Vec<f32> = audio.iter().chain(std::iter::repeat_n(&0.0, 4800)).map(|v| 0.5 * v + (0.05 * r.gauss()) as f32).collect();
                    for c in noisy.chunks(2048) { for bytes in rx.push(c) { y.receive(&Frame::from_bytes(&bytes).unwrap(), t); } }
                    x.tx_done(t);
                }
                for e in b.events.drain(..) { if let LinkEv::Data(d) = e { got.extend(d); } }
                a.events.clear();
                if a.state == State::Connected && !sent { a.send(&text, t); sent = true; }
                if sent && got.len() == text.len() && a.pending() == 0 && a.state == State::Connected { a.disconnect(t); }
                t += 0.05;
                a.tick(t); b.tick(t);
            }
            assert_eq!(got, text, "{sp:?}");
            assert_eq!((a.state, b.state), (State::Disconnected, State::Disconnected), "{sp:?}");
            println!("{sp:?}: {} Bytes übertragen, fertig nach {t:.1} s", got.len());
        }
    }

    /// Vergleich mit Direwolf: PACKET_WAV=/tmp/pk.wav PACKET_SPEED=9600 cargo test --release export_packet -- --ignored
    /// atest -B 9600 /tmp/pk.wav
    #[test]
    #[ignore]
    fn export_packet() {
        let s = Speed::from_key(&std::env::var("PACKET_SPEED").unwrap_or("1200".into())).unwrap();
        let frames: Vec<Vec<u8>> = sample_frames().iter().map(|f| f.to_bytes()).collect();
        let mut w = hound::WavWriter::create(std::env::var("PACKET_WAV").unwrap(), hound::WavSpec { channels: 1, sample_rate: 48000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int }).unwrap();
        for f in &frames {
            for v in std::iter::repeat_n(0.0, 4800).chain(modulate(std::slice::from_ref(f), 48000.0, 200.0, s)) { w.write_sample((v * 16000.0) as i16).unwrap(); }
        }
        w.finalize().unwrap();
    }

    /// Einstellungen des 9600-Empfängers durchprobieren: PACKET_IN=/tmp/gp9600.wav cargo test --release tune_9600 -- --ignored --nocapture
    #[test]
    #[ignore]
    fn tune_9600() {
        let mut rd = hound::WavReader::open(std::env::var("PACKET_IN").unwrap()).unwrap();
        let fs = rd.spec().sample_rate as f64;
        let x: Vec<f32> = rd.samples::<i16>().map(|v| v.unwrap() as f32 / 32768.0).collect();
        for lpf in [0.45, 0.5, 0.6, 0.7, 0.8] {
            let mut line = format!("TUNE lp {lpf}:");
            for gain in [0.1, 0.2, 0.3, 0.45] {
                let mut rx = Rx9600::with(fs, lpf, gain);
                let n: usize = x.chunks(4096).map(|c| rx.push_raw(c, 0.5).len()).sum();
                line += &format!("  pll {gain}: {n}");
            }
            println!("{line}");
        }
    }

    /// Fremde Aufnahme dekodieren: PACKET_IN=/tmp/x.wav PACKET_SPEED=9600 cargo test --release import_packet -- --ignored --nocapture
    #[test]
    #[ignore]
    fn import_packet() {
        let s = Speed::from_key(&std::env::var("PACKET_SPEED").unwrap_or("1200".into())).unwrap();
        let mut rd = hound::WavReader::open(std::env::var("PACKET_IN").unwrap()).unwrap();
        let fs = rd.spec().sample_rate as f64;
        let ch = rd.spec().channels as usize;
        let x: Vec<f32> = rd.samples::<i16>().step_by(ch).map(|v| v.unwrap() as f32 / 32768.0).collect();
        let mut rx = Receiver::new(fs, s);
        let mut n = 0;
        for c in x.chunks(4096) { for b in rx.push(c) { n += 1; println!("RW {}", Frame::from_bytes(&b).map_or("?".into(), |f| f.monitor())); } }
        println!("RW-ANZAHL {n}");
    }
}
