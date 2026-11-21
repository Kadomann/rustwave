//! Selbsttest: prüft Audio, CAT, WLAN, Access Point, GPS, Uhr, Standort, Darstellung und Ablage
//! und zeigt die Rohdaten – als Bericht zum Kopieren oder Speichern.

use crate::{apmode, cat, App, GREEN, ORANGE, RED};
use eframe::egui::{self, Color32, RichText};
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
pub enum Lvl { Ok, Warn, Err, Info }

#[derive(Default)]
pub struct Diag {
    pub lines: Vec<(Lvl, String)>,
    /// läuft: Start und Zahl der bis dahin empfangenen Audio-Samples (für die Ratenmessung)
    audio_probe: Option<(Instant, u64)>,
    pub waiting_location: bool,
}

fn hex(b: &[u8]) -> String { if b.is_empty() { "(nichts)".into() } else { b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ") } }

impl App {
    fn d(&mut self, l: Lvl, s: impl Into<String>) { self.diag.lines.push((l, s.into())); }

    /// alles prüfen, was sofort geht; Audiorate und Windows-Standort kommen nach
    pub(crate) fn diag_start(&mut self, ctx: &egui::Context) {
        self.diag = Default::default();
        let now = chrono::Utc::now();
        self.d(Lvl::Info, format!("Rustwave {} · {} · {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, now.format("%Y-%m-%d %H:%M:%S UTC")));

        // Darstellung
        let r = ctx.content_rect();
        self.d(Lvl::Info, format!("Darstellung: {:.0}×{:.0} Punkte, Skalierung {:.2}, Zoom {:.0} %, Touch {}, Berührung erkannt: {}",
            r.width(), r.height(), ctx.pixels_per_point(), self.zoom * 100.0,
            match self.touch { Some(true) => "an", Some(false) => "aus", None => "automatisch" }, if self.touch_seen { "ja" } else { "nein" }));

        // Audio
        if self.net.is_some() {
            self.d(Lvl::Info, "Audio: über WLAN/LAN vom Funkgerät (48 kHz)");
        } else if let Some(i) = &self.input {
            let fs = i.fs;
            self.d(Lvl::Info, format!("Audio: Soundkarte „{}“, {fs:.0} Hz", if self.in_dev.is_empty() { "Standard" } else { &self.in_dev }));
        } else {
            self.d(Lvl::Warn, "Audio: nicht gestartet");
        }
        if self.net.is_some() || self.input.is_some() {
            self.diag.audio_probe = Some((Instant::now(), self.rx_sh.rx_samples.load(Relaxed)));
            self.d(Lvl::Info, format!("Eingangspegel {:.0} % (Effektivwert)", self.level * 100.0));
        }

        // WLAN
        if let Some(n) = &self.net {
            let sh = &n.shared;
            let (rx, rec, lost) = (sh.audio_rx.load(Relaxed), sh.audio_recovered.load(Relaxed), sh.audio_lost.load(Relaxed));
            let pct = 100.0 * lost as f64 / (rx + lost).max(1) as f64;
            let name = n.name.clone();
            self.d(if pct > 0.5 { Lvl::Warn } else { Lvl::Ok }, format!("WLAN: {name}, Audiopakete {rx}, nachgefordert {rec}, verloren {lost} ({pct:.2} %)"));
        }
        let host = if self.net_ap { apmode::AP_IP.to_string() } else { self.net_host.trim().to_string() };
        if self.use_net && !host.is_empty() {
            match apmode::local_addr_towards(&host) {
                Some(a) => self.d(Lvl::Info, format!("Weg zum Funkgerät {host}: PC-Adresse {a}")),
                None => self.d(Lvl::Err, format!("Kein Netzweg zum Funkgerät {host}")),
            }
        }
        if self.net_ap {
            match apmode::pc_in_ap_net() {
                Some(a) => self.d(Lvl::Ok, format!("Access Point: PC ist im WLAN des IC-705 ({a})")),
                None => self.d(Lvl::Err, "Access Point: PC ist nicht im WLAN des IC-705 (keine Adresse 192.168.59.x)"),
            }
        }

        // CAT: Frequenz und GPS mit Rohdaten
        let model = self.rig.as_ref().map(|r| r.model.clone());
        match (model, &mut self.rig) {
            (Some(m), Some(rig)) => {
                let q = cat::get_freq(&m);
                let raw = rig.query_raw(&q);
                let f = raw.as_ref().ok().and_then(|b| cat::parse_freq(&m, b));
                let lines = vec![
                    (Lvl::Info, format!("CAT: {} ({})", m.name, if rig.is_net() { "WLAN/LAN".to_string() } else { format!("{}, {} Bd", self.port, m.baud) })),
                    match (&raw, f) {
                        (Ok(b), Some(f)) => (Lvl::Ok, format!("Frequenz lesen: {:.4} MHz · gesendet {} · empfangen {}", f as f64 / 1e6, hex(&q), hex(b))),
                        (Ok(b), None) => (Lvl::Err, format!("Frequenz lesen: keine passende Antwort · gesendet {} · empfangen {}", hex(&q), hex(b))),
                        (Err(e), _) => (Lvl::Err, format!("Frequenz lesen: {e}")),
                    },
                ];
                let mut more = lines;
                if m.proto == cat::Proto::Civ {
                    let q = cat::get_gps(&m);
                    match rig.query_raw(&q) {
                        Ok(b) => {
                            let line = match cat::parse_gps_full(&m, &b) {
                                Some(Ok(g)) => {
                                    let t = g.utc.and_then(|t| chrono::DateTime::from_timestamp(t, 0)).map_or("–".into(), |d| d.format("%H:%M:%S UTC").to_string());
                                    let off = g.utc.map_or(String::new(), |t| format!(", PC-Uhr {:+} s", chrono::Utc::now().timestamp() - t));
                                    (Lvl::Ok, format!("GPS: {:.5}, {:.5} · Höhe {} · Kurs {} · {} · Zeit {t}{off}", g.lat, g.lon,
                                        g.alt_m.map_or("–".into(), |a| format!("{a:.1} m")), g.course.map_or("–".into(), |c| format!("{c:.0}°")),
                                        g.speed_kmh.map_or("–".into(), |v| format!("{v:.1} km/h"))))
                                }
                                Some(Err(e)) => (Lvl::Warn, format!("GPS: {e}")),
                                None => (Lvl::Warn, "GPS: keine Antwort".into()),
                            };
                            more.push(line);
                            more.push((Lvl::Info, format!("GPS roh: gesendet {} · empfangen {}", hex(&q), hex(&b))));
                        }
                        Err(e) => more.push((Lvl::Err, format!("GPS: {e}"))),
                    }
                }
                self.diag.lines.extend(more);
            }
            _ => self.d(Lvl::Warn, "CAT: kein Funkgerät verbunden"),
        }
        if let Some(o) = self.gps.clock_offset { self.d(Lvl::Warn, format!("Uhr: PC weicht {:+} s von der GPS-Zeit ab (Logbuch nimmt GPS-Zeit)", -o)); }

        // Ablage
        for (what, dir) in [("Bilder/Aufnahmen", crate::save_dir()), ("Einstellungen/Logbuch", crate::data_dir())] {
            let probe = dir.join(".schreibtest");
            let ok = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&probe, b"x")).and_then(|_| std::fs::remove_file(&probe));
            match ok {
                Ok(()) => self.d(Lvl::Ok, format!("{what}: {} beschreibbar", dir.display())),
                Err(e) => self.d(Lvl::Err, format!("{what}: {} – {e}", dir.display())),
            }
        }

        // Windows-Standort (dauert einige Sekunden)
        self.diag.waiting_location = true;
        let tx = self.ev_tx.clone();
        std::thread::spawn(move || {
            let r = crate::aprs_ui::windows_location();
            let _ = tx.send(crate::UiEv::DiagLine(match r {
                Ok((a, o, acc)) => (Lvl::Ok, format!("Windows-Standort: {a:.5}, {o:.5} (±{acc:.0} m)")),
                Err(e) => (Lvl::Warn, format!("Windows-Standort: {e}")),
            }));
        });
    }

    /// je Bild: Messung der Audiorate abschließen
    pub(crate) fn diag_tick(&mut self) {
        let Some((t0, n0)) = self.diag.audio_probe else { return };
        if t0.elapsed() < Duration::from_secs(3) { return; }
        self.diag.audio_probe = None;
        let n = self.rx_sh.rx_samples.load(Relaxed) - n0;
        let rate = n as f64 / t0.elapsed().as_secs_f64();
        let nominal = if self.net.is_some() { 48000.0 } else { self.input.as_ref().map_or(0.0, |i| i.fs) };
        let dev = if nominal > 0.0 { (rate / nominal - 1.0) * 100.0 } else { 0.0 };
        self.d(if dev.abs() < 2.0 { Lvl::Ok } else { Lvl::Warn }, format!("Audio kommt an: {rate:.0} Samples/s (Soll {nominal:.0}, {dev:+.1} %)"));
    }

    fn diag_text(&self) -> String {
        self.diag.lines.iter().map(|(l, s)| format!("{} {s}", match l { Lvl::Ok => "[ok]  ", Lvl::Warn => "[!]   ", Lvl::Err => "[FEHLER]", Lvl::Info => "[i]   " })).collect::<Vec<_>>().join("\n")
    }

    pub(crate) fn ui_diag(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Selbsttest");
            if ui.button("Prüfung starten").clicked() { self.diag_start(ctx); }
            let has = !self.diag.lines.is_empty();
            if ui.add_enabled(has, egui::Button::new("Bericht kopieren")).clicked() { ctx.copy_text(self.diag_text()); self.log("Bericht in die Zwischenablage kopiert"); }
            if ui.add_enabled(has, egui::Button::new("Bericht speichern")).clicked() {
                let p = crate::save_dir().join(format!("selbsttest-{}.txt", chrono::Local::now().format("%Y%m%d-%H%M%S")));
                let r = std::fs::create_dir_all(crate::save_dir()).and_then(|_| std::fs::write(&p, self.diag_text()));
                self.log(match r { Ok(()) => format!("Bericht gespeichert: {}", p.display()), Err(e) => format!("Bericht: {e}") });
            }
            if ui.button("Tastatur testen").on_hover_text("Bildschirmtastatur einblenden").clicked() { crate::touchkbd::show(self.ev_tx.clone()); }
            if ui.button("Zurück").clicked() { self.tab = crate::Tab::Operate; }
        });
        ui.label(RichText::new("Prüft Audio, Funkgerät (mit Rohdaten), WLAN, Access Point, GPS, Uhr, Standort und Ablage. Vorher Audio bzw. Funkgerät verbinden.").small().weak());
        if self.diag.audio_probe.is_some() || self.diag.waiting_location { ui.label(RichText::new("messe noch …").color(ORANGE)); ctx.request_repaint_after(Duration::from_millis(300)); }
        egui::Frame::new().fill(Color32::from_gray(12)).inner_margin(8.0).corner_radius(4.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if self.diag.lines.is_empty() { ui.label(RichText::new("Noch nicht geprüft.").weak()); }
                for (l, s) in &self.diag.lines {
                    let (sym, c) = match l { Lvl::Ok => ("✔", GREEN), Lvl::Warn => ("!", ORANGE), Lvl::Err => ("✘", RED), Lvl::Info => ("·", Color32::GRAY) };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(sym).strong().color(c));
                        ui.add(egui::Label::new(RichText::new(s).monospace().size(12.0)).wrap());
                    });
                }
            });
        });
    }
}
