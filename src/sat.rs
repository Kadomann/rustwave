//! Satelliten und ISS: Bahndaten (TLE von CelesTrak), Bahnberechnung (SGP4), Überflüge, Richtung und Doppler –
//! dazu Nachführen der Frequenz per CAT und Voreinstellungen für ISS-APRS und ISS-SSTV.

/// Quellen für Bahndaten, der Reihe nach: CelesTrak (Amateurfunk + Stationen), AMSAT, SatNOGS (JSON)
pub const TLE_SOURCES: [(&str, &str); 4] = [
    ("CelesTrak", "https://celestrak.org/NORAD/elements/gp.php?GROUP=amateur&FORMAT=tle"),
    ("CelesTrak", "https://celestrak.org/NORAD/elements/gp.php?GROUP=stations&FORMAT=tle"),
    ("AMSAT", "https://www.amsat.org/tle/current/nasabare.txt"),
    ("SatNOGS", "https://db.satnogs.org/api/tle/?format=json"),
];

/// SatNOGS-JSON ([{"tle0","tle1","tle2"}, …]) in TLE-Text
pub fn satnogs_to_tle(json: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return String::new() };
    let mut out = String::new();
    for e in v.as_array().into_iter().flatten() {
        let g = |k: &str| e.get(k).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
        let (n, a, b) = (g("tle0"), g("tle1"), g("tle2"));
        if a.starts_with("1 ") && b.starts_with("2 ") { out += &format!("{}\n{a}\n{b}\n", n.trim_start_matches("0 ")); }
    }
    out
}

/// IPv4 zuerst: manche Netze haben eine kaputte IPv6-Route, dann liefe jede Verbindung erst in die Zeitüberschreitung
struct V4First;
impl ureq::Resolver for V4First {
    fn resolve(&self, netloc: &str) -> std::io::Result<Vec<std::net::SocketAddr>> {
        use std::net::ToSocketAddrs;
        let mut v: Vec<std::net::SocketAddr> = netloc.to_socket_addrs()?.collect();
        v.sort_by_key(|a| a.is_ipv6());
        Ok(v)
    }
}

/// alle Quellen versuchen; Ergebnis: TLE-Text und Bericht je Quelle
pub fn download() -> (String, Vec<String>) {
    let agent = ureq::AgentBuilder::new().resolver(V4First).timeout_connect(std::time::Duration::from_secs(8))
        .timeout(std::time::Duration::from_secs(30)).user_agent(concat!("Rustwave/", env!("CARGO_PKG_VERSION"))).build();
    let mut all = String::new();
    let mut report = vec![];
    for (i, (name, url)) in TLE_SOURCES.iter().enumerate() {
        // AMSAT und SatNOGS nur, wenn CelesTrak nichts geliefert hat
        if i >= 2 && !parse_tle(&all).is_empty() { break; }
        match agent.get(url).call().map_err(|e| e.to_string()).and_then(|r| r.into_string().map_err(|e| e.to_string())) {
            Ok(t) => {
                let t = if url.contains("format=json") { satnogs_to_tle(&t) } else { t };
                let n = parse_tle(&t).len();
                report.push(format!("{name}: {n} Satelliten"));
                all += &t;
                all.push('\n');
            }
            Err(e) => report.push(format!("{name}: {}", e.chars().take(120).collect::<String>())),
        }
    }
    (all, report)
}
const C: f64 = 299_792.458; // km/s
const OMEGA_E: f64 = 7.292_115_146_706_979e-5; // rad/s

/// ein Satellit mit vorbereiteter Bahnberechnung
pub struct Sat { pub name: String, pub norad: u32, el: sgp4::Elements, k: sgp4::Constants }

/// TLE-Text (Name + 2 Zeilen je Satellit) lesen; doppelte Nummern nur einmal
pub fn parse_tle(text: &str) -> Vec<Sat> {
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end()).filter(|l| !l.trim().is_empty()).collect();
    let mut out: Vec<Sat> = vec![];
    let mut i = 0;
    while i < lines.len() {
        let (name, l1, l2, step) = if lines[i].starts_with("1 ") && lines.get(i + 1).is_some_and(|l| l.starts_with("2 ")) {
            (None, lines[i], lines[i + 1], 2)
        } else if lines.get(i + 1).is_some_and(|l| l.starts_with("1 ")) && lines.get(i + 2).is_some_and(|l| l.starts_with("2 ")) {
            (Some(lines[i].trim().to_string()), lines[i + 1], lines[i + 2], 3)
        } else { i += 1; continue; };
        i += step;
        let Ok(el) = sgp4::Elements::from_tle(name.clone(), l1.as_bytes(), l2.as_bytes()) else { continue };
        let Ok(k) = sgp4::Constants::from_elements(&el) else { continue };
        let norad = el.norad_id as u32;
        if out.iter().any(|s| s.norad == norad) { continue; }
        out.push(Sat { name: name.unwrap_or_else(|| format!("#{norad}")), norad, el, k });
    }
    out
}

/// Beobachter: Breite, Länge (Grad), Höhe (km)
#[derive(Clone, Copy, Debug)]
pub struct Obs { pub lat: f64, pub lon: f64, pub alt_km: f64 }

/// Blick vom Beobachter: Azimut, Elevation (Grad), Entfernung (km), Entfernungsänderung (km/s, + = entfernt sich)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look { pub az: f64, pub el: f64, pub range: f64, pub rate: f64, pub sub_lat: f64, pub sub_lon: f64, pub height: f64 }

fn julian(t: f64) -> f64 { t / 86400.0 + 2_440_587.5 }
/// mittlere Sternzeit Greenwich (rad)
fn gmst(t: f64) -> f64 {
    let d = julian(t) - 2_451_545.0;
    let tc = d / 36525.0;
    let deg = 280.460_618_37 + 360.985_647_366_29 * d + 0.000_387_933 * tc * tc - tc * tc * tc / 38_710_000.0;
    deg.rem_euclid(360.0).to_radians()
}
fn obs_ecef(o: &Obs) -> [f64; 3] {
    let (a, f) = (6378.137, 1.0 / 298.257_223_563);
    let e2 = f * (2.0 - f);
    let (la, lo) = (o.lat.to_radians(), o.lon.to_radians());
    let n = a / (1.0 - e2 * la.sin().powi(2)).sqrt();
    [(n + o.alt_km) * la.cos() * lo.cos(), (n + o.alt_km) * la.cos() * lo.sin(), (n * (1.0 - e2) + o.alt_km) * la.sin()]
}

impl Sat {
    /// Lage zum Zeitpunkt t (Unix-Sekunden)
    pub fn look(&self, o: &Obs, t: f64) -> Option<Look> {
        let dt = chrono::DateTime::from_timestamp(t.floor() as i64, ((t.fract()) * 1e9) as u32)?.naive_utc();
        let m = self.el.datetime_to_minutes_since_epoch(&dt).ok()?;
        let p = self.k.propagate(m).ok()?;
        // TEME → erdfest (Drehung um die Sternzeit), Geschwindigkeit mit Erddrehung
        let th = gmst(t);
        let (c, s) = (th.cos(), th.sin());
        let r = [c * p.position[0] + s * p.position[1], -s * p.position[0] + c * p.position[1], p.position[2]];
        let v0 = [c * p.velocity[0] + s * p.velocity[1], -s * p.velocity[0] + c * p.velocity[1], p.velocity[2]];
        let v = [v0[0] + OMEGA_E * r[1], v0[1] - OMEGA_E * r[0], v0[2]];
        let ob = obs_ecef(o);
        let d = [r[0] - ob[0], r[1] - ob[1], r[2] - ob[2]];
        let range = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let (la, lo) = (o.lat.to_radians(), o.lon.to_radians());
        // Ost, Nord, Oben
        let e = -lo.sin() * d[0] + lo.cos() * d[1];
        let n = -la.sin() * lo.cos() * d[0] - la.sin() * lo.sin() * d[1] + la.cos() * d[2];
        let u = la.cos() * lo.cos() * d[0] + la.cos() * lo.sin() * d[1] + la.sin() * d[2];
        let az = e.atan2(n).to_degrees().rem_euclid(360.0);
        let el = (u / range).asin().to_degrees();
        let rate = (d[0] * v[0] + d[1] * v[1] + d[2] * v[2]) / range;
        let rr = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
        let sub_lat = (r[2] / rr).asin().to_degrees();
        let sub_lon = r[1].atan2(r[0]).to_degrees();
        Some(Look { az, el, range, rate, sub_lat, sub_lon, height: rr - 6371.0 })
    }

    /// Überflüge ab t0 für `hours` Stunden mit mindestens `min_el` Grad
    pub fn passes(&self, o: &Obs, t0: f64, hours: f64, min_el: f64) -> Vec<Pass> {
        let mut out = vec![];
        let el = |t: f64| self.look(o, t).map_or(-90.0, |l| l.el);
        let step = 30.0;
        let mut t = t0;
        let mut prev = el(t);
        // läuft gerade ein Überflug: Anfang zurück suchen
        if prev > 0.0 { let mut a = t; while el(a - step) > 0.0 && t0 - a < 1200.0 { a -= step; } t = a; prev = el(t); }
        let end = t0 + hours * 3600.0;
        while t < end {
            let n = t + step;
            let cur = el(n);
            if prev <= 0.0 && cur > 0.0 {
                // Aufgang fein bestimmen
                let aos = refine(&el, t, n);
                let mut k = n;
                let (mut max_el, mut t_max) = (cur, n);
                while el(k) > 0.0 && k - aos < 3600.0 { let x = el(k); if x > max_el { max_el = x; t_max = k; } k += 10.0; }
                let los = refine(&el, k - 10.0, k);
                if max_el >= min_el {
                    let az = |t: f64| self.look(o, t).map_or(0.0, |l| l.az);
                    out.push(Pass { aos, los, t_max, max_el, aos_az: az(aos), los_az: az(los) });
                }
                t = k;
                prev = el(t);
                continue;
            }
            prev = cur;
            t = n;
        }
        out
    }
}

fn refine(el: &dyn Fn(f64) -> f64, mut a: f64, mut b: f64) -> f64 {
    let rising = el(a) <= 0.0;
    for _ in 0..20 {
        let m = (a + b) / 2.0;
        if (el(m) > 0.0) == rising { b = m; } else { a = m; }
    }
    (a + b) / 2.0
}

#[derive(Clone, Copy, Debug)]
pub struct Pass { pub aos: f64, pub los: f64, pub t_max: f64, pub max_el: f64, pub aos_az: f64, pub los_az: f64 }

/// Frequenz am Empfänger (Downlink) bzw. die zu sendende (Uplink), damit sie beim Satelliten passt
pub fn doppler_rx(f_hz: f64, rate: f64) -> f64 { f_hz * (1.0 - rate / C) }
pub fn doppler_tx(f_hz: f64, rate: f64) -> f64 { f_hz * (1.0 + rate / C) }

/// Voreinstellungen (Name, Satellit, Downlink Hz, Uplink Hz, Reiter)
pub const PRESETS: [(&str, u32, u64, u64, crate::Op); 3] = [
    ("ISS – APRS-Digipeater (Pfad ARISS)", 25544, 145_825_000, 145_825_000, crate::Op::Aprs),
    ("ISS – SSTV-Aussendungen (Frequenz änderbar)", 25544, 145_800_000, 145_800_000, crate::Op::Sstv),
    ("ISS – Sprechfunk Region 1", 25544, 145_800_000, 145_200_000, crate::Op::Sstv),
];

#[cfg(test)]
mod tests {
    use super::*;
    // ISS-Bahndaten (Beispiel aus der SGP4-Literatur, Epoche 2008-09-20)
    const ISS: &str = "ISS (ZARYA)
1 25544U 98067A   08264.51782528 -.00002182  00000-0 -11606-4 0  2927
2 25544  51.6416 247.4627 0006703 130.5360 325.0288 15.72125391563537";

    #[test]
    fn eigene_frequenz() {
        let mut st = SatState::load(&|_| None);
        st.preset = Some(1);
        assert_eq!(st.freqs().0, Some(145_800_000));
        st.ovr[1].0 = "437,800".into();
        assert_eq!(st.freqs(), (Some(437_800_000), Some(145_800_000), Some(Op::Sstv)));
        st.ovr[1].0.clear();
        assert_eq!(st.freqs().0, Some(145_800_000));
    }

    #[test]
    fn satnogs_json() {
        let j = r#"[{"tle0":"0 ISS (ZARYA)","tle1":"1 25544U 98067A   08264.51782528 -.00002182  00000-0 -11606-4 0  2927","tle2":"2 25544  51.6416 247.4627 0006703 130.5360 325.0288 15.72125391563537","norad_cat_id":25544}]"#;
        let s = parse_tle(&satnogs_to_tle(j));
        assert_eq!((s.len(), s[0].name.as_str()), (1, "ISS (ZARYA)"));
    }

    #[test]
    fn bahn_und_ueberfluege() {
        let s = parse_tle(ISS);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].name.as_str(), s[0].norad), ("ISS (ZARYA)", 25544));
        let o = Obs { lat: 52.52, lon: 13.40, alt_km: 0.05 };
        let t0 = 1_221_955_200.0; // 2008-09-21 00:00 UTC
        let l = s[0].look(&o, t0).unwrap();
        // Bahnhöhe der ISS damals um 350 km, Bahnneigung 51,6°
        assert!(l.height > 300.0 && l.height < 420.0, "{l:?}");
        assert!(l.sub_lat.abs() <= 52.0);
        let p = s[0].passes(&o, t0, 24.0, 0.0);
        assert!(p.len() >= 3 && p.len() <= 8, "{}", p.len());
        for x in &p {
            assert!(x.los > x.aos && x.los - x.aos < 15.0 * 60.0);
            assert!(x.max_el > 0.0 && x.max_el <= 90.0);
            // am Aufgang ist die Elevation ~0, Doppler dort deutlich (über 2 kHz auf 145,8 MHz)
            let la = s[0].look(&o, x.aos + 1.0).unwrap();
            assert!(la.el.abs() < 1.0);
            assert!((doppler_rx(145.8e6, la.rate) - 145.8e6).abs() > 2000.0);
        }
        // Doppler-Vorzeichen: nähert sich (Rate < 0) → höhere Frequenz
        assert!(doppler_rx(145.8e6, -7.0) > 145.8e6 && doppler_tx(145.8e6, -7.0) < 145.8e6);
    }
}

#[cfg(test)]
#[test]
fn vergleich_skyfield() {
    // Werte aus skyfield (Python) für dieselbe TLE und Berlin
    let s = parse_tle("ISS (ZARYA)
1 25544U 98067A   08264.51782528 -.00002182  00000-0 -11606-4 0  2927
2 25544  51.6416 247.4627 0006703 130.5360 325.0288 15.72125391563537");
    let o = Obs { lat: 52.52, lon: 13.40, alt_km: 0.05 };
    for (dt, az, el, range, rate) in [(0.0, 326.01, -82.01, 12988.9, -0.8644), (10800.0, 96.93, -73.62, 12601.4, 1.0243),
                                      (27000.0, 156.6, -53.03, 10610.1, 1.0623), (54000.0, 257.28, -71.03, 12427.3, -1.139)] {
        let l = s[0].look(&o, 1_221_955_200.0 + dt).unwrap();
        let daz = ((l.az - az + 540.0) % 360.0 - 180.0).abs();
        assert!(daz < 0.5 && (l.el - el).abs() < 0.3 && (l.range - range).abs() < 15.0 && (l.rate - rate).abs() < 0.01, "{dt}: {l:?}");
    }
}

// ---------------- Bedienung ----------------

use crate::{App, Op, GREEN, ORANGE};
use eframe::egui::{self, Color32, RichText};
use std::time::Instant;

pub struct SatState {
    pub show: bool,
    pub sats: Vec<Sat>,
    pub tle_status: String,
    pub tle_time: Option<std::time::SystemTime>,
    loading: Option<std::sync::mpsc::Receiver<(String, Vec<String>)>>,
    /// gewählter Satellit (NORAD-Nummer), Voreinstellung (Index) bzw. eigene Frequenzen in MHz
    pub norad: u32,
    pub preset: Option<usize>,
    pub down: String,
    pub up: String,
    pub search: String,
    /// geänderte Frequenzen der Voreinstellungen (MHz als Text, leer = Standard)
    pub ovr: [(String, String); PRESETS.len()],
    /// Doppler per CAT nachführen; bei Überflug Reiter und Frequenz automatisch einstellen
    pub doppler: bool,
    pub auto: bool,
    pub min_el: f64,
    passes: Vec<Pass>,
    passes_at: Option<(Instant, u32)>,
    last_set: Option<u64>,
    next_tick: Option<Instant>,
    in_pass: bool,
    saved_path: Option<String>,
}

impl SatState {
    pub fn load(get: &dyn Fn(&str) -> Option<String>) -> SatState {
        let mut s = SatState { show: false, sats: vec![], tle_status: String::new(), tle_time: None, loading: None,
            norad: get("sat_norad").and_then(|v| v.parse().ok()).unwrap_or(25544),
            preset: get("sat_preset").and_then(|v| v.parse().ok()).filter(|&i: &usize| i < PRESETS.len()).or(Some(0)),
            down: get("sat_down").unwrap_or_default(), up: get("sat_up").unwrap_or_default(), search: String::new(),
            ovr: std::array::from_fn(|i| (get(&format!("sat_ovr{i}_down")).unwrap_or_default(), get(&format!("sat_ovr{i}_up")).unwrap_or_default())),
            doppler: get("sat_doppler").as_deref() == Some("1"), auto: get("sat_auto").as_deref() == Some("1"),
            min_el: get("sat_min_el").and_then(|v| v.parse().ok()).unwrap_or(10.0),
            passes: vec![], passes_at: None, last_set: None, next_tick: None, in_pass: false, saved_path: None };
        if s.preset.is_some() && get("sat_preset").as_deref() == Some("-") { s.preset = None; }
        s.load_file();
        s
    }
    pub fn save(&self, st: &mut dyn eframe::Storage) {
        st.set_string("sat_norad", self.norad.to_string());
        st.set_string("sat_preset", self.preset.map_or("-".into(), |i| i.to_string()));
        st.set_string("sat_down", self.down.clone());
        st.set_string("sat_up", self.up.clone());
        st.set_string("sat_doppler", if self.doppler { "1" } else { "0" }.into());
        st.set_string("sat_auto", if self.auto { "1" } else { "0" }.into());
        st.set_string("sat_min_el", self.min_el.to_string());
        for (i, (d, u)) in self.ovr.iter().enumerate() {
            st.set_string(&format!("sat_ovr{i}_down"), d.clone());
            st.set_string(&format!("sat_ovr{i}_up"), u.clone());
        }
    }
    fn file() -> std::path::PathBuf { crate::save_dir().join("tle.txt") }
    fn load_file(&mut self) {
        match std::fs::read_to_string(Self::file()) {
            Ok(t) => {
                self.sats = parse_tle(&t);
                self.tle_time = std::fs::metadata(Self::file()).and_then(|m| m.modified()).ok();
                self.tle_status = format!("{} Satelliten aus {}", self.sats.len(), Self::file().display());
            }
            Err(_) => self.tle_status = "noch keine Bahndaten – „Bahndaten laden“ holt sie von CelesTrak".into(),
        }
        self.passes_at = None;
    }
    pub fn sat(&self) -> Option<&Sat> { self.sats.iter().find(|s| s.norad == self.norad) }
    /// Downlink/Uplink in Hz und Reiter für die aktuelle Auswahl
    pub fn freqs(&self) -> (Option<u64>, Option<u64>, Option<Op>) {
        let mhz = |s: &str| s.trim().replace(',', ".").parse::<f64>().ok().filter(|v| *v > 1.0).map(|v| (v * 1e6).round() as u64);
        if let Some(i) = self.preset {
            let p = &PRESETS[i];
            let (d, u) = &self.ovr[i];
            return (Some(mhz(d).unwrap_or(p.2)), Some(mhz(u).unwrap_or(p.3)), Some(p.4));
        }
        let d = mhz(&self.down);
        (d, mhz(&self.up).or(d), None)
    }
}

fn now_s() -> f64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64()) }
fn local(t: f64) -> chrono::DateTime<chrono::Local> { chrono::DateTime::from_timestamp(t as i64, 0).unwrap_or_default().with_timezone(&chrono::Local) }

impl App {
    fn sat_obs(&self) -> Option<Obs> {
        let (lat, lon) = self.aprs_pos_pub()?;
        let alt = self.gps.fix.and_then(|f| f.0.alt_m).unwrap_or(0.0) / 1000.0;
        Some(Obs { lat, lon, alt_km: alt })
    }

    /// je Bild: Bahndaten-Download, Überflüge, Doppler, Automatik
    pub(crate) fn sat_tick(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.sat.loading {
            if let Ok((t, report)) = rx.try_recv() {
                self.sat.loading = None;
                let rep = report.join(" · ");
                self.sat_store(&t, &rep);
            } else { ctx.request_repaint_after(std::time::Duration::from_millis(300)); }
        }
        if !(self.sat.doppler || self.sat.auto || self.sat.show) { return; }
        if self.sat.next_tick.is_some_and(|t| Instant::now() < t) { return; }
        self.sat.next_tick = Some(Instant::now() + std::time::Duration::from_millis(1000));
        ctx.request_repaint_after(std::time::Duration::from_millis(1000));
        let Some(o) = self.sat_obs() else { return };
        let t = now_s();
        // Überflüge alle 5 min neu
        let need = self.sat.passes_at.is_none_or(|(at, n)| at.elapsed().as_secs() > 300 || n != self.sat.norad);
        let (look, name, passes) = {
            let Some(s) = self.sat.sat() else { return };
            (s.look(&o, t), s.name.clone(), need.then(|| s.passes(&o, t, 24.0, self.sat.min_el)))
        };
        if let Some(p) = passes { self.sat.passes = p; self.sat.passes_at = Some((Instant::now(), self.sat.norad)); }
        let Some(l) = look else { return };
        let (down, up, op) = self.sat.freqs();
        let visible = l.el > 0.0;
        // Automatik: zu Beginn Reiter, Betriebsart und Pfad einstellen, am Ende Pfad zurück
        if self.sat.auto && visible && !self.sat.in_pass {
            if let Some(op) = op { if self.op != op && self.tx.is_none() { self.switch_op(op); } }
            if let Some(d) = down {
                let f = if self.sat.doppler { doppler_rx(d as f64, l.rate).round() as u64 } else { d };
                self.tune_to(f as f64 / 1000.0, Some(crate::cat::Sb::Fm));
                self.sat.last_set = Some(f);
            }
            if op == Some(Op::Aprs) && self.sat.saved_path.is_none() {
                self.sat.saved_path = Some(std::mem::replace(&mut self.aprs.path, "ARISS".into()));
            }
            self.log(format!("Satellit {} geht auf (max. {:.0}°)", name, self.sat.passes.first().map_or(0.0, |p| p.max_el)));
        }
        if self.sat.auto && !visible && self.sat.in_pass {
            if let Some(p) = self.sat.saved_path.take() { self.aprs.path = p; }
            self.log(format!("Satellit {name} ist untergegangen"));
        }
        self.sat.in_pass = visible;
        // Doppler: im Empfang Downlink, beim Senden Uplink korrigieren
        if self.sat.doppler && l.el > -1.0 && self.rig.is_some() {
            let sending = self.tx.is_some() || self.talk.is_some();
            let target = match (sending, down, up) {
                (false, Some(d), _) => Some(doppler_rx(d as f64, l.rate)),
                (true, _, Some(u)) => Some(doppler_tx(u as f64, l.rate)),
                _ => None,
            };
            if let Some(f) = target {
                let step = if self.poll.mode.is_some_and(|m| m.0 == "FM") { 500.0 } else { 20.0 };
                let f = ((f / step).round() * step) as u64;
                if self.sat.last_set.is_none_or(|x| x.abs_diff(f) as f64 >= step) {
                    if let Some(r) = &self.rig { self.poll.cmdq.push_back((0, crate::cat::set_freq(&r.model, f))); }
                    self.sat.last_set = Some(f);
                }
            }
        }
    }

    /// geladene Bahndaten speichern (oder melden, dass keine kamen)
    fn sat_store(&mut self, t: &str, report: &str) {
        let n = parse_tle(t).len();
        if n == 0 {
            self.sat.tle_status = format!("keine Bahndaten erhalten – {report}. Ausweg: tle.txt im Browser laden und mit „aus Datei …“ einlesen");
            self.log(format!("Satelliten: keine Bahndaten ({report})"));
            return;
        }
        match std::fs::create_dir_all(crate::save_dir()).and_then(|_| std::fs::write(SatState::file(), t)) {
            Ok(()) => { self.sat.load_file(); self.sat.tle_status += &format!(" ({report})"); self.log(format!("Satelliten: {n} Bahndaten geladen ({report})")); }
            Err(e) => self.log(format!("Satelliten: {e}")),
        }
    }

    pub(crate) fn sat_download(&mut self) {
        if self.sat.loading.is_some() { return; }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || { let _ = tx.send(download()); });
        self.sat.loading = Some(rx);
        self.sat.tle_status = "lade Bahndaten (CelesTrak, sonst AMSAT bzw. SatNOGS) …".into();
    }

    /// Bahndaten aus einer Datei (TLE-Text oder SatNOGS-JSON), z. B. im Browser geladen
    fn sat_import(&mut self) {
        #[cfg(windows)]
        if let Some(p) = rfd::FileDialog::new().add_filter("Bahndaten", &["txt", "tle", "json"]).pick_file() {
            match std::fs::read_to_string(&p) {
                Ok(t) => { let t = if t.trim_start().starts_with('[') { satnogs_to_tle(&t) } else { t }; self.sat_store(&t, &format!("aus {}", p.display())); }
                Err(e) => self.log(format!("Satelliten: {e}")),
            }
        }
        #[cfg(not(windows))]
        self.log(format!("Satelliten: Datei als {} speichern", SatState::file().display()));
    }

    /// Fenster „Satelliten“
    pub(crate) fn ui_sat_window(&mut self, ctx: &egui::Context) {
        if !self.sat.show { return; }
        let mut open = true;
        egui::Window::new("Satelliten").open(&mut open).resizable(true).default_width(520.0).show(ctx, |ui| egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&self.sat.tle_status).small().weak());
                if let Some(t) = self.sat.tle_time.and_then(|t| t.elapsed().ok()) {
                    let d = t.as_secs() / 86400;
                    ui.label(RichText::new(format!("· {d} Tage alt")).small().color(if d > 7 { ORANGE } else { Color32::GRAY }));
                }
                if ui.add_enabled(self.sat.loading.is_none(), egui::Button::new("Bahndaten laden").small())
                    .on_hover_text("Amateurfunk- und Stationssatelliten von celestrak.org; klappt das nicht, von amsat.org bzw. db.satnogs.org. \
                        Etwa wöchentlich erneuern – CelesTrak sperrt Abfragen, die öfter als alle paar Stunden kommen.").clicked() { self.sat_download(); }
                if ui.small_button("aus Datei …").on_hover_text("TLE-Datei (z. B. https://www.amsat.org/tle/current/nasabare.txt im Browser gespeichert) oder SatNOGS-JSON einlesen").clicked() { self.sat_import(); }
            });
            // Auswahl
            ui.horizontal_wrapped(|ui| {
                ui.label("Satellit");
                let cur = match self.sat.preset { Some(i) => PRESETS[i].0.to_string(), None => self.sat.sat().map_or(format!("#{}", self.sat.norad), |s| s.name.clone()) };
                egui::ComboBox::from_id_salt("sat_sel").width(300.0).height(400.0).selected_text(cur).show_ui(ui, |ui| {
                    for (i, p) in PRESETS.iter().enumerate() {
                        if ui.selectable_label(self.sat.preset == Some(i), p.0).clicked() { self.sat.preset = Some(i); self.sat.norad = p.1; self.sat.passes_at = None; }
                    }
                    ui.separator();
                    ui.add(egui::TextEdit::singleline(&mut self.sat.search).hint_text("Suche, z. B. SO-50").desired_width(200.0));
                    let q = self.sat.search.to_lowercase();
                    for s in self.sat.sats.iter().filter(|s| q.is_empty() || s.name.to_lowercase().contains(&q)).take(200) {
                        if ui.selectable_label(self.sat.preset.is_none() && self.sat.norad == s.norad, &s.name).clicked() {
                            self.sat.preset = None; self.sat.norad = s.norad; self.sat.passes_at = None;
                        }
                    }
                });
            });
            match self.sat.preset {
                None => {
                    ui.horizontal(|ui| {
                        ui.label("Downlink");
                        ui.add(egui::TextEdit::singleline(&mut self.sat.down).desired_width(80.0).hint_text("MHz"));
                        ui.label("Uplink");
                        ui.add(egui::TextEdit::singleline(&mut self.sat.up).desired_width(80.0).hint_text("MHz (leer = gleich)"));
                    }).response.on_hover_text("Frequenzen des Satelliten laut aktueller Liste (z. B. AMSAT); Rustwave rechnet den Doppler darauf");
                }
                Some(i) => {
                    let p = &PRESETS[i];
                    let mhz = |hz: u64| format!("{:.4}", hz as f64 / 1e6);
                    let (d, u) = &mut self.sat.ovr[i];
                    let changed = !d.trim().is_empty() || !u.trim().is_empty();
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Downlink");
                        ui.add(egui::TextEdit::singleline(d).desired_width(80.0).hint_text(mhz(p.2)))
                            .on_hover_text(format!("MHz; leer = Standard {} MHz. Die ISS weicht für SSTV-Aktionen manchmal aus, z. B. ins 70-cm-Band – \
                                die Frequenz steht in der Ankündigung von ARISS.", mhz(p.2)));
                        ui.label("Uplink");
                        ui.add(egui::TextEdit::singleline(u).desired_width(80.0).hint_text(mhz(p.3))).on_hover_text(format!("MHz; leer = Standard {} MHz", mhz(p.3)));
                        if ui.add_enabled(changed, egui::Button::new("Standard").small()).on_hover_text("eigene Frequenzen löschen").clicked() { d.clear(); u.clear(); }
                    });
                    if changed { ui.label(RichText::new("eigene Frequenz statt der Voreinstellung").small().color(ORANGE)); }
                }
            }
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.sat.doppler, "Doppler per CAT nachführen").on_hover_text(
                    "Im Empfang die Downlink-, beim Senden die Uplink-Frequenz korrigieren (FM in 500-Hz-, sonst 20-Hz-Schritten)");
                ui.checkbox(&mut self.sat.auto, "bei Überflug Reiter und Frequenz einstellen").on_hover_text(
                    "Beim Aufgang: Reiter (APRS bzw. SSTV), FM und Frequenz; bei der ISS-APRS-Voreinstellung Pfad ARISS, danach wieder der alte Pfad");
                ui.add(egui::DragValue::new(&mut self.sat.min_el).range(0.0..=60.0).suffix("° min.")).on_hover_text("Überflüge erst ab dieser Höhe zeigen");
            });
            ui.separator();
            let Some(o) = self.sat_obs() else {
                ui.label(RichText::new("Eigene Position fehlt – bei der APRS-Bake eintragen oder Locator unter Einstellungen › Station angeben.").color(ORANGE));
                return;
            };
            let Some(s) = self.sat.sat() else { ui.label(RichText::new("Satellit nicht in den Bahndaten – „Bahndaten laden“.").color(ORANGE)); return; };
            let t = now_s();
            let (down, up, _) = self.sat.freqs();
            if let Some(l) = s.look(&o, t) {
                ui.horizontal_top(|ui| {
                    polar(ui, s, &o, self.sat.passes.first().copied(), &l, t);
                    ui.vertical(|ui| {
                        let vis = l.el > 0.0;
                        ui.label(RichText::new(if vis { format!("{} ist zu sehen", s.name) } else { s.name.clone() }).strong().color(if vis { GREEN } else { ui.visuals().text_color() }));
                        ui.label(format!("Azimut {:.0}° ({}) · Elevation {:.1}°", l.az, crate::aprs::compass(l.az), l.el));
                        ui.label(format!("Entfernung {:.0} km · Höhe {:.0} km", l.range, l.height));
                        ui.label(format!("Unterpunkt {:.1}°, {:.1}°", l.sub_lat, l.sub_lon));
                        if let Some(d) = down {
                            ui.label(RichText::new(format!("Downlink {:.4} MHz {:+.0} Hz = {:.4} MHz", d as f64 / 1e6, doppler_rx(d as f64, l.rate) - d as f64,
                                doppler_rx(d as f64, l.rate) / 1e6)).monospace().small());
                        }
                        if let Some(u) = up {
                            ui.label(RichText::new(format!("Uplink   {:.4} MHz {:+.0} Hz = {:.4} MHz", u as f64 / 1e6, doppler_tx(u as f64, l.rate) - u as f64,
                                doppler_tx(u as f64, l.rate) / 1e6)).monospace().small());
                        }
                        if let Some(p) = self.sat.passes.iter().find(|p| p.los > t) {
                            let w = p.aos - t;
                            ui.label(RichText::new(if w > 0.0 { format!("nächster Überflug in {}:{:02} min", (w / 60.0) as u64, (w % 60.0) as u64) }
                                else { format!("noch {:.0} s sichtbar", p.los - t) }).color(ORANGE));
                        }
                    });
                });
            }
            ui.separator();
            ui.label(RichText::new(format!("Überflüge der nächsten 24 h (ab {:.0}°)", self.sat.min_el)).strong());
            egui::Grid::new("sat_passes").num_columns(5).striped(true).spacing([14.0, 2.0]).show(ui, |ui| {
                for h in ["Tag", "Aufgang", "höchster Stand", "Untergang", "Dauer"] { ui.label(RichText::new(h).small().strong()); }
                ui.end_row();
                for p in self.sat.passes.iter().take(12) {
                    const WT: [&str; 7] = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"];
                    use chrono::Datelike;
                    ui.label(format!("{} {}", WT[local(p.aos).weekday().num_days_from_monday() as usize], local(p.aos).format("%d.%m.")));
                    ui.label(format!("{} {}", local(p.aos).format("%H:%M"), crate::aprs::compass(p.aos_az)));
                    ui.label(RichText::new(format!("{} {:.0}°", local(p.t_max).format("%H:%M"), p.max_el)).color(if p.max_el >= 40.0 { GREEN } else { ui.visuals().text_color() }));
                    ui.label(format!("{} {}", local(p.los).format("%H:%M"), crate::aprs::compass(p.los_az)));
                    ui.label(format!("{:.0} min", (p.los - p.aos) / 60.0));
                    ui.end_row();
                }
            });
            ui.label(RichText::new("ISS-APRS: Pfad ARISS, Bake mit wenig Text; ISS-SSTV: Aktionen werden vorher bei ARISS angekündigt – der SSTV-Reiter \
                dekodiert die Bilder automatisch. Uhrzeiten in Ortszeit.").small().weak());
        }));
        if !open { self.sat.show = false; }
    }
}

/// Himmelskarte: Bahn des (nächsten bzw. laufenden) Überflugs und aktuelle Position
fn polar(ui: &mut egui::Ui, s: &Sat, o: &Obs, pass: Option<Pass>, now: &Look, t: f64) {
    let size = 170.0;
    let (r, p) = ui.allocate_painter(egui::vec2(size, size), egui::Sense::hover());
    let c = r.rect.center();
    let rad = size / 2.0 - 14.0;
    let at = |az: f64, el: f64| { let k = (90.0 - el.max(0.0)) as f32 / 90.0 * rad; let a = (az as f32).to_radians(); c + egui::vec2(a.sin(), -a.cos()) * k };
    p.circle_filled(c, rad + 2.0, Color32::from_gray(20));
    for e in [0.0, 30.0, 60.0] { p.circle_stroke(c, (90.0 - e) / 90.0 * rad, egui::Stroke::new(1.0_f32, Color32::from_gray(if e == 0.0 { 110 } else { 60 }))); }
    for (az, txt) in [(0.0, "N"), (90.0, "O"), (180.0, "S"), (270.0, "W")] {
        p.text(at(az, -13.0) + (at(az, 0.0) - c) * 0.12, egui::Align2::CENTER_CENTER, txt, egui::FontId::proportional(11.0), Color32::from_gray(170));
    }
    if let Some(ps) = pass {
        let mut pts = vec![];
        let mut k = ps.aos;
        while k <= ps.los { if let Some(l) = s.look(o, k) { pts.push(at(l.az, l.el)); } k += 15.0; }
        p.add(egui::Shape::line(pts, egui::Stroke::new(2.0_f32, ORANGE.gamma_multiply(0.7))));
        if let Some(l) = s.look(o, ps.aos) { p.circle_filled(at(l.az, l.el), 3.0, ORANGE); }
    }
    if now.el > 0.0 { p.circle_filled(at(now.az, now.el), 5.0, GREEN); }
    let _ = t;
}
