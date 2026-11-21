//! Verbindung zu APRS-IS (Internet): Anmelden, Zeilen empfangen, Zeilen senden; bei Abbruch nach 30 s neu.
//! Läuft in einem eigenen Thread; Meldungen kommen als `UiEv::Is` in die Oberfläche.

use crate::UiEv;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum IsEv {
    /// Verbindungsstand (Text, angemeldet und geprüft)
    Status(String, bool),
    /// empfangene Zeile (kein Serverhinweis)
    Line(String),
}

pub struct Conn { tx: mpsc::Sender<String>, stop: Arc<AtomicBool>, pub key: String }

impl Conn {
    /// `key` beschreibt die Anmeldung (Server, Login-Zeile); ändert er sich, wird neu verbunden
    pub fn start(server: String, login: String, ev: mpsc::Sender<UiEv>, ctx: Option<eframe::egui::Context>) -> Conn {
        let (tx, rx) = mpsc::channel::<String>();
        let stop = Arc::new(AtomicBool::new(false));
        let key = format!("{server}|{login}");
        let st = stop.clone();
        std::thread::spawn(move || {
            let send = |e: IsEv| { let _ = ev.send(UiEv::Is(e)); if let Some(c) = &ctx { c.request_repaint(); } };
            while !st.load(Relaxed) {
                match session(&server, &login, &rx, &st, &send) {
                    Ok(()) => break,
                    Err(e) => send(IsEv::Status(format!("APRS-IS: {e} – neuer Versuch in 30 s"), false)),
                }
                for _ in 0..300 { if st.load(Relaxed) { return; } std::thread::sleep(Duration::from_millis(100)); }
            }
        });
        Conn { tx, stop, key }
    }
    pub fn send(&self, line: String) { let _ = self.tx.send(line); }
}

impl Drop for Conn {
    fn drop(&mut self) { self.stop.store(true, Relaxed); }
}

fn session(server: &str, login: &str, rx: &mpsc::Receiver<String>, stop: &AtomicBool, send: &dyn Fn(IsEv)) -> Result<(), String> {
    send(IsEv::Status(format!("APRS-IS: verbinde mit {server} …"), false));
    let addr = server.to_socket_addrs().map_err(|e| format!("{server}: {e}"))?.next().ok_or("keine Adresse")?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_millis(500))).map_err(|e| e.to_string())?;
    s.write_all(format!("{login}\r\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut rd = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
    let mut line = String::new();
    let mut keep = Instant::now();
    loop {
        if stop.load(Relaxed) { return Ok(()); }
        // ausgehende Zeilen
        while let Ok(l) = rx.try_recv() { s.write_all(format!("{l}\r\n").as_bytes()).map_err(|e| e.to_string())?; }
        if keep.elapsed() > Duration::from_secs(600) { keep = Instant::now(); s.write_all(b"#keepalive\r\n").map_err(|e| e.to_string())?; }
        // bei Zeitüberschreitung bleibt eine halbe Zeile in `line` stehen und wird beim nächsten Mal ergänzt
        match rd.read_line(&mut line) {
            Ok(0) => return Err("Server hat die Verbindung beendet".into()),
            Ok(_) if !line.ends_with('\n') => {}
            Ok(_) => {
                let l = std::mem::take(&mut line);
                let l = l.trim_end();
                if let Some(r) = l.strip_prefix("# logresp ") {
                    let ok = r.contains(" verified");
                    send(IsEv::Status(format!("APRS-IS: angemeldet ({}){}", server, if ok { "" } else { " – nicht geprüft, Weiterleiten nicht möglich" }), ok));
                } else if !l.starts_with('#') && !l.is_empty() {
                    send(IsEv::Line(l.to_string()));
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
}
