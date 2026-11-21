//! CAT-Steuerung: Icom CI-V, Kenwood/Elecraft, Yaesu (ASCII + 5-Byte), PTT per RTS/DTR.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Proto { Civ, Kenwood, Yaesu, Yaesu5, None }

/// Betriebsart am Funkgerät
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sb { Usb, Lsb, Fm, Cw }

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Ptt { Cat, Rts, Dtr, Vox }
impl Ptt {
    pub const ALL: [Ptt; 4] = [Ptt::Cat, Ptt::Rts, Ptt::Dtr, Ptt::Vox];
    pub fn label(self) -> &'static str {
        match self { Ptt::Cat => "CAT", Ptt::Rts => "RTS", Ptt::Dtr => "DTR", Ptt::Vox => "VOX" }
    }
}

#[derive(Clone, Debug)]
pub struct RigModel {
    pub name: &'static str,
    pub proto: Proto,
    pub addr: u8,   // CI-V
    pub digits: usize, // Yaesu ASCII
    pub baud: u32,
    pub stop_bits: u8,
    pub ptt: Ptt,
}

const fn icom(name: &'static str, addr: u8) -> RigModel { RigModel { name, proto: Proto::Civ, addr, digits: 0, baud: 19200, stop_bits: 1, ptt: Ptt::Cat } }
const fn ynew(name: &'static str, digits: usize) -> RigModel { RigModel { name, proto: Proto::Yaesu, addr: 0, digits, baud: 38400, stop_bits: 1, ptt: Ptt::Cat } }
const fn yold(name: &'static str) -> RigModel { RigModel { name, proto: Proto::Yaesu5, addr: 0, digits: 0, baud: 4800, stop_bits: 2, ptt: Ptt::Cat } }
const fn kw(name: &'static str, baud: u32) -> RigModel { RigModel { name, proto: Proto::Kenwood, addr: 0, digits: 0, baud, stop_bits: 1, ptt: Ptt::Cat } }

pub static RIGS: [RigModel; 32] = [
    icom("Icom IC-7300", 0x94), icom("Icom IC-705", 0xA4), icom("Icom IC-9700", 0xA2), icom("Icom IC-7610", 0x98),
    icom("Icom IC-7100", 0x88), icom("Icom IC-905", 0xAC), icom("Icom IC-7851", 0x8E), icom("Icom IC-7200", 0x76),
    icom("Icom IC-7000", 0x70), icom("Icom IC-746PRO", 0x66), icom("Icom IC-706MkIIG", 0x58), icom("Icom IC-718", 0x5E),
    ynew("Yaesu FT-991/991A", 9), ynew("Yaesu FT-710", 9), ynew("Yaesu FT-891", 9), ynew("Yaesu FTDX10", 9),
    ynew("Yaesu FTDX101D/MP", 9), ynew("Yaesu FT-DX3000", 8), ynew("Yaesu FT-950", 8), ynew("Yaesu FT-450D", 8),
    yold("Yaesu FT-817/818"), yold("Yaesu FT-857"), yold("Yaesu FT-897"),
    kw("Kenwood TS-590S/SG", 115200), kw("Kenwood TS-890S", 115200), kw("Kenwood TS-2000", 9600), kw("Kenwood TS-480", 9600),
    kw("Elecraft K3/K3S", 38400), kw("Elecraft K4", 38400), kw("Elecraft KX3/KX2", 38400),
    RigModel { name: "Nur PTT über RTS (z. B. Digirig)", proto: Proto::None, addr: 0, digits: 0, baud: 9600, stop_bits: 1, ptt: Ptt::Rts },
    RigModel { name: "Nur PTT über DTR", proto: Proto::None, addr: 0, digits: 0, baud: 9600, stop_bits: 1, ptt: Ptt::Dtr },
];

// ---- Byte-Folgen (rein, testbar) ----
pub(crate) fn bcd_le(v: u64, n: usize) -> Vec<u8> { // niederwertigste Ziffern zuerst
    let mut v = v;
    (0..n).map(|_| { let lo = (v % 10) as u8; v /= 10; let hi = (v % 10) as u8; v /= 10; (hi << 4) | lo }).collect()
}
pub(crate) fn bcd_val(bytes: &[u8]) -> u64 { // höchstwertig zuerst
    bytes.iter().fold(0, |v, b| v * 100 + (b >> 4) as u64 * 10 + (b & 15) as u64)
}
pub(crate) fn civ(r: &RigModel, body: &[u8]) -> Vec<u8> {
    [&[0xFE, 0xFE, r.addr, 0xE0][..], body, &[0xFD]].concat()
}
/// letzte Zahl nach `FA` mit min..=max Ziffern und `;`
fn last_fa(b: &[u8], min: usize, max: usize) -> Option<u64> {
    let s = String::from_utf8_lossy(b);
    s.match_indices("FA").filter_map(|(i, _)| {
        let rest = &s[i + 2..];
        let end = rest.find(';')?;
        let d = &rest[..end];
        (d.len() >= min && d.len() <= max && d.bytes().all(|c| c.is_ascii_digit())).then(|| d.parse().ok()).flatten()
    }).last()
}

pub fn set_freq(r: &RigModel, hz: u64) -> Vec<u8> {
    match r.proto {
        Proto::Civ => civ(r, &[&[0x05][..], &bcd_le(hz, 5)].concat()),
        Proto::Kenwood => format!("FA{hz:011};").into_bytes(),
        Proto::Yaesu => format!("FA{hz:0w$};", w = r.digits).into_bytes(),
        Proto::Yaesu5 => { let mut v = bcd_le(hz / 10, 4); v.reverse(); v.push(0x01); v }
        Proto::None => vec![],
    }
}
pub fn set_mode(r: &RigModel, m: Sb) -> Vec<u8> {
    // Kennungen je Protokoll: LSB, USB, FM
    let (civ_m, kw, ya, y5) = match m { Sb::Lsb => (0x00, "1", "1", 0x00), Sb::Usb => (0x01, "2", "2", 0x01), Sb::Fm => (0x05, "4", "4", 0x08), Sb::Cw => (0x03, "3", "3", 0x02) };
    match r.proto {
        Proto::Civ => civ(r, &[0x06, civ_m, 0x01]),
        Proto::Kenwood => format!("MD{kw};").into_bytes(),
        Proto::Yaesu => format!("MD0{ya};").into_bytes(),
        Proto::Yaesu5 => vec![y5, 0, 0, 0, 0x07],
        Proto::None => vec![],
    }
}
/// Betriebsart abfragen (Antwort: Icom 04 mm ff, Kenwood MDn;, Yaesu MD0n;)
pub fn get_mode(r: &RigModel) -> Vec<u8> {
    match r.proto { Proto::Civ => civ(r, &[0x04]), Proto::Kenwood => b"MD;".to_vec(), Proto::Yaesu => b"MD0;".to_vec(), _ => vec![] }
}
/// Sendeleistung abfragen (Icom 14 0A: 0000–0255, Kenwood/Yaesu PC: Watt)
pub fn get_power(r: &RigModel) -> Vec<u8> {
    match r.proto { Proto::Civ => civ(r, &[0x14, 0x0A]), Proto::Kenwood | Proto::Yaesu => b"PC;".to_vec(), _ => vec![] }
}
/// Einheit der Leistungseinstellung: Icom in Prozent (wie im Menü RF POWER), Kenwood/Yaesu in Watt
pub fn power_scale(r: &RigModel) -> (f32, &'static str) {
    if r.proto == Proto::Civ { (100.0, "%") } else { (max_watts(r), "W") }
}
/// Sendeleistung setzen; `v` in der Einheit von `power_scale` (Icom %, sonst Watt)
pub fn set_power(r: &RigModel, v: f32) -> Vec<u8> {
    match r.proto {
        Proto::Civ => {
            let v = ((v / 100.0).clamp(0.0, 1.0) * 255.0).round() as u64;
            civ(r, &[&[0x14, 0x0A][..], &bcd_le(v, 2).into_iter().rev().collect::<Vec<_>>()].concat())
        }
        Proto::Kenwood | Proto::Yaesu => format!("PC{:03};", v.round().clamp(0.0, 999.0) as u32).into_bytes(),
        _ => vec![],
    }
}
/// Messwert beim Senden abfragen (Icom 15 11 = Po, 15 12 = SWR, 15 13 = ALC; Antwort 0000–0255)
pub fn get_meter(r: &RigModel, m: u8) -> Vec<u8> {
    if r.proto == Proto::Civ { civ(r, &[0x15, m]) } else { vec![] }
}
/// stückweise linear zwischen Stützpunkten (Rohwert, Anzeige); über den letzten Punkt hinaus mit gleicher Steigung
fn interp(raw: f32, pts: &[(f32, f32)]) -> f32 {
    let i = pts.windows(2).position(|w| raw <= w[1].0).unwrap_or(pts.len() - 2);
    let ((x0, y0), (x1, y1)) = (pts[i], pts[i + 1]);
    y0 + (raw - x0) * (y1 - y0) / (x1 - x0)
}
/// Icom-Messskalen laut CI-V-Referenz: Po 0000=0 %, 0143=50 %, 0213=100 %
pub fn meter_po_pct(raw: u32) -> f32 { interp(raw as f32, &[(0.0, 0.0), (143.0, 50.0), (213.0, 100.0)]).max(0.0) }
/// SWR 0000=1,0 · 0048=1,5 · 0080=2,0 · 0120=3,0
pub fn meter_swr(raw: u32) -> f32 { interp(raw as f32, &[(0.0, 1.0), (48.0, 1.5), (80.0, 2.0), (120.0, 3.0)]).max(1.0) }
/// Versorgungsspannung (V) laut CI-V-Referenz: IC-705 0000=0 V, 0075=5 V, 0241=16 V; IC-7300 u. a. 0013=10 V, 0241=16 V
pub fn meter_vd(r: &RigModel, raw: u32) -> f32 {
    let pts: &[(f32, f32)] = if r.name.contains("705") { &[(0.0, 0.0), (75.0, 5.0), (241.0, 16.0)] } else { &[(0.0, 0.0), (13.0, 10.0), (241.0, 16.0)] };
    interp(raw as f32, pts).max(0.0)
}
/// Strom (A): IC-705 0121=2 A, 0241=4 A; IC-7300 u. a. 0097=10 A, 0146=15 A, 0241=25 A
pub fn meter_id(r: &RigModel, raw: u32) -> f32 {
    let pts: &[(f32, f32)] = if r.name.contains("705") { &[(0.0, 0.0), (121.0, 2.0), (241.0, 4.0)] } else { &[(0.0, 0.0), (97.0, 10.0), (146.0, 15.0), (241.0, 25.0)] };
    interp(raw as f32, pts).max(0.0)
}
/// Betriebsart mit Filter setzen (06 mm ff), z. B. um nur das Filter zu wechseln
pub fn set_mode_filter(r: &RigModel, mode_raw: u8, fil: u8) -> Vec<u8> { civ(r, &[0x06, mode_raw, fil]) }
/// ALC 0000–0120 = Regelbereich (über 100 % = ALC regelt zu stark)
pub fn meter_alc_pct(raw: u32) -> f32 { raw as f32 / 120.0 * 100.0 }
/// Höchstleistung nach Modell (für die Umrechnung der Icom-Skala 0–255 in Watt)
pub fn max_watts(r: &RigModel) -> f32 {
    let n = r.name;
    if n.contains("705") || n.contains("905") { 10.0 } else if n.contains("817") || n.contains("818") { 6.0 } else { 100.0 }
}
/// Spektrum ein/aus: Bild (27 10) und Datenausgabe (27 11)
pub fn scope_cmds(r: &RigModel, on: bool) -> Vec<Vec<u8>> {
    if r.proto != Proto::Civ { return vec![]; }
    let v = on as u8;
    if on { vec![civ(r, &[0x27, 0x10, 0x01]), civ(r, &[0x27, 0x11, 0x01])] } else { vec![civ(r, &[0x27, 0x11, v])] }
}

/// Spektrum: Mitten- (false) oder Festmodus (true) – 27 14
pub fn scope_mode(r: &RigModel, fixed: bool) -> Vec<u8> { civ(r, &[0x27, 0x14, 0x00, fixed as u8]) }
/// Spektrum im Mittenmodus: Spannweite ± `half_hz` (2,5 k … 500 kHz) – 27 15
pub fn scope_span(r: &RigModel, half_hz: u64) -> Vec<u8> { civ(r, &[&[0x27, 0x15, 0x00][..], &bcd_le(half_hz, 5)].concat()) }
/// mögliche Spannweiten im Mittenmodus (±Hz)
pub const SCOPE_SPANS: [u64; 8] = [2_500, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000, 500_000];
/// Feste Kanten (IC-705): Frequenzbereiche für 27 1E laut CI-V-Referenz (Code, von, bis in Hz)
const IC705_RANGES: [(u8, u64, u64); 17] = [
    (1, 30_000, 1_600_000), (2, 1_600_000, 2_000_000), (3, 2_000_000, 6_000_000), (4, 6_000_000, 8_000_000), (5, 8_000_000, 11_000_000),
    (6, 11_000_000, 15_000_000), (7, 15_000_000, 20_000_000), (8, 20_000_000, 22_000_000), (9, 22_000_000, 26_000_000), (10, 26_000_000, 30_000_000),
    (11, 30_000_000, 45_000_000), (12, 45_000_000, 60_000_000), (13, 60_000_000, 74_800_000), (14, 74_800_000, 108_000_000),
    (15, 108_000_000, 137_000_000), (16, 137_000_000, 200_000_000), (17, 400_000_000, 470_000_000)];
/// Bandansicht (feste Kanten) wird unterstützt – bisher nur IC-705, dessen Bereichstabelle bekannt ist
pub fn scope_fixed_ok(r: &RigModel) -> bool { r.proto == Proto::Civ && r.name.contains("705") }
/// Festmodus mit eigenen Kanten: 27 1E (Bereich, Kante, untere, obere Frequenz), dann Kante wählen (27 16) und Festmodus (27 14)
pub fn scope_fixed_cmds(r: &RigModel, lo: u64, hi: u64, edge: u8) -> Option<[Vec<u8>; 3]> {
    let (code, _, _) = *IC705_RANGES.iter().find(|(_, a, b)| lo >= *a && hi <= *b)?;
    let bcd = |v: u8| ((v / 10) << 4) | (v % 10);
    let (lo, hi) = (lo / 100 * 100, hi / 100 * 100); // 100-Hz-Raster
    Some([civ(r, &[&[0x27, 0x1E, bcd(code), bcd(edge)][..], &bcd_le(lo, 5), &bcd_le(hi, 5)].concat()),
          civ(r, &[0x27, 0x16, 0x00, bcd(edge)]),
          scope_mode(r, true)])
}

/// CW senden über CI-V (Icom): Text (höchstens 30 Zeichen je Befehl), Stopp, Tempo, Break-in
pub fn cw_text(r: &RigModel, t: &str) -> Vec<u8> {
    if r.proto != Proto::Civ { return vec![]; }
    let mut b = vec![0x17];
    b.extend(t.bytes().take(30));
    civ(r, &b)
}
pub fn cw_stop(r: &RigModel) -> Vec<u8> { if r.proto == Proto::Civ { civ(r, &[0x17, 0xFF]) } else { vec![] } }
/// Tastgeschwindigkeit: 0000 = 6 WpM … 0255 = 48 WpM
pub fn cw_speed(r: &RigModel, wpm: u32) -> Vec<u8> {
    if r.proto != Proto::Civ { return vec![]; }
    let v = ((wpm.clamp(6, 48) - 6) as f64 * 255.0 / 42.0).round() as u64;
    civ(r, &[&[0x14, 0x0C][..], &bcd_le(v, 2).into_iter().rev().collect::<Vec<_>>()].concat())
}
/// Break-in: 00 aus, 01 Semi, 02 Voll – ohne Break-in (oder PTT) sendet das Gerät CI-V-Text nicht
pub fn cw_breakin(r: &RigModel, mode: u8) -> Vec<u8> { if r.proto == Proto::Civ { civ(r, &[0x16, 0x47, mode]) } else { vec![] } }

pub fn ptt_cmd(r: &RigModel, on: bool) -> Vec<u8> {
    match r.proto {
        Proto::Civ => civ(r, &[0x1C, 0x00, on as u8]),
        Proto::Kenwood => (if on { "TX;" } else { "RX;" }).into(),
        Proto::Yaesu => (if on { "TX1;" } else { "TX0;" }).into(),
        Proto::Yaesu5 => vec![0, 0, 0, 0, if on { 0x08 } else { 0x88 }],
        Proto::None => vec![],
    }
}
pub fn get_freq(r: &RigModel) -> Vec<u8> {
    match r.proto {
        Proto::Civ => civ(r, &[0x03]),
        Proto::Kenwood | Proto::Yaesu => b"FA;".to_vec(),
        Proto::Yaesu5 => vec![0, 0, 0, 0, 0x03],
        Proto::None => vec![],
    }
}
pub fn parse_freq(r: &RigModel, b: &[u8]) -> Option<u64> {
    match r.proto {
        Proto::Civ => (0..b.len().saturating_sub(10)).rev().find_map(|i| {
            // FE FE E0|00 <rig> 03|00 f0..f4 FD
            let ok = b[i] == 0xFE && b[i + 1] == 0xFE && (b[i + 2] == 0xE0 || b[i + 2] == 0x00) && b[i + 3] == r.addr
                && (b[i + 4] == 0x03 || b[i + 4] == 0x00) && b[i + 10] == 0xFD;
            ok.then(|| { let mut f = b[i + 5..i + 10].to_vec(); f.reverse(); bcd_val(&f) })
        }),
        Proto::Kenwood => last_fa(b, 11, 11),
        Proto::Yaesu => last_fa(b, 8, 9),
        Proto::Yaesu5 => (b.len() >= 5).then(|| bcd_val(&b[b.len() - 5..b.len() - 1]) * 10),
        Proto::None => None,
    }
}

/// Eigene GPS-Position eines Icom-Geräts mit GPS (IC-705, IC-9700, IC-905, ID-52 …): Befehl 23 00.
pub fn get_gps(r: &RigModel) -> Vec<u8> {
    if r.proto == Proto::Civ { civ(r, &[0x23, 0x00]) } else { vec![] }
}
/// GPS-Daten des Funkgeräts
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpsFix {
    pub lat: f64,
    pub lon: f64,
    pub alt_m: Option<f64>,
    pub course: Option<f64>,
    pub speed_kmh: Option<f64>,
    /// Uhrzeit des Empfängers (UTC, Unix-Sekunden)
    pub utc: Option<i64>,
}

/// Antwort auf 23 00 auswerten. Laut CI-V-Referenz (Byte 1–27 des Datenteils):
/// Breite  1–5:  dd mm mm(1/10, 1/100) m(1/1000)0 0N  (N: 1 = Nord, 0 = Süd)
/// Länge   6–11: 0d dd mm mm m0 0E                    (E: 1 = Ost, 0 = West)
/// Höhe 12–15 (0,1 m, letzte Stelle Vorzeichen), Kurs 16–17 (Grad), Tempo 18–20 (0,1 km/h),
/// Datum 21–27 (UTC jjjjmmttHHMMSS). Fehlende Teile sind FF.
/// `Some(Err)` = Gerät hat geantwortet, aber keine gültige Position (kein Empfang, GPS aus).
pub fn parse_gps(r: &RigModel, b: &[u8]) -> Option<Result<(f64, f64), String>> {
    parse_gps_full(r, b).map(|x| x.map(|f| (f.lat, f.lon)))
}
fn bcd_ok(b: &[u8]) -> bool { b.iter().all(|x| x >> 4 <= 9 && x & 15 <= 9) }
pub fn parse_gps_full(r: &RigModel, b: &[u8]) -> Option<Result<GpsFix, String>> {
    for i in 0..b.len().saturating_sub(5) {
        if !(b[i] == 0xFE && b[i + 1] == 0xFE && b[i + 2] == 0xE0 && b[i + 3] == r.addr) { continue; }
        if b[i + 4] == 0xFA { return Some(Err("Gerät lehnt die GPS-Abfrage ab (NG) – kein GPS eingebaut oder GPS aus".into())); }
        if !(b[i + 4] == 0x23 && b[i + 5] == 0x00) { continue; }
        let end = b[i + 6..].iter().position(|&x| x == 0xFD)?; // Antwort noch nicht vollständig
        let d = &b[i + 6..i + 6 + end];
        let digits_ok = d.len() >= 11 && d[..11].iter().all(|x| x >> 4 <= 9 && x & 15 <= 9);
        if !digits_ok { return Some(Err("noch keine gültige GPS-Position (kein Satellitenempfang?)".into())); }
        let v = |x: u8| ((x >> 4) * 10 + (x & 15)) as f64;
        let lat = v(d[0]) + (v(d[1]) + v(d[2]) / 100.0 + (d[3] >> 4) as f64 / 1000.0) / 60.0;
        let lon = (d[5] & 15) as f64 * 100.0 + v(d[6]) + (v(d[7]) + v(d[8]) / 100.0 + (d[9] >> 4) as f64 / 1000.0) / 60.0;
        let lat = if d[4] & 15 == 1 { lat } else { -lat };
        let lon = if d[10] & 15 == 1 { lon } else { -lon };
        if lat.abs() > 90.0 || lon.abs() > 180.0 || v(d[1]) >= 60.0 || v(d[7]) >= 60.0 {
            return Some(Err("ungültige GPS-Daten".into()));
        }
        if lat == 0.0 && lon == 0.0 { return Some(Err("noch keine gültige GPS-Position (kein Satellitenempfang?)".into())); }
        let num = |s: &[u8]| s.iter().fold(0u64, |a, x| a * 100 + ((x >> 4) * 10 + (x & 15)) as u64);
        let alt_m = (d.len() >= 15 && bcd_ok(&d[11..15])).then(|| {
            let raw = num(&d[11..15]); // 6 Ziffern Höhe, 0, Vorzeichen
            let v = (raw / 100) as f64 / 10.0;
            if raw % 10 == 1 { -v } else { v }
        });
        let course = (d.len() >= 17 && bcd_ok(&d[15..17])).then(|| num(&d[15..17]) as f64).filter(|c| *c < 360.0);
        let speed_kmh = (d.len() >= 20 && bcd_ok(&d[17..20])).then(|| num(&d[17..20]) as f64 / 10.0);
        let utc = (d.len() >= 27 && bcd_ok(&d[20..27])).then(|| {
            let t = num(&d[20..27]); // jjjjmmttHHMMSS
            let (dt, tm) = (t / 1_000_000, t % 1_000_000);
            chrono::NaiveDate::from_ymd_opt((dt / 10000) as i32, (dt / 100 % 100) as u32, (dt % 100) as u32)
                .and_then(|d| d.and_hms_opt((tm / 10000) as u32, (tm / 100 % 100) as u32, (tm % 100) as u32))
                .map(|x| x.and_utc().timestamp())
        }).flatten().filter(|&t| t > 1_500_000_000);
        return Some(Ok(GpsFix { lat, lon, alt_m, course, speed_kmh, utc }));
    }
    None
}

// ---- Verbindung: seriell (USB/COM) oder Icom-Netzwerk (WLAN/LAN) ----
enum Link {
    Serial(Box<dyn serialport::SerialPort>),
    Net(std::sync::Arc<crate::icomnet::Shared>),
}

pub struct Rig {
    pub model: RigModel,
    link: Link,
}

impl Rig {
    pub fn open(port_name: &str, model: RigModel) -> Result<Rig, String> {
        let mut port = serialport::new(port_name, model.baud)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(if model.stop_bits == 2 { serialport::StopBits::Two } else { serialport::StopBits::One })
            .flow_control(serialport::FlowControl::None)
            .timeout(Duration::from_millis(50))
            .open()
            .map_err(|e| format!("{port_name}: {e}"))?;
        // Steuerleitungen sofort low: steht am Gerät USB SEND = RTS/DTR, würde es sonst dauerhaft senden
        let _ = port.write_request_to_send(false);
        let _ = port.write_data_terminal_ready(false);
        Ok(Rig { model, link: Link::Serial(port) })
    }

    /// CI-V über eine bestehende Netzwerkverbindung (PTT immer per CAT)
    pub fn open_net(shared: std::sync::Arc<crate::icomnet::Shared>, mut model: RigModel) -> Rig {
        model.ptt = Ptt::Cat;
        Rig { model, link: Link::Net(shared) }
    }
    pub fn is_net(&self) -> bool { matches!(self.link, Link::Net(_)) }

    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.is_empty() { return Ok(()); }
        match &mut self.link {
            Link::Serial(p) => p.write_all(bytes).and_then(|_| p.flush()).map_err(|e| e.to_string()),
            Link::Net(n) => { n.send_civ(bytes); Ok(()) }
        }
    }
    pub fn set_freq(&mut self, hz: u64) -> Result<(), String> { let b = set_freq(&self.model, hz); self.send(&b) }
    pub fn set_mode(&mut self, m: Sb) -> Result<(), String> { let b = set_mode(&self.model, m); self.send(&b) }
    /// beliebigen fertigen Befehl senden (z. B. aus `cw_text`)
    pub fn send_raw(&mut self, b: &[u8]) -> Result<(), String> { self.send(b) }
    pub fn ptt(&mut self, on: bool) -> Result<(), String> {
        match (&mut self.link, self.model.ptt) {
            (Link::Serial(p), Ptt::Rts) => p.write_request_to_send(on).map_err(|e| e.to_string()),
            (Link::Serial(p), Ptt::Dtr) => p.write_data_terminal_ready(on).map_err(|e| e.to_string()),
            (_, Ptt::Vox) => Ok(()),
            _ => { let b = ptt_cmd(&self.model, on); self.send(&b) }
        }
    }
    pub fn read_freq(&mut self) -> Result<Option<u64>, String> {
        let q = get_freq(&self.model);
        let m = self.model.clone();
        self.query(&q, |b| parse_freq(&m, b))
    }
    /// GPS-Position des Geräts lesen (nur Icom mit GPS)
    pub fn read_gps(&mut self) -> Result<(f64, f64), String> {
        let q = get_gps(&self.model);
        if q.is_empty() { return Err("GPS-Abfrage gibt es nur bei Icom-Geräten mit GPS".into()); }
        let m = self.model.clone();
        self.query(&q, |b| parse_gps(&m, b))?.unwrap_or_else(|| Err("Gerät antwortet nicht auf die GPS-Abfrage".into()))
    }
    /// bereits angekommene Bytes abholen, ohne zu warten
    pub fn take_rx(&mut self) -> Vec<u8> {
        match &mut self.link {
            Link::Serial(p) => {
                let n = p.bytes_to_read().unwrap_or(0) as usize;
                let mut b = vec![0u8; n];
                if n > 0 { if let Ok(k) = p.read(&mut b) { b.truncate(k); } else { b.clear(); } }
                b
            }
            Link::Net(n) => n.take_civ(),
        }
    }
    /// Rohdaten einer Abfrage (für die Diagnose): alles, was innerhalb von 800 ms zurückkommt
    pub fn query_raw(&mut self, q: &[u8]) -> Result<Vec<u8>, String> {
        let mut all = vec![];
        let _ = self.query(q, |b| { all = b.to_vec(); None::<()> })?;
        Ok(all)
    }
    /// Befehl senden und bis zu 800 ms auf eine passende Antwort warten
    fn query<T>(&mut self, q: &[u8], mut parse: impl FnMut(&[u8]) -> Option<T>) -> Result<Option<T>, String> {
        if q.is_empty() { return Ok(None); }
        match &mut self.link {
            Link::Serial(p) => { let _ = p.clear(serialport::ClearBuffer::Input); }
            Link::Net(n) => n.clear_civ(),
        }
        self.send(q)?;
        let (mut rx, mut buf, t0) = (Vec::new(), [0u8; 256], Instant::now());
        while t0.elapsed() < Duration::from_millis(800) {
            match &mut self.link {
                Link::Serial(p) => match p.read(&mut buf) {
                    Ok(n) => rx.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                    Err(e) => return Err(e.to_string()),
                },
                Link::Net(n) => { rx.extend(n.take_civ()); std::thread::sleep(Duration::from_millis(10)); }
            }
            if let Some(f) = parse(&rx) { return Ok(Some(f)); }
        }
        Ok(None)
    }
}

impl Drop for Rig {
    fn drop(&mut self) { let _ = self.ptt(false); }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ") }
    fn rig(name: &str) -> &'static RigModel { RIGS.iter().find(|r| r.name == name).unwrap() }

    #[test]
    fn cw_befehle() {
        let r = rig("Icom IC-705");
        assert_eq!(hex(&cw_text(r, "CQ DE DL1ABC")), "FE FE A4 E0 17 43 51 20 44 45 20 44 4C 31 41 42 43 FD");
        assert_eq!(hex(&cw_stop(r)), "FE FE A4 E0 17 FF FD");
        assert_eq!(hex(&cw_speed(r, 6)), "FE FE A4 E0 14 0C 00 00 FD");
        assert_eq!(hex(&cw_speed(r, 48)), "FE FE A4 E0 14 0C 02 55 FD");
        assert_eq!(hex(&cw_speed(r, 20)), "FE FE A4 E0 14 0C 00 85 FD");
        assert_eq!(hex(&cw_breakin(r, 1)), "FE FE A4 E0 16 47 01 FD");
        assert_eq!(cw_text(r, &"X".repeat(40)).len(), 4 + 1 + 30 + 1);
    }

    #[test]
    fn gps_antwort() {
        let r = rig("Icom IC-705");
        assert_eq!(hex(&get_gps(r)), "FE FE A4 E0 23 00 FD");
        // 52°31.250' N, 013°22.500' E, danach Höhe/Kurs/Tempo/Datum; davor das Echo der Anfrage
        let mut b = vec![0xFE, 0xFE, 0xA4, 0xE0, 0x23, 0x00, 0xFD, 0xFE, 0xFE, 0xE0, 0xA4, 0x23, 0x00,
            0x52, 0x31, 0x25, 0x00, 0x01, 0x00, 0x13, 0x22, 0x50, 0x00, 0x01];
        b.extend([0x00, 0x03, 0x45, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x20, 0x26, 0x11, 0x16, 0x20, 0x00, 0x00, 0xFD]);
        let (la, lo) = parse_gps(r, &b).unwrap().unwrap();
        assert!((la - (52.0 + 31.25 / 60.0)).abs() < 1e-9 && (lo - (13.0 + 22.5 / 60.0)).abs() < 1e-9);
        // Höhe 34,5 m, Kurs 0, Tempo 0, 16.11.2026 20:00:00 UTC
        let f = parse_gps_full(r, &b).unwrap().unwrap();
        assert_eq!(f.alt_m, Some(34.5));
        assert_eq!(f.utc, Some(chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap().and_hms_opt(20, 0, 0).unwrap().and_utc().timestamp()));
        // Kurs 275°, 52,3 km/h, Höhe -12,0 m
        let mut m = b.clone();
        let i = m.len() - 17; // Beginn Höhe im Datenteil (16 Byte vor FD)
        m[i..i + 9].copy_from_slice(&[0x00, 0x01, 0x20, 0x01, 0x02, 0x75, 0x00, 0x05, 0x23]);
        let f = parse_gps_full(r, &m).unwrap().unwrap();
        assert_eq!((f.alt_m, f.course, f.speed_kmh), (Some(-12.0), Some(275.0), Some(52.3)));
        // Süd/West: 33°51.900' S, 151°12.600' W
        let s = [0xFE, 0xFE, 0xE0, 0xA4, 0x23, 0x00, 0x33, 0x51, 0x90, 0x00, 0x00, 0x01, 0x51, 0x12, 0x60, 0x00, 0x00, 0xFD];
        let (la, lo) = parse_gps(r, &s).unwrap().unwrap();
        assert!((la + 33.865).abs() < 1e-9 && (lo + 151.21).abs() < 1e-9);
        // ohne Empfang: FF; abgelehnt: NG; unvollständig: noch nichts
        assert!(parse_gps(r, &[0xFE, 0xFE, 0xE0, 0xA4, 0x23, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFD]).unwrap().is_err());
        assert!(parse_gps(r, &[0xFE, 0xFE, 0xE0, 0xA4, 0xFA, 0xFD]).unwrap().is_err());
        assert!(parse_gps(r, &[0xFE, 0xFE, 0xE0, 0xA4, 0x23, 0x00, 0x52, 0x31]).is_none());
    }

    #[test]
    fn referenz_frames() {
        let ic = rig("Icom IC-7300");
        assert_eq!(hex(&set_freq(ic, 14_230_000)), "FE FE 94 E0 05 00 00 23 14 00 FD");
        assert_eq!(hex(&set_freq(ic, 145_500_000)), "FE FE 94 E0 05 00 00 50 45 01 FD");
        assert_eq!(hex(&set_mode(ic, Sb::Usb)), "FE FE 94 E0 06 01 01 FD");
        assert_eq!(hex(&set_mode(ic, Sb::Fm)), "FE FE 94 E0 06 05 01 FD");
        assert_eq!(hex(&ptt_cmd(ic, true)), "FE FE 94 E0 1C 00 01 FD");
        let echo_und_antwort = [0xFE, 0xFE, 0x94, 0xE0, 0x03, 0xFD, 0xFE, 0xFE, 0xE0, 0x94, 0x03, 0x00, 0x00, 0x23, 0x14, 0x00, 0xFD];
        assert_eq!(parse_freq(ic, &echo_und_antwort), Some(14_230_000));

        let ts = rig("Kenwood TS-590S/SG");
        assert_eq!(set_freq(ts, 14_230_000), b"FA00014230000;");
        assert_eq!(set_mode(ts, Sb::Usb), b"MD2;");
        assert_eq!(set_mode(ts, Sb::Fm), b"MD4;");
        assert_eq!(set_mode(rig("Yaesu FT-991/991A"), Sb::Fm), b"MD04;");
        assert_eq!(hex(&set_mode(rig("Yaesu FT-817/818"), Sb::Fm)), "08 00 00 00 07");
        assert_eq!(hex(&set_mode(ic, Sb::Cw)), "FE FE 94 E0 06 03 01 FD");
        assert_eq!(set_mode(ts, Sb::Cw), b"MD3;");
        assert_eq!(set_mode(rig("Yaesu FT-991/991A"), Sb::Cw), b"MD03;");
        assert_eq!(hex(&set_mode(rig("Yaesu FT-817/818"), Sb::Cw)), "02 00 00 00 07");
        assert_eq!(parse_freq(ts, b"FA00007171000;"), Some(7_171_000));

        let ft = rig("Yaesu FT-991/991A");
        assert_eq!(set_freq(ft, 14_230_000), b"FA014230000;");
        assert_eq!(set_freq(rig("Yaesu FT-950"), 14_230_000), b"FA14230000;");
        assert_eq!([ptt_cmd(ft, true), ptt_cmd(ft, false)].concat(), b"TX1;TX0;");
        assert_eq!(parse_freq(ft, b"FA014230000;"), Some(14_230_000));

        let f8 = rig("Yaesu FT-817/818");
        assert_eq!(hex(&set_freq(f8, 14_230_000)), "01 42 30 00 01");
        assert_eq!(hex(&ptt_cmd(f8, false)), "00 00 00 00 88");
        assert_eq!(parse_freq(f8, &[0x01, 0x42, 0x30, 0x00, 0x01]), Some(14_230_000));
    }

    /// Echte serielle Strecke gegen ein simuliertes IC-7300. Braucht ein Portpaar:
    /// socat pty,raw,echo=0,link=/tmp/ttyA pty,raw,echo=0,link=/tmp/ttyB
    #[test]
    #[ignore]
    fn fake_ic7300_ueber_pty() {
        let mut far = serialport::new("/tmp/ttyB", 19200).timeout(Duration::from_millis(100)).open().unwrap();
        let rig_side = std::thread::spawn(move || {
            let (mut got, mut buf, t) = (Vec::new(), [0u8; 64], Instant::now());
            while t.elapsed() < Duration::from_secs(2) {
                if let Ok(n) = far.read(&mut buf) {
                    got.extend_from_slice(&buf[..n]);
                    if got.ends_with(&[0xFE, 0xFE, 0x94, 0xE0, 0x03, 0xFD]) { // Frequenzabfrage beantworten
                        far.write_all(&[0xFE, 0xFE, 0xE0, 0x94, 0x03, 0x00, 0x00, 0x23, 0x14, 0x00, 0xFD]).unwrap();
                    }
                }
            }
            got
        });
        let mut rig = Rig::open("/tmp/ttyA", rig("Icom IC-7300").clone()).unwrap();
        rig.set_freq(14_230_000).unwrap();
        rig.ptt(true).unwrap();
        rig.ptt(false).unwrap();
        assert_eq!(rig.read_freq().unwrap(), Some(14_230_000));
        drop(rig); // sendet nochmals PTT aus
        let got = hex(&rig_side.join().unwrap());
        println!("am Gerät angekommen: {got}");
        for want in ["FE FE 94 E0 05 00 00 23 14 00 FD", "FE FE 94 E0 1C 00 01 FD", "FE FE 94 E0 1C 00 00 FD", "FE FE 94 E0 03 FD"] {
            assert!(got.contains(want), "fehlt: {want}");
        }
    }
}
