//! Rustwave als TNC für andere Programme: KISS über TCP (wie Dire Wolf, Port 8001) und eine Teilmenge der
//! AGWPE-Schnittstelle (Port 8000) – für Winlink Express, YAAC, APRSIS32, Xastir, Outpost, BPQ …
//! Empfangene Rahmen gehen an alle Programme, deren Rahmen sendet Rustwave über das Modem des APRS- bzw. Packet-Reiters.
//! Verbindungen (AX.25 „connected mode“) macht das Programm selbst über KISS; über AGW nur Monitor, Rohrahmen und Unproto.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const FEND: u8 = 0xC0;
const FESC: u8 = 0xDB;
const TFEND: u8 = 0xDC;
const TFESC: u8 = 0xDD;

/// Rahmen (ohne FCS) als KISS-Datenrahmen für Port 0
pub fn kiss_encode(frame: &[u8]) -> Vec<u8> {
    let mut o = vec![FEND, 0x00];
    for &b in frame {
        match b { FEND => o.extend([FESC, TFEND]), FESC => o.extend([FESC, TFESC]), _ => o.push(b) }
    }
    o.push(FEND);
    o
}

/// fortlaufender KISS-Leser: Bytes rein, fertige Datenrahmen (ohne Befehlsbyte) raus
#[derive(Default)]
pub struct KissDec { buf: Vec<u8>, esc: bool, inside: bool }
impl KissDec {
    pub fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        let mut out = vec![];
        for &b in data {
            if b == FEND {
                if self.inside && self.buf.len() > 1 {
                    // Befehlsbyte: unteres Halbbyte 0 = Daten (andere: TXDELAY, P, … – übergehen)
                    if self.buf[0] & 0x0F == 0 { out.push(self.buf[1..].to_vec()); }
                }
                self.buf.clear();
                self.inside = true;
                self.esc = false;
                continue;
            }
            if !self.inside { continue; }
            if self.esc {
                self.buf.push(match b { TFEND => FEND, TFESC => FESC, x => x });
                self.esc = false;
            } else if b == FESC { self.esc = true; } else { self.buf.push(b); }
            if self.buf.len() > 4096 { self.buf.clear(); self.inside = false; }
        }
        out
    }
}

// ---------------- AGWPE ----------------

/// AGW-Kopf (36 Bytes) und Daten
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Agw { pub port: u8, pub kind: u8, pub pid: u8, pub from: String, pub to: String, pub data: Vec<u8> }

fn call10(s: &str) -> [u8; 10] { let mut a = [0u8; 10]; for (i, b) in s.bytes().take(9).enumerate() { a[i] = b; } a }
fn cstr(b: &[u8]) -> String { String::from_utf8_lossy(b.split(|&c| c == 0).next().unwrap_or(&[])).trim().to_string() }

impl Agw {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = vec![self.port, 0, 0, 0, self.kind, 0, self.pid, 0];
        o.extend(call10(&self.from));
        o.extend(call10(&self.to));
        o.extend((self.data.len() as u32).to_le_bytes());
        o.extend([0u8; 4]);
        o.extend(&self.data);
        o
    }
    /// ein vollständiges Paket vom Anfang des Puffers nehmen
    pub fn take(buf: &mut Vec<u8>) -> Option<Agw> {
        if buf.len() < 36 { return None; }
        let len = u32::from_le_bytes([buf[28], buf[29], buf[30], buf[31]]) as usize;
        if len > 65536 { buf.clear(); return None; }
        if buf.len() < 36 + len { return None; }
        let p: Vec<u8> = buf.drain(..36 + len).collect();
        Some(Agw { port: p[0], kind: p[4], pid: p[6], from: cstr(&p[8..18]), to: cstr(&p[18..28]), data: p[36..].to_vec() })
    }
}

/// Adressen und Art aus einem Rahmen (ohne FCS) für die Monitor-Zeile
fn monitor_text(frame: &[u8]) -> Option<(String, String, String)> {
    let mut b = frame.to_vec();
    let c = crate::aprs::crc(&b);
    b.extend([c as u8, (c >> 8) as u8]);
    let f = crate::ax25::Frame::from_bytes(&b)?;
    let via: Vec<String> = f.digis.iter().map(|(d, h)| if *h { format!("{d}*") } else { d.clone() }).collect();
    let mut head = format!("1:Fm {} To {}", f.src, f.dst);
    if !via.is_empty() { head += &format!(" Via {}", via.join(",")); }
    let t = chrono::Local::now().format("%H:%M:%S");
    let (kind, line) = match f.kind {
        crate::ax25::Kind::Ui => ("U", format!("{head} <UI pid={:02X} Len={} >[{t}]\r{}\r", f.pid.unwrap_or(0xF0), f.info.len(), String::from_utf8_lossy(&f.info))),
        crate::ax25::Kind::I { .. } => ("I", format!("{head} <I pid={:02X} Len={} >[{t}]\r{}\r", f.pid.unwrap_or(0xF0), f.info.len(), String::from_utf8_lossy(&f.info))),
        _ => ("S", format!("{head} <{:?} >[{t}]\r", f.kind)),
    };
    Some((kind.into(), f.src, line))
}

/// Antwort auf eine AGW-Anfrage; Rahmen zum Senden kommen über `tx`
fn agw_handle(q: &Agw, st: &mut AgwClient, tx: &Sender<crate::UiEv>, port_name: &str) -> Vec<Agw> {
    let reply = |kind: u8, data: Vec<u8>| Agw { port: q.port, kind, from: q.from.clone(), to: q.to.clone(), data, ..Default::default() };
    match q.kind {
        b'R' => vec![reply(b'R', [2005u16.to_le_bytes(), [0, 0], 127u16.to_le_bytes(), [0, 0]].concat())],
        b'G' => vec![reply(b'G', format!("1;Port1 {port_name};\0").into_bytes())],
        b'g' => vec![reply(b'g', vec![0, 1, 0x19, 4, 0xC0, 4, 7, 0, 1, 0, 0, 0])],
        b'X' => { st.calls.push(q.from.to_uppercase()); vec![reply(b'X', vec![1])] }
        b'x' => { st.calls.retain(|c| *c != q.from.to_uppercase()); vec![] }
        b'k' => { st.raw = !st.raw; vec![] }
        b'm' => { st.mon = !st.mon; vec![] }
        b'y' => vec![reply(b'y', 0u32.to_le_bytes().to_vec())],
        b'K' if q.data.len() > 15 => { let _ = tx.send(crate::UiEv::KissTx(with_fcs(&q.data[1..]))); vec![] }
        b'M' | b'V' => {
            let (via, info): (Vec<String>, &[u8]) = if q.kind == b'V' && !q.data.is_empty() {
                let n = q.data[0] as usize;
                if q.data.len() < 1 + 10 * n { return vec![]; }
                ((0..n).map(|k| cstr(&q.data[1 + 10 * k..11 + 10 * k])).collect(), &q.data[1 + 10 * n..])
            } else { (vec![], &q.data[..]) };
            let mut f = crate::ax25::Frame::new(&q.from, &q.to, &via, true, crate::ax25::Kind::Ui, false);
            f.pid = Some(if q.pid == 0 { 0xF0 } else { q.pid });
            f.info = info.to_vec();
            let _ = tx.send(crate::UiEv::KissTx(f.to_bytes()));
            vec![]
        }
        // Verbindungen über AGW kann Rustwave nicht: gleich wieder „getrennt“ melden, damit das Programm nicht wartet
        b'C' | b'v' | b'c' => vec![reply(b'd', format!("*** DISCONNECTED From Station {} (Rustwave: Verbindungen bitte über KISS)\r\0", q.to).into_bytes())],
        _ => vec![],
    }
}

fn with_fcs(frame: &[u8]) -> Vec<u8> {
    let mut b = frame.to_vec();
    let c = crate::aprs::crc(&b);
    b.extend([c as u8, (c >> 8) as u8]);
    b
}

#[derive(Default)]
struct AgwClient { raw: bool, mon: bool, calls: Vec<String> }

struct Client { id: u64, s: TcpStream, agw: Option<Arc<Mutex<AgwClient>>> }

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// laufende Server; beim Fallenlassen werden sie beendet
pub struct Server {
    clients: Arc<Mutex<Vec<Client>>>,
    stop: Arc<AtomicBool>,
    pub n_rx: Arc<AtomicU32>,
    pub n_tx: Arc<AtomicU32>,
    pub status: String,
}

impl Server {
    /// `kiss`/`agw`: Ports (0 = aus); `lan`: auch aus dem Netzwerk erreichbar (sonst nur dieser PC)
    pub fn start(kiss: u16, agw: u16, lan: bool, port_name: String, tx: Sender<crate::UiEv>) -> Server {
        let clients: Arc<Mutex<Vec<Client>>> = Default::default();
        let stop = Arc::new(AtomicBool::new(false));
        let n_tx = Arc::new(AtomicU32::new(0));
        let host = if lan { "0.0.0.0" } else { "127.0.0.1" };
        let mut status = vec![];
        for (port, is_agw) in [(kiss, false), (agw, true)] {
            if port == 0 { continue; }
            let l = match TcpListener::bind((host, port)) {
                Ok(l) => l,
                Err(e) => { status.push(format!("{} Port {port}: {e}", if is_agw { "AGW" } else { "KISS" })); continue; }
            };
            let _ = l.set_nonblocking(true);
            status.push(format!("{} auf Port {port}", if is_agw { "AGW" } else { "KISS" }));
            let (clients, stop, tx, n_tx, name) = (clients.clone(), stop.clone(), tx.clone(), n_tx.clone(), port_name.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match l.accept() {
                        Ok((s, addr)) => {
                            let _ = s.set_nonblocking(false);
                            let _ = s.set_nodelay(true);
                            let _ = tx.send(crate::UiEv::Log(format!("{}: Programm verbunden ({addr})", if is_agw { "AGW" } else { "KISS" })));
                            let agw = is_agw.then(|| Arc::new(Mutex::new(AgwClient::default())));
                            let Ok(w) = s.try_clone() else { continue };
                            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                            clients.lock().unwrap().push(Client { id, s: w, agw: agw.clone() });
                            let (tx, stop, n_tx, name, clients) = (tx.clone(), stop.clone(), n_tx.clone(), name.clone(), clients.clone());
                            let peer = addr.to_string();
                            std::thread::spawn(move || reader(s, agw, tx, stop, n_tx, name, clients, id, peer));
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(200)),
                    }
                }
            });
        }
        Server { clients, stop, n_rx: Arc::new(AtomicU32::new(0)), n_tx, status: status.join(" · ") }
    }

    pub fn clients(&self) -> usize { self.clients.lock().map_or(0, |c| c.len()) }

    /// empfangenen Rahmen (mit FCS) an alle Programme
    pub fn broadcast(&self, frame_fcs: &[u8]) {
        if frame_fcs.len() < 17 { return; }
        let frame = &frame_fcs[..frame_fcs.len() - 2];
        let k = kiss_encode(frame);
        let mon = monitor_text(frame);
        let mut cl = self.clients.lock().unwrap();
        if cl.is_empty() { return; }
        self.n_rx.fetch_add(1, Ordering::Relaxed);
        cl.retain_mut(|c| {
            let ok = match &c.agw {
                None => c.s.write_all(&k).is_ok(),
                Some(a) => {
                    let a = a.lock().unwrap();
                    let mut ok = true;
                    if a.raw {
                        let mut d = vec![0u8];
                        d.extend(frame);
                        ok &= c.s.write_all(&Agw { kind: b'K', data: d, ..Default::default() }.encode()).is_ok();
                    }
                    if let (true, Some((kind, src, text))) = (a.mon, &mon) {
                        ok &= c.s.write_all(&Agw { kind: kind.as_bytes()[0], pid: 0xF0, from: src.clone(), data: text.clone().into_bytes(), ..Default::default() }.encode()).is_ok();
                    }
                    ok
                }
            };
            ok
        });
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut c) = self.clients.lock() { for x in c.iter() { let _ = x.s.shutdown(std::net::Shutdown::Both); } c.clear(); }
    }
}

#[allow(clippy::too_many_arguments)]
fn reader(mut s: TcpStream, agw: Option<Arc<Mutex<AgwClient>>>, tx: Sender<crate::UiEv>, stop: Arc<AtomicBool>, n_tx: Arc<AtomicU32>, name: String,
          clients: Arc<Mutex<Vec<Client>>>, id: u64, peer: String) {
    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
    let mut dec = KissDec::default();
    let mut abuf = vec![];
    let mut buf = [0u8; 2048];
    let Ok(mut w) = s.try_clone() else { return };
    loop {
        if stop.load(Ordering::Relaxed) { return; }
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => match &agw {
                None => for f in dec.push(&buf[..n]) {
                    if f.len() >= 15 { n_tx.fetch_add(1, Ordering::Relaxed); let _ = tx.send(crate::UiEv::KissTx(with_fcs(&f))); }
                },
                Some(a) => {
                    abuf.extend_from_slice(&buf[..n]);
                    while let Some(q) = Agw::take(&mut abuf) {
                        if matches!(q.kind, b'K' | b'M' | b'V') { n_tx.fetch_add(1, Ordering::Relaxed); }
                        let replies = agw_handle(&q, &mut a.lock().unwrap(), &tx, &name);
                        for r in replies { if w.write_all(&r.encode()).is_err() { break; } }
                    }
                }
            },
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
    }
    // Verbindung zu: aus der Liste nehmen (beim nächsten Schreiben ohnehin), Hinweis
    if let Ok(mut c) = clients.lock() { c.retain(|x| x.id != id); }
    let _ = tx.send(crate::UiEv::Log(format!("{}: Programm getrennt ({peer})", if agw.is_some() { "AGW" } else { "KISS" })));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn ui_frame() -> Vec<u8> {
        let mut f = crate::ax25::Frame::new("DL1ABC", "APRS", &["WIDE1-1".into()], true, crate::ax25::Kind::Ui, false);
        f.info = b"!5230.00N/01324.00E-Test \xC0\xDB".to_vec();
        f.to_bytes()
    }

    #[test]
    fn kiss_kodierung() {
        let f = ui_frame();
        let body = &f[..f.len() - 2];
        let k = kiss_encode(body);
        assert!(k.windows(2).any(|w| w == [FESC, TFEND]) && k.windows(2).any(|w| w == [FESC, TFESC]));
        let mut d = KissDec::default();
        // in Stücken, mit Befehlsrahmen (TXDELAY) dazwischen
        let mut all = vec![FEND, 0x01, 30, FEND];
        all.extend(&k);
        let mut got = vec![];
        for c in all.chunks(5) { got.extend(d.push(c)); }
        assert_eq!(got, vec![body.to_vec()]);
        assert_eq!(with_fcs(body), f);
    }

    #[test]
    fn agw_pakete() {
        let q = Agw { port: 0, kind: b'M', pid: 0xF0, from: "DL1ABC".into(), to: "CQ".into(), data: b"Hallo".to_vec() };
        let mut b = q.encode();
        assert_eq!(b.len(), 36 + 5);
        b.extend([1, 2, 3]); // Anfang des nächsten
        assert_eq!(Agw::take(&mut b), Some(q.clone()));
        assert_eq!(b, vec![1, 2, 3]);
        let (tx, rx) = channel();
        let mut st = AgwClient::default();
        assert!(agw_handle(&q, &mut st, &tx, "x").is_empty());
        match rx.try_recv().unwrap() {
            crate::UiEv::KissTx(f) => {
                let f = crate::ax25::Frame::from_bytes(&f).unwrap();
                assert_eq!((f.src.as_str(), f.dst.as_str(), f.info.as_slice()), ("DL1ABC", "CQ", &b"Hallo"[..]));
            }
            _ => panic!(),
        }
        let r = agw_handle(&Agw { kind: b'R', ..Default::default() }, &mut st, &tx, "x");
        assert_eq!(r[0].kind, b'R');
        let r = agw_handle(&Agw { kind: b'X', from: "DL1ABC".into(), ..Default::default() }, &mut st, &tx, "x");
        assert_eq!(r[0].data, vec![1]);
        agw_handle(&Agw { kind: b'k', ..Default::default() }, &mut st, &tx, "x");
        assert!(st.raw);
        // Unproto über Digipeater
        let mut d = vec![1u8];
        d.extend(call10("WIDE2-1"));
        d.extend(b"via");
        agw_handle(&Agw { kind: b'V', from: "DL1ABC".into(), to: "CQ".into(), data: d, ..Default::default() }, &mut st, &tx, "x");
        let crate::UiEv::KissTx(f) = rx.try_recv().unwrap() else { panic!() };
        assert_eq!(crate::ax25::Frame::from_bytes(&f).unwrap().digis[0].0, "WIDE2-1");
    }

    /// echter TCP-Weg: Programm schickt einen KISS-Rahmen, Rustwave verteilt einen empfangenen
    #[test]
    fn server_kiss() {
        let (tx, rx) = channel();
        let srv = Server::start(18_001, 0, false, "Test".into(), tx);
        let mut c = TcpStream::connect(("127.0.0.1", 18_001)).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let f = ui_frame();
        c.write_all(&kiss_encode(&f[..f.len() - 2])).unwrap();
        let mut sent = None;
        for _ in 0..30 {
            match rx.recv_timeout(Duration::from_millis(200)) { Ok(crate::UiEv::KissTx(b)) => { sent = Some(b); break; } _ => {} }
        }
        assert_eq!(sent.unwrap(), f);
        for _ in 0..20 { if srv.clients() == 1 { break; } std::thread::sleep(Duration::from_millis(50)); }
        srv.broadcast(&f);
        let mut buf = [0u8; 200];
        let n = c.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], &kiss_encode(&f[..f.len() - 2])[..]);
    }
}

/// mit Dire Wolfs kissutil gegenprüfen: cargo test --release kiss_interop -- --ignored --nocapture
/// (parallel: echo "DL9XYZ>APRS:>Test aus kissutil" | kissutil -h 127.0.0.1 -p 18002)
#[cfg(test)]
#[test]
#[ignore]
fn kiss_interop() {
    let (tx, rx) = std::sync::mpsc::channel();
    let srv = Server::start(18_002, 0, false, "Test".into(), tx);
    let mut f = crate::ax25::Frame::new("DL1ABC", "APRS", &["WIDE1-1".into()], true, crate::ax25::Kind::Ui, false);
    f.info = b"!5230.00N/01324.00E-von Rustwave".to_vec();
    let b = f.to_bytes();
    for _ in 0..40 {
        srv.broadcast(&b);
        while let Ok(e) = rx.try_recv() {
            match e {
                crate::UiEv::KissTx(x) => println!("VON KISSUTIL: {:?}", crate::aprs::Frame::from_bytes(&x).map(|f| f.monitor())),
                crate::UiEv::Log(l) => println!("LOG {l}"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}
