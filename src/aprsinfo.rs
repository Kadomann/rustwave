//! APRS-Pakete im Klartext: Kurs/Tempo, Höhe, PHG, Mic-E-Status, Gerät, Zeitstempel, Wetter – das, was im Kommentar
//! sonst als „088/036/A=001234PHG5130“ o. Ä. steht. Nach APRS 1.0.1 (Kap. 7, 9, 10, 12) und den gängigen Erweiterungen.

use crate::aprs::{self, Frame, Wx};

/// alles, was sich aus einem Paket über eine Station sagen lässt
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    /// Art der Meldung: Position, komprimiert, Mic-E, Objekt, Wetter …
    pub kind: &'static str,
    pub course: Option<f64>,
    pub speed_kmh: Option<f64>,
    pub alt_m: Option<f64>,
    /// PHG im Klartext (Leistung, Antennenhöhe, Gewinn, Richtung)
    pub phg: Option<String>,
    /// Funkreichweite (RNG, komprimierter Bereich)
    pub range_km: Option<f64>,
    /// Mic-E-Statusmeldung (En Route, Off Duty …)
    pub mice: Option<&'static str>,
    /// Gerät bzw. Programm (aus Mic-E-Kennung oder Zielrufzeichen)
    pub device: Option<String>,
    /// Zeitstempel im Paket
    pub time: Option<String>,
    pub wx: Option<Wx>,
    /// Kommentar ohne die oben ausgewerteten Teile
    pub comment: String,
    /// Telemetrie im Kommentar (|…|) war dabei
    pub telemetry: bool,
}

fn b91(b: &[u8]) -> f64 { b.iter().fold(0.0, |a, &c| a * 91.0 + (c as f64 - 33.0)) }

/// Gerät aus dem Zielrufzeichen (Tocall), soweit gängig
pub fn tocall_device(dst: &str) -> Option<&'static str> {
    let d = aprs_split(dst);
    const T: &[(&str, &str)] = &[
        ("APDR", "APRSdroid"), ("APDW", "Dire Wolf"), ("APRX", "aprx"), ("APX", "Xastir"), ("APU25", "UI-View"), ("APWW", "APRSIS32"),
        ("APWM", "APRSIS32 mobil"), ("APAGW", "AGWtracker"), ("APOT", "OpenTracker"), ("APOTC", "OpenTracker"), ("APLRG", "LoRa-APRS-iGate"),
        ("APLRT", "LoRa-APRS-Tracker"), ("APLT", "LoRa-APRS-Tracker"), ("APLG", "LoRa-APRS-Gateway"), ("APBM", "BrandMeister (DMR)"),
        ("APDPRS", "D-PRS (D-STAR)"), ("APDG", "ircDDB-Gateway"), ("APAT", "Anytone"), ("APK0", "Kenwood"), ("APK1", "Kenwood TM-D700"),
        ("APK003", "Kenwood TH-D72"), ("APK004", "Kenwood TH-D74"), ("APK005", "Kenwood TH-D75"), ("APY", "Yaesu"), ("APMI", "SQ5EKU-Digi"),
        ("APNU", "UIdigi"), ("APN3", "Kantronics KPC-3"), ("APNW", "SQ3FYK-Digi"), ("APTT", "TinyTrak"), ("APT3", "TinyTrak3"), ("APT4", "TinyTrak4"),
        ("APZMDM", "MMDVM"), ("APMDM", "MMDVM"), ("APLIG", "LoRa-iGate"), ("APPIC", "PicoAPRS"), ("APRARX", "radiosonde_auto_rx"),
        ("APE", "Telemetrie"), ("APJ", "JavAPRS"), ("APAND", "APRSdroid (alt)"), ("APZSST", "Rustwave"), ("APMG", "MiniGate"),
        ("APTW", "WXTrak"), ("APFII", "aprs.fi-App"), ("APIN", "PinPoint"), ("APHBL", "HamBeacon"), ("APNK", "Kenwood TM-D710 (Digi)"),
    ];
    // längster passender Anfang gewinnt
    T.iter().filter(|(p, _)| d.starts_with(p)).max_by_key(|(p, _)| p.len()).map(|t| t.1)
}
fn aprs_split(c: &str) -> &str { c.split('-').next().unwrap_or(c) }

/// Mic-E-Gerätekennung: erstes Zeichen des Kommentars und Endung
fn mice_device(c: &str) -> (Option<String>, String) {
    let b = c.as_bytes();
    let Some(&first) = b.first() else { return (None, String::new()) };
    let ends = |s: &str| c.ends_with(s);
    let strip = |pre: usize, suf: usize| c.get(pre..c.len().saturating_sub(suf)).unwrap_or("").to_string();
    match first {
        b'>' => {
            if ends("=") { (Some("Kenwood TH-D72".into()), strip(1, 1)) }
            else if ends("^") { (Some("Kenwood TH-D74".into()), strip(1, 1)) }
            else if ends("&") { (Some("Kenwood TH-D75".into()), strip(1, 1)) }
            else { (Some("Kenwood TH-D7A".into()), strip(1, 0)) }
        }
        b']' => if ends("=") { (Some("Kenwood TM-D710".into()), strip(1, 1)) } else { (Some("Kenwood TM-D700".into()), strip(1, 0)) },
        b'`' | b'\'' if c.len() >= 3 => {
            const Y: &[(&str, &str)] = &[("_ ", "Yaesu VX-8"), ("_\"", "Yaesu FTM-350"), ("_#", "Yaesu VX-8G"), ("_$", "Yaesu FT1D"),
                ("_%", "Yaesu FTM-400DR"), ("_)", "Yaesu FTM-100D"), ("_(", "Yaesu FT2D"), ("_0", "Yaesu FT3D"), ("_3", "Yaesu FT5D"),
                ("_1", "Yaesu FTM-300D"), ("_5", "Yaesu FTM-500D"), ("_2", "Yaesu FTM-200D"), ("|3", "Byonics TinyTrak3"), ("|4", "Byonics TinyTrak4"),
                ("^v", "HinzTec anyfrog"), (":4", "SCS P4dragon DR-7400"), (":8", "SCS P4dragon DR-7800")];
            match Y.iter().find(|(s, _)| ends(s)) {
                Some((s, n)) => (Some(n.to_string()), strip(1, s.len())),
                None => (None, strip(1, 0)),
            }
        }
        _ => (None, c.to_string()),
    }
}

/// Mic-E-Statusmeldung aus den ersten drei Zeichen des Zielfelds
fn mice_status(dst: &str) -> Option<&'static str> {
    let d = dst.as_bytes();
    if d.len() < 3 { return None; }
    let bit = |c: u8| match c { b'P'..=b'Z' => Some((1, false)), b'A'..=b'K' => Some((1, true)), b'0'..=b'9' | b'L' => Some((0, false)), _ => None };
    let (mut v, mut custom) = (0, false);
    for &c in &d[..3] { let (b, cu) = bit(c)?; v = v * 2 + b; custom |= cu; }
    const STD: [&str; 8] = ["Notfall", "Priorität", "Besonderes", "Im Einsatz (Committed)", "Auf dem Rückweg", "Im Dienst", "Unterwegs", "Außer Dienst"];
    const CUS: [&str; 8] = ["Notfall", "Eigene Meldung 6", "Eigene Meldung 5", "Eigene Meldung 4", "Eigene Meldung 3", "Eigene Meldung 2", "Eigene Meldung 1", "Eigene Meldung 0"];
    Some(if custom && v != 0 { CUS[v] } else { STD[v] })
}

/// PHGphgd → Klartext und Reichweitenschätzung
fn phg(s: &str) -> Option<(String, f64)> {
    let b = s.as_bytes();
    if b.len() < 4 || !b.iter().take(4).all(|c| c.is_ascii_digit()) { return None; }
    let d = |i: usize| (b[i] - b'0') as f64;
    let (p, h_ft, g) = (d(0) * d(0), 10.0 * 2f64.powf(d(1)), d(2));
    let dir = match b[3] { b'0' => "rundum".to_string(), c => format!("Richtung {}", aprs::compass((c - b'0') as f64 * 45.0)) };
    // Faustformel aus der APRS-Spezifikation (Meilen) → km
    let range = (2.0 * h_ft * (p / 10.0 * 10f64.powf(g / 10.0)).sqrt()).sqrt() * 1.609;
    Some((format!("{p:.0} W, Antenne {:.0} m über Grund, {g:.0} dBd, {dir}", h_ft * 0.3048), range))
}

/// Zeitstempel DDHHMMz / DDHHMM/ / HHMMSSh
fn timestamp(t: &[u8]) -> Option<String> {
    if t.len() < 7 || !t[..6].iter().all(|c| c.is_ascii_digit()) { return None; }
    let s = String::from_utf8_lossy(&t[..6]);
    Some(match t[6] {
        b'z' => format!("Tag {} {}:{} UTC", &s[0..2], &s[2..4], &s[4..6]),
        b'/' => format!("Tag {} {}:{} Ortszeit", &s[0..2], &s[2..4], &s[4..6]),
        b'h' => format!("{}:{}:{} UTC", &s[0..2], &s[2..4], &s[4..6]),
        _ => return None,
    })
}

/// Erweiterungen im Kommentar: Kurs/Tempo bzw. Wind am Anfang, PHG, RNG, /A=, !DAO!, Telemetrie
fn comment_ext(d: &mut Details, c: &str, wx_symbol: bool) {
    let mut c = c.to_string();
    let b = c.as_bytes();
    // Wetter: mit Wettersymbol immer, sonst wenn es so aussieht („DDD/SSSg…“ bzw. „…t…“)
    let looks_wx = b.len() >= 8 && b[3] == b'/' && matches!(b[7], b'g' | b't');
    let wx = if wx_symbol || looks_wx { aprs::parse_wx_len(&c) } else { None };
    if let Some((w, n)) = wx {
        d.wx = Some(w);
        c = c[n..].to_string();
    } else if b.len() >= 7 && b[3] == b'/' && b[..3].iter().chain(&b[4..7]).all(|x| x.is_ascii_digit() || *x == b'.' || *x == b' ') {
        let course: Option<f64> = c[0..3].parse().ok();
        let speed: Option<f64> = c[4..7].parse().ok();
        if let (Some(co), Some(sp)) = (course, speed) {
            if co > 0.0 { d.course = Some(co % 360.0); }
            d.speed_kmh = Some(sp * 1.852);
        }
        c = c[7..].to_string();
        // Peilung/Signal (/BRG/NRQ) – selten, nur abschneiden
        if c.len() >= 8 && c.as_bytes()[0] == b'/' && c.as_bytes()[4] == b'/' { c = c[8..].to_string(); }
    }
    if let Some(i) = c.find("PHG") {
        if let Some((t, r)) = c.get(i + 3..i + 7).and_then(phg) { d.phg = Some(t); d.range_km.get_or_insert(r); c.replace_range(i..i + 7, ""); }
    }
    if let Some(i) = c.find("RNG") {
        if let Some(r) = c.get(i + 3..i + 7).and_then(|v| v.parse::<f64>().ok()) { d.range_km = Some(r * 1.609); c.replace_range(i..i + 7, ""); }
    }
    if let Some(i) = c.find("/A=") {
        if let Some(a) = c.get(i + 3..i + 9).and_then(|v| v.parse::<f64>().ok()) { d.alt_m = Some(a * 0.3048); c.replace_range(i..i + 9, ""); }
    }
    // !DAO! (Positionsverfeinerung)
    if let Some(i) = c.find('!') {
        if c.get(i + 4..i + 5) == Some("!") { c.replace_range(i..i + 5, ""); }
    }
    // Base91-Telemetrie |ss11223344|
    if let Some(i) = c.find('|') {
        if let Some(j) = c[i + 1..].find('|') {
            if (4..=14).contains(&j) && j % 2 == 0 { d.telemetry = true; c.replace_range(i..i + j + 2, ""); }
        }
    }
    d.comment = c.trim().to_string();
}

/// Paket auswerten; None, wenn es nichts über die Station sagt (Nachricht, Bestätigung …)
pub fn details(f: &Frame) -> Option<Details> {
    let i = &f.info;
    let t = *i.first()?;
    let mut d = Details { device: tocall_device(&f.dst).map(str::to_string), ..Default::default() };
    match t {
        b'!' | b'=' | b'/' | b'@' | b';' => {
            let (start, kind) = match t {
                b'/' | b'@' => { d.time = i.get(1..8).and_then(timestamp); (8, "Position mit Zeit") }
                b';' => { d.time = i.get(11..18).and_then(timestamp); (18, "Objekt") }
                _ => (1, "Position"),
            };
            let s = i.get(start..)?;
            if s.first().is_some_and(|c| c.is_ascii_digit()) {
                if s.len() < 19 { return None; }
                d.kind = kind;
                let sym = s[18];
                let comment = String::from_utf8_lossy(&s[19..]).to_string();
                comment_ext(&mut d, &comment, sym == b'_');
            } else {
                if s.len() < 13 { return None; }
                d.kind = if t == b';' { "Objekt (komprimiert)" } else { "Position (komprimiert)" };
                let (c, sp, ty) = (s[10], s[11], s[12]);
                if c != b' ' {
                    if (ty.wrapping_sub(33)) & 0x18 == 0x10 {
                        d.alt_m = Some(1.002f64.powf(b91(&s[10..12])) * 0.3048);
                    } else if c == b'{' {
                        d.range_km = Some(2.0 * 1.08f64.powf(sp as f64 - 33.0) * 1.609);
                    } else if (b'!'..=b'z').contains(&c) {
                        let co = (c as f64 - 33.0) * 4.0;
                        if co > 0.0 { d.course = Some(co % 360.0); }
                        d.speed_kmh = Some((1.08f64.powf(sp as f64 - 33.0) - 1.0) * 1.852);
                    }
                }
                let comment = String::from_utf8_lossy(&s[13..]).to_string();
                let wx_sym = s[9] == b'_';
                comment_ext(&mut d, &comment, wx_sym);
                if wx_sym {
                    // bei Wetterstationen steht im Kurs/Tempo-Feld der Wind
                    let w = d.wx.get_or_insert_with(Default::default);
                    if w.dir.is_none() { w.dir = d.course.take(); }
                    if w.wind_kmh.is_none() { w.wind_kmh = d.speed_kmh.take().map(|v| v / 1.852 * 1.609); }
                    d.course = None;
                    d.speed_kmh = None;
                }
            }
        }
        b'`' | b'\'' | 0x1c | 0x1d if i.len() >= 9 => {
            d.kind = "Mic-E";
            let sp = (i[4] as i32 - 28) * 10 + (i[5] as i32 - 28) / 10;
            let sp = if sp >= 800 { sp - 800 } else { sp };
            let co = ((i[5] as i32 - 28) % 10) * 100 + (i[6] as i32 - 28);
            let co = if co >= 400 { co - 400 } else { co };
            if (0..=360).contains(&co) && co > 0 { d.course = Some(co as f64 % 360.0); }
            if (0..800).contains(&sp) { d.speed_kmh = Some(sp as f64 * 1.852); }
            d.mice = mice_status(&f.dst);
            let raw = String::from_utf8_lossy(&i[9..]).to_string();
            let (dev, mut c) = mice_device(&raw);
            if dev.is_some() { d.device = dev; } else if d.device.is_some() { /* Tocall ist bei Mic-E die Breite */ d.device = None; }
            // Höhe: drei Base91-Zeichen und '}' (Meter über −10 km)
            if let Some(k) = c.find('}') {
                if k >= 3 {
                    let a = c.as_bytes()[k - 3..k].to_vec();
                    if a.iter().all(|x| (33..=123).contains(x)) { d.alt_m = Some(b91(&a) - 10000.0); c.replace_range(k - 3..=k, ""); }
                }
            }
            comment_ext(&mut d, &c, i[7] == b'_');
        }
        b'_' => {
            d.kind = "Wetterbericht";
            d.time = i.get(1..9).map(|t| String::from_utf8_lossy(t).to_string()).map(|s| format!("{}.{}. {}:{}", &s[2..4], &s[0..2], &s[4..6], &s[6..8]));
            let body = i.get(9..).map(|w| String::from_utf8_lossy(w).to_string()).unwrap_or_default();
            if let Some((w, n)) = aprs::parse_wx_len(&body) { d.wx = Some(w); d.comment = body[n..].trim().to_string(); }
        }
        b'>' => { d.kind = "Status"; d.comment = String::from_utf8_lossy(&i[1..]).trim().to_string(); }
        _ => return None,
    }
    Some(d)
}

/// Maidenhead-Locator (6 Stellen)
pub fn locator(lat: f64, lon: f64) -> String {
    let (x, y) = ((lon + 180.0).clamp(0.0, 359.9999), (lat + 90.0).clamp(0.0, 179.9999));
    let f = |v: f64, d: f64| (v / d).floor() as u8;
    let (a, b) = (f(x, 20.0), f(y, 10.0));
    let (c, d) = (f(x % 20.0, 2.0), f(y % 10.0, 1.0));
    let (e, g) = (f(x % 2.0, 2.0 / 24.0), f(y % 1.0, 1.0 / 24.0));
    format!("{}{}{}{}{}{}", (b'A' + a) as char, (b'A' + b) as char, c, d, (b'a' + e) as char, (b'a' + g) as char)
}
/// 52°30,12' N
pub fn dm(v: f64, pos: char, neg: char) -> String {
    let a = v.abs();
    format!("{}°{:05.2}' {}", a.trunc() as u32, a.fract() * 60.0, if v >= 0.0 { pos } else { neg }).replace('.', ",")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fr(dst: &str, info: &str) -> Frame { Frame { src: "DL1ABC-9".into(), dst: dst.into(), path: vec![], info: info.as_bytes().to_vec() } }

    #[test]
    fn unkomprimiert_mit_kurs_hoehe_phg() {
        let d = details(&fr("APDR16", "!5230.12N/01324.56E>088/036/A=001234 PHG5130 Hallo")).unwrap();
        assert_eq!(d.course, Some(88.0));
        assert!((d.speed_kmh.unwrap() - 66.67).abs() < 0.1);
        assert!((d.alt_m.unwrap() - 376.1).abs() < 0.2);
        assert!(d.phg.as_deref().unwrap().starts_with("25 W, Antenne 6 m"));
        assert_eq!(d.device.as_deref(), Some("APRSdroid"));
        assert_eq!(d.comment, "Hallo");
    }
    #[test]
    fn wetter_mit_wind() {
        let d = details(&fr("APRS", "@201130z5230.12N/01324.56E_270/012g020t041r000p001P000h85b10132 WX DB0ABC")).unwrap();
        let w = d.wx.unwrap();
        assert_eq!(w.dir, Some(270.0));
        assert!((w.temp_c.unwrap() - 5.0).abs() < 0.01);
        assert_eq!(w.hum, Some(85));
        assert_eq!(d.time.as_deref(), Some("Tag 20 11:30 UTC"));
        assert_eq!(d.comment, "WX DB0ABC");
    }
    #[test]
    fn mic_e() {
        // Beispiel aus APRS 1.0.1 Kap. 10: Ziel S32U6T, Kurs 251°, 20 Knoten; Kenwood TM-D700 mit Höhe
        let mut info = vec![b'`', b'(', b'_', b'f', b'n', b'\"', b'O', b'j', b'/', b']'];
        info.extend_from_slice(b"\"4T}Test=");
        let f = Frame { src: "DL1ABC-9".into(), dst: "S32U6T".into(), path: vec![], info };
        let d = details(&f).unwrap();
        assert_eq!(d.kind, "Mic-E");
        assert_eq!(d.course, Some(251.0));
        assert!((d.speed_kmh.unwrap() - 20.0 * 1.852).abs() < 0.01);
        assert_eq!(d.mice, Some("Auf dem Rückweg"));
        assert_eq!(d.device.as_deref(), Some("Kenwood TM-D710"));
        assert!(d.alt_m.is_some());
        assert_eq!(d.comment, "Test");
    }
    #[test]
    fn komprimiert_kurs() {
        // „/5L!!<*e7>7P[“: Kurs 88°, 36,2 Knoten (Beispiel der Spezifikation)
        let d = details(&fr("APRS", "!/5L!!<*e7>7P[")).unwrap();
        assert_eq!(d.course, Some(88.0));
        assert!((d.speed_kmh.unwrap() / 1.852 - 36.2).abs() < 0.2);
    }
    #[test]
    fn wetter_varianten() {
        // ohne Wettersymbol, aber Wetterdaten; Stationskennung; Kommentar bleibt übrig
        let d = details(&fr("APRS", "!5230.12N/01324.56E-180/005g010t050h70b10150wRSW Garten")).unwrap();
        let w = d.wx.unwrap();
        assert_eq!(w.dir, Some(180.0));
        assert!(w.station.unwrap().contains("Radio Shack"));
        assert_eq!(d.comment, "Garten");
        assert_eq!(d.course, None);
        // komprimiert mit Wettersymbol: Kurs/Tempo-Feld ist der Wind
        let d = details(&fr("APRS", "=/5L!!<*e7_7P[g012t045h60b10100")).unwrap();
        let w = d.wx.unwrap();
        assert_eq!(w.dir, Some(88.0));
        assert!((w.wind_kmh.unwrap() / 1.609 - 36.2).abs() < 0.3);
        assert!((w.temp_c.unwrap() - 7.22).abs() < 0.01);
        assert_eq!((d.course, d.speed_kmh), (None, None));
        // positionslos mit Kennung
        let d = details(&fr("APRS", "_11201830c090s003g008t038r001p004P002h91b10088eCumulusDsVP")).unwrap();
        assert!(d.wx.as_ref().unwrap().station.as_deref().unwrap().starts_with("Cumulus"));
        assert_eq!(d.comment, "");
    }

    #[test]
    fn mic_e_kodieren() {
        for &(la, lo) in &[(52.52345, 13.40567), (-33.8688, 151.2093), (40.7128, -74.006), (0.5, -9.5), (64.1, -105.2)] {
            let (dst, info) = crate::aprs::mice_encode(la, lo, Some(251.0), Some(37.0), ('/', '>'), Some(61.0), crate::aprs::MICE_EN_ROUTE, "Test");
            let f = Frame { src: "DL1ABC-9".into(), dst: dst.clone(), path: vec![], info };
            match crate::aprs::parse(&f) {
                crate::aprs::Body::Position { lat, lon, symbol, .. } => {
                    assert!((lat - la).abs() < 0.0002 && (lon - lo).abs() < 0.0002, "{la},{lo} → {lat},{lon} ({dst})");
                    assert_eq!(symbol, ('/', '>'));
                }
                b => panic!("{b:?}"),
            }
            let d = details(&f).unwrap();
            assert_eq!(d.course, Some(251.0));
            assert!((d.speed_kmh.unwrap() - 20.0 * 1.852).abs() < 0.01);
            assert!((d.alt_m.unwrap() - 61.0).abs() < 0.6);
            assert_eq!(d.mice, Some("Unterwegs"));
            assert_eq!(d.comment, "Test");
        }
    }

    #[test]
    fn loc() {
        assert_eq!(locator(52.5, 13.4), "JO62qm");
        assert_eq!(dm(52.502, 'N', 'S'), "52°30,12' N");
    }
}

/// Demo-Aufnahme mit typischen Paketen (Mobil mit Kurs, Mic-E, Wetterstation, Digipeater):
/// APRS_DEMO=/pfad.wav cargo test --release aprs_demo_wav -- --ignored
#[cfg(test)]
#[test]
#[ignore]
fn aprs_demo_wav() {
    let mk = |src: &str, dst: &str, path: &[&str], info: &[u8]| Frame { src: src.into(), dst: dst.into(), path: path.iter().map(|s| s.to_string()).collect(), info: info.to_vec() };
    let mut mice = vec![b'`', b')', b'4', b':', b'n', b'\"', b'O', b'>', b'/', b']'];
    mice.extend_from_slice(b"\"4T}Unterwegs nach Hause=");
    let frames = vec![
        mk("DL1ABC-9", "APDR16", &["DB0XYZ*", "WIDE2-1"], b"!5234.12N/01318.56E>088/036/A=000164 PHG2360 QRV 145.500"),
        mk("DB0WX", "APRS", &["WIDE1-1"], b"@201130z5226.40N/01330.20E_250/012g025t035r002p010P004h87b10132s001L250eCumulusDsVP Wetter Kreuzberg"),
        mk("DO2XY-7", "UR3Q50", &["DB0XYZ*"], &mice),
        mk("DB0XYZ", "APMI06", &[], b"!5230.00N/01324.00E#PHG7530 Digi + iGate Berlin"),
        mk("DL9WX", "APRS", &[], b"_11201830c090s003g008t038r001p004P002h91b10088xDvs"),
        mk("DL1ABC-9", "APDR16", &["DB0XYZ*"], b"!5235.00N/01320.10E>090/040"),
        mk("DL1ABC-9", "APDR16", &["DB0XYZ*"], b"!5235.90N/01322.40E>070/045"),
        mk("DL1ABC-9", "APDR16", &["DB0XYZ*"], b"!5236.20N/01325.80E>100/050"),
        mk("DB0XYZ", "APMI06", &[], b":DB0XYZ   :PARM.Akku,Temp,Pakete,,,Tuer,Netz"),
        mk("DB0XYZ", "APMI06", &[], b":DB0XYZ   :UNIT.V,deg.C,/h,,,offen,an"),
        mk("DB0XYZ", "APMI06", &[], b":DB0XYZ   :EQNS.0,0.1,0,0,0.5,-20,0,1,0,0,1,0,0,1,0"),
        mk("DB0XYZ", "APMI06", &[], b":DB0XYZ   :BITS.11111111,Digi Berlin-Mitte"),
        mk("DB0XYZ", "APMI06", &[], b"T#042,138,061,215,000,000,01000000"),
        mk("DL2XYZ", "APDR16", &[], b";FIELDDAY *201200z5231.50N/01326.00E;Fieldday OV D01, 145.500"),
    ];
    let x = crate::aprs::modulate(&frames, 48000.0, 300.0, crate::aprs::VHF);
    let mut w = hound::WavWriter::create(std::env::var("APRS_DEMO").unwrap(), hound::WavSpec { channels: 1, sample_rate: 48000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int }).unwrap();
    for v in std::iter::repeat_n(0.0, 4800).chain(x).chain(std::iter::repeat_n(0.0, 4800)) { w.write_sample((v * 16000.0) as i16).unwrap(); }
    w.finalize().unwrap();
}
