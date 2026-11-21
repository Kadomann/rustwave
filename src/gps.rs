//! GPS des Funkgeräts (Icom 23 00) im Hintergrund abfragen: Position, Tempo, Kurs und UTC-Zeit.
//! Die Abfrage blockiert die Oberfläche nicht: senden, Antwort in späteren Bildern einsammeln.

use crate::cat::{self, GpsFix};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct GpsState {
    /// letzte gültige Position und wann sie kam
    pub fix: Option<(GpsFix, Instant)>,
    /// letzte Meldung „kein Empfang“ o. Ä.
    pub status: String,
    next: Option<Instant>,
    misses: u32,
    /// Gerät kann kein GPS (NG oder keine Antwort) – nicht mehr fragen
    pub unsupported: bool,
    /// Abweichung der PC-Uhr von der GPS-Zeit (s), wenn mindestens 2 s
    pub clock_offset: Option<i64>,
    /// Zeitpunkt der Verbindung mit dem Funkgerät
    started: Option<Instant>,
}

impl GpsState {
    /// frische Position (höchstens `max_age` alt)
    pub fn fresh(&self, max_age: Duration) -> Option<GpsFix> {
        self.fix.filter(|(_, t)| t.elapsed() <= max_age).map(|(f, _)| f)
    }
    pub fn reset(&mut self) { *self = GpsState { clock_offset: self.clock_offset, started: Some(Instant::now()), ..Default::default() }; }
    /// Gerät verbunden, GPS möglich, aber noch keine Position – höchstens `max` lang
    pub fn waiting_first(&self, max: Duration) -> bool {
        self.fix.is_none() && !self.unsupported && self.started.is_some_and(|t| t.elapsed() < max)
    }
}

/// Uhrzeit (UTC, Unix-Sekunden), bei falsch gehender PC-Uhr nach GPS korrigiert
pub fn now_utc(g: &GpsState) -> i64 { chrono::Utc::now().timestamp() + g.clock_offset.unwrap_or(0) }

impl crate::App {
    /// einmal pro Bild aufrufen: GPS über die laufende CAT-Abfrage anfordern und Antworten auswerten
    pub(crate) fn gps_tick(&mut self, interval: Duration) {
        let Some(rig) = &self.rig else { return };
        if rig.model.proto != cat::Proto::Civ || self.gps.unsupported { return; }
        let model = rig.model.clone();
        let now = Instant::now();
        let mut msgs = vec![];
        let g = &mut self.gps;
        if let Some(r) = self.poll.gps.take() {
            match r.map(|frame| cat::parse_gps_full(&model, &frame)) {
                Ok(Some(Ok(f))) => {
                    g.misses = 0;
                    g.status.clear();
                    if let Some(t) = f.utc {
                        let off = t - chrono::Utc::now().timestamp();
                        let new = (off.abs() >= 2).then_some(off);
                        if new.is_some() && g.clock_offset.is_none_or(|o| (o - off).abs() > 2) {
                            msgs.push(format!("PC-Uhr weicht um {off:+} s von der GPS-Zeit ab – das Logbuch nimmt die GPS-Zeit"));
                        }
                        g.clock_offset = new;
                    }
                    g.fix = Some((f, now));
                }
                Ok(Some(Err(e))) => g.status = e,
                Ok(None) => {}
                Err(e) if e == "NG" => {
                    g.unsupported = true;
                    msgs.push("Funkgerät liefert keine GPS-Daten (NG) – GPS-Abfrage beendet".into());
                }
                Err(e) => {
                    g.misses += 1;
                    g.status = e;
                    if g.misses >= 3 { g.unsupported = true; }
                }
            }
        }
        if g.next.is_none_or(|t| now >= t) {
            self.poll.want_gps = true;
            g.next = Some(now + interval);
        }
        for m in msgs { self.log(m); }
    }
}
