//! Laufende CAT-Abfrage ohne Blockieren: Frequenz, Betriebsart, Sendeleistung (und GPS) im Wechsel abfragen,
//! alle Antworten zentral auswerten – auch unaufgeforderte Meldungen (Icom „CI-V Transceive“) und das Spektrum.
//! So folgt das Programm dem Abstimmknopf und dem Leistungsregler am Gerät.

use crate::cat::{self, Proto, RigModel, Sb};
use std::time::{Duration, Instant};

/// eine ausgewertete Meldung des Funkgeräts
#[derive(Debug, PartialEq)]
pub enum Ev {
    Freq(u64),
    /// Betriebsart als Text (z. B. „CW-R“) und, wenn passend, als `Sb`
    Mode(&'static str, Option<Sb>),
    /// Icom: 0–255; Kenwood/Yaesu: Watt
    Power(u32),
    /// Icom-Messwert: 0x02 S-Meter, 0x11 Po, 0x12 SWR, 0x13 ALC, 0x15 Spannung, 0x16 Strom; Rohwert 0–255
    Meter(u8, u32),
    /// Icom-Schalter (Vorverstärker, Abschwächer, NB, NR, AGC, Tuner): Wert
    Ctl(Kind, u8),
    /// Betriebsart roh und Filter (FIL1–3)
    Filter(u8, u8),
    Gps(Vec<u8>),
    /// Daten nach „27 00“
    Scope(Vec<u8>),
    Ack,
    Ng,
}

fn icom_mode(m: u8) -> (&'static str, Option<Sb>) {
    match m {
        0x00 => ("LSB", Some(Sb::Lsb)), 0x01 => ("USB", Some(Sb::Usb)), 0x02 => ("AM", None), 0x03 => ("CW", Some(Sb::Cw)),
        0x04 => ("RTTY", None), 0x05 => ("FM", Some(Sb::Fm)), 0x06 => ("WFM", None), 0x07 => ("CW-R", Some(Sb::Cw)),
        0x08 => ("RTTY-R", None), 0x17 => ("DV", None), _ => ("?", None),
    }
}
fn kenwood_mode(c: char) -> (&'static str, Option<Sb>) {
    match c { '1' => ("LSB", Some(Sb::Lsb)), '2' => ("USB", Some(Sb::Usb)), '3' => ("CW", Some(Sb::Cw)), '4' => ("FM", Some(Sb::Fm)),
        '5' => ("AM", None), '6' => ("FSK", None), '7' => ("CW-R", Some(Sb::Cw)), '9' => ("FSK-R", None), _ => ("?", None) }
}
fn yaesu_mode(c: char) -> (&'static str, Option<Sb>) {
    match c { '1' => ("LSB", Some(Sb::Lsb)), '2' => ("USB", Some(Sb::Usb)), '3' => ("CW-U", Some(Sb::Cw)), '4' => ("FM", Some(Sb::Fm)),
        '5' => ("AM", None), '6' => ("RTTY-L", None), '7' => ("CW-L", Some(Sb::Cw)), '8' => ("DATA-L", Some(Sb::Lsb)), '9' => ("RTTY-U", None),
        'A' => ("DATA-FM", Some(Sb::Fm)), 'B' => ("FM-N", Some(Sb::Fm)), 'C' => ("DATA-U", Some(Sb::Usb)), _ => ("?", None) }
}

/// vollständige Meldungen aus dem Puffer holen (Rest bleibt für später)
pub fn parse(r: &RigModel, buf: &mut Vec<u8>) -> Vec<Ev> {
    let mut out = vec![];
    match r.proto {
        Proto::Civ => loop {
            let Some(st) = buf.windows(2).position(|w| w == [0xFE, 0xFE]) else { buf.clear(); break };
            let Some(len) = buf[st..].iter().position(|&b| b == 0xFD) else { buf.drain(..st); break };
            let f: Vec<u8> = buf.drain(..st + len + 1).skip(st).collect();
            // FE FE <an> <von> cmd … FD; nur Meldungen des Funkgeräts an uns (E0) oder an alle (00)
            if f.len() < 6 || f[3] != r.addr || !(f[2] == 0xE0 || f[2] == 0x00) { continue; }
            let d = &f[4..f.len() - 1];
            match d {
                [0xFB] => out.push(Ev::Ack),
                [0xFA] => out.push(Ev::Ng),
                [0x00 | 0x03, b @ ..] if b.len() == 5 => { let mut x = b.to_vec(); x.reverse(); out.push(Ev::Freq(cat::bcd_val(&x))) }
                [0x01 | 0x04, m, rest @ ..] => {
                    let (n, sb) = icom_mode(*m);
                    out.push(Ev::Mode(n, sb));
                    if let [fil, ..] = rest { out.push(Ev::Filter(*m, *fil)); }
                }
                [0x14, 0x0A, a, b] => out.push(Ev::Power(cat::bcd_val(&[*a, *b]) as u32)),
                [0x15, m @ (0x02 | 0x11..=0x13 | 0x15 | 0x16), a, b] => out.push(Ev::Meter(*m, cat::bcd_val(&[*a, *b]) as u32)),
                [0x16, 0x02, v] => out.push(Ev::Ctl(Kind::Pre, *v)),
                [0x11, v] => out.push(Ev::Ctl(Kind::Att, *v)),
                [0x16, 0x22, v] => out.push(Ev::Ctl(Kind::Nb, *v)),
                [0x16, 0x40, v] => out.push(Ev::Ctl(Kind::Nr, *v)),
                [0x16, 0x12, v] => out.push(Ev::Ctl(Kind::Agc, *v)),
                [0x1C, 0x01, v] => out.push(Ev::Ctl(Kind::Tuner, *v)),
                [0x23, 0x00, ..] => out.push(Ev::Gps(f.clone())),
                [0x27, 0x00, rest @ ..] => out.push(Ev::Scope(rest.to_vec())),
                _ => {}
            }
        },
        Proto::Kenwood | Proto::Yaesu => {
            let text = String::from_utf8_lossy(buf).to_string();
            let Some(last) = text.rfind(';') else { return out };
            for m in text[..last].split(';') {
                let m = m.trim_matches(|c: char| c.is_control());
                if let Some(d) = m.strip_prefix("FA") { if let Ok(f) = d.parse() { out.push(Ev::Freq(f)) } }
                else if let Some(d) = m.strip_prefix("MD") {
                    let c = d.chars().last().unwrap_or('?');
                    let (n, sb) = if r.proto == Proto::Kenwood { kenwood_mode(c) } else { yaesu_mode(c) };
                    out.push(Ev::Mode(n, sb));
                }
                else if let Some(d) = m.strip_prefix("PC") { if let Ok(w) = d[d.len().saturating_sub(3)..].parse() { out.push(Ev::Power(w)) } }
                else if m == "?" { out.push(Ev::Ng) }
            }
            buf.drain(..text[..=last].len());
        }
        _ => buf.clear(),
    }
    out
}

/// Spektrum: Frequenzbereich und Pegel 0–160 je Punkt
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeFrame { pub lo_hz: u64, pub hi_hz: u64, pub data: Vec<u8> }

/// setzt die Teile von „27 00“ zusammen (USB: 11 Teile, WLAN: alles in einem)
#[derive(Default)]
pub struct ScopeAsm { range: Option<(u64, u64)>, data: Vec<u8>, max: u8, next: u8 }

fn bcd_freq(b: &[u8]) -> u64 { let mut x = b.to_vec(); x.reverse(); cat::bcd_val(&x) }
fn bcd_byte(b: u8) -> u8 { (b >> 4) * 10 + (b & 15) }

impl ScopeAsm {
    /// `d` = Daten nach 27 00: 00, Teil, Teile gesamt, [Modus, f1 (5), f2 (5), außerhalb, Daten…] bzw. [Daten…]
    pub fn push(&mut self, d: &[u8]) -> Option<ScopeFrame> {
        if d.len() < 3 { return None; }
        let (part, total) = (bcd_byte(d[1]), bcd_byte(d[2]));
        if part == 1 {
            if d.len() < 15 { return None; }
            let (f1, f2) = (bcd_freq(&d[4..9]), bcd_freq(&d[9..14]));
            // Mittenmodus: Mitte und halbe Breite (±); fester Modus: Unter- und Oberkante
            // nur der Mittenmodus (00) meldet Mitte und halbe Breite; Fest- und beide Scroll-Modi melden Unter- und Oberkante
            self.range = Some(if d[3] == 0x00 { (f1.saturating_sub(f2), f1 + f2) } else { (f1, f2) });
            self.data = d[15..].to_vec();
            self.max = total;
            self.next = 2;
            if d[14] == 0x01 { self.range = None; return None; } // außerhalb des Bereichs: keine Daten
        } else {
            // Teile müssen lückenlos der Reihe nach kommen, sonst Bild verwerfen
            if self.range.is_none() || total != self.max || part != self.next { self.range = None; return None; }
            self.next += 1;
            self.data.extend_from_slice(&d[3..]);
        }
        if part == total && !self.data.is_empty() {
            let (lo, hi) = self.range.take()?;
            return Some(ScopeFrame { lo_hz: lo, hi_hz: hi, data: std::mem::take(&mut self.data) });
        }
        None
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind { Freq, Mode, Power, Gps, Po, Swr, Alc, Id, Vd, Tuner, Pre, Att, Nb, Nr, Agc, S }
const KINDS: usize = 16;
/// gelegentlich abgefragt (eins alle 600 ms): Spannung, Tuner und die Empfangsschalter
const SLOW: [Kind; 7] = [Kind::Vd, Kind::Tuner, Kind::Pre, Kind::Att, Kind::Nb, Kind::Nr, Kind::Agc];

/// CI-V-Befehl (ohne Daten) zum Lesen bzw. (mit einem Datenbyte) zum Setzen eines Schalters
pub fn ctl_body(k: Kind) -> &'static [u8] {
    match k { Kind::Pre => &[0x16, 0x02], Kind::Att => &[0x11], Kind::Nb => &[0x16, 0x22], Kind::Nr => &[0x16, 0x40],
              Kind::Agc => &[0x16, 0x12], Kind::Tuner => &[0x1C, 0x01], Kind::Vd => &[0x15, 0x15], Kind::Id => &[0x15, 0x16], _ => &[] }
}

/// Messwerte beim Senden (Icom): Ausgangsleistung in % der Höchstleistung, SWR, ALC in % des Regelbereichs
#[derive(Default, Clone, Copy, Debug)]
pub struct Meters { pub po_pct: Option<f32>, pub swr: Option<f32>, pub alc_pct: Option<f32>, pub at: Option<Instant> }

/// Abfrage-Zustand
pub struct RigPoll {
    buf: Vec<u8>,
    inflight: Option<(Kind, Instant)>,
    next: Instant,
    cycle: usize,
    /// nach wiederholtem Ausbleiben einer Antwort eine Weile nicht mehr fragen
    pause: [Option<Instant>; KINDS],
    misses: [u32; KINDS],
    /// Gerät hat die Abfrage abgelehnt (NG)
    pub unsupported: [bool; KINDS],
    pub mode: Option<(&'static str, Option<Sb>)>,
    /// eingestellte Sendeleistung in der Einheit von `cat::power_scale` (Icom %, sonst Watt)
    pub power: Option<f32>,
    /// es wird gesendet: statt Frequenz/Betriebsart die Messwerte abfragen
    pub tx: bool,
    pub meters: Meters,
    /// während der Regler bewegt wird, keine Werte vom Gerät übernehmen
    pub power_hold: Option<Instant>,
    pub want_gps: bool,
    /// zuletzt gelesene Schalterstellungen (Index = Kind)
    pub ctl: [Option<u8>; KINDS],
    /// Betriebsart roh und Filter
    pub filter: Option<(u8, u8)>,
    /// Versorgungsspannung (V) und Strom (A)
    pub vd: Option<f32>,
    pub id: Option<f32>,
    slow_next: Instant,
    slow_i: usize,
    pub gps: Option<Result<Vec<u8>, String>>,
    pub scope_on: bool,
    pub scope_asm: ScopeAsm,
    pub scope: Option<ScopeFrame>,
    pub scope_frames: u64,
    /// Einstellbefehle (Kennung, Bytes), die nacheinander mit Antwort OK/NG geschickt werden
    pub cmdq: std::collections::VecDeque<(u8, Vec<u8>)>,
    cmd_inflight: Option<(u8, Instant)>,
    /// abgelehnte Einstellbefehle (Kennungen)
    pub cmd_ng: Vec<u8>,
    /// S-Meter im Empfang: Verlauf der letzten Sekunden (Zeit, Rohwert 0–255)
    pub smeter: std::collections::VecDeque<(Instant, u32)>,
}

/// S-Meter-Rohwert (Icom: 0 = S0, 120 = S9, 241 = S9+60 dB) als Text und in dB über S0 (6 dB je S-Stufe)
pub fn s_text(raw: u32) -> String {
    if raw <= 120 { format!("S{}", ((raw as f32 / 120.0 * 9.0).round() as u32).min(9)) }
    else { format!("S9+{:.0}", ((raw - 120) as f32 / 121.0 * 60.0 / 10.0).round() * 10.0) }
}
pub fn s_db(raw: u32) -> f32 { if raw <= 120 { raw as f32 / 120.0 * 54.0 } else { 54.0 + (raw - 120) as f32 / 121.0 * 60.0 } }

impl Default for RigPoll {
    fn default() -> Self {
        RigPoll { buf: vec![], inflight: None, next: Instant::now(), cycle: 0, pause: [None; KINDS], misses: [0; KINDS], unsupported: [false; KINDS],
            mode: None, power: None, tx: false, meters: Meters::default(), power_hold: None, want_gps: false, ctl: [None; KINDS], filter: None, vd: None, id: None, slow_next: Instant::now() + Duration::from_millis(600), slow_i: 0, gps: None, scope_on: false, scope_asm: ScopeAsm::default(), scope: None, scope_frames: 0,
            cmdq: Default::default(), cmd_inflight: None, cmd_ng: vec![], smeter: Default::default() }
    }
}

/// im Empfang: S-Meter jede zweite Abfrage (etwa alle 0,25 s), damit jedes Paket einen Pegel bekommt
const ORDER: [Kind; 6] = [Kind::Freq, Kind::S, Kind::Mode, Kind::S, Kind::Power, Kind::S];
const ORDER_TX: [Kind; 5] = [Kind::Po, Kind::Swr, Kind::Alc, Kind::Id, Kind::Tuner];
const TIMEOUT: Duration = Duration::from_millis(500);

impl RigPoll {
    pub fn supported(&self, k: Kind) -> bool { !self.unsupported[k as usize] }

    /// höchster S-Meter-Wert der letzten `secs` Sekunden (für ein gerade dekodiertes Paket)
    pub fn s_recent(&self, secs: f32) -> Option<u32> {
        let now = Instant::now();
        self.smeter.iter().filter(|x| now.duration_since(x.0).as_secs_f32() <= secs).map(|x| x.1).max()
    }
    /// aktueller S-Meter-Wert (höchstens 1 s alt)
    pub fn s_now(&self) -> Option<u32> { self.smeter.back().filter(|x| x.0.elapsed() < Duration::from_secs(1)).map(|x| x.1) }

    /// Ankommende Bytes auswerten; liefert die Ereignisse für die Oberfläche (Frequenz usw.)
    pub fn feed(&mut self, r: &RigModel, bytes: &[u8], now: Instant) -> Vec<Ev> {
        self.buf.extend_from_slice(bytes);
        let evs = parse(r, &mut self.buf);
        let mut out = vec![];
        for e in evs {
            let k = match &e { Ev::Freq(_) => Some(Kind::Freq), Ev::Mode(..) => Some(Kind::Mode), Ev::Power(_) => Some(Kind::Power), Ev::Gps(_) => Some(Kind::Gps),
                Ev::Meter(0x11, _) => Some(Kind::Po), Ev::Meter(0x12, _) => Some(Kind::Swr), Ev::Meter(0x13, _) => Some(Kind::Alc),
                Ev::Meter(0x15, _) => Some(Kind::Vd), Ev::Meter(0x16, _) => Some(Kind::Id), Ev::Meter(0x02, _) => Some(Kind::S), Ev::Ctl(k, _) => Some(*k), _ => None };
            if let Some(k) = k { self.misses[k as usize] = 0; if self.inflight.is_some_and(|i| i.0 == k) { self.inflight = None; } }
            match e {
                // Antwort auf einen Einstellbefehl
                Ev::Ng if self.cmd_inflight.is_some() => { let (t, _) = self.cmd_inflight.take().unwrap(); self.cmd_ng.push(t); }
                Ev::Ack if self.cmd_inflight.is_some() => self.cmd_inflight = None,
                Ev::Ng => if let Some((k, _)) = self.inflight.take() {
                    self.unsupported[k as usize] = true;
                    if k == Kind::Gps { self.gps = Some(Err("NG".into())); }
                },
                Ev::Mode(n, sb) => self.mode = Some((n, sb)),
                Ev::Power(v) => if self.power_hold.is_none_or(|t| now >= t) {
                    self.power = Some(if r.proto == Proto::Civ { v as f32 / 255.0 * 100.0 } else { v as f32 });
                },
                Ev::Ctl(k, v) => self.ctl[k as usize] = Some(v),
                Ev::Filter(m, f) => self.filter = Some((m, f)),
                Ev::Meter(0x15, v) => self.vd = Some(cat::meter_vd(r, v)),
                Ev::Meter(0x16, v) => self.id = Some(cat::meter_id(r, v)),
                Ev::Meter(0x14, _) => {}
                Ev::Meter(0x02, v) => {
                    self.smeter.push_back((now, v));
                    while self.smeter.front().is_some_and(|x| now.duration_since(x.0) > Duration::from_secs(10)) { self.smeter.pop_front(); }
                }
                Ev::Meter(m, v) => {
                    let mt = &mut self.meters;
                    match m { 0x11 => mt.po_pct = Some(cat::meter_po_pct(v)), 0x12 => mt.swr = Some(cat::meter_swr(v)), _ => mt.alc_pct = Some(cat::meter_alc_pct(v)) }
                    mt.at = Some(now);
                }
                Ev::Gps(f) => self.gps = Some(Ok(f)),
                Ev::Scope(d) => if let Some(f) = self.scope_asm.push(&d) { self.scope = Some(f); self.scope_frames += 1; },
                ev @ Ev::Freq(_) => out.push(ev),
                Ev::Ack => {}
            }
        }
        out
    }

    /// nächste Abfrage, wenn keine offen ist
    pub fn next_query(&mut self, r: &RigModel, now: Instant) -> Option<Vec<u8>> {
        if let Some((k, t)) = self.inflight {
            if now.duration_since(t) < TIMEOUT { return None; }
            self.inflight = None;
            let m = &mut self.misses[k as usize];
            *m += 1;
            if k == Kind::Gps { self.gps = Some(Err("keine Antwort auf die GPS-Abfrage".into())); }
            if *m >= 3 { self.pause[k as usize] = Some(now + Duration::from_secs(10)); *m = 0; }
        }
        if let Some((_, t)) = self.cmd_inflight {
            if now.duration_since(t) < TIMEOUT { return None; }
            self.cmd_inflight = None;
        }
        if now < self.next { return None; }
        // Einstellbefehle haben Vorrang vor Abfragen
        if let Some((tag, b)) = self.cmdq.pop_front() {
            self.cmd_inflight = Some((tag, now));
            self.next = now + Duration::from_millis(60);
            return Some(b);
        }
        let ready = |s: &Self, k: Kind| s.supported(k) && s.pause[k as usize].is_none_or(|t| now >= t);
        let slow = if now >= self.slow_next {
            self.slow_next = now + Duration::from_millis(600);
            let mut pick = None;
            for _ in 0..SLOW.len() {
                let k = SLOW[self.slow_i % SLOW.len()];
                self.slow_i += 1;
                if ready(self, k) { pick = Some(k); break; }
            }
            pick
        } else { None };
        let k = if self.want_gps && ready(self, Kind::Gps) { self.want_gps = false; Kind::Gps } else if let Some(k) = slow { k } else {
            let order: &[Kind] = if self.tx { &ORDER_TX } else { &ORDER };
            let mut pick = None;
            for _ in 0..order.len() {
                let k = order[self.cycle % order.len()];
                self.cycle += 1;
                if ready(self, k) { pick = Some(k); break; }
            }
            pick?
        };
        let q = match k { Kind::Freq => cat::get_freq(r), Kind::Mode => cat::get_mode(r), Kind::Power => cat::get_power(r), Kind::Gps => cat::get_gps(r),
            Kind::Po => cat::get_meter(r, 0x11), Kind::S => cat::get_meter(r, 0x02), Kind::Swr => cat::get_meter(r, 0x12), Kind::Alc => cat::get_meter(r, 0x13),
            k => if r.proto == Proto::Civ { cat::civ(r, ctl_body(k)) } else { vec![] } };
        if q.is_empty() { self.unsupported[k as usize] = true; return None; }
        self.inflight = Some((k, now));
        self.next = now + Duration::from_millis(120);
        Some(q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rig(n: &str) -> &'static RigModel { cat::RIGS.iter().find(|r| r.name == n).unwrap() }

    #[test]
    fn icom_meldungen() {
        let r = rig("Icom IC-705");
        // Echo unserer Abfrage, Antwort Frequenz, unaufgeforderte Meldung (Transceive, an 00), Betriebsart, Leistung, halbe Meldung
        let mut b = vec![0xFE, 0xFE, 0xA4, 0xE0, 0x03, 0xFD,
            0xFE, 0xFE, 0xE0, 0xA4, 0x03, 0x00, 0x40, 0x07, 0x07, 0x00, 0xFD,
            0xFE, 0xFE, 0x00, 0xA4, 0x00, 0x00, 0x30, 0x23, 0x14, 0x00, 0xFD,
            0xFE, 0xFE, 0xE0, 0xA4, 0x04, 0x07, 0x01, 0xFD,
            0xFE, 0xFE, 0xE0, 0xA4, 0x14, 0x0A, 0x01, 0x28, 0xFD,
            0xFE, 0xFE, 0xE0, 0xA4, 0xFA, 0xFD,
            0xFE, 0xFE, 0xE0];
        let ev = parse(r, &mut b);
        assert_eq!(ev, vec![Ev::Freq(7_074_000), Ev::Freq(14_233_000), Ev::Mode("CW-R", Some(Sb::Cw)), Ev::Filter(0x07, 0x01), Ev::Power(128), Ev::Ng]);
        assert_eq!(b, vec![0xFE, 0xFE, 0xE0]); // Rest wartet
    }

    #[test]
    fn kenwood_yaesu() {
        let mut b = b"FA00014074000;MD2;PC050;FA0001".to_vec();
        assert_eq!(parse(rig("Kenwood TS-590S/SG"), &mut b), vec![Ev::Freq(14_074_000), Ev::Mode("USB", Some(Sb::Usb)), Ev::Power(50)]);
        assert_eq!(b, b"FA0001");
        let mut b = b"MD0C;PC005;?;".to_vec();
        assert_eq!(parse(rig("Yaesu FT-991/991A"), &mut b), vec![Ev::Mode("DATA-U", Some(Sb::Usb)), Ev::Power(5), Ev::Ng]);
    }

    #[test]
    fn leistung_befehle() {
        let r = rig("Icom IC-705");
        // Icom in Prozent (50 % = 0128), Kenwood in Watt
        assert_eq!(cat::set_power(r, 50.0), vec![0xFE, 0xFE, 0xA4, 0xE0, 0x14, 0x0A, 0x01, 0x28, 0xFD]);
        assert_eq!(cat::set_power(rig("Kenwood TS-590S/SG"), 50.0), b"PC050;");
        let mut p = RigPoll::default();
        let ev = p.feed(r, &[0xFE, 0xFE, 0xE0, 0xA4, 0x14, 0x0A, 0x02, 0x55, 0xFD], Instant::now());
        assert!(ev.is_empty());
        assert_eq!(p.power, Some(100.0));
    }

    #[test]
    fn messwerte_beim_senden() {
        let r = rig("Icom IC-705");
        assert_eq!(cat::meter_po_pct(143), 50.0);
        assert_eq!(cat::meter_po_pct(213), 100.0);
        assert_eq!(cat::meter_swr(0), 1.0);
        assert_eq!(cat::meter_swr(48), 1.5);
        assert_eq!(cat::meter_swr(100), 2.5);
        assert_eq!(cat::meter_swr(160), 4.0); // über 3,0 weiter mit gleicher Steigung
        let mut p = RigPoll::default();
        p.tx = true;
        let t = Instant::now();
        // beim Senden nur noch Po, SWR, ALC reihum
        assert_eq!(p.next_query(r, t).unwrap(), cat::get_meter(r, 0x11));
        p.feed(r, &[0xFE, 0xFE, 0xE0, 0xA4, 0x15, 0x11, 0x01, 0x43, 0xFD], t);
        assert_eq!(p.next_query(r, t + Duration::from_millis(130)).unwrap(), cat::get_meter(r, 0x12));
        p.feed(r, &[0xFE, 0xFE, 0xE0, 0xA4, 0x15, 0x12, 0x00, 0x48, 0xFD], t);
        assert_eq!(p.next_query(r, t + Duration::from_millis(260)).unwrap(), cat::get_meter(r, 0x13));
        p.feed(r, &[0xFE, 0xFE, 0xE0, 0xA4, 0x15, 0x13, 0x00, 0x60, 0xFD], t);
        assert_eq!((p.meters.po_pct, p.meters.swr, p.meters.alc_pct), (Some(50.0), Some(1.5), Some(50.0)));
        // Kenwood: keine Messwerte -> nicht unterstützt, keine Abfrage
        let k = rig("Kenwood TS-590S/SG");
        let mut p = RigPoll::default();
        p.tx = true;
        assert!(p.next_query(k, t).is_none());
    }

    #[test]
    fn spektrum_usb_und_wlan() {
        // WLAN: alles in einer Meldung (Mittenmodus 14,100 MHz ±25 kHz)
        let mut d = vec![0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x10, 0x14, 0x00, 0x00, 0x50, 0x02, 0x00, 0x00, 0x00];
        d.extend((0..475).map(|i| (i % 161) as u8));
        let mut a = ScopeAsm::default();
        let f = a.push(&d).unwrap();
        assert_eq!((f.lo_hz, f.hi_hz, f.data.len()), (14_075_000, 14_125_000, 475));
        // USB: Kopf ohne Daten, dann 10 Teile (fester Modus 7,000–7,200 MHz)
        let mut a = ScopeAsm::default();
        assert!(a.push(&[0x00, 0x01, 0x11, 0x01, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x20, 0x07, 0x00, 0x00]).is_none());
        let mut got = None;
        for p in 2..=11u8 {
            let mut d = vec![0x00, ((p / 10) << 4) | (p % 10), 0x11];
            d.extend(std::iter::repeat_n(p, if p == 11 { 25 } else { 50 }));
            got = a.push(&d);
        }
        let f = got.unwrap();
        assert_eq!((f.lo_hz, f.hi_hz, f.data.len()), (7_000_000, 7_200_000, 475));
        // außerhalb des Bereichs: nichts
        let mut a = ScopeAsm::default();
        assert!(a.push(&[0x00, 0x01, 0x01, 0x00, 0, 0, 0x10, 0x14, 0, 0, 0x50, 0x02, 0, 0, 0x01]).is_none());
    }

    #[test]
    fn s_meter() {
        let r = rig("Icom IC-705");
        let mut p = RigPoll::default();
        let t = Instant::now();
        // Reihenfolge im Empfang: Frequenz, S, Betriebsart, S …
        assert_eq!(p.next_query(r, t).unwrap(), cat::get_freq(r));
        p.inflight = None;
        assert_eq!(p.next_query(r, t + Duration::from_millis(130)).unwrap(), cat::get_meter(r, 0x02));
        p.feed(r, &[0xFE, 0xFE, 0xE0, 0xA4, 0x15, 0x02, 0x01, 0x20, 0xFD], t);
        assert_eq!(p.s_recent(5.0), Some(120));
        assert_eq!(s_text(120), "S9");
        assert_eq!(s_text(60), "S5");
        assert_eq!(s_text(241), "S9+60");
        assert_eq!(s_text(160), "S9+20");
        assert!((s_db(241) - 114.0).abs() < 0.01);
    }

    #[test]
    fn schalter_spannung_filter() {
        let r = rig("Icom IC-705");
        let mut p = RigPoll::default();
        let t = Instant::now();
        // Vorverstärker an, ATT 20 dB, NR an, AGC MID, Tuner stimmt ab, Spannung 0075 (5 V), Betriebsart USB mit FIL2
        let b: Vec<u8> = [&[0x16u8, 0x02, 0x01][..], &[0x11, 0x20], &[0x16, 0x40, 0x01], &[0x16, 0x12, 0x02], &[0x1C, 0x01, 0x02], &[0x15, 0x15, 0x00, 0x75], &[0x04, 0x01, 0x02]]
            .iter().flat_map(|d| [&[0xFE, 0xFE, 0xE0, 0xA4][..], d, &[0xFD]].concat()).collect();
        p.feed(r, &b, t);
        assert_eq!((p.ctl[Kind::Pre as usize], p.ctl[Kind::Att as usize], p.ctl[Kind::Nr as usize]), (Some(1), Some(0x20), Some(1)));
        assert_eq!((p.ctl[Kind::Agc as usize], p.ctl[Kind::Tuner as usize]), (Some(2), Some(2)));
        assert_eq!(p.vd, Some(5.0));
        assert_eq!(p.filter, Some((0x01, 0x02)));
        assert!((cat::meter_id(r, 121) - 2.0).abs() < 1e-6);
        assert!((cat::meter_vd(rig("Icom IC-7300"), 13) - 10.0).abs() < 1e-6);
        // die langsamen Abfragen kommen reihum dazwischen
        let mut p = RigPoll::default();
        assert_eq!(p.next_query(r, Instant::now()).unwrap(), cat::get_freq(r));
        p.inflight = None;
        assert_eq!(p.next_query(r, Instant::now() + Duration::from_millis(700)).unwrap(), cat::civ(r, &[0x15, 0x15]));
    }

    #[test]
    fn spektrum_kantenmodi_und_befehle() {
        // Scroll-C (02) meldet Unter- und Oberkante, nicht Mitte ± Breite (Fehler am echten IC-705 auf 2 m)
        let mut d = vec![0x00, 0x01, 0x01, 0x02, 0x00, 0x00, 0x55, 0x44, 0x01, 0x00, 0x00, 0x05, 0x45, 0x01, 0x00];
        d.extend([20u8; 475]);
        let f = ScopeAsm::default().push(&d).unwrap();
        assert_eq!((f.lo_hz, f.hi_hz), (144_550_000, 145_050_000));
        // feste Kanten für 2 m: Bereich 16, Kante 3
        let r = rig("Icom IC-705");
        let c = cat::scope_fixed_cmds(r, 144_000_000, 145_000_000, 3).unwrap();
        assert_eq!(c[0], vec![0xFE, 0xFE, 0xA4, 0xE0, 0x27, 0x1E, 0x16, 0x03, 0x00, 0x00, 0x00, 0x44, 0x01, 0x00, 0x00, 0x00, 0x45, 0x01, 0xFD]);
        assert!(cat::scope_fixed_cmds(r, 146_000_000, 430_000_000, 3).is_none()); // nicht in einem Bereich
        assert_eq!(cat::scope_span(r, 25_000), vec![0xFE, 0xFE, 0xA4, 0xE0, 0x27, 0x15, 0x00, 0x00, 0x50, 0x02, 0x00, 0x00, 0xFD]);
        // Befehle gehen vor Abfragen; NG landet beim Befehl, nicht bei einer Abfrage
        let mut p = RigPoll::default();
        let t = Instant::now();
        p.cmdq.push_back((7, c[0].clone()));
        assert_eq!(p.next_query(r, t).unwrap(), c[0]);
        p.feed(r, &[0xFE, 0xFE, 0xE0, 0xA4, 0xFA, 0xFD], t);
        assert_eq!(p.cmd_ng, vec![7]);
        assert!(p.supported(Kind::Freq) && p.supported(Kind::Mode));
        assert_eq!(p.next_query(r, t + Duration::from_millis(100)).unwrap(), cat::get_freq(r));
    }

    #[test]
    fn abfragen_reihum_und_ng() {
        let r = rig("Icom IC-705");
        let mut p = RigPoll::default();
        let t = Instant::now();
        let q1 = p.next_query(r, t).unwrap();
        assert_eq!(q1, cat::get_freq(r));
        assert!(p.next_query(r, t).is_none()); // Antwort steht aus
        p.feed(r, &[0xFE, 0xFE, 0xE0, 0xA4, 0x03, 0x00, 0x40, 0x07, 0x07, 0x00, 0xFD], t);
        // dazwischen das S-Meter; NG darauf: wird nicht mehr gefragt
        assert_eq!(p.next_query(r, t + Duration::from_millis(130)).unwrap(), cat::get_meter(r, 0x02));
        p.feed(r, &[0xFE, 0xFE, 0xE0, 0xA4, 0xFA, 0xFD], t);
        assert!(!p.supported(Kind::S));
        let q2 = p.next_query(r, t + Duration::from_millis(260)).unwrap();
        assert_eq!(q2, cat::get_mode(r));
        p.feed(r, &[0xFE, 0xFE, 0xE0, 0xA4, 0xFA, 0xFD], t); // NG: Betriebsart wird nicht mehr gefragt
        assert!(!p.supported(Kind::Mode));
        assert_eq!(p.next_query(r, t + Duration::from_millis(400)).unwrap(), cat::get_power(r));
        // GPS hat Vorrang, wenn angefordert; ohne Antwort -> Fehler
        p.inflight = None;
        p.want_gps = true;
        assert_eq!(p.next_query(r, t + Duration::from_millis(600)).unwrap(), cat::get_gps(r));
        let _ = p.next_query(r, t + Duration::from_millis(1200));
        assert!(matches!(p.gps, Some(Err(_))));
    }
}
