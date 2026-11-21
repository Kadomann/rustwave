//! POCSAG (Funkruf, z. B. DAPNET): Dekoder und Kodierer.
//!
//! Der Dekoder folgt multimon-ng (pocsag.c): gleiche Zustandsmaschine (Sync, Adresse, Nachricht),
//! BCH(31,21)-Korrektur bis 2 Bit, automatische Polarität und dieselben Ausgabe-Modi
//! (Standard, numerisch, alphanumerisch, Skyper, automatisch) samt Zeichensätzen (US, DE, FR, DK, SE, SI).
//! Eingang ist das FM-demodulierte NF-Signal (am besten vom Daten-/9600-Anschluss, ohne Deemphasis).

use std::f64::consts::TAU;
use std::collections::VecDeque;
use std::sync::OnceLock;

pub const SYNC: u32 = 0x7CD2_15D8;
pub const IDLE: u32 = 0x7A89_C197;
const MSG_FLAG: u32 = 0x8000_0000;
/// Baudraten wie die multimon-ng-Dekoder POCSAG512, POCSAG1200, POCSAG2400
pub const BAUDS: [u32; 3] = [512, 1200, 2400];
/// Länge der Präambel (Bits 1010…), wie im Standard und in multimon-ng gen_pocsag
const PREAMBLE_BITS: usize = 576;
/// Bitfehler, die beim Einrasten auf das Sync-Wort zugelassen sind (multimon-ng: wie -b, also bis 2;
/// hier höchstens 1 – gleiche Empfindlichkeit, aber weniger Fehlstarts auf Rauschen)
const SYNC_ERR: u32 = 1;
/// Puffer für Nachrichteninhalt wie multimon-ng (512 Byte = 1024 Halbbytes)
const BUF: usize = 512;

/// Ausgabe-Modus (multimon-ng `-f`): Standard wertet die Funktionsbits aus
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format { Standard, Auto, Numeric, Alpha, Skyper }
impl Format {
    pub const ALL: [(Format, &'static str, &'static str); 5] = [
        (Format::Standard, "Standard", "nach Funktionsbits: Funktion 0 numerisch, 1–3 alphanumerisch (Vorgabe von multimon-ng)"),
        (Format::Auto, "Automatisch", "-f auto: rät anhand des Inhalts, ob numerisch, alphanumerisch oder Skyper; bei Unsicherheit alles"),
        (Format::Numeric, "Numerisch", "-f numeric: immer als Ziffern (0–9, Leerzeichen, U, -, ., [ ])"),
        (Format::Alpha, "Alphanumerisch", "-f alpha: immer als Text (7-Bit-ASCII)"),
        (Format::Skyper, "Skyper", "-f skyper: Text, um eins verschoben (Skyper-Dienst)"),
    ];
    pub fn key(self) -> &'static str { match self { Format::Standard => "std", Format::Auto => "auto", Format::Numeric => "num", Format::Alpha => "alpha", Format::Skyper => "skyper" } }
    pub fn from_key(s: &str) -> Option<Format> { Self::ALL.iter().map(|f| f.0).find(|f| f.key() == s) }
    pub fn label(self) -> &'static str { Self::ALL.iter().find(|f| f.0 == self).map_or("", |f| f.1) }
}

/// Polarität (multimon-ng `-P`): automatisch probiert beide Lagen beim Sync
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Polarity { Auto, Normal, Inverted }
impl Polarity {
    pub const ALL: [(Polarity, &'static str); 3] = [(Polarity::Auto, "automatisch"), (Polarity::Normal, "normal"), (Polarity::Inverted, "invertiert")];
    pub fn label(self) -> &'static str { Self::ALL.iter().find(|p| p.0 == self).map_or("", |p| p.1) }
}

/// Zeichensatz für alphanumerische Nachrichten (multimon-ng `-C`): nationale Varianten von ISO 646
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Charset { Us, De, Fr, Dk, Se, Si }
impl Charset {
    pub const ALL: [(Charset, &'static str); 6] = [(Charset::Us, "US"), (Charset::De, "DE"), (Charset::Fr, "FR"), (Charset::Dk, "DK"), (Charset::Se, "SE"), (Charset::Si, "SI")];
    pub fn label(self) -> &'static str { Self::ALL.iter().find(|c| c.0 == self).map_or("US", |c| c.1) }
}

/// Voreinstellung: DAPNET-Rubriken für Skyper-Empfänger
pub const SKYPER_RICS: [u32; 4] = [4520, 0, 0, 0];

/// Einstellungen des Dekoders – entsprechen den POCSAG-Schaltern von multimon-ng
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Params {
    /// aktive Dekoder: POCSAG512, POCSAG1200, POCSAG2400
    pub rates: [bool; 3],
    pub format: Format,
    pub charset: Charset,
    /// BCH-Fehlerkorrektur in Bit je Codewort (0–2, `-b`)
    pub ecc: u8,
    pub polarity: Polarity,
    /// leere Nachrichten (nur Adresse, „Nur-Ton“) ausblenden (`-e`)
    pub prune_empty: bool,
    /// unwahrscheinliche Inhalte verwerfen (`-u`)
    pub heuristic: bool,
    /// teilweise empfangene Nachrichten zeigen (`-p`)
    pub partial: bool,
    /// RICs, deren Text im Standard-Format als Skyper (um eins verschoben) gelesen wird; 0 = frei.
    /// DAPNET verschlüsselt nur die Skyper-Rubriken auf RIC 4520 (0x11A8) so.
    pub skyper: [u32; 4],
}
impl Default for Params {
    fn default() -> Self {
        Params { rates: [true; 3], format: Format::Standard, charset: Charset::Us, ecc: 2, polarity: Polarity::Auto,
                 prune_empty: false, heuristic: false, partial: false, skyper: SKYPER_RICS }
    }
}

/// Art der Nachricht
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind { Tone, Numeric, Alpha, Skyper }
impl Kind {
    pub fn label(self) -> &'static str { match self { Kind::Tone => "Ton", Kind::Numeric => "Numerisch", Kind::Alpha => "Alpha", Kind::Skyper => "Skyper" } }
}

/// eine dekodierte Nachricht (eine Ausgabezeile von multimon-ng)
#[derive(Clone, Debug, PartialEq)]
pub struct Msg {
    pub baud: u32,
    /// RIC (Adresse); None = Teilnachricht ohne Adresswort
    pub addr: Option<u32>,
    /// Funktionsbits 0–3 (oft als A–D bezeichnet)
    pub func: Option<u8>,
    pub kind: Kind,
    pub text: String,
    /// bis zum Ende im Sync (sonst Teilnachricht)
    pub complete: bool,
    /// intern: korrigierte Wörter und Empfangsvariante (für die Auswahl unter den Varianten)
    fixed: u32,
    variant: usize,
    /// intern: Anfang (Adresswort) und Ende der Nachricht in Samples
    t0: u64,
    t1: u64,
}
impl Msg {
    /// Zahl der korrigierten Codewörter (0 = fehlerfrei empfangen)
    pub fn corrected(&self) -> u32 { self.fixed }
    /// selbst gesendeter Ruf (für die Anzeige in der Liste)
    pub fn sent(baud: u32, addr: u32, func: u8, kind: Kind, text: String) -> Msg {
        Msg { baud, addr: Some(addr), func: Some(func), kind, text, complete: true, fixed: 0, variant: 0, t0: 0, t1: 0 }
    }
    /// Ausgabezeile wie multimon-ng
    pub fn multimon_line(&self) -> String {
        let head = match (self.addr, self.func) {
            (Some(a), Some(f)) => format!("POCSAG{}: Address: {a:7}  Function: {f:1}", self.baud),
            _ => format!("POCSAG{}: Address:       -  Function: -", self.baud),
        };
        let lost = if self.complete { "" } else { " <LOST SYNC>" };
        match self.kind {
            Kind::Tone => format!("{head} {lost}"),
            Kind::Numeric => format!("{head}  Numeric: {}{lost}", self.text),
            Kind::Alpha => format!("{head}  Alpha:   {}{lost}", self.text),
            Kind::Skyper => format!("{head}  Skyper:  {}{lost}", self.text),
        }
    }
    /// Text ohne die Füllzeichen am Ende (<NUL>, <EOT>, <ETX> …), für die Anzeige
    /// Skyper-Rubriken: die ersten zwei Zeichen sind ein Kopf (Rubrik/Platz o. Ä., von DAPNET nicht dokumentiert)
    pub fn skyper_split(&self) -> Option<(String, String)> {
        if self.kind != Kind::Skyper { return None; }
        let t = self.clean_text();
        let n = t.char_indices().nth(2).map(|(i, _)| i)?;
        // Steuerzeichen als <..> gehören nicht in den Kopf
        if t[..n].contains('<') { return None; }
        Some((t[..n].to_string(), t[n..].to_string()))
    }
    pub fn clean_text(&self) -> String {
        let mut t = self.text.trim_end().to_string();
        loop {
            let before = t.len();
            for tag in ["<NUL>", "<EOT>", "<ETX>", "<ETB>", "<DEL>"] {
                if let Some(s) = t.strip_suffix(tag) { t = s.trim_end().to_string(); }
            }
            if t.len() == before { return t; }
        }
    }
}

// ---------------- BCH(31,21) mit Paritätsbit ----------------

/// Generatorpolynom x^10+x^9+x^8+x^6+x^5+x^3+1
const GEN: u32 = 0x769;
fn bch_rem(v: u32) -> u32 {
    let mut r = v & 0x7FFF_FFFF;
    for i in (10..31).rev() { if r & (1 << i) != 0 { r ^= GEN << (i - 10); } }
    r & 0x3FF
}
/// Codewort aus 21 Datenbits (Bit 20 = Nachrichtenkennung): Daten | 10 BCH-Bits | gerade Parität
pub fn encode_cw(data21: u32) -> u32 {
    let d = data21 & 0x1F_FFFF;
    let cw = (d << 11) | (bch_rem(d << 10) << 1);
    cw | (cw.count_ones() & 1)
}
/// 11-Bit-Syndrom: BCH-Rest über Bit 31..1 und Gesamtparität (wie multimon-ng bch.c)
fn syndrome(cw: u32) -> u32 { bch_rem(cw >> 1) | ((cw.count_ones() & 1) << 10) }
fn err_table() -> &'static [u32] {
    static T: OnceLock<Vec<u32>> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = vec![0u32; 2048];
        for a in 0..32 {
            let e = 1u32 << a;
            t[syndrome(e) as usize] = e;
        }
        for a in 0..32 {
            for b in a + 1..32 {
                let e = (1u32 << a) | (1u32 << b);
                let s = syndrome(e) as usize;
                if t[s] == 0 { t[s] = e; }
            }
        }
        t
    })
}
/// Fehler korrigieren: Some(Anzahl korrigierter Bits), None = nicht korrigierbar
pub fn correct(cw: &mut u32) -> Option<u32> {
    let s = syndrome(*cw);
    if s == 0 { return Some(0); }
    let e = err_table()[s as usize];
    if e == 0 { return None; }
    *cw ^= e;
    Some(e.count_ones())
}

// ---------------- Zeichen ----------------

const NUM_TABLE: &[u8; 16] = b"084 2.6]195-3U7[";

fn ctrl_name(c: u8) -> &'static str {
    const N: [&str; 32] = ["<NUL>", "<SOH>", "<STX>", "<ETX>", "<EOT>", "<ENQ>", "<ACK>", "<BEL>", "<BS>", "<HT>", "<LF>", "<VT>", "<FF>", "<CR>", "<SO>", "<SI>",
        "<DLE>", "<DC1>", "<DC2>", "<DC3>", "<DC4>", "<NAK>", "<SYN>", "<ETB>", "<CAN>", "<EM>", "<SUB>", "<ESC>", "<FS>", "<GS>", "<RS>", "<US>"];
    if c == 0x7F { "<DEL>" } else { N[c as usize & 31] }
}
/// 7-Bit-Zeichen in Text (nationale Varianten wie multimon-ng mit UTF-8)
fn translate(c: u8, cs: Charset) -> String {
    let c = c & 0x7F;
    let nat: Option<&str> = match (cs, c) {
        (Charset::De, 0x5B) => Some("Ä"), (Charset::De, 0x5C) => Some("Ö"), (Charset::De, 0x5D) => Some("Ü"),
        (Charset::De, 0x7B) => Some("ä"), (Charset::De, 0x7C) => Some("ö"), (Charset::De, 0x7D) => Some("ü"), (Charset::De, 0x7E) => Some("ß"),
        (Charset::Dk, 0x5B) => Some("Æ"), (Charset::Dk, 0x5C) => Some("Ø"), (Charset::Dk, 0x5D) => Some("Å"),
        (Charset::Dk, 0x7B) => Some("æ"), (Charset::Dk, 0x7C) => Some("ø"), (Charset::Dk, 0x7D) => Some("å"),
        (Charset::Se, 0x5B) => Some("Ä"), (Charset::Se, 0x5C) => Some("Ö"), (Charset::Se, 0x5D) => Some("Å"),
        (Charset::Se, 0x7B) => Some("ä"), (Charset::Se, 0x7C) => Some("ö"), (Charset::Se, 0x7D) => Some("å"),
        (Charset::Fr, 0x24) => Some("£"), (Charset::Fr, 0x40) => Some("à"), (Charset::Fr, 0x5B) => Some("°"), (Charset::Fr, 0x5C) => Some("ç"),
        (Charset::Fr, 0x5D) => Some("§"), (Charset::Fr, 0x60) => Some("µ"), (Charset::Fr, 0x7B) => Some("é"), (Charset::Fr, 0x7C) => Some("ù"),
        (Charset::Fr, 0x7D) => Some("è"), (Charset::Fr, 0x7E) => Some("¨"),
        (Charset::Si, 0x40) => Some("Ž"), (Charset::Si, 0x5B) => Some("Š"), (Charset::Si, 0x5C) => Some("Đ"), (Charset::Si, 0x5D) => Some("Ć"),
        (Charset::Si, 0x5E) => Some("Č"), (Charset::Si, 0x60) => Some("ž"), (Charset::Si, 0x7B) => Some("š"), (Charset::Si, 0x7C) => Some("đ"),
        (Charset::Si, 0x7D) => Some("ć"), (Charset::Si, 0x7E) => Some("č"),
        _ => None,
    };
    if let Some(s) = nat { return s.to_string(); }
    if c < 0x20 || c == 0x7F { ctrl_name(c).to_string() } else { (c as char).to_string() }
}
/// Gegenstück zu `translate` für den Kodierer: Zeichen -> 7-Bit-Code (unbekannte als '?')
fn code_of(ch: char, cs: Charset) -> u8 {
    for c in 0x20u8..0x7F {
        if translate(c, cs) == ch.to_string() { return c; }
    }
    match ch { '\n' => 0x0A, '\r' => 0x0D, _ => b'?' }
}
fn rev7(b: u8) -> u8 {
    ((b << 6) & 64) | ((b >> 6) & 1) | ((b << 4) & 32) | ((b >> 4) & 2) | ((b << 2) & 16) | ((b >> 2) & 4) | (b & 8)
}
fn get7(buf: &[u8], n: usize) -> u8 {
    let a = *buf.get(n * 7 / 8).unwrap_or(&0) as u32;
    let b = *buf.get((n * 7 + 6) / 8).unwrap_or(&0) as u32;
    (((a << 8) | b) >> ((n + 1) % 8)) as u8
}
fn guess_alpha(c: u8) -> i32 {
    if (c > 0 && c < 32) || c == 127 { -5 }
    else if (c > 32 && c < 48) || (c > 57 && c < 65) || (c > 90 && c < 97) || (c > 122 && c < 127) { -2 }
    else { 1 }
}
fn guess_numeric(c: u8, pos: usize) -> i32 {
    match c { b'U' => -10, b'[' | b']' => -5, b' ' | b'.' | b'-' => -2, _ if pos < 10 => 5, _ => 0 }
}

// ---------------- Protokoll (Zustandsmaschine wie multimon-ng) ----------------

/// Zähler je Baudrate für die Diagnose (ähnlich multimon-ng -v1)
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Stats {
    /// Präambeln (mindestens 64 Bit 1010…): ein POCSAG-Sender ist zu hören
    pub preambles: u32,
    /// Sync-Wörter, auf die eingerastet wurde
    pub syncs: u32,
    /// Codewörter im Takt: fehlerfrei, korrigiert, nicht korrigierbar
    pub ok: u32,
    pub fixed: u32,
    pub bad: u32,
    /// ausgegebene Nachrichten
    pub msgs: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum St { NoSync, Sync, Losing, Lost, Address, Message, End }

struct Layer {
    baud: u32,
    rx: u32,
    st: St,
    rx_bit: u32,
    rx_word: u32,
    inverted: bool,
    /// -1 = keine, -2 = Teilnachricht ohne Adresse (wie multimon-ng)
    addr: i64,
    func: i32,
    buf: Vec<u8>,
    nib: usize,
    /// Wörter der laufenden Nachricht (Adresse, Inhalt, Ende) und wie viele davon korrigiert werden mussten
    words: u32,
    fixed: u32,
    /// das zuletzt gelesene Wort musste korrigiert werden
    last_fixed: bool,
    /// Nummer der Empfangsvariante (0 = ohne Ausgleich)
    variant: usize,
    /// aktuelle Zeit (Sample) und Beginn der laufenden Nachricht
    now: u64,
    t0: u64,
    /// letzte Wörter im Sync: (Zeit, musste korrigiert werden oder war unlesbar)
    log: std::collections::VecDeque<(u64, bool)>,
    /// Präambel-Erkennung: Bits im Wechsel 1010… und voriges Bit
    alt: u32,
    last_bit: bool,
    stats: Stats,
}

impl Layer {
    fn new(baud: u32, variant: usize) -> Self {
        Layer { variant, baud, rx: 0, st: St::NoSync, rx_bit: 0, rx_word: 0, inverted: false, addr: -1, func: -1, buf: vec![0; BUF], nib: 0, words: 0, fixed: 0, last_fixed: false, now: 0, t0: 0, log: Default::default(), alt: 0, last_bit: false, stats: Stats::default() }
    }
    fn synced(&self) -> bool { self.st != St::NoSync }
    fn repair(&mut self, p: &Params, cw: &mut u32) -> bool {
        // lauter Nullen oder Einsen: Stille (Rauschsperre zu) oder Dauerträger, kein Funkruf
        // (das Nullwort wäre formal RIC 0, die in der Praxis nicht vergeben wird)
        if *cw == 0 || *cw == u32::MAX { return false; }
        let mut t = *cw;
        self.last_fixed = false;
        match correct(&mut t) {
            Some(0) => true,
            Some(n) if n <= p.ecc as u32 => { *cw = t; self.last_fixed = true; true }
            _ => false,
        }
    }
    fn reset_msg(&mut self) {
        self.nib = 0;
        self.addr = -1;
        self.func = -1;
        self.words = 0;
        self.fixed = 0;
        self.buf.iter_mut().for_each(|b| *b = 0);
    }

    /// ein empfangenes Bit (wie multimon-ng: positives Signal = 0)
    fn bit(&mut self, positive: bool, p: &Params, out: &mut Vec<Msg>) {
        self.rx = (self.rx << 1) | (!positive) as u32;
        if positive != self.last_bit { self.alt += 1; if self.alt == 64 { self.stats.preambles += 1; } } else { self.alt = 0; }
        self.last_bit = positive;
        let mut d = self.rx;
        if !self.synced() {
            for inv in [false, true] {
                if (inv && p.polarity == Polarity::Normal) || (!inv && p.polarity == Polarity::Inverted) { continue; }
                let t = if inv { !d } else { d };
                // Sync mit höchstens SYNC_ERR Bitfehlern: weniger Fehlstarts auf Rauschen
                let ok = (t ^ SYNC).count_ones() <= SYNC_ERR.min(p.ecc as u32);
                if ok {
                    self.stats.syncs += 1;
                    self.st = St::Sync;
                    self.inverted = inv;
                    self.rx_bit = 0;
                    self.rx_word = 0;
                    return;
                }
            }
            return;
        }
        if self.inverted { d = !d; }
        self.rx_bit = (self.rx_bit + 1) % 32;
        if self.rx_bit != 0 { return; }
        // 17 Wörter je Batch: 0–15 Daten (je zwei pro Rahmen), 16 = nächstes Sync
        let rxword = self.rx_word;
        self.rx_word = (self.rx_word + 1) % 17;
        if self.st == St::Sync { self.st = St::Address; }
        let good = self.repair(p, &mut d);
        if !good { self.stats.bad += 1 } else if self.last_fixed { self.stats.fixed += 1 } else { self.stats.ok += 1 }
        self.log.push_back((self.now, !good || self.last_fixed));
        if self.log.len() > 4096 { self.log.pop_front(); }
        if !good {
            if self.st != St::Lost { self.st = St::Losing; }
        } else if self.st == St::Lost {
            self.st = St::Address; // Sync wiedergefunden
        }
        if d == SYNC { return; }
        loop {
            match self.st {
                St::Losing => {
                    self.emit(false, p, out);
                    self.reset_msg();
                    self.st = St::Lost;
                    return;
                }
                St::Lost => {
                    self.st = St::NoSync;
                    self.rx_word = 0;
                    return;
                }
                St::Address => {
                    if d == IDLE { return; }
                    if d & MSG_FLAG != 0 { // Nachricht ohne Adresse: Teilnachricht
                        self.t0 = self.now;
                        self.func = -2;
                        self.addr = -2;
                        self.st = St::Message;
                        continue;
                    }
                    self.func = ((d >> 11) & 3) as i32;
                    self.addr = (((d >> 10) & 0x1F_FFF8) | ((rxword >> 1) & 7)) as i64;
                    self.words = 1;
                    self.fixed = self.last_fixed as u32;
                    self.t0 = self.now;
                    self.st = St::Message;
                    return;
                }
                St::Message => {
                    self.words += 1;
                    self.fixed += self.last_fixed as u32;
                    if d & MSG_FLAG == 0 { // Adresse oder Idle: Nachricht zu Ende
                        self.st = St::End;
                        continue;
                    }
                    if self.nib > BUF * 2 - 5 { // zu lang: vermutlich Unsinn
                        self.st = St::Losing;
                        continue;
                    }
                    let data = d >> 11;
                    let i = self.nib >> 1;
                    if self.nib & 1 == 1 {
                        self.buf[i] = (self.buf[i] & 0xF0) | ((data >> 16) & 0xF) as u8;
                        self.buf[i + 1] = (data >> 8) as u8;
                        self.buf[i + 2] = data as u8;
                    } else {
                        self.buf[i] = (data >> 12) as u8;
                        self.buf[i + 1] = (data >> 4) as u8;
                        self.buf[i + 2] = (data << 4) as u8;
                    }
                    self.nib += 5;
                    return;
                }
                St::End => {
                    self.emit(true, p, out);
                    self.reset_msg();
                    self.st = St::Address;
                    continue;
                }
                St::NoSync | St::Sync => return,
            }
        }
    }

    fn numeric(&self) -> (String, i32) {
        let mut s = String::new();
        for k in 0..self.nib.min(BUF * 2 - 1) {
            let b = self.buf[k / 2];
            let n = if k % 2 == 0 { b >> 4 } else { b & 0xF };
            s.push(NUM_TABLE[n as usize] as char);
        }
        let g = s.bytes().enumerate().map(|(i, c)| guess_numeric(c, i)).sum();
        (s, g)
    }
    fn alpha(&self, caesar: u8, cs: Charset) -> (String, i32) {
        let len = self.nib * 4 / 7;
        let (mut s, mut g) = (String::new(), 0);
        for i in 0..len {
            let c = rev7(get7(&self.buf, i)).wrapping_sub(caesar);
            g += guess_alpha(c);
            s += &translate(c, cs);
        }
        (s, g)
    }

    /// Ausgabe wie pocsag_printmessage in multimon-ng
    fn emit(&mut self, sync: bool, p: &Params, out: &mut Vec<Msg>) {
        let before = out.len();
        self.emit_inner(sync, p, out);
        self.stats.msgs += (out.len() - before) as u32;
        for m in &mut out[before..] { m.fixed = self.fixed; m.variant = self.variant; m.t0 = self.t0; m.t1 = self.now; }
    }
    fn emit_inner(&self, sync: bool, p: &Params, out: &mut Vec<Msg>) {
        let partial = self.addr == -2 || self.func == -2 || !sync;
        if !p.partial && partial { return; }
        if p.prune_empty && self.nib == 0 { return; }
        if self.addr == -1 && self.func == -1 { return; }
        // viele korrigierte Wörter: so sieht Rauschen aus (ein zufälliges Wort ist zu 26 % „korrigierbar“);
        // kurze Nachrichten (nur Adresse, wenig Inhalt) müssen fehlerfrei sein
        // Varianten mit Ausgleich (Deemphasis, Hochpass) verstärken Rauschen: dort strenger
        let lim = if self.variant == 0 { 2 } else { 4 };
        if self.fixed * lim > self.words || (self.words <= 3 && self.fixed > 0) { return; }
        let (addr, func) = if self.addr >= 0 && self.func >= 0 { (Some(self.addr as u32), Some(self.func as u8)) } else { (None, None) };
        let mk = |kind, text: String| Msg { baud: self.baud, addr, func, kind, text, complete: sync, fixed: 0, variant: 0, t0: 0, t1: 0 };
        if self.nib == 0 {
            out.push(mk(Kind::Tone, String::new()));
            return;
        }
        let (num, g_num) = self.numeric();
        let (alpha, g_alpha) = self.alpha(0, p.charset);
        let (sky, g_sky) = self.alpha(1, p.charset);
        let unsure = g_num < 20 && g_alpha < 20 && g_sky < 20;
        if unsure && p.heuristic { return; }
        let f = self.func;
        // feste Skyper-RICs: alphanumerische Rufe dort im Standard-Format gleich entschlüsseln
        let sky_ric = p.format == Format::Standard && f != 0 && addr.is_some_and(|a| a != 0 && p.skyper.contains(&a));
        if sky_ric { out.push(mk(Kind::Skyper, sky)); return; }
        if p.format == Format::Numeric || (p.format == Format::Standard && f == 0) || (p.format == Format::Auto && (g_num >= 20 || unsure)) {
            out.push(mk(Kind::Numeric, num));
        }
        if p.format == Format::Alpha || (p.format == Format::Standard && f != 0) || (p.format == Format::Auto && (g_alpha >= g_sky || unsure)) {
            out.push(mk(Kind::Alpha, alpha));
        }
        if p.format == Format::Skyper || (p.format == Format::Auto && (g_sky >= g_alpha || unsure)) {
            out.push(mk(Kind::Skyper, sky));
        }
    }
}

// ---------------- Bitsynchronisation ----------------

/// Biquad-Tiefpass (2. Ordnung)
struct Lp { b0: f64, b1: f64, a1: f64, a2: f64, z1: f64, z2: f64 }
impl Lp {
    fn new(fs: f64, fc: f64) -> Self {
        let w = TAU * fc.min(fs * 0.45) / fs;
        let (c, a) = (w.cos(), w.sin() / (2.0 * std::f64::consts::FRAC_1_SQRT_2));
        let a0 = 1.0 + a;
        Lp { b0: (1.0 - c) / 2.0 / a0, b1: (1.0 - c) / a0, a1: -2.0 * c / a0, a2: (1.0 - a) / a0, z1: 0.0, z2: 0.0 }
    }
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b0 * x - self.a2 * y;
        y
    }
}

/// Dekoder für eine Baudrate in einer Empfangsvariante: Gleichanteil entfernen, Tiefpass, Taktrückgewinnung
/// an den Nulldurchgängen, Integrieren über ein Bit (Matched Filter für NRZ), dann Protokoll.
/// Varianten für Audio vom Lautsprecher: Deemphasis ausgleichen (Höhen +6 dB/Oktave ab 300 Hz) und
/// Hochpass ausgleichen (Grundlinie per Rückkopplung der Entscheidung, „quantized feedback“).
struct Demod {
    step: f64,
    phase: f64,
    acc: f64,
    dc: f64,
    dc_a: f64,
    lp: Lp,
    last: f64,
    /// Deemphasis-Ausgleich: voriges Sample und Faktor (None = aus)
    pre: Option<(f64, f64)>,
    /// Hochpass-Ausgleich: Koeffizient (0 = aus), Grundlinie, Amplitude
    qfb_a: f64,
    z: f64,
    amp: f64,
    /// lernender Entzerrer (DFE) gegen Hochpass und Deemphasis des Empfängers
    dfe: Option<Dfe>,
    /// Entzerrer, der je Aussendung an Präambel und Sync angelernt wird (für stark gefiltertes Lautsprecher-Audio)
    eq: Option<Box<BurstEq>>,
    /// Samples im laufenden Bit
    nb: f64,
    /// verarbeitete Samples; Sync-Abschnitte (Beginn, Ende) der letzten Zeit und der laufende
    n: u64,
    runs: std::collections::VecDeque<(u64, u64)>,
    run: Option<u64>,
    layer: Layer,
}
/// Entzerrer mit Entscheidungsrückkopplung: Was Hochpass (CTCSS-Filter) und Deemphasis vom Signal wegnehmen
/// bzw. in die folgenden Bits verschmieren, hängt nur von den vorigen Bits ab. Diese Nachwirkung wird je Bit
/// aus den letzten Entscheidungen geschätzt (Gewichte per LMS gelernt) und abgezogen.
struct Dfe { taps: Vec<f64>, hist: VecDeque<f64>, amp: f64, corr: f64 }
const DFE_TAPS: usize = 24;
const DFE_MU: f64 = 0.02;
impl Dfe {
    fn new() -> Self { Dfe { taps: vec![0.0; DFE_TAPS], hist: VecDeque::from(vec![0.0; DFE_TAPS]), amp: 0.0, corr: 0.0 } }
    /// Mittelwert des (schon korrigierten) Bits -> Entscheidung; Gewichte nachführen, Korrektur fürs nächste Bit
    fn bit(&mut self, m: f64) -> bool {
        let b = if m > 0.0 { 1.0 } else { -1.0 };
        self.amp += (m.abs() - self.amp) * 0.02;
        let e = m - b * self.amp;
        let g = DFE_MU / (self.amp.max(1e-9) * DFE_TAPS as f64).max(1e-9) * self.amp; // normiert auf die Amplitude
        for (t, h) in self.taps.iter_mut().zip(&self.hist) { *t = (*t + g * e * h) * 0.9999; }
        self.hist.pop_back();
        self.hist.push_front(b);
        self.corr = self.taps.iter().zip(&self.hist).map(|(t, h)| t * h).sum();
        b > 0.0
    }
}

impl Demod {
    fn new_eq(fs: f64, baud: u32, variant: usize) -> Self {
        let mut d = Self::new(fs, baud, false, 0.0, false, variant);
        d.eq = Some(Box::new(BurstEq::new(fs, baud)));
        d
    }
    fn new(fs: f64, baud: u32, deemph: bool, hp_hz: f64, dfe: bool, variant: usize) -> Self {
        let b = baud as f64;
        let coef = |hz: f64| 1.0 - (-TAU * hz / fs).exp();
        Demod { step: b / fs, phase: 0.0, acc: 0.0, dc: 0.0, dc_a: coef(b / 300.0),
                lp: Lp::new(fs, b * 0.8), last: 0.0, pre: deemph.then_some((0.0, fs / (TAU * 300.0))),
                qfb_a: if hp_hz > 0.0 { coef(hp_hz) } else { 0.0 }, z: 0.0, amp: 0.0, dfe: dfe.then(Dfe::new), eq: None, nb: 0.0, n: 0, runs: Default::default(), run: None, layer: Layer::new(baud, variant) }
    }
    /// ein Bit an das Protokoll; Sync-Abschnitte mitschreiben
    fn on_bit(&mut self, positive: bool, p: &Params, out: &mut Vec<Msg>) { self.on_bit_at(positive, self.n, p, out) }
    /// Bit mit seinem Zeitpunkt (Sample); der Entzerrer liefert blockweise verspätet, aber mit richtiger Zeit
    fn on_bit_at(&mut self, positive: bool, t: u64, p: &Params, out: &mut Vec<Msg>) {
        self.layer.now = t;
        self.layer.bit(positive, p, out);
        match (self.layer.synced(), self.run) {
            (true, None) => self.run = Some(self.n),
            (false, Some(s)) => {
                self.runs.push_back((s, self.n));
                if self.runs.len() > 64 { self.runs.pop_front(); }
                self.run = None;
            }
            _ => {}
        }
    }
    fn push(&mut self, x: &[f32], p: &Params, out: &mut Vec<Msg>) {
        if let Some(mut eq) = self.eq.take() {
            let mut bits = vec![];
            for &v in x {
                bits.clear();
                eq.push(v as f64, &mut bits);
                for &(b, t) in &bits { self.on_bit_at(b, t, p, out); }
                self.n += 1;
            }
            self.eq = Some(eq);
            return;
        }
        for &v in x {
            let mut v = v as f64;
            if let Some((prev, k)) = &mut self.pre {
                let x = v;
                v = x + *k * (x - *prev);
                *prev = x;
            }
            self.dc += (v - self.dc) * self.dc_a;
            let mut y = self.lp.run(v - self.dc);
            if self.qfb_a > 0.0 {
                // was der Hochpass weggenommen hat, aus den eigenen Entscheidungen wieder addieren
                y += self.z;
                self.amp += (y.abs() - self.amp) * self.qfb_a * 0.05;
                let d = if y > 0.0 { self.amp } else { -self.amp };
                self.z += (d - self.z) * self.qfb_a;
            }
            if let Some(d) = &self.dfe { y -= d.corr; }
            if (y > 0.0) != (self.last > 0.0) {
                // Flanke: Phase in Richtung Bitgrenze (0) ziehen
                let e = if self.phase < 0.5 { self.phase } else { self.phase - 1.0 };
                self.phase -= e * 0.2;
            }
            self.last = y;
            self.acc += y;
            self.nb += 1.0;
            self.phase += self.step;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
                let m = self.acc / self.nb.max(1.0);
                let positive = match &mut self.dfe { Some(d) => d.bit(m), None => m > 0.0 };
                self.acc = 0.0;
                self.nb = 0.0;
                self.on_bit(positive, p, out);
            }
            self.n += 1;
        }
    }
    /// war die ganze Zeit von t0 bis t1 im Sync und hat dabei fast nur fehlerfreie Wörter gelesen
    fn reads_well(&self, t0: u64, t1: u64) -> bool {
        let covered = self.run.is_some_and(|s| s <= t0) || self.runs.iter().any(|&(s, e)| s <= t0 && e >= t1);
        let (n, bad) = self.layer.log.iter().filter(|w| w.0 >= t0 && w.0 <= t1)
            .fold((0, 0), |(n, b), w| (n + 1, b + w.1 as u32));
        covered && n > 0 && bad * 4 <= n
    }
}

// ---------------- Entzerrer je Aussendung ----------------

/// Samples je Bit im Entzerrer
const EQ_SPS: usize = 4;
/// Länge des Entzerrers in Samples (8 Bit)
const EQ_L: usize = 32;
/// Länge des Präambel-Fensters in Samples (64 Bit)
const EQ_PRE: usize = 64 * EQ_SPS;

/// Entzerrer für stark gefiltertes Audio (Lautsprecher-/NF-Weg mit Deemphasis und Hochpass gegen CTCSS):
/// Das Audio wird auf 4 Samples je Bit gebracht. Nach einer Präambel (1010…) wird das Sync-Wort gesucht und ein
/// linearer Entzerrer (FIR, 32 Taps) per kleinster Quadrate an Präambel und Sync angelernt; danach lernt er mit
/// jedem Codewort weiter, das die BCH-Prüfung bestätigt (höchstens 1 Bit korrigiert). Der Bittakt wird an jedem
/// Sync nachgeführt. Weil die Verzerrung vom eigenen Funkgerät kommt, gilt der Gelernte auch für andere Sender.
struct BurstEq {
    /// Eingangssamples je Entzerrer-Sample
    ratio: f64,
    frac: f64,
    lp: [Lp; 2],
    dc: f64,
    dc_a: f64,
    prev: f64,
    /// Samples (4 je Bit) und absoluter Index von buf[0]
    buf: Vec<f64>,
    base: usize,
    /// Präambel: letzter Index mit hoher Korrelation, Anzahl hoher Werte in Folge
    pre_hi: Option<usize>,
    pre_run: usize,
    lock: Option<EqLock>,
    g: [f64; EQ_L],
    have_g: bool,
    ata: Vec<[f64; EQ_L]>,
    atb: [f64; EQ_L],
}
struct EqLock {
    /// Beginn des Sync-Worts des nächsten Batches (Samples, absolut, gebrochen)
    s0: f64,
    /// Samples je Bit (nachgeführt, für Sender mit ungenauem Takt)
    spb: f64,
    /// Batches ohne brauchbares Ergebnis
    weak: u32,
}

impl BurstEq {
    fn new(fs: f64, baud: u32) -> Self {
        let b = baud as f64;
        let out = b * EQ_SPS as f64;
        let fc = (0.85 * out / 2.0).min(fs * 0.45);
        BurstEq { ratio: fs / out, frac: 0.0, lp: [Lp::new(fs, fc), Lp::new(fs, fc)], dc: 0.0, dc_a: 1.0 - (-TAU * 3.0 / fs).exp(),
                  prev: 0.0, buf: vec![], base: 0, pre_hi: None, pre_run: 0, lock: None, g: [0.0; EQ_L], have_g: false,
                  ata: vec![[0.0; EQ_L]; EQ_L], atb: [0.0; EQ_L] }
    }
    fn end(&self) -> usize { self.base + self.buf.len() }
    fn at(&self, i: usize) -> f64 { self.buf[i - self.base] }
    /// Fenster um den (gebrochenen) Abtastzeitpunkt c, linear zwischen den Samples
    fn win(&self, c: f64) -> Option<[f64; EQ_L]> {
        let start = c - (EQ_L / 2) as f64;
        if start < self.base as f64 + 1.0 || start + EQ_L as f64 + 1.0 >= self.end() as f64 { return None; }
        let i0 = start.floor() as usize;
        let f = start - i0 as f64;
        let mut w = [0.0; EQ_L];
        for (k, v) in w.iter_mut().enumerate() { *v = self.at(i0 + k) * (1.0 - f) + self.at(i0 + k + 1) * f; }
        Some(w)
    }
    fn out(g: &[f64; EQ_L], w: &[f64; EQ_L]) -> f64 { g.iter().zip(w).map(|(a, b)| a * b).sum() }
    fn learn(&mut self, rows: &[[f64; EQ_L]], targets: &[f64], forget: f64) {
        for r in self.ata.iter_mut() { for v in r.iter_mut() { *v *= forget; } }
        for v in self.atb.iter_mut() { *v *= forget; }
        for (w, &t) in rows.iter().zip(targets) {
            for i in 0..EQ_L {
                self.atb[i] += w[i] * t;
                for j in 0..EQ_L { self.ata[i][j] += w[i] * w[j]; }
            }
        }
        if let Some(g) = solve(&self.ata, &self.atb) { self.g = g; self.have_g = true; }
    }

    /// ein Eingangssample; liefert fertige Bitentscheidungen (true = positiv = 0 wie multimon-ng)
    fn push(&mut self, v: f64, bits: &mut Vec<(bool, u64)>) {
        self.dc += (v - self.dc) * self.dc_a;
        let y0 = self.lp[0].run(v - self.dc);
        let y = self.lp[1].run(y0);
        // auf 4 Samples je Bit umrechnen (linear interpoliert)
        self.frac += 1.0;
        while self.frac >= self.ratio {
            self.frac -= self.ratio;
            let t = 1.0 - self.frac; // Anteil des aktuellen Samples
            let s = self.prev + (y - self.prev) * t.clamp(0.0, 1.0);
            self.buf.push(s);
            self.sample(bits);
        }
        self.prev = y;
        // alten Puffer wegwerfen
        let keep = self.lock.as_ref().map_or(self.end().saturating_sub(4 * EQ_PRE), |l| (l.s0 as usize).saturating_sub(EQ_PRE + 8 * EQ_SPS));
        if keep > self.base + 4096 {
            let d = keep - self.base;
            self.buf.drain(..d);
            self.base += d;
        }
    }

    fn sample(&mut self, bits: &mut Vec<(bool, u64)>) {
        if self.lock.is_some() { self.run_lock(bits); return; }
        // Präambel-Erkennung: Korrelation der letzten 64 Bit mit 1010… (normiert)
        let n = self.end();
        if n < self.base + EQ_PRE { return; }
        let (mut c, mut e) = (0.0, 0.0);
        for i in 0..EQ_PRE {
            let v = self.at(n - EQ_PRE + i);
            c += if (i / EQ_SPS).is_multiple_of(2) { v } else { -v };
            e += v * v;
        }
        let c = c.abs() / (e * EQ_PRE as f64).sqrt().max(1e-12);
        if c > 0.8 { self.pre_run += 1; self.pre_hi = Some(n - 1); return; }
        // Präambel zu Ende (lang genug gesehen): Sync suchen, sobald genug Samples da sind
        let Some(hi) = self.pre_hi else { return };
        if self.pre_run < 32 * EQ_SPS { if n - 1 > hi + 64 { self.pre_hi = None; self.pre_run = 0; } return; }
        if n < hi + 48 + 32 * EQ_SPS + EQ_L + 2 { return; }
        self.pre_hi = None;
        self.pre_run = 0;
        self.find_sync(hi);
    }

    fn sync_targets() -> Vec<f64> { (0..32).map(|i| if (SYNC >> (31 - i)) & 1 == 0 { 1.0 } else { -1.0 }).collect() }

    /// Sync-Wort hinter der Präambel suchen; ohne gelernten Entzerrer an Präambel + Sync anlernen
    fn find_sync(&mut self, hi: usize) {
        let st = Self::sync_targets();
        let mut best: Option<(f64, usize, f64, [f64; EQ_L])> = None; // (Güte, s0, Polarität, g)
        for s0 in hi.saturating_sub(48)..=hi + 48 {
            let center = |k: isize| s0 as f64 + (k * EQ_SPS as isize) as f64 + (EQ_SPS / 2) as f64;
            let sync_rows: Option<Vec<[f64; EQ_L]>> = (0..32).map(|k| self.win(center(k))).collect();
            let Some(sync_rows) = sync_rows else { continue };
            if self.have_g {
                let sc: f64 = sync_rows.iter().zip(&st).map(|(w, t)| Self::out(&self.g, w) * t).sum();
                let p = if sc >= 0.0 { 1.0 } else { -1.0 };
                let mut gp = self.g; for v in gp.iter_mut() { *v *= p; }
                // Güte: richtige Bits im Sync, dann Abstand
                let ok = sync_rows.iter().zip(&st).filter(|(w, t)| Self::out(&gp, w) * **t > 0.0).count() as f64;
                let q = ok + sc.abs() * 1e-6;
                if best.as_ref().is_none_or(|b| q > b.0) { best = Some((q, s0, p, gp)); }
            } else {
                // Präambel (96 Bit, endet mit …1 0) und Sync als bekannte Bits
                let pre_rows: Option<Vec<[f64; EQ_L]>> = (-96..0).map(|k| self.win(center(k))).collect();
                let Some(pre_rows) = pre_rows else { continue };
                {
                    let p = 1.0; // das Vorzeichen bestimmt die Ausgleichsrechnung selbst
                    let mut ata = vec![[0.0; EQ_L]; EQ_L];
                    let mut atb = [0.0; EQ_L];
                    let tg: Vec<f64> = (0..96).map(|i| if i % 2 == 0 { -p } else { p }).chain(st.iter().map(|t| t * p)).collect();
                    let rows: Vec<&[f64; EQ_L]> = pre_rows.iter().chain(sync_rows.iter()).collect();
                    for (w, &t) in rows.iter().zip(&tg) {
                        for i in 0..EQ_L { atb[i] += w[i] * t; for j in 0..EQ_L { ata[i][j] += w[i] * w[j]; } }
                    }
                    let Some(g) = solve(&ata, &atb) else { continue };
                    let err: f64 = rows.iter().zip(&tg).map(|(w, t)| (Self::out(&g, w) - t).powi(2)).sum::<f64>() / rows.len() as f64;
                    let ok = sync_rows.iter().zip(&st).filter(|(w, t)| Self::out(&g, w) * **t * p > 0.0).count() as f64;
                    let q = ok - err;
                    if best.as_ref().is_none_or(|b| q > b.0) { best = Some((q, s0, 1.0, g)); }
                }
            }
        }
        let Some((q, s0, _, g)) = best else { return };
        if q < 29.0 { return; } // höchstens 3 falsche Bits im Sync
        if !self.have_g {
            self.g = g;
            self.have_g = true;
            // Startwerte fürs Weiterlernen: Präambel + Sync
            let rows: Vec<[f64; EQ_L]> = (-96..32).filter_map(|k: isize| self.win(s0 as f64 + (k * EQ_SPS as isize) as f64 + (EQ_SPS / 2) as f64)).collect();
            let tg: Vec<f64> = rows.iter().map(|w| if Self::out(&g, w) > 0.0 { 1.0 } else { -1.0 }).collect();
            self.learn(&rows, &tg, 0.0);
            self.g = g;
        } else if g[0] * self.g[0] < 0.0 || g.iter().zip(&self.g).map(|(a, b)| a * b).sum::<f64>() < 0.0 {
            // dieser Sender kommt mit umgekehrter Polarität: Gelerntes mitdrehen
            for v in self.atb.iter_mut() { *v = -*v; }
            self.g = g;
        }
        self.lock = Some(EqLock { s0: s0 as f64, spb: EQ_SPS as f64, weak: 0 });
    }

    /// Wort ab Sample-Position `pos` lesen: Fenster, weiche Werte (positiv = 0) und hartes Codewort
    fn word_at(&self, g: &[f64; EQ_L], pos: f64, spb: f64) -> Option<(Vec<[f64; EQ_L]>, Vec<f64>, u32)> {
        let rows: Option<Vec<[f64; EQ_L]>> = (0..32).map(|j| self.win(pos + j as f64 * spb + spb / 2.0)).collect();
        let rows = rows?;
        let v: Vec<f64> = rows.iter().map(|w| Self::out(g, w)).collect();
        let cw = v.iter().fold(0u32, |a, &x| (a << 1) | (x <= 0.0) as u32);
        Some((rows, v, cw))
    }

    /// einen ganzen Batch (Sync + 16 Wörter) dekodieren, sobald seine Samples da sind: dreimal, dazwischen nachlernen;
    /// je Wort den Takt um bis zu ±1 Sample nachführen
    fn run_lock(&mut self, bits: &mut Vec<(bool, u64)>) {
        let Some(l) = &self.lock else { return };
        let (s0, spb0) = (l.s0, l.spb);
        let need = s0 + 17.0 * 32.0 * spb0 + 3.0 * spb0 + (EQ_L / 2) as f64 + 2.0;
        if (self.end() as f64) < need { return; }
        let mut g = self.g;
        let mut result: Vec<u32> = vec![];
        let mut raw: Vec<u32> = vec![];
        let mut learned: (Vec<[f64; EQ_L]>, Vec<f64>) = (vec![], vec![]);
        let (mut end_pos, mut spb_end, mut good) = (s0, spb0, 0usize);
        for _ in 0..3 {
            let (mut pos, mut spb) = (s0, spb0);
            let mut words = vec![];
            let mut hard = vec![];
            let mut train: (Vec<[f64; EQ_L]>, Vec<f64>) = (vec![], vec![]);
            good = 0;
            for slot in 0..17 {
                // bis zu ±1 Sample verschoben lesen; ein fehlerfreies Wort zeigt die richtige Lage
                let mut best: Option<(u32, f64, u32, Vec<[f64; EQ_L]>, Option<u32>)> = None; // (Fehler, d, cw, rows, wahr)
                for d in [0.0, -0.5, 0.5, -1.0, 1.0] {
                    let Some((rows, v, cw)) = self.word_at(&g, pos + d, spb) else { continue };
                    let (errs, truth) = if slot == 0 {
                        let e = (cw ^ SYNC).count_ones();
                        (e, (e <= 4).then_some(SYNC))
                    } else {
                        match chase(&v) { Some((t, _)) => ((t ^ cw).count_ones(), Some(t)), None => (99, None) }
                    };
                    // Verschiebung nur bei fehlerfreiem Wort übernehmen (zufällige Wörter sind oft „korrigierbar“)
                    let take = best.as_ref().is_none_or(|b| errs < b.0 && (d == 0.0 || errs == 0));
                    if take { best = Some((errs, d, cw, rows, truth)); }
                    if errs == 0 { break; }
                }
                let Some((errs, d, cw, rows, truth)) = best else { break };
                if d != 0.0 { pos += d; spb += d / 32.0 * 0.3; }
                // ohne sichere Lesart: höchstens 1 Bit Korrektur zulassen, sonst als unlesbar weitergeben (0 lehnt das
                // Protokoll ab) – sonst „repariert“ die normale 2-Bit-Korrektur dahinter es womöglich zu einer falschen RIC
                let safe = || { let mut t = cw; if matches!(correct(&mut t), Some(n) if n <= 1) { cw } else { 0 } };
                words.push(if slot == 0 { cw } else { truth.unwrap_or_else(safe) });
                hard.push(cw);
                if let Some(t) = truth.filter(|_| errs <= 2) {
                    if slot > 0 { good += 1; }
                    train.0.extend(rows);
                    train.1.extend((0..32).map(|i| if (t >> (31 - i)) & 1 == 0 { 1.0 } else { -1.0 }));
                }
                pos += 32.0 * spb;
            }
            result = words;
            raw = hard;
            end_pos = pos;
            spb_end = spb;
            // nachlernen (vorläufig, zusammen mit dem bisher Gelernten)
            if !train.0.is_empty() {
                let (mut ata, mut atb) = (self.ata.clone(), self.atb);
                for (w, &t) in train.0.iter().zip(&train.1) {
                    for i in 0..EQ_L { atb[i] += w[i] * t; for j in 0..EQ_L { ata[i][j] += w[i] * w[j]; } }
                }
                if let Some(ng) = solve(&ata, &atb) { g = ng; }
            }
            learned = train;
        }
        // Ergebnis übernehmen: Bits ausgeben, Gelerntes dauerhaft machen. In einem schwachen Batch (meist schon
        // Rauschen nach dem Ende der Aussendung) nur die unkorrigierten Bits – dort wäre die Chase-Korrektur Raten
        let out = if good >= 3 { &result } else { &raw };
        // Zeitpunkt jedes Bits in Eingangssamples (Mitte des Bits)
        let ratio = self.ratio;
        for (k, w) in out.iter().enumerate() {
            for (j, i) in (0..32).rev().enumerate() {
                let c = s0 + ((k * 32 + j) as f64 + 0.5) * spb0;
                bits.push(((w >> i) & 1 == 0, (c * ratio).max(0.0) as u64));
            }
        }
        if !learned.0.is_empty() { self.learn(&learned.0, &learned.1, 0.98); }
        let l = self.lock.as_mut().unwrap();
        l.s0 = end_pos;
        l.spb = spb_end;
        if good < 3 { l.weak += 1; } else { l.weak = 0; }
        if l.weak >= 1 || result.len() < 17 { self.lock = None; }
    }
}

/// Chase-Dekodierung mit weichen Werten (positiv = Bit 0): die 6 unsichersten Bits in allen Kombinationen
/// umdrehen, jeweils BCH-korrigieren und die Lesart mit der kleinsten Summe umgedrehter Zuverlässigkeiten nehmen.
/// Abgelehnt wird, was mehr als 0,8 mittlere Bits umdrehen müsste – mehr ist an echten Aufnahmen Raten
/// (bei 1,6 kam schon die Hälfte Unsinn heraus).
const CHASE_LIMIT: f64 = 0.8;
/// Adresswörter, die mehr als 2 Bit Korrektur bräuchten: nur bei sehr sicherer und eindeutiger Lesart
const CHASE_ADDR_LIMIT: f64 = 0.6;
const CHASE_ADDR_MARGIN: f64 = 0.6;
fn chase(v: &[f64]) -> Option<(u32, f64)> {
    let hard = v.iter().fold(0u32, |a, &x| (a << 1) | (x <= 0.0) as u32);
    let rel: Vec<f64> = v.iter().map(|x| x.abs()).collect();
    let mean = rel.iter().sum::<f64>() / 32.0;
    let mut order: Vec<usize> = (0..32).collect();
    order.sort_by(|&a, &b| rel[a].total_cmp(&rel[b]));
    let weak = &order[..6];
    let mut cands: Vec<(u32, f64)> = vec![];
    for pat in 0u32..64 {
        let mut t = hard;
        for (k, &i) in weak.iter().enumerate() { if pat >> k & 1 == 1 { t ^= 1 << (31 - i); } }
        if correct(&mut t).is_none() || t == 0 || t == u32::MAX { continue; }
        if cands.iter().any(|c| c.0 == t) { continue; }
        let diff = t ^ hard;
        let m: f64 = (0..32).filter(|i| diff >> (31 - i) & 1 == 1).map(|i| rel[i]).sum();
        cands.push((t, m));
    }
    cands.sort_by(|a, b| a.1.total_cmp(&b.1));
    let best = *cands.first()?;
    let second = cands.get(1).map_or(f64::INFINITY, |c| c.1);
    // Adresswörter (und Idle) strenger: eine falsche RIC wäre schlimmer als ein verlorener Ruf. Inhaltswörter dürfen
    // mehr – dort verfälscht ein Fehler höchstens ein Zeichen
    let (lim, margin) = if best.0 & MSG_FLAG == 0 && (best.0 ^ hard).count_ones() > 2 { (CHASE_ADDR_LIMIT, CHASE_ADDR_MARGIN) } else { (CHASE_LIMIT, 0.0) };
    (best.1 <= lim * mean && second - best.1 >= margin * mean).then_some(best)
}

/// Gleichungssystem (A + Regularisierung) x = b lösen (Gauß mit Pivotsuche)
fn solve(a: &[[f64; EQ_L]], b: &[f64; EQ_L]) -> Option<[f64; EQ_L]> {
    let tr: f64 = (0..EQ_L).map(|i| a[i][i]).sum::<f64>() / EQ_L as f64;
    if tr <= 0.0 { return None; }
    let mut m: Vec<[f64; EQ_L + 1]> = (0..EQ_L).map(|i| {
        let mut r = [0.0; EQ_L + 1];
        r[..EQ_L].copy_from_slice(&a[i]);
        r[i] += 1e-3 * tr;
        r[EQ_L] = b[i];
        r
    }).collect();
    for c in 0..EQ_L {
        let p = (c..EQ_L).max_by(|&x, &y| m[x][c].abs().total_cmp(&m[y][c].abs()))?;
        if m[p][c].abs() < 1e-15 { return None; }
        m.swap(c, p);
        for rr in 0..EQ_L {
            if rr == c { continue; }
            let f = m[rr][c] / m[c][c];
            if f == 0.0 { continue; }
            for k in c..=EQ_L { m[rr][k] -= f * m[c][k]; }
        }
    }
    let mut x = [0.0; EQ_L];
    for i in 0..EQ_L { x[i] = m[i][EQ_L] / m[i][i]; }
    Some(x)
}

/// Empfangsvarianten je Baudrate: (Deemphasis ausgleichen, Hochpass ausgleichen bei Hz).
/// Direkt vom Datenanschluss braucht keine; Lautsprecher-Audio ist bei Funkgeräten deemphasiert und
/// unterhalb von 200–300 Hz beschnitten (CTCSS-Filter).
const VARIANTS: [(bool, f64, bool); 8] = [(false, 0.0, false), (true, 0.0, false), (false, 120.0, false), (true, 120.0, false),
    (false, 250.0, false), (true, 250.0, false), (false, 0.0, true), (true, 0.0, true)];
/// Nachrichten der Varianten sammeln: so lange nach der letzten Meldung warten (Sekunden) …
const GROUP_QUIET_S: f64 = 0.3;
/// … aber höchstens so lange
const GROUP_MAX_S: f64 = 20.0;

/// so lange wird ein gemeldeter Ruf nicht noch einmal gemeldet (s); länger als ein Batch bei 512 Bd
const RECENT_S: f64 = 5.0;

/// gleiche Nachricht aus mehreren Empfangsvarianten
struct Group { first: u64, last: u64, key: Msg, cands: Vec<Msg> }

/// Empfänger für alle eingeschalteten Baudraten (wie multimon-ng mit -a POCSAG512 -a POCSAG1200 -a POCSAG2400)
pub struct Receiver {
    pub p: Params,
    fs: f64,
    demods: Vec<Demod>,
    t: u64,
    pending: Vec<Group>,
    /// zuletzt gemeldete Rufe (Zeit): Lesarten, die später kommen (Entzerrer arbeitet blockweise), nicht doppelt melden
    recent: Vec<(u64, Msg)>,
}
impl Receiver {
    pub fn new(fs: f64, p: Params) -> Self {
        let mut demods = vec![];
        for (&b, on) in BAUDS.iter().zip(p.rates) {
            if !on { continue; }
            for (k, &(de, hp, dfe)) in VARIANTS.iter().enumerate() { demods.push(Demod::new(fs, b, de, hp, dfe, k)); }
            demods.push(Demod::new_eq(fs, b, VARIANTS.len()));
        }
        Receiver { p, fs, demods, t: 0, pending: vec![], recent: vec![] }
    }
    pub fn push(&mut self, x: &[f32]) -> Vec<Msg> {
        let mut all = vec![];
        let p = self.p;
        for d in &mut self.demods { d.push(x, &p, &mut all); }
        self.t += x.len() as u64;
        let now = self.t;
        // derselbe Ruf aus mehreren Varianten: gleiche Kennung und gleicher Beginn (innerhalb eines halben Codeworts);
        // verschiedene Rufe an dieselbe RIC kurz hintereinander bleiben getrennt
        let same = |a: &Msg, b: &Msg| a.baud == b.baud && a.addr == b.addr && a.func == b.func && a.kind == b.kind
            && (a.t0 as f64 - b.t0 as f64).abs() <= 16.0 * self.fs / a.baud as f64;
        for m in all {
            match self.pending.iter_mut().find(|g| same(&g.key, &m)) {
                Some(g) => { g.cands.push(m); g.last = now; }
                None => self.pending.push(Group { first: now, last: now, cands: vec![m.clone()], key: m }),
            }
        }
        // fertig, wenn eine Weile nichts mehr kam und keine Variante noch mitten in dieser Nachricht steckt
        let (quiet, max) = ((GROUP_QUIET_S * self.fs) as u64, (GROUP_MAX_S * self.fs) as u64);
        let demods = &self.demods;
        let mut out = vec![];
        self.pending.retain(|g| {
            let busy = demods.iter().any(|d| d.layer.baud == g.key.baud && d.layer.st == St::Message
                && g.key.addr.is_some_and(|a| d.layer.addr == a as i64));
            let done = now - g.first > max || (now - g.last > quiet && !busy);
            if done && trusted(demods, &g.cands) { out.push(best(&g.cands)); }
            !done
        });
        self.dedup(out)
    }
    /// am Ende einer Datei: alles Gesammelte ausgeben
    pub fn flush(&mut self) -> Vec<Msg> {
        let demods = &self.demods;
        let out = self.pending.drain(..).filter(|g| trusted(demods, &g.cands)).map(|g| best(&g.cands)).collect();
        self.dedup(out)
    }
    fn dedup(&mut self, out: Vec<Msg>) -> Vec<Msg> {
        let now = self.t;
        let hold = (RECENT_S * self.fs) as u64;
        self.recent.retain(|(t, _)| now - t < hold);
        let same = |a: &Msg, b: &Msg| a.baud == b.baud && a.addr == b.addr && a.func == b.func && a.kind == b.kind
            && (a.text == b.text || a.clean_text() == b.clean_text());
        let mut res = vec![];
        for m in out {
            if self.recent.iter().any(|(_, r)| same(r, &m)) { continue; }
            self.recent.push((now, m.clone()));
            res.push(m);
        }
        res
    }
    /// Zähler je Baudrate (512, 1200, 2400): Präambeln und Sync über alle Varianten das Maximum,
    /// Wörter von der Variante, die am meisten fehlerfrei gelesen hat
    pub fn stats(&self) -> [Stats; 3] {
        let mut out = [Stats::default(); 3];
        for (i, b) in BAUDS.iter().enumerate() {
            let ds: Vec<&Stats> = self.demods.iter().filter(|d| d.layer.baud == *b).map(|d| &d.layer.stats).collect();
            let Some(best) = ds.iter().max_by_key(|s| s.ok) else { continue };
            out[i] = Stats { preambles: ds.iter().map(|s| s.preambles).max().unwrap_or(0), syncs: ds.iter().map(|s| s.syncs).max().unwrap_or(0),
                             msgs: ds.iter().map(|s| s.msgs).max().unwrap_or(0), ..**best };
        }
        out
    }
    /// Baudraten, die gerade im Sync sind
    pub fn synced(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.demods.iter().filter(|d| d.layer.synced()).map(|d| d.layer.baud).collect();
        v.dedup();
        v
    }
}

/// Eine Lesart, die nur aus Ausgleichs-Varianten kommt, zählt nicht, wenn die Variante ohne Ausgleich während
/// der ganzen Nachricht im Sync war und dabei sauber gelesen hat: dann liest sie das Signal selbst, und die andere Lesart ist ein Fehllesen
/// (z. B. invertierte Inhaltswörter, die wie Adressen aussehen – das Komplement eines Codeworts ist auch eins).
fn trusted(demods: &[Demod], c: &[Msg]) -> bool {
    let b = best(c);
    b.variant == 0 || c.iter().any(|m| m.variant == 0 && m.text == b.text) || !demods.iter().any(|d| d.layer.variant == 0 && d.layer.baud == b.baud && d.reads_well(b.t0, b.t1))
}
/// Lesart, die die meisten Varianten haben; bei Gleichstand die mit den wenigsten Korrekturen,
/// dann die Variante ohne Ausgleich
fn best(c: &[Msg]) -> Msg {
    let count = |m: &Msg| c.iter().filter(|x| x.text == m.text).count();
    c.iter().max_by(|a, b| count(a).cmp(&count(b)).then(b.fixed.cmp(&a.fixed)).then(b.variant.cmp(&a.variant)))
        .cloned().expect("Gruppe ohne Nachricht")
}

// ---------------- Kodierer ----------------

/// Inhalt eines Funkrufs
#[derive(Clone, Debug, PartialEq)]
pub enum Content { Tone, Numeric(String), Alpha(String) }

/// ein Funkruf an eine RIC
#[derive(Clone, Debug, PartialEq)]
pub struct Page { pub addr: u32, pub func: u8, pub content: Content }

/// höchste RIC (21 Bit)
pub const MAX_RIC: u32 = 0x1F_FFFF;

fn message_words(c: &Content, cs: Charset) -> Vec<u32> {
    let mut words = vec![];
    match c {
        Content::Tone => {}
        Content::Numeric(s) => {
            let nib: Vec<u32> = s.chars().map(|ch| {
                let ch = ch.to_ascii_uppercase();
                NUM_TABLE.iter().position(|&t| t as char == ch).unwrap_or(3) as u32 // unbekannt -> Leerzeichen
            }).collect();
            for chunk in nib.chunks(5) {
                let mut d = 0u32;
                for k in 0..5 { d = (d << 4) | chunk.get(k).copied().unwrap_or(3); }
                words.push(encode_cw((1 << 20) | d));
            }
        }
        Content::Alpha(s) => {
            // 7 Bit je Zeichen, niederwertiges Bit zuerst; Rest des letzten Worts mit EOT auffüllen
            let mut bits: Vec<bool> = vec![];
            for ch in s.chars() {
                let c = code_of(ch, cs);
                for k in 0..7 { bits.push((c >> k) & 1 == 1); }
            }
            let mut k = 0;
            while !bits.len().is_multiple_of(20) { bits.push((0x04 >> (k % 7)) & 1 == 1); k += 1; }
            for chunk in bits.chunks(20) {
                let d = chunk.iter().fold(0u32, |a, &b| (a << 1) | b as u32);
                words.push(encode_cw((1 << 20) | d));
            }
        }
    }
    words
}

/// Codewörter einer Aussendung (ohne Präambel): Batches aus Sync + 16 Wörtern;
/// jede Adresse steht in ihrem Rahmen (RIC mod 8), danach die Nachricht, am Ende mindestens ein Idle
pub fn codewords(pages: &[Page], cs: Charset) -> Vec<u32> {
    let mut out = vec![SYNC];
    let mut pos = 0usize; // Platz im Batch 0..16
    let push = |out: &mut Vec<u32>, pos: &mut usize, w: u32| {
        if *pos == 16 { out.push(SYNC); *pos = 0; }
        out.push(w);
        *pos += 1;
    };
    for pg in pages {
        let slot = ((pg.addr & 7) * 2) as usize;
        if pos > slot { while pos < 16 { push(&mut out, &mut pos, IDLE); } }
        if pos == 16 { out.push(SYNC); pos = 0; }
        while pos < slot { push(&mut out, &mut pos, IDLE); }
        push(&mut out, &mut pos, encode_cw((((pg.addr & MAX_RIC) >> 3) << 2) | (pg.func as u32 & 3)));
        for w in message_words(&pg.content, cs) { push(&mut out, &mut pos, w); }
    }
    // Ende der letzten Nachricht: zwei Idle, damit es auch dann ankommt, wenn der Empfänger das letzte Bit verschluckt
    push(&mut out, &mut pos, IDLE);
    push(&mut out, &mut pos, IDLE);
    while pos < 16 { push(&mut out, &mut pos, IDLE); }
    out
}

/// Bitfolge einer Aussendung: Präambel 1010… und die Codewörter (höchstes Bit zuerst)
pub fn bits(pages: &[Page], cs: Charset) -> Vec<bool> {
    let mut b: Vec<bool> = (0..PREAMBLE_BITS).map(|i| i % 2 == 0).collect();
    for w in codewords(pages, cs) { for k in (0..32).rev() { b.push((w >> k) & 1 == 1); } }
    b
}

/// Basisband-NRZ für den Modulationseingang eines FM-Senders (1 = negativ, 0 = positiv wie multimon-ng;
/// `invert` dreht das um). Die Flanken werden geglättet, damit das Signal schmalbandig bleibt.
pub fn modulate(pages: &[Page], cs: Charset, baud: u32, fs: f64, invert: bool) -> Vec<f32> {
    let b = bits(pages, cs);
    let spb = fs / baud as f64;
    let n = (b.len() as f64 * spb).ceil() as usize;
    let sign = if invert { -1.0 } else { 1.0 };
    let raw: Vec<f64> = (0..n).map(|i| {
        let k = ((i as f64 / spb) as usize).min(b.len() - 1);
        if b[k] { -sign } else { sign }
    }).collect();
    // zweimal gleitender Mittelwert über ein Viertelbit: dreieckige Glättung der Flanken
    let l = ((spb * 0.25).round() as usize).max(1);
    let smooth = |x: &[f64]| -> Vec<f64> {
        let mut acc = 0.0;
        (0..x.len()).map(|i| {
            acc += x[i];
            if i >= l { acc -= x[i - l]; }
            acc / l.min(i + 1) as f64
        }).collect()
    };
    let y = smooth(&smooth(&raw));
    y.into_iter().map(|v| (0.9 * v) as f32).collect()
}

/// Dauer einer Aussendung in Sekunden
pub fn seconds(pages: &[Page], cs: Charset, baud: u32) -> f64 { bits(pages, cs).len() as f64 / baud as f64 }

#[cfg(test)]
mod tests {
    use super::*;

    pub struct Rng(pub u64);
    impl Rng {
        pub fn u(&mut self) -> f64 { self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (self.0 >> 11) as f64 / (1u64 << 53) as f64 }
        pub fn gauss(&mut self) -> f64 { (-2.0 * (self.u() + 1e-12).ln()).sqrt() * (TAU * self.u()).cos() }
    }

    #[test]
    fn bch_sync_idle_gueltig() {
        assert_eq!(syndrome(SYNC), 0);
        assert_eq!(syndrome(IDLE), 0);
        // Sync und Idle sind selbst gültige Codewörter: aus ihren Daten neu kodiert
        assert_eq!(encode_cw(SYNC >> 11), SYNC);
        assert_eq!(encode_cw(IDLE >> 11), IDLE);
    }

    #[test]
    fn bch_korrigiert_zwei_bit() {
        let mut r = Rng(3);
        for _ in 0..2000 {
            let cw = encode_cw((r.u() * 2097152.0) as u32);
            let (a, b) = ((r.u() * 32.0) as u32, (r.u() * 32.0) as u32);
            let mut x = cw ^ (1 << a);
            assert_eq!(correct(&mut x), Some(1));
            assert_eq!(x, cw);
            if a != b {
                let mut x = cw ^ (1 << a) ^ (1 << b);
                assert_eq!(correct(&mut x), Some(2));
                assert_eq!(x, cw);
            }
        }
    }

    fn decode(x: &[f32], fs: f64, p: Params) -> Vec<Msg> {
        let mut rx = Receiver::new(fs, p);
        let mut out = vec![];
        for c in x.chunks(1000) { out.extend(rx.push(c)); }
        out.extend(rx.flush());
        out
    }

    fn pages() -> Vec<Page> {
        vec![
            Page { addr: 1234567, func: 3, content: Content::Alpha("Hallo DAPNET! [Test] 73 de DL1ABC".into()) },
            Page { addr: 8, func: 0, content: Content::Numeric("0123456789 -.U[]".into()) },
            Page { addr: 2_000_005, func: 1, content: Content::Tone },
            Page { addr: 165_000, func: 3, content: Content::Alpha("Ein etwas laengerer Text, der ueber mehrere Batches geht: ".repeat(3)) },
        ]
    }

    fn check(got: &[Msg], baud: u32) {
        let want = pages();
        let full: Vec<&Msg> = got.iter().filter(|m| m.complete && m.baud == baud).collect();
        assert_eq!(full.len(), want.len(), "{baud}: {got:#?}");
        for (m, w) in full.iter().zip(&want) {
            assert_eq!(m.addr, Some(w.addr));
            assert_eq!(m.func, Some(w.func));
            match &w.content {
                Content::Tone => assert_eq!(m.kind, Kind::Tone),
                Content::Numeric(s) => { assert_eq!(m.kind, Kind::Numeric); assert_eq!(m.text.trim_end(), s); }
                Content::Alpha(s) => { assert_eq!(m.kind, Kind::Alpha); assert_eq!(&m.clean_text(), s.trim_end()); }
            }
        }
    }

    #[test]
    fn rundlauf_alle_baudraten() {
        for &baud in &BAUDS {
            for &fs in &[48000.0, 44100.0, 22050.0] {
                for invert in [false, true] {
                    let x = modulate(&pages(), Charset::Us, baud, fs, invert);
                    let p = Params { rates: [baud == 512, baud == 1200, baud == 2400], ..Params::default() };
                    check(&decode(&x, fs, p), baud);
                }
            }
        }
    }

    /// RIC 4520 (DAPNET-Skyper-Rubriken): im Standard-Format gleich entschlüsselt, Kopf abgetrennt; andere RICs bleiben Klartext
    #[test]
    fn skyper_ric() {
        let plain = "Wetter: Sonnig, 12 Grad";
        let rot1: String = format!("1A{plain}").chars().map(|c| (c as u8 + 1) as char).collect();
        let pg = vec![Page { addr: 4520, func: 3, content: Content::Alpha(rot1.clone()) },
                      Page { addr: 4512, func: 3, content: Content::Alpha(plain.into()) }];
        let fs = 48000.0;
        let x = modulate(&pg, Charset::Us, 1200, fs, false);
        let got = decode(&x, fs, Params::default());
        let a = got.iter().find(|m| m.addr == Some(4520)).expect("4520 fehlt");
        assert_eq!(a.kind, Kind::Skyper);
        assert_eq!(a.skyper_split(), Some(("1A".into(), plain.into())));
        let b = got.iter().find(|m| m.addr == Some(4512)).expect("4512 fehlt");
        assert_eq!((b.kind, b.clean_text().as_str()), (Kind::Alpha, plain));
        let got = decode(&x, fs, Params { skyper: [0; 4], ..Params::default() });
        assert_eq!(got.iter().find(|m| m.addr == Some(4520)).unwrap().clean_text(), rot1);
    }

    /// Takt ±0,5 %, Rauschen, Pegel und Gleichanteil: wie ein mäßiges Signal vom Empfänger
    #[test]
    fn rauschen_und_taktfehler() {
        for &baud in &BAUDS {
            for &(ppm, sigma) in &[(5000.0, 0.5), (-5000.0, 0.5), (1000.0, 0.8)] {
                let fs = 48000.0;
                let mut x = modulate(&pages(), Charset::Us, baud, fs * (1.0 + ppm * 1e-6), false);
                let mut r = Rng(baud as u64);
                let mut pre: Vec<f32> = (0..(fs * 0.3) as usize).map(|_| 0.0).collect();
                pre.append(&mut x);
                let y: Vec<f32> = pre.iter().map(|v| 0.3 * v + 0.05 + (sigma * 0.3 * r.gauss()) as f32).collect();
                check(&decode(&y, fs, Params::default()), baud);
            }
        }
    }

    /// Lautsprecher-Audio: Deemphasis (6 dB/Oktave ab 300 Hz) und Hochpass des Empfängers (CTCSS-Filter)
    /// in allen Kombinationen – jede Nachricht genau einmal und richtig
    #[test]
    fn lautsprecher_audio() {
        let fs = 48000.0;
        let pg = [Page { addr: 1234567, func: 3, content: Content::Alpha("Hallo DAPNET! [Test] 73 de DL1ABC".into()) }];
        for baud in BAUDS {
            for (de, hpf) in [(300.0, 20.0), (300.0, 100.0), (300.0, 150.0), (300.0, 250.0), (0.0, 250.0)] {
                let x = modulate(&pg, Charset::Us, baud, fs, false);
                let a = 1.0 - (-TAU * de / fs).exp();
                let h = (-TAU * hpf / fs).exp();
                let (mut lp, mut hp_prev, mut hp) = (0.0f64, 0.0f64, 0.0f64);
                let y: Vec<f32> = x.iter().map(|&v| {
                    if de > 0.0 { lp += (v as f64 - lp) * a; } else { lp = v as f64; }
                    hp = h * (hp + lp - hp_prev);
                    hp_prev = lp;
                    (hp * 3.0) as f32
                }).chain(std::iter::repeat_n(0.0, 4800)).collect();
                let m = decode(&y, fs, Params::default());
                assert_eq!(m.len(), 1, "{baud} Bd, Deemphasis {de} Hz, Hochpass {hpf} Hz: {m:?}");
                assert_eq!(m[0].clean_text(), "Hallo DAPNET! [Test] 73 de DL1ABC");
            }
        }
    }

    /// Empfindlichkeit und Fehlmeldungen bei starkem Rauschen vor, während und nach der Aussendung
    #[test]
    fn empfindlichkeit() {
        let fs = 48000.0;
        let text = "Hallo DAPNET! [Test] 73 de DL1ABC 0123456789";
        let pg = [Page { addr: 1234567, func: 3, content: Content::Alpha(text.into()) }];
        let x = modulate(&pg, Charset::Us, 1200, fs, false);
        for sigma in [1.0, 1.8] {
            let (mut ok, mut wrong) = (0, 0);
            for trial in 0..12u64 {
                let mut r = Rng(trial * 7919 + 1);
                let y: Vec<f32> = std::iter::repeat_n(0.0f32, 48000).chain(x.iter().copied()).chain(std::iter::repeat_n(0.0, 48000))
                    .map(|v| 0.3 * v + (sigma * 0.3 * r.gauss()) as f32).collect();
                let m = decode(&y, fs, Params::default());
                ok += m.iter().filter(|m| m.addr == Some(1234567) && m.clean_text() == text).count();
                wrong += m.iter().filter(|m| m.addr != Some(1234567) || m.clean_text() != text).count();
            }
            println!("σ {sigma}: {ok}/12 richtig, {wrong} falsch");
            assert_eq!((ok, wrong), (12, 0), "σ {sigma}");
        }
    }

    /// NF-Weg eines Funkgeräts wie in der Aufnahme eines IC-705: Deemphasis 300 Hz, Hochpass 300 Hz 2. Ordnung
    /// (gegen CTCSS), Tiefpass 3 kHz, dazu Rauschen. Ohne Entzerrer liest hier auch multimon-ng nichts.
    #[test]
    fn nf_weg_mit_hochpass() {
        let fs = 48000.0;
        let biq = |x: &[f64], hp: bool, fc: f64, q: f64| -> Vec<f64> {
            let w = TAU * fc / fs;
            let (c, a) = (w.cos(), w.sin() / (2.0 * q));
            let (b0, b1, b2) = if hp { ((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0) } else { ((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0) };
            let (a0, a1, a2) = (1.0 + a, -2.0 * c, 1.0 - a);
            let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
            x.iter().map(|&v| { let o = (b0 * v + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2) / a0; x2 = x1; x1 = v; y2 = y1; y1 = o; o }).collect()
        };
        let mut got = 0;
        for (i, pg) in pages().into_iter().enumerate() {
            let x: Vec<f64> = std::iter::repeat_n(0.0, 24000).chain(modulate(std::slice::from_ref(&pg), Charset::Us, 1200, fs, false).iter().map(|&v| v as f64))
                .chain(std::iter::repeat_n(0.0, 24000)).collect();
            let a = 1.0 - (-TAU * 300.0 / fs).exp();
            let mut l = 0.0;
            let de: Vec<f64> = x.iter().map(|&v| { l += (v - l) * a; l * 3.0 }).collect();
            let y = biq(&biq(&de, true, 300.0, std::f64::consts::FRAC_1_SQRT_2), false, 3000.0, std::f64::consts::FRAC_1_SQRT_2);
            let m = y.iter().fold(0.0f64, |m, v| m.max(v.abs()));
            let mut r = Rng(i as u64 + 5);
            let z: Vec<f32> = y.iter().map(|v| (0.6 * v / m + 0.05 * r.gauss()) as f32).collect();
            let out = decode(&z, fs, Params { rates: [false, true, false], ..Params::default() });
            assert!(out.iter().all(|o| o.addr == Some(pg.addr)), "falsche RIC: {out:?}");
            if out.iter().any(|o| o.addr == Some(pg.addr)) { got += 1; }
        }
        assert!(got >= 3, "nur {got} von 4 Rufen gelesen");
    }

    #[test]
    fn rauschen_ohne_fehlmeldungen() {
        let fs = 48000.0;
        let mut r = Rng(11);
        let x: Vec<f32> = (0..(fs * 60.0) as usize).map(|_| (0.3 * r.gauss()) as f32).collect();
        let got = decode(&x, fs, Params::default());
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn formate_wie_multimon() {
        let fs = 48000.0;
        let pg = [Page { addr: 42, func: 3, content: Content::Alpha("Grüße aus Köln".into()) }];
        let x = modulate(&pg, Charset::De, 1200, fs, false);
        let de = Params { charset: Charset::De, ..Params::default() };
        let m = decode(&x, fs, de);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].clean_text(), "Grüße aus Köln");
        assert!(m[0].multimon_line().starts_with("POCSAG1200: Address:      42  Function: 3  Alpha:   Gr"), "{}", m[0].multimon_line());
        // numerisch erzwungen: Ziffernfolge statt Text
        let m = decode(&x, fs, Params { format: Format::Numeric, ..de });
        assert_eq!(m[0].kind, Kind::Numeric);
        // leere ausblenden
        let t = modulate(&[Page { addr: 7, func: 2, content: Content::Tone }], Charset::Us, 1200, fs, false);
        assert_eq!(decode(&t, fs, Params::default()).len(), 1);
        assert!(decode(&t, fs, Params { prune_empty: true, ..Params::default() }).is_empty());
    }

    /// Hilfsmittel für den Vergleich mit multimon-ng:
    /// POCSAG_RAW=/tmp/p.raw cargo test --release export_raw -- --ignored
    /// multimon-ng -t raw -a POCSAG512 -a POCSAG1200 -a POCSAG2400 /tmp/p.raw
    #[test]
    #[ignore]
    fn export_raw() {
        use std::io::Write;
        let mut f = std::fs::File::create(std::env::var("POCSAG_RAW").unwrap()).unwrap();
        for &baud in &BAUDS {
            let x = modulate(&pages(), Charset::Us, baud, 22050.0, false);
            for v in std::iter::repeat_n(0.0f32, 11025).chain(x) { f.write_all(&((v * 20000.0) as i16).to_le_bytes()).unwrap(); }
        }
        for v in std::iter::repeat_n(0i16, 22050) { f.write_all(&v.to_le_bytes()).unwrap(); }
    }

    /// Vergleich mit multimon-ng auf derselben Datei:
    /// POCSAG_IN=/tmp/x.raw (22050 Hz, s16le) cargo test --release vergleich_raw -- --ignored --nocapture
    #[test]
    #[ignore]
    fn vergleich_raw() {
        let b = std::fs::read(std::env::var("POCSAG_IN").unwrap()).unwrap();
        let x: Vec<f32> = b.chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect();
        let mut rx = Receiver::new(22050.0, Params::default());
        let mut out = vec![];
        for c in x.chunks(1000) { out.extend(rx.push(c)); }
        out.extend(rx.flush());
        for m in out { println!("RW {}", m.multimon_line()); }
        for (b, st) in BAUDS.iter().zip(rx.stats()) { println!("STAT {b}: {st:?}"); }
    }
}
#[cfg(test)]
mod demo {
    use super::*;
    /// Demo-Datei für Bildschirmfotos: POCSAG_DEMO=/tmp/demo.wav cargo test --release demo_wav -- --ignored
    #[test]
    #[ignore]
    fn demo_wav() {
        let fs = 48000.0;
        let mut w = hound::WavWriter::create(std::env::var("POCSAG_DEMO").unwrap(), hound::WavSpec { channels: 1, sample_rate: 48000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int }).unwrap();
        let sets: [(u32, Vec<Page>); 3] = [
            (1200, vec![Page { addr: 8, func: 3, content: Content::Alpha("XTIME=1027201125201125".into()) },
                        Page { addr: 2504, func: 3, content: Content::Alpha("DB0XYZ: Relais wieder QRV, 73".into()) }]),
            (1200, vec![Page { addr: 123456, func: 3, content: Content::Alpha("Hallo Peter, Sked heute 20 Uhr auf 145,500. 73 de DL1ABC".into()) },
                        Page { addr: 165000, func: 0, content: Content::Numeric("0815 4711".into()) }]),
            (512, vec![Page { addr: 1040, func: 1, content: Content::Tone }]),
        ];
        for (baud, pg) in sets {
            for v in std::iter::repeat_n(0.0, 24000).chain(modulate(&pg, Charset::De, baud, fs, false)) { w.write_sample((v * 12000.0) as i16).unwrap(); }
        }
        w.finalize().unwrap();
    }

    /// Aufnahme dekodieren: POCSAG_WAV=/pfad.wav cargo test --release vergleich_wav -- --ignored --nocapture
    #[test]
    #[ignore]
    fn vergleich_wav() {
        let mut rd = hound::WavReader::open(std::env::var("POCSAG_WAV").unwrap()).unwrap();
        let fs = rd.spec().sample_rate as f64;
        let ch = rd.spec().channels as usize;
        let x: Vec<f32> = rd.samples::<i16>().step_by(ch).map(|v| v.unwrap() as f32 / 32768.0).collect();
        let mut rx = Receiver::new(fs, Params::default());
        let mut out = vec![];
        for c in x.chunks(4096) { for m in rx.push(c) { out.push(m); } }
        out.extend(rx.flush());
        for m in &out { println!("RW v{} {}", m.variant, m.multimon_line()); }
        for (b, st) in BAUDS.iter().zip(rx.stats()) { println!("STAT {b}: {st:?}"); }
    }
}
