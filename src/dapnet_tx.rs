//! DAPNET-Sender (wie UniPager/Pi-Star-DAPNETGateway): Verbindung zum DAPNET-Kern über das TCP-Protokoll auf Port 43434,
//! Rufe entgegennehmen, quittieren und in den zugeteilten Zeitschlitzen über das Funkgerät aussenden.
//! Außerdem: einen Ruf über die REST-Schnittstelle von hampager.de ins Netz geben (dann senden die DAPNET-Sender).
//!
//! Protokoll (nach UniPager 1.0.3, src/connection.rs):
//!   Anmeldung   → `[Typ vVersion Rufzeichen AuthKey]`
//!   `2:ID`      ← Kennung, Antwort `2:ID:0000` und `+`
//!   `3:…`       ← Zeitabgleich, Antwort `+`
//!   `4:0123`    ← erlaubte Zeitschlitze (hex), Antwort `+`
//!   `#NN T:S:RIC:F:Text` ← Ruf (NN laufende Nummer hex, T 5 = numerisch, 6 = Text, S 0/1/2 = 512/1200/2400 Bd,
//!                  RIC hex, F Funktion), Antwort `#MM +` mit MM = NN + 1
//! Zeitschlitze: 16 je 6,4 s (Zehntelsekunden seit 1970 >> 6, & 15).

use crate::pocsag::{Content, Page};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const DEFAULT_SERVER: &str = "dapnet.afu.rwth-aachen.de:43434";
/// Ausweichserver (aus UniPager 1.0.3); viele nur über HAMNET erreichbar
pub const FALLBACK: [&str; 3] = ["dapnet.db0sda.ampr.org:43434", "dapnet.db0vvs.ampr.org:43434", "funkruf.db0del.db0sda.ampr.org:43434"];
/// REST-Schnittstelle für eigene Rufe ins Netz
pub const DEFAULT_API: &str = "http://www.hampager.de:8080/calls";

/// Länge eines Zeitschlitzes
pub const SLOT_S: f64 = 6.4;

/// Zeitschlitz (0–15) zum Zeitpunkt t (Sekunden seit 1970)
pub fn slot_at(t: f64) -> usize { (((t * 10.0).floor() as u64 >> 6) & 15) as usize }

/// wie lange ab t noch ununterbrochen gesendet werden darf (0, wenn der laufende Schlitz nicht erlaubt ist);
/// höchstens fünf Schlitze am Stück wie UniPager
pub fn budget_s(slots: &[bool; 16], t: f64) -> f64 {
    let s = slot_at(t);
    if !slots[s] { return 0.0; }
    let start = ((t * 10.0).floor() as u64 >> 6 << 6) as f64 / 10.0; // Beginn des laufenden Schlitzes
    let mut n = 1;
    while n < 5 && slots[(s + n) % 16] { n += 1; }
    (start + n as f64 * SLOT_S - t).max(0.0)
}

/// Sekunden bis zum nächsten erlaubten Schlitz (0 = jetzt erlaubt, None = keiner erlaubt)
pub fn wait_s(slots: &[bool; 16], t: f64) -> Option<f64> {
    let s = slot_at(t);
    if slots[s] { return Some(0.0); }
    let start = ((t * 10.0).floor() as u64 >> 6 << 6) as f64 / 10.0;
    (1..16).find(|k| slots[(s + k) % 16]).map(|k| start + k as f64 * SLOT_S - t)
}

/// „0123“ → erlaubte Schlitze
pub fn parse_slots(s: &str) -> [bool; 16] {
    let mut r = [false; 16];
    for c in s.chars() { if let Some(i) = c.to_digit(16) { r[i as usize] = true; } }
    r
}

/// ein vom Kern empfangener Ruf
#[derive(Clone, Debug, PartialEq)]
pub struct Call { pub page: Page, pub baud: u32, pub got: Instant }

/// Zeile vom Kern auswerten: (Antwort, Ereignis)
pub fn handle_line(line: &str) -> (Vec<String>, Option<Event>) {
    let l = line.trim();
    let Some(first) = l.chars().next() else { return (vec![], None) };
    match first {
        '#' => {
            let mut parts = l.split(':');
            let head = parts.next().unwrap_or("");
            let id = head.get(1..3).and_then(|h| u8::from_str_radix(h, 16).ok());
            let typ = head.get(4..5);
            let speed = parts.next().and_then(|s| s.parse::<u8>().ok());
            let addr = parts.next().and_then(|s| u32::from_str_radix(s, 16).ok());
            let func = parts.next().and_then(|s| s.parse::<u8>().ok()).filter(|f| *f < 4);
            let text = parts.collect::<Vec<_>>().join(":");
            match (id, typ, addr, func) {
                (Some(id), Some(t @ ("5" | "6")), Some(addr), Some(func)) if addr <= crate::pocsag::MAX_RIC => {
                    let baud = match speed { Some(0) => 512, Some(2) => 2400, _ => 1200 };
                    let content = if t == "5" { Content::Numeric(text) } else { Content::Alpha(text) };
                    (vec![format!("#{:02x} +", id.wrapping_add(1))], Some(Event::Call(Call { page: Page { addr, func, content }, baud, got: Instant::now() })))
                }
                _ => (vec!["-".into()], Some(Event::Log(format!("unverständlicher Ruf vom Kern: {l}")))),
            }
        }
        '2' => {
            let id = l.split(':').nth(1).unwrap_or("");
            (vec![format!("2:{id}:0000"), "+".into()], None)
        }
        '3' => (vec!["+".into()], None),
        '4' => match l.split(':').nth(1) {
            Some(s) => (vec!["+".into()], Some(Event::Slots(parse_slots(s)))),
            None => (vec!["-".into()], None),
        },
        _ => (vec!["-".into()], Some(Event::Log(format!("unbekannte Zeile vom Kern: {l}")))),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Connected(String),
    Disconnected(String),
    Slots([bool; 16]),
    Call(Call),
    Log(String),
}

/// laufende Verbindung zum DAPNET-Kern (eigener Thread, verbindet bei Abbruch neu)
pub struct Link { pub rx: Receiver<Event>, stop: Arc<AtomicBool> }

impl Link {
    pub fn start(server: &str, call: &str, auth: &str) -> Link {
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (servers, call, auth, st) = {
            let mut v = vec![server.trim().to_string()];
            if server.trim() == DEFAULT_SERVER { v.extend(FALLBACK.iter().map(|s| s.to_string())); }
            (v, call.trim().to_lowercase(), auth.trim().to_string(), stop.clone())
        };
        std::thread::spawn(move || {
            let mut k = 0;
            while !st.load(Ordering::Relaxed) {
                let host = &servers[k % servers.len()];
                match session(host, &call, &auth, &tx, &st) {
                    Ok(()) => {}
                    Err(e) => { let _ = tx.send(Event::Disconnected(format!("{host}: {e}"))); k += 1; }
                }
                for _ in 0..50 { if st.load(Ordering::Relaxed) { break; } std::thread::sleep(Duration::from_millis(100)); }
            }
        });
        Link { rx, stop }
    }
}
impl Drop for Link { fn drop(&mut self) { self.stop.store(true, Ordering::Relaxed); } }

fn session(host: &str, call: &str, auth: &str, tx: &Sender<Event>, stop: &AtomicBool) -> std::io::Result<()> {
    let addr = host.to_socket_addrs()?.next().ok_or_else(|| std::io::Error::other("Name nicht auflösbar"))?;
    let s = TcpStream::connect_timeout(&addr, Duration::from_secs(10))?;
    s.set_read_timeout(Some(Duration::from_secs(1)))?;
    s.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut w = s.try_clone()?;
    write!(w, "[Rustwave v{} {call} {auth}]\r\n", env!("CARGO_PKG_VERSION"))?;
    let _ = tx.send(Event::Connected(host.to_string()));
    let mut r = BufReader::new(s);
    // read_until statt read_line: bei Zeitüberschreitung bleibt eine halbe Zeile im Puffer erhalten
    let mut line: Vec<u8> = vec![];
    let mut last = Instant::now();
    loop {
        if stop.load(Ordering::Relaxed) { let _ = r.get_ref().shutdown(std::net::Shutdown::Both); return Ok(()); }
        match r.read_until(b'\n', &mut line) {
            Ok(0) => return Err(std::io::Error::other("Verbindung vom Kern geschlossen (AuthKey/Rufzeichen richtig?)")),
            Ok(_) => {
                if line.last() != Some(&b'\n') { continue; } // Rest der Zeile kommt noch
                last = Instant::now();
                let (replies, ev) = handle_line(&String::from_utf8_lossy(&line));
                for a in replies { write!(w, "{a}\r\n")?; }
                w.flush()?;
                if let Some(e) = ev { let _ = tx.send(e); }
                line.clear();
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                // der Kern meldet sich mindestens alle zwei Minuten (Zeitabgleich)
                if last.elapsed() > Duration::from_secs(150) { return Err(std::io::Error::other("Kern antwortet nicht mehr")); }
            }
            Err(e) => return Err(e),
        }
    }
}

// ---------------- REST: Ruf ins Netz geben ----------------

fn b64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 { s.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' }); }
    }
    s
}

/// JSON-Körper für POST /calls
pub fn call_json(text: &str, calls: &[String], groups: &[String], emergency: bool) -> String {
    serde_json::json!({ "text": text, "callSignNames": calls, "transmitterGroupNames": groups, "emergency": emergency }).to_string()
}

/// Ruf über die REST-Schnittstelle senden (blockiert; im Hintergrund aufrufen). Ok = HTTP-Status 2xx.
pub fn api_send(url: &str, user: &str, pass: &str, body: &str) -> Result<u16, String> {
    let rest = url.strip_prefix("http://").ok_or("nur http:// wird unterstützt")?;
    let (hostport, path) = rest.split_once('/').map_or((rest, "/".to_string()), |(h, p)| (h, format!("/{p}")));
    let hp = if hostport.contains(':') { hostport.to_string() } else { format!("{hostport}:80") };
    let host = hostport.split(':').next().unwrap_or(hostport);
    let addr = hp.to_socket_addrs().map_err(|e| e.to_string())?.next().ok_or("Name nicht auflösbar")?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(15))).ok();
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Basic {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        b64(format!("{user}:{pass}").as_bytes()), body.len());
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut status = String::new();
    BufReader::new(s).read_line(&mut status).map_err(|e| e.to_string())?;
    let code: u16 = status.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or_else(|| format!("unerwartete Antwort: {}", status.trim()))?;
    if (200..300).contains(&code) { Ok(code) } else {
        Err(match code { 401 => "Anmeldung abgelehnt (Benutzername/Passwort von hampager.de)".into(),
                         400 => "Anfrage abgelehnt (Rufzeichen oder Sendergruppe unbekannt?)".into(),
                         c => format!("HTTP {c}") })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn zeitschlitze() {
        // t = 0: Schlitz 0; 6,4 s weiter Schlitz 1; nach 16 Schlitzen (102,4 s) wieder 0
        assert_eq!(slot_at(0.0), 0);
        assert_eq!(slot_at(6.45), 1);
        assert_eq!(slot_at(102.4), 0);
        let s = parse_slots("0123");
        assert!(s[0] && s[3] && !s[4]);
        // am Anfang von Schlitz 0 mit 0–3 frei: 4 Schlitze = 25,6 s
        assert!((budget_s(&s, 0.0) - 25.6).abs() < 1e-9);
        assert!((budget_s(&s, 10.0) - 15.6).abs() < 1e-9);
        assert_eq!(budget_s(&s, 30.0), 0.0);
        // in Schlitz 4 (25,6 s): nächster freier ist 0 im nächsten Block (102,4 s)
        assert!((wait_s(&s, 26.0).unwrap() - 76.4).abs() < 1e-9);
        assert_eq!(wait_s(&[false; 16], 0.0), None);
        // höchstens fünf Schlitze am Stück
        assert!((budget_s(&[true; 16], 0.0) - 32.0).abs() < 1e-9);
    }

    #[test]
    fn zeilen() {
        let (a, e) = handle_line("#1F 6:1:11A8:3:Hallo: Welt\r\n");
        assert_eq!(a, vec!["#20 +"]);
        match e { Some(Event::Call(c)) => {
            assert_eq!((c.page.addr, c.page.func, c.baud), (4520, 3, 1200));
            assert_eq!(c.page.content, Content::Alpha("Hallo: Welt".into()));
        } other => panic!("{other:?}") }
        assert_eq!(handle_line("#FF 5:1:9C8:0:0815").0, vec!["#00 +"]);
        assert_eq!(handle_line("2:ABCD").0, vec!["2:ABCD:0000", "+"]);
        assert_eq!(handle_line("3:+1234").0, vec!["+"]);
        assert_eq!(handle_line("4:0F").1, Some(Event::Slots(parse_slots("0F"))));
        assert_eq!(handle_line("#12 7:1:1:3:x").0, vec!["-"]);
    }

    /// kleiner Ersatz-Kern: Anmeldung, Kennung, Zeitschlitze, ein Ruf – Antworten prüfen
    #[test]
    fn mit_kern() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let srv = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut w = s;
            let mut got = vec![];
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            got.push(line.trim().to_string());
            w.write_all(b"2:CAFE\r\n4:0123\r\n#05 6:1:1F40:3:Test DAPNET\r\n").unwrap();
            for _ in 0..4 { line.clear(); r.read_line(&mut line).unwrap(); got.push(line.trim().to_string()); }
            got
        });
        let link = Link::start(&addr, "DL1ABC", "geheim");
        let mut evs = vec![];
        let t = Instant::now();
        while evs.len() < 3 && t.elapsed() < Duration::from_secs(5) {
            if let Ok(e) = link.rx.recv_timeout(Duration::from_millis(200)) { evs.push(e); }
        }
        let got = srv.join().unwrap();
        assert!(got[0].starts_with("[Rustwave v") && got[0].ends_with(" dl1abc geheim]"), "{}", got[0]);
        assert_eq!(&got[1..], ["2:CAFE:0000", "+", "+", "#06 +"]);
        assert!(matches!(evs[0], Event::Connected(_)));
        assert_eq!(evs[1], Event::Slots(parse_slots("0123")));
        assert!(matches!(&evs[2], Event::Call(c) if c.page.addr == 8000));
    }

    #[test]
    fn rest() {
        assert_eq!(b64(b"dl1abc:geheim"), "ZGwxYWJjOmdlaGVpbQ==");
        assert_eq!(b64(b"ab"), "YWI=");
        let j = call_json("Hallo", &["dl1abc".into()], &["dl-all".into()], false);
        assert_eq!(j, r#"{"callSignNames":["dl1abc"],"emergency":false,"text":"Hallo","transmitterGroupNames":["dl-all"]}"#);
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let srv = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut head = String::new();
            loop { let mut x = String::new(); r.read_line(&mut x).unwrap(); if x == "\r\n" { break; } head += &x; }
            let mut w = s;
            w.write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n").unwrap();
            head
        });
        let code = api_send(&format!("http://127.0.0.1:{port}/calls"), "dl1abc", "geheim", &j).unwrap();
        let head = srv.join().unwrap();
        assert_eq!(code, 201);
        assert!(head.starts_with("POST /calls HTTP/1.1") && head.contains("Authorization: Basic ZGwxYWJjOmdlaGVpbQ=="));
    }
}
