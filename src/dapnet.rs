//! DAPNET: Rubriken (Liste nach pi-star.de), bekannte Dienst-RICs, Zeitsignale und Zusammenführen der Rubrik-Nachrichten.
//!
//! Rubriken gehen doppelt raus: im Klartext an RIC 1000 + Nummer (AlphaPoc, Swissphone) und für Skyper-Empfänger
//! an RIC 4520, dort um ein Zeichen verschoben und mit zwei Kopfzeichen. Im Skyper-Text (schon um eins zurückgeschoben)
//! gilt: Rubrik = Zeichen 1 − 0x1E, Platz = Zeichen 2 − 0x1F. Nachgeprüft an einer echten Aufnahme:
//! „o“ (0x6F) gehört zu Rubrik 81 (DWD-Warnungen), derselbe Text kam parallel auf RIC 1081.

use crate::pocsag::{Kind, Msg};
use std::sync::OnceLock;

/// Zeitsignale für Pager (RIC, Beschreibung) – lassen sich in der Liste ausblenden
pub const TIME_RICS: [(u32, &str); 4] = [
    (200, "Uhrzeit UTC für Swissphone-Pager"),
    (208, "Ortszeit (Deutschland) für Swissphone-Pager"),
    (216, "Uhrzeit UTC für AlphaPoc-Pager"),
    (224, "Ortszeit (Deutschland) für AlphaPoc-Pager"),
];
/// numerische Zeitansage (HHMMSS TTMMJJ, UTC) – zählt ebenfalls zu den Zeitsignalen
pub const TIME_NUM_RIC: u32 = 2504;
/// Dienst-RICs ohne Inhalt für Menschen
pub const SERVICE_RICS: [(u32, &str); 2] = [
    (8, "Senderkennung: Rufzeichen des sendenden DAPNET-Senders"),
    (4512, "Skyper: Namen der Rubriken"),
];
/// Skyper-Rubrikinhalte (um eins verschoben)
pub const SKYPER_RIC: u32 = 4520;
/// Klartext-Rubriken: RIC = 1000 + Nummer
pub const RUBRIC_BASE: u32 = 1000;

/// eine Zeile der Rubrikliste (Name = DAPNET-interner Bezeichner)
pub struct Rubric { pub num: u8, #[allow(dead_code)] pub name: &'static str, pub label: &'static str, pub group: &'static str }

/// alle Rubriken (383 Einträge, je Sendergruppe eine Zeile)
pub fn rubrics() -> &'static [Rubric] {
    static R: OnceLock<Vec<Rubric>> = OnceLock::new();
    R.get_or_init(|| {
        include_str!("dapnet_rubriken.txt").lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).filter_map(|l| {
            let mut f = l.split('|');
            Some(Rubric { num: f.next()?.parse().ok()?, name: f.next()?, label: f.next()?, group: f.next()? })
        }).collect()
    })
}

/// passt die Sendergruppe zur eingestellten Region? („dl-all“ deckt alle dl-…-Gruppen ab)
pub fn in_region(group: &str, region: &[String]) -> bool {
    group == "all" || group == "dxclusters" || region.iter().any(|t| {
        t == group || t.strip_suffix("-all").is_some_and(|p| group.starts_with(&format!("{p}-")))
            || group.strip_suffix("-all").is_some_and(|p| t.starts_with(&format!("{p}-")))
    })
}

/// „dl-all, dl-by“ → Liste in Kleinbuchstaben
pub fn region_tokens(s: &str) -> Vec<String> {
    s.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()).collect()
}

/// Bezeichnung einer Rubrik: zur Region passend, sonst alle vorkommenden (höchstens drei)
pub fn label(num: u8, region: &[String]) -> String {
    let rows: Vec<&Rubric> = rubrics().iter().filter(|r| r.num == num).collect();
    if rows.is_empty() { return format!("Rubrik {num}"); }
    let mut labels: Vec<&str> = vec![];
    for r in rows.iter().filter(|r| in_region(r.group, region)) { if !labels.contains(&r.label) { labels.push(r.label); } }
    if labels.is_empty() { for r in &rows { if !labels.contains(&r.label) { labels.push(r.label); } } }
    let more = labels.len() > 3;
    labels.truncate(3);
    format!("{}{}", labels.join(" / "), if more { " / …" } else { "" })
}

/// Bedeutung einer RIC im DAPNET (für Tooltips)
pub fn ric_info(ric: u32, region: &[String]) -> Option<String> {
    if let Some((_, d)) = TIME_RICS.iter().find(|t| t.0 == ric) { return Some(format!("Zeitsignal: {d}")); }
    if let Some((_, d)) = SERVICE_RICS.iter().find(|t| t.0 == ric) { return Some(format!("Dienst-RIC – {d}")); }
    match ric {
        TIME_NUM_RIC => Some("Zeitsignal numerisch: HHMMSS TTMMJJ (UTC)".into()),
        SKYPER_RIC => Some("Skyper-Rubriken (Text um ein Zeichen verschoben, zwei Kopfzeichen: Rubrik und Platz)".into()),
        r if (RUBRIC_BASE + 1..RUBRIC_BASE + 100).contains(&r) => {
            let n = (r - RUBRIC_BASE) as u8;
            Some(format!("Rubrik {n} für AlphaPoc/Swissphone: {}", label(n, region)))
        }
        _ => None,
    }
}

pub fn is_time(ric: u32) -> bool { TIME_RICS.iter().any(|t| t.0 == ric) || ric == TIME_NUM_RIC }
pub fn is_service(ric: u32) -> bool { SERVICE_RICS.iter().any(|t| t.0 == ric) }
/// in Listen ausblenden? (Zeitsignale bzw. Dienst-RICs, je nach Schalter)
pub fn hidden(ric: Option<u32>, hide_time: bool, hide_service: bool) -> bool {
    ric.is_some_and(|r| (hide_time && is_time(r)) || (hide_service && is_service(r)))
}

/// Rubrik-Nachricht erkennen: (Nummer, Platz bei Skyper, Text)
pub fn rubric_of(m: &Msg) -> Option<(u8, Option<u8>, String)> {
    let a = m.addr?;
    match m.kind {
        Kind::Skyper if a == SKYPER_RIC => {
            let (h, t) = m.skyper_split()?;
            let mut c = h.chars().map(|c| c as u32);
            let (r, s) = (c.next()?, c.next()?);
            let num = r.checked_sub(0x1E).filter(|n| (1..100).contains(n))? as u8;
            let slot = s.checked_sub(0x1F).filter(|n| (1..=10).contains(n)).map(|n| n as u8);
            Some((num, slot, t.trim().to_string()))
        }
        Kind::Alpha if (RUBRIC_BASE + 1..RUBRIC_BASE + 100).contains(&a) => Some(((a - RUBRIC_BASE) as u8, None, m.clean_text().trim().to_string())),
        _ => None,
    }
}

/// eine Zeile der DAPNET-Ansicht: Rubrik-Nachricht (beide Wege zusammengeführt, `num` > 0) oder sonstiger Ruf (`num` = 0)
pub struct News { pub time: String, pub num: u8, pub slot: Option<u8>, pub text: String, pub via: Vec<u32> }

/// in die DAPNET-Ansicht übernehmen; gleiche Rubrik-Nachricht über den zweiten Weg (1000+n bzw. 4520) wird angehängt.
/// Rückgabe: war es eine Rubrik?
pub fn add_news(news: &mut Vec<News>, time: &str, m: &Msg) -> bool {
    let Some((num, slot, text)) = rubric_of(m) else {
        // sonstiger Ruf: dieselbe Meldung kommt oft von mehreren Sendern kurz nacheinander – zusammenfassen
        let (Some(ric), false) = (m.addr, m.kind == Kind::Tone) else { return false };
        let text = m.clean_text().trim().to_string();
        if let Some(n) = news.iter_mut().rev().take(30).find(|n| n.num == 0 && n.via == [ric] && n.text == text) { n.time = time.to_string(); return false; }
        news.push(News { time: time.to_string(), num: 0, slot: None, text, via: vec![ric] });
        if news.len() > 500 { news.drain(..news.len() - 500); }
        return false;
    };
    if text.is_empty() { return false; }
    let ric = m.addr.unwrap_or(0);
    if let Some(n) = news.iter_mut().rev().take(100).find(|n| n.num == num && num > 0 && n.text == text) {
        n.time = time.to_string();
        if n.slot.is_none() { n.slot = slot; }
        if !n.via.contains(&ric) { n.via.push(ric); }
        return true;
    }
    news.push(News { time: time.to_string(), num, slot, text, via: vec![ric] });
    if news.len() > 500 { news.drain(..news.len() - 500); }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(addr: u32, kind: Kind, text: &str) -> Msg {
        let mut m = Msg::sent(1200, addr, 3, kind, text.into());
        m.kind = kind;
        m
    }

    #[test]
    fn liste() {
        let r = rubrics();
        assert_eq!(r.len(), 383);
        assert!(r.iter().all(|r| r.num >= 1 && r.num <= 83));
        let dl = region_tokens("dl-all");
        assert_eq!(label(81, &dl), "DWD-Alerts");
        assert_eq!(label(11, &dl), "Berlin");
        assert_eq!(label(11, &region_tokens("oe-all")), "Wien");
        assert_eq!(label(13, &region_tokens("dl-by")), "Bayern");
        assert_eq!(label(1, &[]), "DAPNET NEWS");
        assert!(ric_info(1081, &dl).unwrap().contains("DWD-Alerts"));
        assert!(is_time(216) && is_time(2504) && !is_time(1081) && is_service(8));
        assert!(hidden(Some(200), true, false) && !hidden(Some(200), false, true) && hidden(Some(8), false, true) && !hidden(Some(1081), true, true));
    }

    /// wie in der IC-705-Aufnahme: dieselbe DWD-Warnung über 1081 und 4520
    #[test]
    fn zusammenfuehren() {
        let mut news = vec![];
        assert!(add_news(&mut news, "12:40:01", &msg(1081, Kind::Alpha, "20.11.26 20:00 - 08:00z: FROST")));
        assert!(add_news(&mut news, "12:40:02", &msg(4520, Kind::Skyper, "o(20.11.26 20:00 - 08:00z: FROST")));
        assert!(add_news(&mut news, "12:40:03", &msg(4520, Kind::Skyper, "o)20.11.26 03:01 - 17:00z: DAUERREGEN")));
        assert!(!add_news(&mut news, "12:40:04", &msg(216, Kind::Alpha, "YYYYMMDDHHMMSS261120114000")));
        assert_eq!(news.len(), 3);
        assert_eq!((news[2].num, news[2].via.clone()), (0, vec![216]));
        news.pop();
        assert_eq!((news[0].num, news[0].slot, news[0].via.clone()), (81, Some(9), vec![1081, 4520]));
        assert_eq!((news[1].num, news[1].slot), (81, Some(10)));
    }
}
