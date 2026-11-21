//! APRS: AFSK 1200 Baud (Bell 202, UKW/FM) und 300 Baud (1600/1800 Hz, Kurzwelle/USB), HDLC mit Bitstopfen und NRZI, AX.25-UI-Rahmen,
//! APRS-Formate (Position unkomprimiert/komprimiert/Mic-E, Nachrichten, Status, Objekte).

use std::f64::consts::TAU;

/// AFSK-Parameter: Baudrate, Mark-/Space-Ton, Durchlassbereich des Vorfilters
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Modem { pub baud: f64, pub mark: f64, pub space: f64, band: (f64, f64) }
/// UKW: Bell 202
pub const VHF: Modem = Modem { baud: 1200.0, mark: 1200.0, space: 2200.0, band: (800.0, 2700.0) };
/// Kurzwelle: 300 Baud, 1600/1800 Hz (wie Direwolf „MODEM 300“), Funkgerät in USB
pub const HF: Modem = Modem { baud: 300.0, mark: 1600.0, space: 1800.0, band: (1200.0, 2200.0) };
impl Modem {
    fn shifted(self, hz: f64) -> Modem { Modem { mark: self.mark + hz, space: self.space + hz, ..self } }
}
/// Zielfeld für Pakete dieser Software (APZxxx = experimentell laut APRS-Spezifikation)
pub const TOCALL: &str = "APZSST";

// ---------------- AX.25 ----------------

/// CRC-16/X.25 (wird niederwertiges Byte zuerst gesendet)
pub fn crc(data: &[u8]) -> u16 {
    let mut c: u16 = 0xffff;
    for &b in data {
        c ^= b as u16;
        for _ in 0..8 { c = if c & 1 != 0 { (c >> 1) ^ 0x8408 } else { c >> 1 }; }
    }
    !c
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub src: String,
    pub dst: String,
    /// Digipeater-Pfad; "*" am Ende = schon wiederholt
    pub path: Vec<String>,
    pub info: Vec<u8>,
}

fn split_call(c: &str) -> (String, u8) {
    let c = c.trim().trim_end_matches('*').to_uppercase();
    match c.split_once('-') {
        Some((a, s)) => (a.to_string(), s.parse().unwrap_or(0)),
        None => (c, 0),
    }
}
fn addr(c: &str, last: bool, top: u8) -> [u8; 7] {
    let (call, ssid) = split_call(c);
    let mut a = [b' ' << 1; 7];
    for (i, ch) in call.bytes().take(6).enumerate() { a[i] = ch << 1; }
    a[6] = top | 0x60 | ((ssid & 0x0f) << 1) | last as u8;
    a
}

impl Frame {
    /// Rahmen ohne Flags, mit FCS
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = vec![];
        b.extend(addr(&self.dst, false, 0x80));
        b.extend(addr(&self.src, self.path.is_empty(), 0x00));
        for (i, d) in self.path.iter().enumerate() { b.extend(addr(d, i + 1 == self.path.len(), if d.ends_with('*') { 0x80 } else { 0x00 })); }
        b.extend([0x03, 0xf0]);
        b.extend(&self.info);
        let c = crc(&b);
        b.extend([c as u8, (c >> 8) as u8]);
        b
    }
    pub fn from_bytes(b: &[u8]) -> Option<Frame> {
        if b.len() < 18 || crc(&b[..b.len() - 2]) != u16::from_le_bytes([b[b.len() - 2], b[b.len() - 1]]) { return None; }
        let b = &b[..b.len() - 2];
        let mut calls = vec![];
        let mut i = 0;
        loop {
            if i + 7 > b.len() || calls.len() > 10 { return None; }
            let a = &b[i..i + 7];
            let call: String = a[..6].iter().map(|c| (c >> 1) as char).collect::<String>().trim().to_string();
            let ssid = (a[6] >> 1) & 0x0f;
            let mut s = if ssid > 0 { format!("{call}-{ssid}") } else { call };
            if calls.len() >= 2 && a[6] & 0x80 != 0 { s.push('*'); }
            calls.push(s);
            i += 7;
            if a[6] & 1 != 0 { break; }
        }
        if calls.len() < 2 || i + 2 > b.len() || b[i] != 0x03 || b[i + 1] != 0xf0 { return None; }
        Some(Frame { dst: calls[0].clone(), src: calls[1].clone(), path: calls[2..].to_vec(), info: b[i + 2..].to_vec() })
    }
    /// Lesbare Form wie in TNC2/direwolf: SRC>DST,PATH:info
    pub fn monitor(&self) -> String {
        let mut s = format!("{}>{}", self.src, self.dst);
        for p in &self.path { s.push(','); s.push_str(p); }
        s.push(':');
        s.push_str(&String::from_utf8_lossy(&self.info));
        s
    }
}

// ---------------- Modulator ----------------

/// Rahmen als AFSK-Audio (mono, volle Aussteuerung): Vorlauf-Flags (txdelay), Rahmen, Nachlauf
#[allow(dead_code)]
pub fn modulate(frames: &[Frame], fs: f64, txdelay_ms: f64, m: Modem) -> Vec<f32> {
    let raw: Vec<Vec<u8>> = frames.iter().map(|f| f.to_bytes()).collect();
    modulate_raw(&raw, fs, txdelay_ms, m)
}

/// HDLC-Bitfolge (ohne NRZI): Vorlauf-Flags (txdelay), je Rahmen Bytes mit Bitstopfen und Nachlauf-Flags
pub fn hdlc_bits(frames: &[Vec<u8>], baud: f64, txdelay_ms: f64) -> Vec<u8> {
    let mut bits: Vec<u8> = vec![];
    let flag = |bits: &mut Vec<u8>| for k in 0..8 { bits.push((0x7e >> k) & 1); };
    for _ in 0..((txdelay_ms / 1000.0 * baud / 8.0).ceil() as usize).max(4) { flag(&mut bits); }
    for f in frames {
        let mut ones = 0;
        for &byte in f {
            for k in 0..8 {
                let bit = (byte >> k) & 1;
                bits.push(bit);
                if bit == 1 { ones += 1; if ones == 5 { bits.push(0); ones = 0; } } else { ones = 0; }
            }
        }
        // Nachlauf (TXTAIL) mindestens 3 Flags bzw. 50 ms
        for _ in 0..((0.05 * baud / 8.0).ceil() as usize).max(3) { flag(&mut bits); }
    }
    bits
}

/// fertige Rahmen (Bytes mit FCS) als AFSK-Audio
pub fn modulate_raw(frames: &[Vec<u8>], fs: f64, txdelay_ms: f64, m: Modem) -> Vec<f32> {
    let Modem { baud, mark, space, .. } = m;
    let bits = hdlc_bits(frames, baud, txdelay_ms);
    // NRZI: 0 = Tonwechsel, 1 = Ton bleibt
    let mut tone = true; // Mark
    let sp = fs / baud;
    let mut out = Vec::with_capacity((bits.len() as f64 * sp) as usize + 1);
    let mut ph = 0.0f64;
    for (i, &b) in bits.iter().enumerate() {
        if b == 0 { tone = !tone; }
        let end = ((i + 1) as f64 * sp).round() as usize;
        let w = TAU * if tone { mark } else { space } / fs;
        while out.len() < end {
            ph = (ph + w) % TAU;
            out.push(ph.sin() as f32);
        }
    }
    out
}

// ---------------- Demodulator ----------------

/// Bandpass (UKW 800–2700 Hz), Korrelator über 1,35 Bitlängen (KW: 1,5) je Ton, getrennte Pegelregelung je Ton (gleicht die
/// FM-Deemphase aus) und mehrere Entscheider mit verschiedener Ton-Gewichtung (wie direwolf);
/// je Entscheider eigene PLL, NRZI- und HDLC-Dekodierung. Doppelt gelesene Rahmen werden verworfen.
pub struct Demod {
    buf: Vec<[f64; 4]>,
    sums: [f64; 4],
    pos: usize,
    ph: [(f64, f64); 2],
    rot: [(f64, f64); 2],
    peak: [f64; 2],
    decay: f64,
    step: i32,
    pre: [crate::sstv::Biquad; 3],
    slicers: Vec<Slicer>,
}

pub(crate) struct Slicer { gain: f64, pll: i32, last_level: bool, last_tone: bool, ones: u32, bits: Vec<u8>, in_frame: bool,
    /// Träger erkannt: zwei Flags direkt hintereinander, bis zum nächsten Abbruch (7 Einsen) – für die Kanalbelegung (DCD)
    pub(crate) carrier: bool, since_flag: u32 }

/// Gewichte Space gegenüber Mark (logarithmisch verteilt)
const SLICER_GAINS: [f64; 7] = [0.5, 0.63, 0.8, 1.0, 1.25, 1.6, 2.0];

impl Demod {
    fn new(fs: f64, m: Modem) -> Self {
        let Modem { baud, mark, space, band } = m;
        // Fenster etwas länger als ein Bit: gegen Direwolfs Testsignale (gen_packets -n 100) am besten
        // mindestens eine Periode des Tonabstands, damit Mark und Space sauber getrennt werden (KW: 200 Hz -> 1,5 Bit)
        let n = (fs * (1.35 / baud).max(1.0 / (space - mark).abs())).round() as usize;
        use crate::sstv::Biquad;
        Demod {
            pre: [Biquad::lowpass(fs, band.1, 0.5412), Biquad::lowpass(fs, band.1, 1.3066), Biquad::lowpass(fs, band.0, 0.7071)],
            buf: vec![[0.0; 4]; n], sums: [0.0; 4], pos: 0, ph: [(1.0, 0.0); 2],
            rot: [mark, space].map(|f| { let w = TAU * f / fs; (w.cos(), w.sin()) }), peak: [1e-6; 2],
            decay: (-1.0 / (0.05 * fs)).exp(), // Pegelregelung fällt in ~50 ms ab
            step: ((baud / fs) * 4294967296.0) as i64 as i32,
            slicers: SLICER_GAINS.iter().map(|&gain| Slicer::new(gain)).collect(),
        }
    }

    /// Audio verarbeiten, Bytes fehlerfrei empfangener Rahmen anhängen
    fn push(&mut self, x: &[f32], out: &mut Vec<Vec<u8>>) {
        for &v in x {
            let a = self.pre[0].run(v as f64);
            let a = self.pre[1].run(a);
            let v = a - self.pre[2].run(a); // Hochpass = Signal minus Tiefpass
            // Mischer: Zeiger (cos, sin) je Ton weiterdrehen statt sin/cos zu rechnen; Betrag auf 1 nachführen
            for k in 0..2 {
                let (c, si) = self.ph[k];
                let (rc, rs) = self.rot[k];
                let (c, si) = (c * rc - si * rs, c * rs + si * rc);
                let g = 1.5 - 0.5 * (c * c + si * si);
                self.ph[k] = (c * g, si * g);
            }
            let cur = [v * self.ph[0].0, v * self.ph[0].1, v * self.ph[1].0, v * self.ph[1].1];
            let old = std::mem::replace(&mut self.buf[self.pos], cur);
            for k in 0..4 { self.sums[k] += cur[k] - old[k]; }
            self.pos = (self.pos + 1) % self.buf.len();
            let m = self.sums[0].hypot(self.sums[1]);
            let s = self.sums[2].hypot(self.sums[3]);
            for (k, a) in [m, s].into_iter().enumerate() { self.peak[k] = if a > self.peak[k] { a } else { self.peak[k] * self.decay }; }
            let (mn, sn) = (m / self.peak[0], s / self.peak[1]);
            for sl in self.slicers.iter_mut() {
                let level = mn > sn * sl.gain; // true = Mark
                // PLL: Bit in der Mitte abtasten, bei jedem Pegelwechsel nachziehen
                let prev = sl.pll;
                sl.pll = sl.pll.wrapping_add(self.step);
                if prev > 0 && sl.pll < 0 {
                    if let Some(b) = sl.bit(level) { out.push(b); }
                }
                if level != sl.last_level {
                    sl.pll = (sl.pll as f64 * 0.92) as i32;
                    sl.last_level = level;
                }
            }
        }
    }
}

/// Empfänger: auf UKW ein Demodulator; auf Kurzwelle sieben, je 30 Hz gegeneinander versetzt (wie Direwolf
/// „7@30“), damit auch um bis zu ±100 Hz verstimmte Stationen gelesen werden.
pub struct Receiver { demods: Vec<Demod>, n: u64, recent: Vec<(u64, Vec<u8>)>, fs: f64, pub modem: Modem }

impl Receiver {
    pub fn new(fs: f64, m: Modem) -> Self {
        let offsets: &[f64] = if m.baud < 1000.0 { &[-90.0, -60.0, -30.0, 0.0, 30.0, 60.0, 90.0] } else { &[0.0] };
        Receiver { demods: offsets.iter().map(|&o| Demod::new(fs, m.shifted(o))).collect(), n: 0, recent: vec![], fs, modem: m }
    }
    /// Audio verarbeiten, fehlerfrei empfangene APRS-Rahmen zurückgeben
    pub fn push(&mut self, x: &[f32]) -> Vec<Frame> {
        self.push_raw(x, 3.0).iter().filter_map(|b| Frame::from_bytes(b)).collect()
    }
    /// Audio verarbeiten, alle Rahmen mit gültiger Prüfsumme als Bytes (mit FCS) zurückgeben;
    /// gleiche Rahmen innerhalb von `dedup_s` nur einmal (mehrere Entscheider und Demodulatoren)
    pub fn push_raw(&mut self, x: &[f32], dedup_s: f64) -> Vec<Vec<u8>> {
        let mut out = vec![];
        for d in &mut self.demods { d.push(x, &mut out); }
        self.n += x.len() as u64;
        let (n, win) = (self.n, (dedup_s * self.fs) as u64);
        self.recent.retain(|(t, _)| n - t < win);
        let mut frames = vec![];
        for bytes in out {
            if self.recent.iter().any(|(_, b)| *b == bytes) { continue; }
            self.recent.push((n, bytes.clone()));
            frames.push(bytes);
        }
        frames
    }
    /// Kanal belegt (ein Entscheider hört gerade einen Rahmen)
    pub fn busy(&self) -> bool { self.demods.iter().any(|d| d.slicers.iter().any(|s| s.carrier)) }
}

/// KW-Skimmer: Demodulatoren für 300 Baud im Abstand von 30 Hz über den ganzen Durchlassbereich
/// (Tonmitte 500–2600 Hz). Das Audio wird dafür auf etwa 8 kHz heruntergetastet, damit auch ein
/// sparsamer Rechner mitkommt. Zu jedem Rahmen kommt die Tonmitte (Hz im Audio) mit.
pub struct Skimmer { lp: [crate::sstv::Biquad; 2], dec: usize, phase: usize, demods: Vec<(f64, Demod)>, n: u64, recent: Vec<(u64, Vec<u8>)>, fs: f64 }

pub const SKIM_RANGE: (f64, f64) = (500.0, 2600.0);

impl Skimmer {
    pub fn new(fs: f64) -> Self {
        use crate::sstv::Biquad;
        let dec = (fs / 8000.0).floor().max(1.0) as usize;
        let f2 = fs / dec as f64;
        let mut demods = vec![];
        let mut c = SKIM_RANGE.0;
        while c <= SKIM_RANGE.1 {
            let m = Modem { baud: 300.0, mark: c - 100.0, space: c + 100.0, band: ((c - 500.0).max(100.0), (c + 500.0).min(f2 / 2.0 - 300.0)) };
            demods.push((c, Demod::new(f2, m)));
            c += 30.0;
        }
        Skimmer { lp: [Biquad::lowpass(fs, 3200.0, 0.5412), Biquad::lowpass(fs, 3200.0, 1.3066)], dec, phase: 0, demods, n: 0, recent: vec![], fs: f2 }
    }
    /// Audio verarbeiten; liefert fehlerfreie Rahmen mit ihrer Tonmitte
    pub fn push(&mut self, x: &[f32]) -> Vec<(Frame, f64)> {
        let mut y = Vec::with_capacity(x.len() / self.dec + 1);
        for &v in x {
            let a = self.lp[0].run(v as f64); let a = self.lp[1].run(a);
            self.phase += 1;
            if self.phase == self.dec { self.phase = 0; y.push(a as f32); }
        }
        let mut hits: Vec<(Vec<u8>, f64)> = vec![];
        for (c, d) in &mut self.demods {
            let mut out = vec![];
            d.push(&y, &mut out);
            hits.extend(out.into_iter().map(|b| (b, *c)));
        }
        self.n += y.len() as u64;
        let (n, win) = (self.n, 3 * self.fs as u64);
        self.recent.retain(|(t, _)| n - t < win);
        let mut frames = vec![];
        for (bytes, _) in &hits {
            if self.recent.iter().any(|(_, b)| b == bytes) { continue; }
            // Tonmitte = Mittel aller Demodulatoren, die diesen Rahmen gelesen haben
            let cs: Vec<f64> = hits.iter().filter(|(b, _)| b == bytes).map(|h| h.1).collect();
            if let Some(f) = Frame::from_bytes(bytes) { frames.push((f, cs.iter().sum::<f64>() / cs.len() as f64)); }
            self.recent.push((n, bytes.clone()));
        }
        frames
    }
}

impl Slicer {
    pub(crate) fn new(gain: f64) -> Self {
        Slicer { gain, pll: 0, last_level: false, last_tone: true, ones: 0, bits: vec![], in_frame: false, carrier: false, since_flag: 0 }
    }
    /// liefert die Bytes eines Rahmens mit gültiger Prüfsumme
    pub(crate) fn bit(&mut self, tone: bool) -> Option<Vec<u8>> {
        let bit = (tone == self.last_tone) as u8; // NRZI
        self.last_tone = tone;
        self.since_flag = self.since_flag.saturating_add(1);
        if bit == 1 {
            self.ones += 1;
            if self.ones >= 7 { self.in_frame = false; self.carrier = false; self.bits.clear(); return None; } // Abbruch
            self.bits.push(1);
            return None;
        }
        let ones = std::mem::replace(&mut self.ones, 0);
        if ones == 5 { return None; } // gestopftes Bit
        if ones == 6 { // Flag: Rahmen abschließen
            if self.since_flag == 8 { self.carrier = true; }
            self.since_flag = 0;
            let mut r = None;
            if self.in_frame && self.bits.len() >= 7 {
                let n = self.bits.len() - 7; // die ersten 7 Flag-Bits gehören nicht zum Rahmen
                if n % 8 == 0 && n >= 17 * 8 { // kürzester Rahmen: zwei Adressen, Steuerbyte, FCS (z. B. RR)
                    let bytes: Vec<u8> = self.bits[..n].chunks(8).map(|c| c.iter().enumerate().fold(0, |a, (k, b)| a | (b << k))).collect();
                    let l = bytes.len();
                    if crc(&bytes[..l - 2]) == u16::from_le_bytes([bytes[l - 2], bytes[l - 1]]) { r = Some(bytes); }
                }
            }
            self.bits.clear();
            self.in_frame = true;
            return r;
        }
        self.bits.push(0);
        if self.bits.len() > 8 * 400 { self.bits.clear(); self.in_frame = false; }
        None
    }
}

// ---------------- APRS-Inhalte ----------------

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Position { lat: f64, lon: f64, symbol: (char, char), comment: String },
    Object { name: String, lat: f64, lon: f64, symbol: (char, char), comment: String },
    Message { to: String, text: String, id: Option<String> },
    Ack { to: String, id: String },
    Status(String),
    Other(String),
}

fn uncompressed(s: &[u8]) -> Option<(f64, f64, (char, char), String)> {
    if s.len() < 19 { return None; }
    let txt = |a: usize, b: usize| String::from_utf8_lossy(&s[a..b]).replace(' ', "0");
    let lat_s = txt(0, 7);
    let lon_s = txt(9, 17);
    let lat = lat_s[..2].parse::<f64>().ok()? + lat_s[2..].parse::<f64>().ok()? / 60.0;
    let lon = lon_s[..3].parse::<f64>().ok()? + lon_s[3..].parse::<f64>().ok()? / 60.0;
    let lat = match s[7] { b'N' => lat, b'S' => -lat, _ => return None };
    let lon = match s[17] { b'E' => lon, b'W' => -lon, _ => return None };
    Some((lat, lon, (s[8] as char, s[18] as char), String::from_utf8_lossy(&s[19..]).trim().to_string()))
}
fn compressed(s: &[u8]) -> Option<(f64, f64, (char, char), String)> {
    if s.len() < 13 || !s[1..9].iter().all(|c| (33..=124).contains(c)) { return None; }
    let b91 = |b: &[u8]| b.iter().fold(0f64, |a, c| a * 91.0 + (*c as f64 - 33.0));
    let lat = 90.0 - b91(&s[1..5]) / 380926.0;
    let lon = -180.0 + b91(&s[5..9]) / 190463.0;
    Some((lat, lon, (s[0] as char, s[9] as char), String::from_utf8_lossy(&s[13..]).trim().to_string()))
}
/// Position nach dem Datentyp-Zeichen (unkomprimiert oder komprimiert)
fn position(s: &[u8]) -> Option<(f64, f64, (char, char), String)> {
    if s.first().is_some_and(|c| c.is_ascii_digit()) { uncompressed(s) } else { compressed(s) }
}
/// Mic-E: Breite im Zielfeld, Länge und Symbol im Infofeld
fn mic_e(dst: &str, info: &[u8]) -> Option<(f64, f64, (char, char), String)> {
    let d: Vec<u8> = split_call(dst).0.bytes().collect();
    if d.len() < 6 || info.len() < 9 { return None; }
    let digit = |c: u8| match c { b'0'..=b'9' => Some(c - b'0'), b'A'..=b'J' => Some(c - b'A'), b'P'..=b'Y' => Some(c - b'P'), b'K' | b'L' | b'Z' => Some(0), _ => None };
    let dg: Vec<u8> = d[..6].iter().map(|&c| digit(c)).collect::<Option<_>>()?;
    let mut lat = (dg[0] * 10 + dg[1]) as f64 + ((dg[2] * 10 + dg[3]) as f64 + (dg[4] * 10 + dg[5]) as f64 / 100.0) / 60.0;
    if !(b'P'..=b'Z').contains(&d[3]) { lat = -lat; } // Süd
    let mut lo = info[1] as i32 - 28;
    if (b'P'..=b'Z').contains(&d[4]) { lo += 100; }
    if (180..=189).contains(&lo) { lo -= 80; } else if (190..=199).contains(&lo) { lo -= 190; }
    let mut mi = info[2] as i32 - 28;
    if mi >= 60 { mi -= 60; }
    let hu = info[3] as i32 - 28;
    let mut lon = lo as f64 + (mi as f64 + hu as f64 / 100.0) / 60.0;
    if (b'P'..=b'Z').contains(&d[5]) { lon = -lon; } // West
    let comment = String::from_utf8_lossy(&info[9..]).trim().to_string();
    Some((lat, lon, (info[8] as char, info[7] as char), comment))
}

// ---------------- APRS-IS (Internet) ----------------

/// Passcode für die Anmeldung bei APRS-IS (öffentlicher Algorithmus, nur aus dem Rufzeichen ohne SSID)
pub fn passcode(call: &str) -> u16 {
    let c: Vec<u8> = call.split('-').next().unwrap_or("").to_uppercase().bytes().collect();
    let mut h: u16 = 0x73e2;
    for p in c.chunks(2) {
        h ^= (p[0] as u16) << 8;
        if let Some(&b) = p.get(1) { h ^= b as u16; }
    }
    h & 0x7fff
}

/// Zeile für APRS-IS aus einem auf Funk gehörten Rahmen (Empfangs-iGate, „qAR“), oder None, wenn er nicht
/// weitergeleitet werden darf: Pfad mit TCPIP/TCPXX/NOGATE/RFONLY, Abfragen („?“), Drittverkehr („}“), leer
pub fn igate_line(f: &Frame, igate: &str) -> Option<String> {
    let banned = |p: &str| { let p = p.trim_end_matches('*').to_uppercase(); ["TCPIP", "TCPXX", "NOGATE", "RFONLY"].contains(&p.as_str()) };
    if f.path.iter().any(|p| banned(p)) || matches!(f.info.first(), None | Some(b'?') | Some(b'}')) { return None; }
    // APRS-IS ist zeilenweise: bei CR/LF abschneiden
    let info = String::from_utf8_lossy(&f.info);
    let info = info.split(['\r', '\n']).next().unwrap_or("");
    if info.is_empty() { return None; }
    let mut s = format!("{}>{}", f.src, f.dst);
    for p in &f.path { s.push(','); s.push_str(p); }
    Some(format!("{s},qAR,{igate}:{info}"))
}

/// Weg für Nachrichten an mich: über Funk, übers Internet (APRS-IS) oder beides
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RxWeg { Funk, Internet, Beides }
/// Weg für eigene Nachrichten und Bestätigungen; Automatisch = auf dem Weg, auf dem die Gegenstelle zuletzt kam
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TxWeg { Auto, Funk, Internet, Beides }

impl RxWeg {
    pub const ALL: [(RxWeg, &'static str, &'static str); 3] = [(RxWeg::Funk, "funk", "Funk"), (RxWeg::Internet, "internet", "Internet"), (RxWeg::Beides, "beides", "Funk + Internet")];
    /// Nachricht annehmen, die über diesen Weg kam (`is` = über APRS-IS)?
    pub fn accepts(self, is: bool) -> bool { match self { RxWeg::Funk => !is, RxWeg::Internet => is, RxWeg::Beides => true } }
    pub fn internet(self) -> bool { self != RxWeg::Funk }
}
impl TxWeg {
    pub const ALL: [(TxWeg, &'static str, &'static str); 4] = [(TxWeg::Auto, "auto", "Automatisch"), (TxWeg::Funk, "funk", "Funk"), (TxWeg::Internet, "internet", "Internet"), (TxWeg::Beides, "beides", "Funk + Internet")];
    /// über welche Wege senden: (Funk, Internet); `partner_is` = die Gegenstelle kam zuletzt übers Internet
    pub fn paths(self, partner_is: bool) -> (bool, bool) {
        match self { TxWeg::Auto => (!partner_is, partner_is), TxWeg::Funk => (true, false), TxWeg::Internet => (false, true), TxWeg::Beides => (true, true) }
    }
}

/// Ist eine Nachricht an mich gerichtet? Genau an mein APRS-Rufzeichen (mit SSID) oder an das Grundrufzeichen
/// ohne SSID – so adressieren viele Online-Dienste und Apps („DL4RE“ statt „DL4RE-9“).
pub fn addressed_to_me(to: &str, me: &str) -> bool {
    let to = to.trim();
    !me.is_empty() && (to.eq_ignore_ascii_case(me) || to.eq_ignore_ascii_case(me.split('-').next().unwrap_or(me)))
}

/// Drittverkehr („}SRC>DST,PFAD:info“): so senden iGates Pakete aus dem Internet auf Funk aus, z. B. Nachrichten,
/// die jemand über aprs.fi oder eine App an dich schickt. Liefert das innere Paket.
pub fn third_party(f: &Frame) -> Option<Frame> {
    let inner = f.info.strip_prefix(b"}")?;
    parse_tnc2(&String::from_utf8_lossy(inner))
}

/// Nachrichtenkennung im Reply-Ack-Format „MM}AA“ zerlegen: eigene Kennung MM und ggf. mitgeschickte Bestätigung AA
pub fn split_id(id: &str) -> (&str, Option<&str>) {
    match id.split_once('}') { Some((m, a)) if !a.is_empty() => (m, Some(a)), Some((m, _)) => (m, None), None => (id, None) }
}

/// Zeile von APRS-IS (SRC>DST,PFAD:info) lesen; Serverhinweise („#“) ergeben None
pub fn parse_tnc2(line: &str) -> Option<Frame> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.starts_with('#') { return None; }
    let (head, info) = line.split_once(':')?;
    let (src, rest) = head.split_once('>')?;
    let mut parts = rest.split(',');
    let dst = parts.next()?.to_string();
    if src.is_empty() || dst.is_empty() { return None; }
    Some(Frame { src: src.to_string(), dst, path: parts.map(|p| p.to_string()).collect(), info: info.as_bytes().to_vec() })
}

/// Wetterdaten aus einem Kommentar („DDD/SSSgGGGtTTTrRRRpPPPhHHbBBBBB“) bzw. einem Wetterbericht („_MMDDhhmmcDDDsSSS…“)
#[derive(Debug, Default, PartialEq, Clone)]
pub struct Wx { pub temp_c: Option<f64>, pub wind_kmh: Option<f64>, pub dir: Option<f64>, pub gust_kmh: Option<f64>, pub hum: Option<u32>, pub hpa: Option<f64>, pub rain_mm: Option<f64>,
    /// Regen der letzten 24 h bzw. seit Mitternacht (mm), Sonneneinstrahlung (W/m²)
    pub rain_24h: Option<f64>, pub rain_mid: Option<f64>, pub lum: Option<f64>,
    /// Neuschnee 24 h (cm), Rohzählerstand des Regenmessers, Akku-/Versorgungsspannung (V)
    pub snow_cm: Option<f64>, pub rain_raw: Option<u32>, pub volt: Option<f64>,
    /// Software und Wetterstation aus der Kennung am Ende (z. B. „eCumulusDsVP“)
    pub station: Option<String> }

/// Wetterdaten lesen; dazu die Zahl der verbrauchten Bytes (Rest ist Kommentar)
pub fn parse_wx_len(s: &str) -> Option<(Wx, usize)> {
    let b = s.as_bytes();
    let mut w = Wx::default();
    let mut i = 0;
    // Feld: Leerzeichen und Punkte heißen „kein Wert“
    let num = |v: &str| { let t = v.trim().trim_start_matches('.'); if t.is_empty() || t.chars().all(|c| c == '.') { None } else { t.parse::<f64>().ok() } };
    // Wind als DDD/SSS am Anfang (Position mit Wettersymbol); danach ist „s“ Schnee statt Windgeschwindigkeit
    let lead = b.len() >= 7 && b[3] == b'/' && b[..3].iter().chain(&b[4..7]).all(|c| c.is_ascii_digit() || *c == b'.' || *c == b' ');
    if lead {
        w.dir = num(&s[0..3]);
        w.wind_kmh = num(&s[4..7]).map(|v| v * 1.609);
        i = 7;
    }
    let mut n = 0;
    while i < b.len() {
        let key = b[i] as char;
        let len = match key { 'c' | 's' | 'g' | 't' | 'r' | 'p' | 'P' | 'L' | 'l' | '#' | 'V' => 3, 'h' => 2, 'b' => 5, _ => break };
        if i + 1 + len > b.len() { break; }
        let v = &s[i + 1..i + 1 + len];
        // Feldinhalt muss nach Zahl oder Lücke aussehen, sonst ist es Kommentartext
        if !v.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ' ' || c == '-') { break; }
        let x = num(v);
        match key {
            'c' => w.dir = x,
            's' if lead => w.snow_cm = x.map(|v| v * 2.54),
            's' => w.wind_kmh = x.map(|v| v * 1.609),
            'g' => w.gust_kmh = x.map(|v| v * 1.609),
            't' => w.temp_c = x.map(|f| (f - 32.0) * 5.0 / 9.0), 'r' => w.rain_mm = x.map(|v| v * 0.254),
            'p' => w.rain_24h = x.map(|v| v * 0.254), 'P' => w.rain_mid = x.map(|v| v * 0.254),
            'L' => w.lum = x, 'l' => w.lum = x.map(|v| v + 1000.0),
            '#' => w.rain_raw = x.map(|v| v as u32), 'V' => w.volt = x.map(|v| v / 10.0),
            'h' => w.hum = x.map(|v| if v == 0.0 { 100 } else { v as u32 }), 'b' => w.hpa = x.map(|v| v / 10.0), _ => {}
        }
        n += 1;
        i += 1 + len;
    }
    if !(n >= 1 && (w.temp_c.is_some() || w.wind_kmh.is_some() || w.hpa.is_some() || w.hum.is_some())) { return None; }
    // Kennung von Software und Station direkt hinter den Daten (ohne Leerzeichen)
    let rest = &s[i..];
    let tok: &str = rest.split(char::is_whitespace).next().unwrap_or("");
    if (2..=16).contains(&tok.len()) && tok.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && tok.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        w.station = Some(wx_station(tok));
        i += tok.len();
    }
    Some((w, i))
}

/// Kennung wie „eCumulusDsVP“, „xDvs“, „wRSW“, „U2k“ in Klartext
fn wx_station(t: &str) -> String {
    const UNITS: &[(&str, &str)] = &[("DsVP", "Davis Vantage Pro"), ("DsWMR", "Oregon WMR"), ("DsWS", "Fine Offset/WS"), ("Dvs", "Davis"),
        ("HKT", "Heathkit"), ("PIC", "PIC-Wetterstation"), ("RSW", "Radio Shack"), ("U-II", "Ultimeter II"), ("U2R", "Ultimeter (Fernabfrage)"),
        ("U2kr", "Ultimeter-Logger"), ("U2k", "Ultimeter 500/2000"), ("U5", "Ultimeter 500"), ("Upkm", "Ultimeter (Paketmodus)")];
    const SW: &[(&str, &str)] = &[("Cumulus", "Cumulus"), ("weewx", "WeeWX"), ("WeeWX", "WeeWX"), ("meteo", "Meteohub"), ("WD", "Weather Display")];
    let mut parts = vec![];
    let mut rest = t;
    if let Some((k, n)) = SW.iter().find(|(k, _)| t.contains(k)) { parts.push(n.to_string()); rest = t.split(k).nth(1).unwrap_or(""); }
    else {
        let sw = match t.as_bytes()[0] { b'd' => Some("APRSdos"), b'M' => Some("MacAPRS"), b'P' => Some("pocketAPRS"), b'S' => Some("APRS+SA"),
            b'W' | b'w' => Some("WinAPRS"), b'X' | b'x' => Some("X-APRS/Xastir"), _ => None };
        if let Some(sw) = sw { parts.push(sw.to_string()); rest = &t[1..]; }
    }
    if let Some((_, n)) = UNITS.iter().find(|(k, _)| rest.starts_with(k) || t == *k) { parts.push(n.to_string()); }
    if parts.is_empty() { format!("Kennung „{t}“") } else { format!("{} („{t}“)", parts.join(", ")) }
}

pub fn parse_wx(s: &str) -> Option<Wx> { parse_wx_len(s).map(|r| r.0) }

/// Taupunkt (Magnus-Formel)
pub fn dew_point(t: f64, rh: u32) -> f64 {
    let (a, b) = (17.62, 243.12);
    let g = (rh.max(1) as f64 / 100.0).ln() + a * t / (b + t);
    b * g / (a - g)
}
/// gefühlte Temperatur: Windchill bei Kälte und Wind, Hitzeindex bei Wärme und Feuchte, sonst None
pub fn feels_like(t: f64, kmh: Option<f64>, rh: Option<u32>) -> Option<(f64, &'static str)> {
    if let Some(v) = kmh.filter(|v| t <= 10.0 && *v > 4.8) {
        let p = v.powf(0.16);
        return Some((13.12 + 0.6215 * t - 11.37 * p + 0.3965 * t * p, "Windchill"));
    }
    if let Some(h) = rh.filter(|h| t >= 26.7 && *h >= 40) {
        let (f, r) = (t * 9.0 / 5.0 + 32.0, h as f64);
        let hi = -42.379 + 2.04901523 * f + 10.14333127 * r - 0.22475541 * f * r - 6.83783e-3 * f * f - 5.481717e-2 * r * r
            + 1.22874e-3 * f * f * r + 8.5282e-4 * f * r * r - 1.99e-6 * f * f * r * r;
        return Some(((hi - 32.0) * 5.0 / 9.0, "Hitzeindex"));
    }
    None
}
/// Beaufort-Stufe und Bezeichnung aus km/h
pub fn beaufort(kmh: f64) -> (u32, &'static str) {
    const N: [&str; 13] = ["Windstille", "leiser Zug", "leichte Brise", "schwache Brise", "mäßige Brise", "frische Brise", "starker Wind",
        "steifer Wind", "stürmischer Wind", "Sturm", "schwerer Sturm", "orkanartiger Sturm", "Orkan"];
    const MAX: [f64; 12] = [1.0, 5.0, 11.0, 19.0, 28.0, 38.0, 49.0, 61.0, 74.0, 88.0, 102.0, 117.0];
    let b = MAX.iter().position(|m| kmh <= *m).unwrap_or(12);
    (b as u32, N[b])
}

impl Wx {
    pub fn text(&self) -> String {
        let mut p = vec![];
        if let Some(t) = self.temp_c { p.push(format!("{t:.0} °C")); }
        if let Some(v) = self.wind_kmh {
            let mut s = format!("Wind {v:.0} km/h");
            if let Some(d) = self.dir.filter(|_| v > 0.0) { s += &format!(" aus {}", compass(d)); }
            if let Some(g) = self.gust_kmh.filter(|g| *g > v) { s += &format!(", Böen {g:.0}"); }
            p.push(s);
        }
        if let Some(h) = self.hum { p.push(format!("{h} %")); }
        if let Some(b) = self.hpa { p.push(format!("{b:.0} hPa")); }
        if let Some(r) = self.rain_mm.filter(|r| *r > 0.0) { p.push(format!("Regen {r:.1} mm/h")); }
        p.join(" · ")
    }
}

pub fn parse(f: &Frame) -> Body {
    let i = &f.info;
    let other = || Body::Other(String::from_utf8_lossy(i).to_string());
    let Some(&t) = i.first() else { return other() };
    let pos = match t {
        b'!' | b'=' => position(&i[1..]),
        b'/' | b'@' if i.len() > 8 => position(&i[8..]),
        b'`' | b'\'' | 0x1c | 0x1d => mic_e(&f.dst, i),
        _ => None,
    };
    if let Some((lat, lon, symbol, comment)) = pos { return Body::Position { lat, lon, symbol, comment }; }
    match t {
        b':' if i.len() >= 11 && i[10] == b':' => {
            let to = String::from_utf8_lossy(&i[1..10]).trim().to_string();
            let text = String::from_utf8_lossy(&i[11..]).to_string();
            if let Some(id) = text.strip_prefix("ack") { return Body::Ack { to, id: id.trim().to_string() }; }
            match text.rsplit_once('{') {
                Some((t, id)) => Body::Message { to, text: t.to_string(), id: Some(id.trim_end_matches('}').trim().to_string()) },
                None => Body::Message { to, text, id: None },
            }
        }
        b'>' => Body::Status(String::from_utf8_lossy(&i[1..]).trim().to_string()),
        b';' if i.len() >= 18 => {
            let name = String::from_utf8_lossy(&i[1..10]).trim().to_string();
            match position(&i[18..]) {
                Some((lat, lon, symbol, comment)) => Body::Object { name, lat, lon, symbol, comment },
                None => other(),
            }
        }
        _ => other(),
    }
}

// ---------------- Erzeugen ----------------

fn lat_str(lat: f64) -> String {
    let a = lat.abs();
    let (d, m) = (a.trunc(), (a.fract() * 60.0 * 100.0).round() / 100.0);
    let (d, m) = if m >= 60.0 { (d + 1.0, 0.0) } else { (d, m) };
    format!("{:02}{:05.2}{}", d as u32, m, if lat >= 0.0 { 'N' } else { 'S' })
}
fn lon_str(lon: f64) -> String {
    let a = lon.abs();
    let (d, m) = (a.trunc(), (a.fract() * 60.0 * 100.0).round() / 100.0);
    let (d, m) = if m >= 60.0 { (d + 1.0, 0.0) } else { (d, m) };
    format!("{:03}{:05.2}{}", d as u32, m, if lon >= 0.0 { 'E' } else { 'W' })
}
/// Positionsbake mit Nachrichtenfähigkeit ('=')
pub fn position_info(lat: f64, lon: f64, symbol: (char, char), comment: &str) -> Vec<u8> {
    format!("={}{}{}{}{}", lat_str(lat), symbol.0, lon_str(lon), symbol.1, comment).into_bytes()
}
/// Objekt: „;NAME_____*DDHHMMz“ + Position; `live` = false löscht es bei den anderen
pub fn object_info(name: &str, live: bool, lat: f64, lon: f64, symbol: (char, char), comment: &str, utc: i64) -> Vec<u8> {
    let t = chrono::DateTime::from_timestamp(utc, 0).unwrap_or_default();
    let n: String = name.chars().take(9).collect();
    format!(";{n:<9}{}{}{}{}{}{}{}", if live { '*' } else { '_' }, t.format("%d%H%Mz"), lat_str(lat), symbol.0, lon_str(lon), symbol.1, comment).into_bytes()
}

/// Höhe für den Kommentar: „/A=001234“ (Fuß)
pub fn alt_ext(m: f64) -> String { format!("/A={:06}", (m / 0.3048).round().clamp(-99999.0, 999999.0) as i64) }

/// Mic-E-Statusmeldungen (Bitmuster A/B/C) – 7 = Außer Dienst, 6 = Unterwegs, 0 = Notfall
pub const MICE_OFF_DUTY: u8 = 7;
pub const MICE_EN_ROUTE: u8 = 6;

/// Mic-E kodieren (APRS 1.0.1 Kap. 10): Ziel (Breite, Status, N/S, Längen-Offset, O/W) und Infofeld
/// (Länge, Tempo/Kurs, Symbol, optional Höhe „xxx}“, Kommentar)
pub fn mice_encode(lat: f64, lon: f64, course: Option<f64>, speed_kmh: Option<f64>, symbol: (char, char), alt_m: Option<f64>, status: u8, comment: &str) -> (String, Vec<u8>) {
    // Breite: sechs Ziffern DDMMmm
    let a = lat.abs();
    let mut dd = a.trunc() as u32;
    let mut mm = ((a.fract() * 60.0) * 100.0).round() as u32;
    if mm >= 6000 { dd += 1; mm -= 6000; }
    let digits = [dd / 10, dd % 10, mm / 1000, mm / 100 % 10, mm / 10 % 10, mm % 10];
    let o = lon.abs();
    let mut ld = o.trunc() as u32;
    let mut lm = ((o.fract() * 60.0) * 100.0).round() as u32;
    if lm >= 6000 { ld += 1; lm -= 6000; }
    let lon_off = !(10..=99).contains(&ld);
    let mut dst = String::new();
    for (k, &dg) in digits.iter().enumerate() {
        let bit = match k {
            0..=2 => (status >> (2 - k)) & 1 == 1, // Meldung A, B, C
            3 => lat >= 0.0,                       // Nord
            4 => lon_off,                          // Länge +100
            _ => lon < 0.0,                        // West
        };
        dst.push(if bit { (b'P' + dg as u8) as char } else { (b'0' + dg as u8) as char });
    }
    // Länge: Grad mit Offset
    let d = if ld < 10 { ld + 90 } else if ld <= 99 { ld } else if ld <= 109 { ld - 100 + 80 } else { ld - 100 };
    let (m, h) = (lm / 100, lm % 100);
    let m = if m < 10 { m + 60 } else { m };
    let sp = speed_kmh.map_or(0, |v| (v / 1.852).round() as u32).min(799);
    let co = course.map_or(0, |c| (c.round() as u32) % 360);
    let mut info = vec![b'`', (d + 28) as u8, (m + 28) as u8, (h + 28) as u8,
        (sp / 10 + 28) as u8, ((sp % 10) * 10 + co / 100 + 28) as u8, (co % 100 + 28) as u8, symbol.1 as u8, symbol.0 as u8];
    if let Some(alt) = alt_m {
        let v = (alt.round() as i64 + 10000).clamp(0, 91 * 91 * 91 - 1) as u32;
        info.extend([(v / 8281 + 33) as u8, (v / 91 % 91 + 33) as u8, (v % 91 + 33) as u8, b'}']);
    }
    info.extend(comment.as_bytes());
    (dst, info)
}

pub fn message_info(to: &str, text: &str, id: Option<&str>) -> Vec<u8> {
    let mut s = format!(":{:<9}:{}", to.to_uppercase(), text);
    if let Some(id) = id { s.push('{'); s.push_str(id); }
    s.into_bytes()
}
pub fn ack_info(to: &str, id: &str) -> Vec<u8> { format!(":{:<9}:ack{}", to.to_uppercase(), id).into_bytes() }

// ---------------- Geo ----------------

/// Mitte eines Maidenhead-Locators (4 oder 6 Stellen)
pub fn locator_to_latlon(loc: &str) -> Option<(f64, f64)> {
    let l: Vec<u8> = loc.trim().to_uppercase().bytes().collect();
    if !(l.len() == 4 || l.len() == 6) { return None; }
    if !(b'A'..=b'R').contains(&l[0]) || !(b'A'..=b'R').contains(&l[1]) || !l[2].is_ascii_digit() || !l[3].is_ascii_digit() { return None; }
    let mut lon = (l[0] - b'A') as f64 * 20.0 - 180.0 + (l[2] - b'0') as f64 * 2.0;
    let mut lat = (l[1] - b'A') as f64 * 10.0 - 90.0 + (l[3] - b'0') as f64;
    if l.len() == 6 {
        if !(b'A'..=b'X').contains(&l[4]) || !(b'A'..=b'X').contains(&l[5]) { return None; }
        lon += (l[4] - b'A') as f64 * (2.0 / 24.0) + 1.0 / 24.0;
        lat += (l[5] - b'A') as f64 * (1.0 / 24.0) + 0.5 / 24.0;
    } else {
        lon += 1.0;
        lat += 0.5;
    }
    Some((lat, lon))
}
/// Entfernung (km) und Richtung (Grad) von a nach b
pub fn dist_bearing(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let (la1, lo1, la2, lo2) = (a.0.to_radians(), a.1.to_radians(), b.0.to_radians(), b.1.to_radians());
    let h = ((la2 - la1) / 2.0).sin().powi(2) + la1.cos() * la2.cos() * ((lo2 - lo1) / 2.0).sin().powi(2);
    let d = 2.0 * 6371.0 * h.sqrt().asin();
    let y = (lo2 - lo1).sin() * la2.cos();
    let x = la1.cos() * la2.sin() - la1.sin() * la2.cos() * (lo2 - lo1).cos();
    (d, (y.atan2(x).to_degrees() + 360.0) % 360.0)
}
pub fn compass(deg: f64) -> &'static str {
    ["N", "NO", "O", "SO", "S", "SW", "W", "NW"][(((deg + 22.5) % 360.0) / 45.0) as usize]
}

/// SmartBeaconing (nach HamHUD/Byonics): Bakenabstand nach Tempo, zusätzliche Bake bei Kurswechseln
#[derive(Clone, Copy, Debug)]
pub struct Smart { pub fast_kmh: f64, pub fast_rate_s: f64, pub slow_kmh: f64, pub slow_rate_s: f64, pub min_turn_deg: f64, pub turn_slope: f64, pub min_turn_s: f64 }
/// übliche Werte für Auto/Fahrrad/zu Fuß gemischt
pub const SMART: Smart = Smart { fast_kmh: 90.0, fast_rate_s: 180.0, slow_kmh: 5.0, slow_rate_s: 1800.0, min_turn_deg: 28.0, turn_slope: 26.0, min_turn_s: 30.0 };
/// Voreinstellungen für SmartBeaconing (Name, Werte)
pub const SMART_PROFILES: [(&str, Smart); 3] = [
    ("Auto", SMART),
    ("Fahrrad", Smart { fast_kmh: 30.0, fast_rate_s: 120.0, slow_kmh: 3.0, slow_rate_s: 1200.0, min_turn_deg: 25.0, turn_slope: 10.0, min_turn_s: 20.0 }),
    ("zu Fuß", Smart { fast_kmh: 8.0, fast_rate_s: 180.0, slow_kmh: 2.0, slow_rate_s: 1200.0, min_turn_deg: 30.0, turn_slope: 5.0, min_turn_s: 30.0 }),
];

/// Ist eine Bake fällig? `since_s` seit der letzten Bake, `last_course` = Kurs bei der letzten Bake
pub fn smart_due(p: &Smart, speed_kmh: f64, course: Option<f64>, last_course: Option<f64>, since_s: f64) -> Option<&'static str> {
    if speed_kmh < p.slow_kmh { return (since_s >= p.slow_rate_s).then_some("Zeit (Stand)"); }
    let rate = if speed_kmh >= p.fast_kmh { p.fast_rate_s } else { p.fast_rate_s * p.fast_kmh / speed_kmh };
    if let (Some(c), Some(l)) = (course, last_course) {
        let turn = ((c - l + 540.0) % 360.0 - 180.0).abs();
        let threshold = p.min_turn_deg + p.turn_slope / (speed_kmh / 1.609_344); // Steigung in Grad·mph
        if turn > threshold && since_s >= p.min_turn_s { return Some("Kurswechsel"); }
    }
    (since_s >= rate).then_some("Zeit")
}
/// Kurs/Tempo-Erweiterung für den Kommentar: „ccc/sss“ (Grad, Knoten)
pub fn course_speed_ext(course: f64, speed_kmh: f64) -> String {
    let c = course.round() as u32 % 360;
    format!("{:03}/{:03}", if c == 0 { 360 } else { c }, (speed_kmh / 1.852).round() as u32)
}

/// Gängige Symbole (Tabelle, Zeichen, Bezeichnung)
pub const SYMBOLS: [(char, char, &str); 8] = [
    ('/', '-', "Haus"), ('/', '>', "Auto"), ('/', '[', "Fußgänger"), ('/', 'b', "Fahrrad"),
    ('/', '<', "Motorrad"), ('/', 'Y', "Segelboot"), ('/', ';', "Zeltlager/Portabel"), ('/', 'k', "Lkw"),
];
/// weitere häufige Symbole: (Tabelle '/' = primär, '\\' = alternativ, Zeichen, Bezeichnung)
const MORE_SYMBOLS: &[(char, char, &str)] = &[
    ('/', '#', "Digipeater"), ('\\', '#', "Digipeater"), ('/', '&', "KW-Gateway"), ('\\', '&', "Gateway/iGate"),
    ('/', '_', "Wetterstation"), ('\\', '_', "Wetterstation"), ('/', 'r', "Relais"), ('/', 'y', "Haus mit Yagi"),
    ('/', 's', "Schiff"), ('/', 'v', "Kleinbus"), ('/', 'j', "Jeep"), ('/', 'R', "Wohnmobil"), ('/', 'U', "Bus"),
    ('/', 'u', "Lkw"), ('/', 'O', "Ballon"), ('/', '\'', "Flugzeug"), ('/', '^', "Flugzeug"), ('/', 'X', "Hubschrauber"),
    ('/', 'a', "Rettungswagen"), ('/', 'f', "Feuerwehr"), ('/', 'p', "Hund"), ('/', 'l', "Laptop"), ('/', '$', "Telefon"),
    ('/', '`', "Antenne"), ('\\', '>', "Fahrzeug"), ('\\', 'n', "Dreieck"), ('\\', 'a', "ARES"), ('/', 'h', "Krankenhaus"),
    ('/', 'K', "Schule"), ('/', '-', "Haus"),
];
pub fn symbol_name(s: (char, char)) -> String {
    let find = |t: char, c: char| SYMBOLS.iter().map(|x| (x.0, x.1, x.2)).chain(MORE_SYMBOLS.iter().copied()).find(|x| x.0 == t && x.1 == c).map(|x| x.2);
    match s.0 {
        '/' | '\\' => find(s.0, s.1).map_or(format!("{}{}", s.0, s.1), str::to_string),
        // Überlagerung: Zeichen der Alternativtabelle mit Buchstabe/Ziffer darüber
        o => find('\\', s.1).map_or(format!("{}{}", s.0, s.1), |n| format!("{n} ({o})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u32);
    impl Rng {
        fn u(&mut self) -> f64 { self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345); self.0 as f64 / 4294967296.0 }
        fn gauss(&mut self) -> f64 { (-2.0 * (self.u() + 1e-12).ln()).sqrt() * (TAU * self.u()).cos() }
    }
    fn frame(n: usize) -> Frame {
        Frame { src: "DL9TST-9".into(), dst: TOCALL.into(), path: vec!["WIDE1-1".into(), "WIDE2-1".into()],
            info: position_info(52.5 + n as f64 * 0.01, 13.4, ('/', '>'), &format!("Testpaket {n} äöü~|}}")) }
    }
    /// Audio durch "Funkgerät": optionale FM-Deemphase (6 dB/Oktave ab 300 Hz) und Rauschen
    fn channel(x: &[f32], fs: f64, deemph: bool, noise: f64, seed: u32) -> Vec<f32> {
        let a = (-TAU * 300.0 / fs).exp();
        let mut y = 0.0f64;
        let mut rng = Rng(seed);
        x.iter().map(|&v| {
            let v = if deemph { y = (1.0 - a) * v as f64 + a * y; y * 4.0 } else { v as f64 };
            (v * 0.5 + noise * rng.gauss()) as f32
        }).collect()
    }
    fn count(x: &[f32], fs: f64, m: Modem) -> Vec<Frame> {
        let mut d = Receiver::new(fs, m);
        x.chunks(1000).flat_map(|c| d.push(c)).collect()
    }

    #[test]
    fn crc_referenzwert() { assert_eq!(crc(b"123456789"), 0x906e); }

    #[test]
    fn rahmen_bytes_roundtrip() {
        let f = frame(1);
        let b = f.to_bytes();
        assert_eq!(Frame::from_bytes(&b), Some(f.clone()));
        let mut bad = b.clone();
        bad[20] ^= 1;
        assert_eq!(Frame::from_bytes(&bad), None, "FCS muss Fehler erkennen");
        assert_eq!(f.monitor()[..37], *"DL9TST-9>APZSST,WIDE1-1,WIDE2-1:=5230");
    }

    #[test]
    fn audio_roundtrip_mit_deemphase_und_rauschen() {
        for fs in [48000.0, 44100.0] {
            let frames: Vec<Frame> = (0..20).map(frame).collect();
            let x = modulate(&frames, fs, 300.0, VHF);
            for (deemph, noise, min) in [(false, 0.0, 20), (true, 0.0, 20), (true, 0.15, 19), (false, 0.2, 19)] {
                let got = count(&channel(&x, fs, deemph, noise, 7), fs, VHF);
                println!("fs {fs} Deemphase {deemph} σ {noise}: {}/20", got.len());
                assert!(got.len() >= min);
                assert!(got.iter().all(|g| frames.contains(g)));
            }
        }
    }

    #[test]
    fn skimmer_drei_stationen_gleichzeitig() {
        // drei KW-Stationen auf verschiedenen Tonlagen, zeitlich überlappend, mit Rauschen
        let fs = 48000.0;
        let centers = [800.0, 1700.0, 2350.0];
        let mut mix = vec![0f32; (fs * 12.0) as usize];
        let mut sent = vec![];
        for (k, c) in centers.iter().enumerate() {
            let frames: Vec<Frame> = (0..2).map(|i| frame(10 * k + i)).collect();
            let m = Modem { mark: c - 100.0, space: c + 100.0, ..HF };
            let x = modulate(&frames, fs, 300.0, m);
            let start = (k as f64 * 0.7 * fs) as usize;
            for (i, v) in x.iter().enumerate() { if start + i < mix.len() { mix[start + i] += v * 0.3; } }
            sent.extend(frames.into_iter().map(|f| (f, *c)));
        }
        let mix = channel(&mix, fs, false, 0.05, 11);
        let mut sk = Skimmer::new(fs);
        let t = std::time::Instant::now();
        let got: Vec<(Frame, f64)> = mix.chunks(4800).flat_map(|c| sk.push(c)).collect();
        println!("12 s Audio in {:?}, {} Rahmen", t.elapsed(), got.len());
        for (f, c) in &sent {
            let g = got.iter().find(|g| g.0 == *f).unwrap_or_else(|| panic!("Rahmen bei {c} Hz fehlt"));
            assert!((g.1 - c).abs() <= 45.0, "Tonmitte {} statt {c}", g.1);
        }
        assert_eq!(got.len(), sent.len());
        // nur Rauschen: nichts
        let mut rng = Rng(5);
        let noise: Vec<f32> = (0..48000 * 30).map(|_| (0.3 * rng.gauss()) as f32).collect();
        let mut sk = Skimmer::new(fs);
        assert_eq!(noise.chunks(4800).flat_map(|c| sk.push(c)).count(), 0);
    }

    #[test]
    fn aprs_is_hilfen() {
        // Passcode: bekannte Werte (öffentlich dokumentierter Algorithmus)
        assert_eq!(passcode("N0CALL"), 13023);
        assert_eq!(passcode("n0call-10"), 13023);
        let f = parse_tnc2("DL1ABC-9>APRS,WIDE1-1,WIDE2-1*:=4807.50N/01134.75E>Mobil").unwrap();
        assert_eq!((f.src.as_str(), f.dst.as_str(), f.path.len()), ("DL1ABC-9", "APRS", 2));
        assert_eq!(igate_line(&f, "DK0TST-10").unwrap(), "DL1ABC-9>APRS,WIDE1-1,WIDE2-1*,qAR,DK0TST-10:=4807.50N/01134.75E>Mobil");
        for bad in ["X>APRS,TCPIP*:>hi", "X>APRS,NOGATE:>hi", "X>APRS,RFONLY:>hi", "X>APRS:?APRS?", "X>APRS:}Y>APRS,TCPIP:>z"] {
            assert!(igate_line(&parse_tnc2(bad).unwrap(), "DK0TST-10").is_none(), "{bad}");
        }
        assert!(parse_tnc2("# aprsc 2.1.14").is_none());
    }

    #[test]
    fn drittverkehr_und_reply_ack() {
        // iGate sendet eine Internet-Nachricht an DK0TST-10 auf Funk aus
        let outer = parse_tnc2("DB0IGT>APRX29,WIDE1-1:}DL1ABC-7>APDR16,TCPIP,DB0IGT*::DK0TST-10:Hallo aus dem Netz{AB}").unwrap();
        let inner = third_party(&outer).unwrap();
        assert_eq!(inner.src, "DL1ABC-7");
        assert_eq!(parse(&inner), Body::Message { to: "DK0TST-10".into(), text: "Hallo aus dem Netz".into(), id: Some("AB".into()) });
        // Reply-Ack: Nachricht MM mit Bestätigung AA für unsere Nachricht
        let f = parse_tnc2("DL1ABC-7>APDR16::DK0TST-10:Danke{MQ}12").unwrap();
        let Body::Message { id: Some(id), .. } = parse(&f) else { panic!() };
        assert_eq!(split_id(&id), ("MQ", Some("12")));
        assert_eq!(split_id("12"), ("12", None));
        assert_eq!(split_id("MQ}"), ("MQ", None));
        assert!(third_party(&f).is_none());
    }

    #[test]
    fn nachricht_an_mich() {
        assert!(addressed_to_me("DL4RE", "DL4RE"));
        assert!(addressed_to_me("DL4RE", "DL4RE-9"));      // Grundrufzeichen
        assert!(addressed_to_me("dl4re-9", "DL4RE-9"));
        assert!(!addressed_to_me("DL4RE-7", "DL4RE-9"));   // andere SSID: anderes Gerät
        assert!(!addressed_to_me("DL4REX", "DL4RE"));
        assert!(!addressed_to_me("DL4RE", ""));
        // so kommt die Nachricht vom Online-Dienst: Absender = eigenes Rufzeichen
        let f = parse_tnc2("DL4RE>APSTKB,WIDE1-1,WIDE2-1::DL4RE    :test{00").unwrap();
        assert_eq!(parse(&f), Body::Message { to: "DL4RE".into(), text: "test".into(), id: Some("00".into()) });
    }

    #[test]
    fn nachrichtenwege() {
        assert!(RxWeg::Funk.accepts(false) && !RxWeg::Funk.accepts(true));
        assert!(!RxWeg::Internet.accepts(false) && RxWeg::Internet.accepts(true));
        assert!(RxWeg::Beides.accepts(false) && RxWeg::Beides.accepts(true));
        assert_eq!(TxWeg::Auto.paths(true), (false, true));   // Gegenstelle im Internet
        assert_eq!(TxWeg::Auto.paths(false), (true, false));  // Funkstation
        assert_eq!(TxWeg::Funk.paths(true), (true, false));
        assert_eq!(TxWeg::Internet.paths(false), (false, true));
        assert_eq!(TxWeg::Beides.paths(false), (true, true));
    }

    #[test]
    fn wetterdaten() {
        // Position mit Wettersymbol: Wind 220° 4 mph, Böen 10, 68 °F, Regen 0, 85 %, 1013,5 hPa
        let w = parse_wx("220/004g010t068r000p000P000h85b10135").unwrap();
        assert_eq!(w.dir, Some(220.0));
        assert!((w.temp_c.unwrap() - 20.0).abs() < 0.01);
        assert_eq!((w.hum, w.hpa), (Some(85), Some(1013.5)));
        assert_eq!(w.text(), "20 °C · Wind 6 km/h aus SW, Böen 16 · 85 % · 1014 hPa");
        // Frost und 100 % (h00)
        let w = parse_wx("c000s000g000t023h00b10200").unwrap();
        assert!((w.temp_c.unwrap() + 5.0).abs() < 0.01);
        assert_eq!(w.hum, Some(100));
        assert!(parse_wx("Feststation 30m").is_none());
        // fehlende Werte, Schnee nach DDD/SSS, Kennung der Station, Kommentar dahinter
        let (w, n) = parse_wx_len(".../...g...t041h87b10132s002eCumulusDsVP Wetter Kreuzberg").unwrap();
        assert_eq!((w.dir, w.wind_kmh, w.gust_kmh), (None, None, None));
        assert!((w.snow_cm.unwrap() - 5.08).abs() < 0.01);
        assert_eq!(w.station.as_deref(), Some("Cumulus, Davis Vantage Pro („eCumulusDsVP“)"));
        assert_eq!(&".../...g...t041h87b10132s002eCumulusDsVP Wetter Kreuzberg"[n..], " Wetter Kreuzberg");
        // positionslos: c/s ist Wind; Leerzeichen als Lücke
        let w = parse_wx("c220s  5g010t-04h50b09980xDvs").unwrap();
        assert_eq!(w.dir, Some(220.0));
        assert!((w.wind_kmh.unwrap() - 8.045).abs() < 0.01);
        assert!((w.temp_c.unwrap() + 20.0).abs() < 0.01);
        assert_eq!(w.station.as_deref(), Some("X-APRS/Xastir, Davis („xDvs“)"));
        assert!((dew_point(20.0, 50) - 9.26).abs() < 0.05);
        assert!((feels_like(-5.0, Some(30.0), None).unwrap().0 + 12.9).abs() < 0.2);
        assert_eq!(beaufort(19.0), (3, "schwache Brise"));
        assert_eq!(beaufort(130.0).0, 12);
    }

    #[test]
    fn rauschen_ergibt_keine_rahmen() {
        let mut rng = Rng(3);
        let x: Vec<f32> = (0..48000 * 60).map(|_| (0.3 * rng.gauss()) as f32).collect();
        assert_eq!(count(&x, 48000.0, VHF).len(), 0);
        assert_eq!(count(&x, 48000.0, HF).len(), 0);
    }

    #[test]
    fn kurzwelle_300_baud_auch_verstimmt() {
        let fs = 48000.0;
        let frames: Vec<Frame> = (0..10).map(frame).collect();
        for (off, noise) in [(0.0, 0.0), (0.0, 0.3), (-85.0, 0.1), (75.0, 0.1), (40.0, 0.3)] {
            let x = modulate(&frames, fs, 300.0, HF.shifted(off));
            let got = count(&channel(&x, fs, false, noise, 5), fs, HF);
            println!("HF Ablage {off} Hz σ {noise}: {}/10", got.len());
            assert!(got.len() >= 9 && got.iter().all(|g| frames.contains(g)));
        }
        // UKW-Empfänger liest kein Kurzwellensignal und umgekehrt
        assert_eq!(count(&modulate(&frames[..2], fs, 300.0, HF), fs, VHF).len(), 0);
        assert_eq!(count(&modulate(&frames[..2], fs, 300.0, VHF), fs, HF).len(), 0);
    }

    #[test]
    fn aprs_formate() {
        let fr = |dst: &str, info: &[u8]| Frame { src: "X".into(), dst: dst.into(), path: vec![], info: info.to_vec() };
        let near = |a: f64, b: f64| (a - b).abs() < 1e-3;
        match parse(&fr("APZ123", b"!4903.50N/07201.75W-Test 001234")) {
            Body::Position { lat, lon, symbol, comment } => {
                assert!(near(lat, 49.058333) && near(lon, -72.029167) && symbol == ('/', '-') && comment == "Test 001234");
            }
            b => panic!("{b:?}"),
        }
        match parse(&fr("APZ123", b"@092345z4903.50N/07201.75W>")) { Body::Position { lat, .. } => assert!(near(lat, 49.058333)), b => panic!("{b:?}") }
        match parse(&fr("APZ123", b"!/5L!!<*e7>7P[")) { // Beispiel aus der APRS-Spezifikation
            Body::Position { lat, lon, symbol, .. } => assert!(near(lat, 49.5) && near(lon, -72.75) && symbol == ('/', '>')),
            b => panic!("{b:?}"),
        }
        // Mic-E, Erwartungswerte aus direwolf decode_aprs
        for (dst, info, lat, lon) in [("S32U6T", &b"`(_fn\"Oj/]Kommentar="[..], 33.0 + 25.64 / 60.0, -(12.0 + 7.74 / 60.0)),
                                      ("T2SP0W", &b"`c51!f?>/'\"3w}Mobile"[..], 42.0 + 30.07 / 60.0, -(71.0 + 25.21 / 60.0)),
                                      ("3V4Q8Y", &b"`|O0l!/k/]="[..], 36.0 + 41.89 / 60.0, -(96.0 + 51.20 / 60.0))] {
            match parse(&fr(dst, info)) {
                Body::Position { lat: a, lon: o, .. } => assert!(near(a, lat) && near(o, lon), "{dst}: {a} {o} statt {lat} {lon}"),
                b => panic!("{dst}: {b:?}"),
            }
        }
        assert_eq!(parse(&fr("APZ", b":DL9TST-9 :Hallo Welt{42")), Body::Message { to: "DL9TST-9".into(), text: "Hallo Welt".into(), id: Some("42".into()) });
        assert_eq!(parse(&fr("APZ", b":DL9TST   :ack42")), Body::Ack { to: "DL9TST".into(), id: "42".into() });
        assert_eq!(parse(&fr("APZ", b">Unterwegs")), Body::Status("Unterwegs".into()));
        // erzeugt -> wieder gelesen
        let p = position_info(52.51234, -13.40567, ('/', '-'), "QRV");
        match parse(&fr(TOCALL, &p)) { Body::Position { lat, lon, .. } => assert!((lat - 52.51234).abs() < 2e-4 && (lon + 13.40567).abs() < 2e-4), b => panic!("{b:?}") }
        assert_eq!(String::from_utf8(message_info("dl1abc-7", "Test", Some("1"))).unwrap(), ":DL1ABC-7 :Test{1");
    }

    #[test]
    fn smartbeaconing() {
        let p = SMART;
        // Stand: erst nach 30 min
        assert_eq!(smart_due(&p, 0.0, None, None, 1700.0), None);
        assert!(smart_due(&p, 0.0, None, None, 1800.0).is_some());
        // 90 km/h: alle 3 min; 45 km/h: alle 6 min
        assert!(smart_due(&p, 100.0, Some(90.0), Some(90.0), 180.0).is_some());
        assert_eq!(smart_due(&p, 45.0, Some(90.0), Some(90.0), 300.0), None);
        assert!(smart_due(&p, 45.0, Some(90.0), Some(90.0), 360.0).is_some());
        // Abbiegen bei 50 km/h (Schwelle ≈ 28 + 26/31 ≈ 29°): 90° Kurswechsel nach 40 s -> Bake, nach 20 s noch nicht
        assert_eq!(smart_due(&p, 50.0, Some(180.0), Some(90.0), 40.0), Some("Kurswechsel"));
        assert_eq!(smart_due(&p, 50.0, Some(180.0), Some(90.0), 20.0), None);
        // über Nord hinweg: 350° -> 10° sind nur 20°
        assert_eq!(smart_due(&p, 50.0, Some(10.0), Some(350.0), 40.0), None);
        // langsam (10 km/h): Schwelle ≈ 28 + 26/6,2 ≈ 32°
        assert_eq!(smart_due(&p, 10.0, Some(120.0), Some(90.0), 40.0), None);
        assert_eq!(course_speed_ext(0.0, 50.0), "360/027");
        // Simulierte Fahrt: 20 min Stadt (30 km/h, alle 2 min abbiegen), Ergebnis: deutlich mehr Baken als im Stand
        let (mut t, mut last_t, mut last_c, mut n, mut course) = (0.0, -1e9, None, 0, 0.0);
        while t < 1200.0 {
            if (t as u32) % 120 == 0 && t > 0.0 { course = (course + 90.0) % 360.0; }
            if smart_due(&p, 30.0, Some(course), last_c, t - last_t).is_some() { n += 1; last_t = t; last_c = Some(course); }
            t += 1.0;
        }
        assert!((5..=15).contains(&n), "{n} Baken");
    }

    #[test]
    fn symbolnamen() {
        assert_eq!(symbol_name(('I', '#')), "Digipeater (I)");
        assert_eq!(symbol_name(('/', '>')), "Auto");
        assert_eq!(symbol_name(('\\', '&')), "Gateway/iGate");
        assert_eq!(symbol_name(('/', '~')), "/~");
    }

    #[test]
    fn geo() {
        let (lat, lon) = locator_to_latlon("JO62QM").unwrap();
        assert!((lat - 52.521).abs() < 0.03 && (lon - 13.375).abs() < 0.05, "{lat} {lon}");
        assert_eq!(locator_to_latlon("XX99"), None);
        let (d, b) = dist_bearing((52.52, 13.40), (48.14, 11.58)); // Berlin -> München
        assert!((d - 504.0).abs() < 5.0 && compass(b) == "S", "{d} {b}");
    }

    /// Vergleich mit Direwolf/multimon-ng: APRS_WAV=datei.wav cargo test --release aprs_wav -- --ignored --nocapture
    /// APRS_OUT=ziel.wav schreibt zusätzlich 50 eigene Pakete (mit Deemphase/Rauschen in Stufen)
    #[test]
    #[ignore]
    fn aprs_wav() {
        if let Ok(p) = std::env::var("APRS_WAV") {
            let mut r = hound::WavReader::open(&p).unwrap();
            let fs = r.spec().sample_rate as f64;
            let x: Vec<f32> = r.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();
            let m = if std::env::var("APRS_HF").is_ok() { HF } else { VHF };
            let got: Vec<Frame> = if std::env::var("APRS_SKIM").is_ok() {
                let mut sk = Skimmer::new(fs);
                x.chunks(4800).flat_map(|c| sk.push(c)).map(|(f, c)| { println!("  {c:.0} Hz"); f }).collect()
            } else { count(&x, fs, m) };
            for f in &got { println!("  {}", f.monitor()); }
            println!("{} Rahmen dekodiert aus {p}", got.len());
        }
        if let Ok(p) = std::env::var("APRS_OUT") {
            let fs = 48000.0;
            let mut all = vec![0f32; 24000];
            for n in 0..50 {
                let x = modulate(&[frame(n)], fs, 200.0, VHF);
                all.extend(channel(&x, fs, n % 2 == 0, 0.02 * (n / 5) as f64, n as u32 + 1));
                all.extend(vec![0f32; 12000]);
            }
            let spec = hound::WavSpec { channels: 1, sample_rate: 48000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
            let mut w = hound::WavWriter::create(&p, spec).unwrap();
            for v in &all { w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16).unwrap(); }
            w.finalize().unwrap();
            println!("{} eigene Pakete selbst dekodiert", count(&all, fs, VHF).len());
        }
    }
}
