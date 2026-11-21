//! Logbuch: QSOs speichern, frühere Verbindungen nachschlagen, ADIF-Export.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
pub struct Qso {
    pub call: String,
    pub name: String,
    pub qth: String,
    pub rsv_sent: String,
    pub rsv_rcvd: String,
    pub note: String,
    /// Unix-Sekunden UTC
    pub start: i64,
    pub end: i64,
    pub freq_hz: Option<u64>,
    /// Untermodus, z. B. "Martin M1" oder "45 Bd / 170 Hz"
    pub sstv_mode: String,
    /// ADIF-Betriebsart ("SSTV" oder "RTTY"); ältere Logbücher ohne Feld sind SSTV
    #[serde(default = "mode_sstv")]
    pub mode: String,
}
fn mode_sstv() -> String { "SSTV".into() }

/// Was über ein Rufzeichen schon bekannt ist
#[derive(Debug, PartialEq)]
pub struct Known { pub count: usize, pub name: String, pub qth: String, pub last: i64 }

pub struct Log { pub qsos: Vec<Qso>, path: PathBuf }

/// Amateurfunkband aus der Frequenz (ADIF-Bandbezeichnung)
pub fn band(hz: u64) -> &'static str {
    const BANDS: [(u64, u64, &str); 13] = [
        (1_800_000, 2_000_000, "160m"), (3_500_000, 4_000_000, "80m"), (5_060_000, 5_450_000, "60m"),
        (7_000_000, 7_300_000, "40m"), (10_100_000, 10_150_000, "30m"), (14_000_000, 14_350_000, "20m"),
        (18_068_000, 18_168_000, "17m"), (21_000_000, 21_450_000, "15m"), (24_890_000, 24_990_000, "12m"),
        (28_000_000, 29_700_000, "10m"), (50_000_000, 54_000_000, "6m"), (144_000_000, 148_000_000, "2m"),
        (420_000_000, 450_000_000, "70cm"),
    ];
    BANDS.iter().find(|(a, b, _)| (*a..=*b).contains(&hz)).map_or("", |b| b.2)
}

pub fn normalize_call(c: &str) -> String { c.trim().to_uppercase() }

impl Log {
    pub fn load(path: PathBuf) -> Log {
        let qsos = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Log { qsos, path }
    }
    pub fn save(&self) -> Result<(), String> {
        if let Some(d) = self.path.parent() { std::fs::create_dir_all(d).map_err(|e| e.to_string())?; }
        // erst in Zwischendatei schreiben, dann umbenennen: kein halbes Logbuch bei Absturz
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&self.qsos).unwrap()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }

    /// Frühere QSOs mit diesem Rufzeichen (neuester Name/QTH gewinnt)
    pub fn lookup(&self, call: &str) -> Option<Known> {
        let call = normalize_call(call);
        if call.is_empty() { return None; }
        let mut hits: Vec<&Qso> = self.qsos.iter().filter(|q| q.call == call).collect();
        hits.sort_by_key(|q| q.start);
        let last = hits.last()?;
        let pick = |f: fn(&Qso) -> &String| hits.iter().rev().map(|q| f(q)).find(|s| !s.is_empty()).cloned().unwrap_or_default();
        Some(Known { count: hits.len(), name: pick(|q| &q.name), qth: pick(|q| &q.qth), last: last.start })
    }

    /// Neues QSO anhängen oder (gleicher Start) aktualisieren; liefert den Index
    pub fn upsert(&mut self, q: Qso) -> usize {
        if let Some(i) = self.qsos.iter().position(|x| x.start == q.start && x.call == q.call) {
            self.qsos[i] = q;
            i
        } else {
            self.qsos.push(q);
            self.qsos.len() - 1
        }
    }

    pub fn export_adif(&self, path: &Path) -> Result<usize, String> {
        if let Some(d) = path.parent() { std::fs::create_dir_all(d).map_err(|e| e.to_string())?; }
        std::fs::write(path, to_adif(&self.qsos)).map_err(|e| e.to_string())?;
        Ok(self.qsos.len())
    }
}

/// ADI-Textfelder erlauben nur ASCII: Umlaute und Akzente umschreiben, Rest durch '?'
fn ascii(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            ' '..='~' => out.push(c),
            'ä' => out.push_str("ae"), 'ö' => out.push_str("oe"), 'ü' => out.push_str("ue"),
            'Ä' => out.push_str("Ae"), 'Ö' => out.push_str("Oe"), 'Ü' => out.push_str("Ue"), 'ß' => out.push_str("ss"),
            'à' | 'á' | 'â' | 'ã' | 'å' => out.push('a'), 'è' | 'é' | 'ê' | 'ë' => out.push('e'), 'ì' | 'í' | 'î' | 'ï' => out.push('i'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ø' => out.push('o'), 'ù' | 'ú' | 'û' => out.push('u'), 'ç' => out.push('c'), 'ñ' => out.push('n'),
            'À' | 'Á' | 'Â' | 'Ã' | 'Å' => out.push('A'), 'È' | 'É' | 'Ê' | 'Ë' => out.push('E'), 'Ç' => out.push('C'), 'Ñ' => out.push('N'),
            '·' | '–' | '—' => out.push('-'),
            _ => out.push('?'),
        }
    }
    out
}

fn field(out: &mut String, name: &str, v: &str) {
    let v = ascii(v);
    if !v.is_empty() { out.push_str(&format!("<{}:{}>{} ", name, v.len(), v)); }
}

/// ADIF 3.1 (Modus SSTV), reines ASCII
pub fn to_adif(qsos: &[Qso]) -> String {
    let mut out = String::from("Rustwave ADIF-Export\n<ADIF_VER:5>3.1.4 <PROGRAMID:8>Rustwave <EOH>\n");
    for q in qsos {
        let t0 = DateTime::<Utc>::from_timestamp(q.start, 0).unwrap_or_default();
        let t1 = DateTime::<Utc>::from_timestamp(q.end.max(q.start), 0).unwrap_or_default();
        field(&mut out, "CALL", &q.call);
        field(&mut out, "QSO_DATE", &t0.format("%Y%m%d").to_string());
        field(&mut out, "TIME_ON", &t0.format("%H%M%S").to_string());
        field(&mut out, "QSO_DATE_OFF", &t1.format("%Y%m%d").to_string());
        field(&mut out, "TIME_OFF", &t1.format("%H%M%S").to_string());
        if let Some(f) = q.freq_hz {
            field(&mut out, "BAND", band(f));
            field(&mut out, "FREQ", &format!("{:.6}", f as f64 / 1e6));
        }
        field(&mut out, "MODE", if q.mode.is_empty() { "SSTV" } else { &q.mode });
        field(&mut out, "RST_SENT", &q.rsv_sent);
        field(&mut out, "RST_RCVD", &q.rsv_rcvd);
        field(&mut out, "NAME", &q.name);
        field(&mut out, "QTH", &q.qth);
        let comment = [q.sstv_mode.as_str(), q.note.as_str()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" - ");
        field(&mut out, "COMMENT", &comment);
        out.push_str("<EOR>\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(call: &str, start: i64, name: &str, qth: &str) -> Qso {
        Qso { call: call.into(), name: name.into(), qth: qth.into(), rsv_sent: "595".into(), rsv_rcvd: "575".into(),
              start, end: start + 600, freq_hz: Some(14_230_000), sstv_mode: "Martin M1".into(), ..Default::default() }
    }

    #[test]
    fn alte_logbuecher_sind_sstv_und_rtty_wird_exportiert() {
        let alt = r#"[{"call":"DL1ABC","name":"","qth":"","rsv_sent":"","rsv_rcvd":"","note":"","start":1,"end":2,"freq_hz":null,"sstv_mode":"Martin M1"}]"#;
        let v: Vec<Qso> = serde_json::from_str(alt).unwrap();
        assert_eq!(v[0].mode, "SSTV");
        let mut r = q("F4XYZ", 1000, "", "");
        r.mode = "RTTY".into();
        assert!(to_adif(&[r]).contains("<MODE:4>RTTY "));
    }

    #[test]
    fn baender() {
        assert_eq!(band(14_230_000), "20m");
        assert_eq!(band(3_735_000), "80m");
        assert_eq!(band(145_500_000), "2m");
        assert_eq!(band(12_000_000), "");
    }

    #[test]
    fn nachschlagen_neuester_name_gewinnt() {
        let dir = std::env::temp_dir().join(format!("sstv_log_{}", std::process::id()));
        let mut log = Log::load(dir.join("logbuch.json"));
        log.upsert(q("DL1ABC", 1000, "Hans", "Berlin"));
        log.upsert(q("DL1ABC", 5000, "", "Potsdam"));   // Name leer gelassen
        log.upsert(q("F4XYZ", 3000, "Marie", "Paris"));
        assert_eq!(log.lookup(" dl1abc "), Some(Known { count: 2, name: "Hans".into(), qth: "Potsdam".into(), last: 5000 }));
        assert_eq!(log.lookup("G0AAA"), None);
        // gleiches QSO erneut: wird aktualisiert, nicht verdoppelt
        let mut upd = q("F4XYZ", 3000, "Marie", "Lyon");
        upd.note = "nachgetragen".into();
        log.upsert(upd);
        assert_eq!(log.qsos.len(), 3);
        log.save().unwrap();
        assert_eq!(Log::load(dir.join("logbuch.json")).qsos, log.qsos);
        // Export in einen Ordner, den es noch nicht gibt
        assert_eq!(log.export_adif(&dir.join("neu").join("x.adi")), Ok(3));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn adif_format() {
        let mut x = q("DL1ABC", 1_763_300_000, "Jürgen", "München");
        x.note = "TNX".into();
        let s = to_adif(&[x]);
        assert!(s.contains("<ADIF_VER:5>3.1.4") && s.contains("<EOH>"));
        assert!(s.contains("<CALL:6>DL1ABC "));
        assert!(s.contains("<QSO_DATE:8>20251116 ") && s.contains("<TIME_ON:6>"), "{s}");
        assert!(s.contains("<BAND:3>20m ") && s.contains("<FREQ:9>14.230000 ") && s.contains("<MODE:4>SSTV "));
        assert!(s.contains("<NAME:7>Juergen ") && s.contains("<QTH:8>Muenchen "), "Umlaute umschreiben: {s}");
        assert!(s.contains("<COMMENT:15>Martin M1 - TNX "), "{s}");
        assert!(s.is_ascii());
        assert!(s.trim_end().ends_with("<EOR>"));
    }
}
