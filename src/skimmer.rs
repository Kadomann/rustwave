//! CW-Skimmer: findet alle CW-Signale im Durchlassbereich (gemitteltes Spektrum, Spitzen über dem Rauschen)
//! und dekodiert jedes mit einem eigenen Dekoder. Signale, die 20 s nicht mehr zu sehen sind, fallen heraus.

use crate::cw;
use rustfft::num_complex::Complex;
use std::sync::Arc;

const N: usize = 4096;
const HOP: usize = 2048;
const MAX_CHANS: usize = 10;
const LO_HZ: f64 = 250.0;
const HI_HZ: f64 = 2300.0;
/// Spitze mindestens so weit über dem Rauschen (Leistung, 10 dB)
const PEAK_OVER: f32 = 10.0;

/// Stand eines Kanals für die Anzeige
#[derive(Clone, Debug)]
pub struct SkimInfo { pub pitch: f64, pub wpm: f32, pub snr: f32, pub text: String, pub call: Option<String> }

struct Chan { pitch: f64, dec: cw::Decoder, text: String, last_seen: u64 }

pub struct Skimmer {
    fs: f64,
    fft: Arc<dyn rustfft::Fft<f32>>,
    win: Vec<f32>,
    ring: Vec<f32>,
    since: usize,
    avg: Vec<f32>,
    chans: Vec<Chan>,
    t: u64,
    last_report: u64,
    squelch: f64,
}

impl Skimmer {
    pub fn new(fs: f64, squelch: f64) -> Self {
        let win = (0..N).map(|i| (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / N as f64).cos()) as f32).collect();
        Skimmer { fs, fft: rustfft::FftPlanner::new().plan_fft_forward(N), win, ring: Vec::with_capacity(N), since: 0,
            avg: vec![0.0; N / 2], chans: vec![], t: 0, last_report: 0, squelch }
    }

    /// Audio verarbeiten; etwa alle 0,5 s den Stand aller Kanäle (nach Tonhöhe sortiert)
    pub fn push(&mut self, x: &[f32]) -> Option<Vec<SkimInfo>> {
        for c in &mut self.chans {
            let t = c.dec.push(x);
            if !t.is_empty() {
                c.text.push_str(&t);
                if c.text.len() > 400 { let cut = c.text.len() - 300; c.text = c.text[c.text.ceil_char_boundary(cut)..].to_string(); }
            }
        }
        for &v in x {
            if self.ring.len() == N { self.ring.remove(0); }
            self.ring.push(v);
            self.since += 1;
            if self.since >= HOP && self.ring.len() == N { self.since = 0; self.spectrum(); }
        }
        self.t += x.len() as u64;
        if self.t - self.last_report >= (self.fs * 0.5) as u64 {
            self.last_report = self.t;
            let mut v: Vec<SkimInfo> = self.chans.iter().map(|c| SkimInfo {
                pitch: c.pitch, wpm: c.dec.wpm, snr: c.dec.snr_db, call: last_call(&c.text),
                text: c.text.chars().rev().take(120).collect::<Vec<_>>().into_iter().rev().collect(),
            }).collect();
            v.sort_by(|a, b| a.pitch.total_cmp(&b.pitch));
            return Some(v);
        }
        None
    }

    fn spectrum(&mut self) {
        let mut buf: Vec<Complex<f32>> = self.ring.iter().zip(&self.win).map(|(v, w)| Complex::new(v * w, 0.0)).collect();
        self.fft.process(&mut buf);
        for (a, c) in self.avg.iter_mut().zip(&buf[..N / 2]) { *a = 0.7 * *a + 0.3 * c.norm_sqr(); }
        let bin = |hz: f64| (hz * N as f64 / self.fs).round() as usize;
        let (lo, hi) = (bin(LO_HZ), bin(HI_HZ).min(N / 2 - 4));
        let mut band: Vec<f32> = self.avg[lo..hi].to_vec();
        band.sort_by(|a, b| a.total_cmp(b));
        let noise = band[band.len() / 2].max(1e-12);
        // Spitzen: höher als die Nachbarn im Abstand von ±3 Bins (±35 Hz) und deutlich über dem Rauschen
        let mut peaks: Vec<(f32, f64)> = vec![];
        for k in lo + 3..hi - 3 {
            let a = self.avg[k];
            if a < PEAK_OVER * noise || (k - 3..=k + 3).any(|j| j != k && self.avg[j] > a) { continue; }
            // Feinlage per Parabel
            let (l, r) = (self.avg[k - 1], self.avg[k + 1]);
            let d = 0.5 * (l - r) / (l - 2.0 * a + r).min(-1e-12);
            peaks.push((a, (k as f64 + d as f64) * self.fs / N as f64));
        }
        peaks.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (_, hz) in peaks {
            if let Some(c) = self.chans.iter_mut().find(|c| (c.pitch - hz).abs() < 30.0) { c.last_seen = self.t; continue; }
            if self.chans.len() >= MAX_CHANS { continue; }
            let p = cw::Params { pitch: (hz / 5.0).round() * 5.0, squelch: self.squelch };
            self.chans.push(Chan { pitch: p.pitch, dec: cw::Decoder::new(self.fs, p), text: String::new(), last_seen: self.t });
        }
        let keep = (20.0 * self.fs) as u64;
        let t = self.t;
        self.chans.retain(|c| t - c.last_seen < keep);
    }
}

/// Rufzeichen des Senders: das Wort nach dem letzten „DE“, sonst das letzte Wort, das wie ein Rufzeichen aussieht
pub fn last_call(t: &str) -> Option<String> {
    let clean = |w: &str| w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '/').to_string();
    let w: Vec<&str> = t.split_whitespace().collect();
    (0..w.len().saturating_sub(1)).rev().find(|&i| w[i] == "DE" && crate::rtty_ui::is_call(w[i + 1])).map(|i| clean(w[i + 1]))
        .or_else(|| w.iter().rev().find(|x| crate::rtty_ui::is_call(x)).map(|x| clean(x)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cw::tests::keyed;

    #[test]
    fn vier_signale_gleichzeitig() {
        let fs = 48000.0;
        let sig = [
            ("CQ CQ DE DL1ABC DL1ABC K", 18.0, 450.0),
            ("CQ TEST OE3XYZ OE3XYZ TEST", 28.0, 700.0),
            ("DK0TST DE G4ABC 599 TU", 22.0, 980.0),
            ("QRZ? DE F5QQQ K", 32.0, 1400.0),
        ];
        let len = 48000 * 22;
        let mut mix = vec![0.0f32; len];
        let mut r = 7u32;
        for (k, (t, w, p)) in sig.iter().enumerate() {
            // Vorlauf, damit sich Tempo und Kanal einstellen; leicht unterschiedliche Lautstärken
            let x = keyed(&format!("VVV VVV {t} {t}"), *w, *p, fs, 0.05, k as u32 + 1);
            for (i, v) in x.iter().enumerate().take(len) { mix[i] += v * [1.0, 0.6, 0.8, 0.5][k]; }
        }
        for v in mix.iter_mut() { r = r.wrapping_mul(1103515245).wrapping_add(12345); *v += ((r >> 8) as f32 / 8388608.0 - 1.0) * 0.05; }
        let mut s = Skimmer::new(fs, 10.0);
        let mut last = vec![];
        for c in mix.chunks(960) { if let Some(v) = s.push(c) { last = v; } }
        for i in &last { println!("{:6.0} Hz {:4.1} WpM {:?} {:?}", i.pitch, i.wpm, i.call, i.text); }
        assert_eq!(last.len(), 4, "genau vier Kanäle");
        let calls: Vec<Option<String>> = last.iter().map(|i| i.call.clone()).collect();
        for c in ["DL1ABC", "OE3XYZ", "G4ABC", "F5QQQ"] {
            assert!(calls.iter().any(|x| x.as_deref() == Some(c)) || last.iter().any(|i| i.text.contains(c)), "{c} fehlt");
        }
        for (i, (_, w, p)) in last.iter().zip(sig.iter().map(|s| (s.0, s.1, s.2))) {
            assert!((i.pitch - p).abs() < 15.0 && (i.wpm as f64 - w).abs() < w * 0.15, "{} Hz {} WpM", i.pitch, i.wpm);
        }
    }

    #[test]
    fn rufzeichen_des_senders() {
        assert_eq!(last_call("DK0TST DE G4ABC 599 TU DK0TST"), Some("G4ABC".into()));
        assert_eq!(last_call("CQ CQ OE3XYZ TEST"), Some("OE3XYZ".into()));
        assert_eq!(last_call("599 TU"), None);
    }

    #[test]
    fn nur_rauschen_keine_kanaele() {
        let mut s = Skimmer::new(48000.0, 10.0);
        let mut r = 3u32;
        let x: Vec<f32> = (0..48000 * 10).map(|_| { r = r.wrapping_mul(1103515245).wrapping_add(12345); ((r >> 8) as f32 / 8388608.0 - 1.0) * 0.2 }).collect();
        let mut last = vec![];
        for c in x.chunks(960) { if let Some(v) = s.push(c) { last = v; } }
        assert!(last.iter().all(|i| i.text.trim().is_empty()), "{last:?}");
    }
}
