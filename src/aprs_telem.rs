//! APRS-Telemetrie (APRS 1.0.1 Kap. 13): Messwerte „T#…“, Base91-Telemetrie im Kommentar „|ss1122…|“ und die
//! Beschreibung per Nachricht an sich selbst (PARM. Namen, UNIT. Einheiten, EQNS. Umrechnung a·x²+b·x+c, BITS. Bits/Titel).

#[derive(Clone, Debug, PartialEq)]
pub struct Telem {
    pub seq: Option<u32>,
    /// Rohwerte der fünf analogen Kanäle
    pub raw: [Option<f64>; 5],
    pub bits: Option<u8>,
    pub names: [String; 13],
    pub units: [String; 13],
    pub eqns: [(f64, f64, f64); 5],
    /// Bit gilt als „an“, wenn es diesem Wert entspricht (BITS.)
    pub sense: u8,
    pub title: String,
    /// Unix-Zeit der letzten Messwerte
    pub time: Option<i64>,
}

impl Default for Telem {
    fn default() -> Self {
        Telem { seq: None, raw: [None; 5], bits: None, names: Default::default(), units: Default::default(),
                eqns: [(0.0, 1.0, 0.0); 5], sense: 0xFF, title: String::new(), time: None }
    }
}

impl Telem {
    /// umgerechneter Wert von Kanal i
    pub fn value(&self, i: usize) -> Option<f64> {
        let x = self.raw[i]?;
        let (a, b, c) = self.eqns[i];
        Some(a * x * x + b * x + c)
    }
    /// Zeilen für die Anzeige: (Name, Wert mit Einheit)
    pub fn rows(&self) -> Vec<(String, String)> {
        let mut r = vec![];
        // mit Beschreibung: unbenannte Kanäle sind unbenutzt
        let named = self.names[..5].iter().any(|n| !n.is_empty());
        for i in 0..5 {
            let Some(v) = self.value(i) else { continue };
            if named && self.names[i].is_empty() { continue; }
            let name = if self.names[i].is_empty() { format!("Kanal {}", i + 1) } else { self.names[i].clone() };
            let digits = if v.abs() >= 100.0 || v.fract() == 0.0 { 0 } else if v.abs() >= 10.0 { 1 } else { 2 };
            r.push((name, format!("{v:.digits$} {}", self.units[i]).trim().to_string()));
        }
        if let Some(b) = self.bits {
            for k in 0..8 {
                let name = &self.names[5 + k];
                if name.is_empty() { continue; } // unbenannte Bits nicht zeigen
                let on = (b >> (7 - k)) & 1 == (self.sense >> (7 - k)) & 1;
                let label = if self.units[5 + k].is_empty() { if on { "an".to_string() } else { "aus".to_string() } }
                    else if on { self.units[5 + k].clone() } else { format!("nicht {}", self.units[5 + k]) };
                r.push((name.clone(), label));
            }
        }
        r
    }
}

/// „T#005,199,000,255,073,123,01101001“ (auch „T#MIC“) → Folgenummer, Werte, Bits
pub fn parse_t(info: &str) -> Option<(Option<u32>, [Option<f64>; 5], Option<u8>)> {
    let s = info.strip_prefix("T#")?;
    let s = s.strip_prefix("MIC").map(|r| r.trim_start_matches(',')).map_or_else(|| s.to_string(), |r| format!("_,{r}"));
    let mut p = s.split(',');
    let seq = p.next()?.trim().parse::<u32>().ok();
    let mut raw = [None; 5];
    for v in raw.iter_mut() { *v = p.next().and_then(|x| x.trim().parse::<f64>().ok()); }
    let bits = p.next().map(|b| b.trim()).filter(|b| b.len() >= 8 && b[..8].chars().all(|c| c == '0' || c == '1'))
        .and_then(|b| u8::from_str_radix(&b[..8], 2).ok());
    raw.iter().any(|v| v.is_some()).then_some((seq, raw, bits))
}

/// Base91-Telemetrie im Kommentar: |ss11223344556677| – Folge, bis zu fünf Werte, optional Bits
pub fn parse_b91(comment: &str) -> Option<(Option<u32>, [Option<f64>; 5], Option<u8>)> {
    let i = comment.find('|')?;
    let j = comment[i + 1..].find('|')? + i + 1;
    let b = comment[i + 1..j].as_bytes();
    if b.len() < 4 || b.len() > 14 || b.len() % 2 != 0 || !b.iter().all(|c| (33..=123).contains(c)) { return None; }
    let v = |k: usize| (b[2 * k] as u32 - 33) * 91 + (b[2 * k + 1] as u32 - 33);
    let n = b.len() / 2;
    let mut raw = [None; 5];
    for (k, r) in raw.iter_mut().enumerate().take((n - 1).min(5)) { *r = Some(v(k + 1) as f64); }
    let bits = (n == 7).then(|| v(6) as u8);
    Some((Some(v(0)), raw, bits))
}

/// Beschreibung per Nachricht (PARM., UNIT., EQNS., BITS.) einarbeiten; true, wenn es eine war
pub fn apply_meta(t: &mut Telem, text: &str) -> bool {
    let list = |r: &str| r.split(',').map(|x| x.trim().to_string()).collect::<Vec<_>>();
    if let Some(r) = text.strip_prefix("PARM.") {
        for (k, v) in list(r).into_iter().take(13).enumerate() { t.names[k] = v; }
    } else if let Some(r) = text.strip_prefix("UNIT.") {
        for (k, v) in list(r).into_iter().take(13).enumerate() { t.units[k] = v; }
    } else if let Some(r) = text.strip_prefix("EQNS.") {
        let v: Vec<f64> = r.split(',').map(|x| x.trim().parse().unwrap_or(0.0)).collect();
        for k in 0..5 {
            if v.len() >= 3 * k + 3 { t.eqns[k] = (v[3 * k], v[3 * k + 1], v[3 * k + 2]); }
        }
    } else if let Some(r) = text.strip_prefix("BITS.") {
        let (bits, title) = r.split_once(',').unwrap_or((r, ""));
        if bits.len() >= 8 { if let Ok(s) = u8::from_str_radix(&bits[..8], 2) { t.sense = s; } }
        t.title = title.trim().to_string();
    } else {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn klassisch() {
        let (seq, raw, bits) = parse_t("T#005,199,000,255,073,123,01101001").unwrap();
        assert_eq!(seq, Some(5));
        assert_eq!(raw[0], Some(199.0));
        assert_eq!(bits, Some(0b0110_1001));
        let mut t = Telem { raw, bits, ..Default::default() };
        assert!(apply_meta(&mut t, "PARM.Batterie,Temp,,,,Tür,Licht"));
        assert!(apply_meta(&mut t, "UNIT.V,°C,,,,offen,an"));
        assert!(apply_meta(&mut t, "EQNS.0,0.075,0,0,0.5,-40,0,1,0,0,1,0,0,1,0"));
        assert!(apply_meta(&mut t, "BITS.11111111,Wetterstation Garten"));
        assert!(!apply_meta(&mut t, "Hallo"));
        let r = t.rows();
        assert!(r[0].0 == "Batterie" && r[0].1.starts_with("14.9") && r[0].1.ends_with(" V"));
        assert_eq!(r[1], ("Temp".into(), "-40 °C".into()));
        assert_eq!(r[2].0, "Tür"); // Kanäle 3–5 ohne Namen fallen weg
        // Bits 01101001: Bit 1 (Tür) = 0 → „nicht offen“, Bit 2 (Licht) = 1 → „an“
        assert!(r.contains(&("Tür".into(), "nicht offen".into())));
        assert!(r.contains(&("Licht".into(), "an".into())));
        assert_eq!(t.title, "Wetterstation Garten");
        assert!(parse_t("T#MIC,199,000,255,073,123,01101001").unwrap().0.is_none());
    }

    #[test]
    fn base91() {
        // Folge 1, Kanal 1 = 100 (je zwei Zeichen Base91)
        let s = format!("|{}{}{}{}|", 33u8 as char, 34u8 as char, 34u8 as char, (33u8 + 9) as char);
        let (seq, raw, bits) = parse_b91(&format!("Kommentar {s}")).unwrap();
        assert_eq!((seq, raw[0], raw[1], bits), (Some(1), Some(100.0), None, None));
        assert!(parse_b91("a|b|c").is_none());
    }
}
