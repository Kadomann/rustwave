//! FSKID wie MMSSTV: Rufzeichen nach dem Bild als FSK mit 45,45 Baud (22 ms je Bit), 1 = 1900 Hz, 0 = 2100 Hz.
//! Ablauf (MMSSTV fskid.txt): 1500 Hz 300 ms, 2100 Hz 100 ms, Startbit 1900 Hz 22 ms, dann 6-Bit-Symbole
//! (niederwertiges Bit zuerst, wie im MMSSTV-Quelltext und pySSTV): $2A, Zeichen − $20, $01, XOR-Prüfsumme.

use std::f64::consts::TAU;

const BIT_S: f64 = 0.022;
const F1: f64 = 1900.0;
const F0: f64 = 2100.0;

fn tone(out: &mut Vec<f32>, ph: &mut f64, f: f64, secs: f64, fs: f64, amp: f32) {
    let n = (secs * fs).round() as usize;
    for _ in 0..n {
        *ph = (*ph + TAU * f / fs) % TAU;
        out.push(amp * ph.sin() as f32);
    }
}

/// Rufzeichen (nur Zeichen $20–$5F) als FSKID-Audio
pub fn encode(call: &str, fs: f64, amp: f32) -> Vec<f32> {
    let codes: Vec<u8> = call.to_uppercase().bytes().filter(|b| (0x20..=0x5F).contains(b)).map(|b| b - 0x20).collect();
    let xsum = codes.iter().fold(0u8, |a, c| a ^ c);
    let mut syms = vec![0x2A];
    syms.extend(&codes);
    syms.extend([0x01, xsum]);
    let (mut out, mut ph) = (vec![], 0.0);
    tone(&mut out, &mut ph, 1500.0, 0.3, fs, amp);
    tone(&mut out, &mut ph, F0, 0.1, fs, amp);
    tone(&mut out, &mut ph, F1, BIT_S, fs, amp);
    for s in syms {
        for k in 0..6 { tone(&mut out, &mut ph, if (s >> k) & 1 == 1 { F1 } else { F0 }, BIT_S, fs, amp); }
    }
    out
}

enum State { Idle { guard_ms: u32 }, Bits { t: u32, bits: Vec<u8>, level: f64 } }

/// Erkennt FSKIDs im laufenden Empfang; liefert das Rufzeichen
pub struct Decoder {
    fs: f64,
    n: usize,
    ph: [f64; 2],
    buf: Vec<[f64; 4]>,
    sums: [f64; 4],
    pos: usize,
    step: f64,
    acc: f64,
    ms: u32,
    state: State,
    /// Pegel während des 2100-Hz-Vorlaufs
    guard_level: f64,
}

impl Decoder {
    pub fn new(fs: f64) -> Self {
        let n = (fs * BIT_S).round() as usize;
        Decoder { fs, n, ph: [0.0; 2], buf: vec![[0.0; 4]; n], sums: [0.0; 4], pos: 0, step: fs / 1000.0, acc: 0.0, ms: 0,
            state: State::Idle { guard_ms: 0 }, guard_level: 0.0 }
    }

    pub fn push(&mut self, x: &[f32]) -> Option<String> {
        let mut found = None;
        let w = [TAU * F1 / self.fs, TAU * F0 / self.fs];
        for &v in x {
            for k in 0..2 { self.ph[k] = (self.ph[k] + w[k]) % TAU; }
            let v = v as f64;
            let cur = [v * self.ph[0].cos(), v * self.ph[0].sin(), v * self.ph[1].cos(), v * self.ph[1].sin()];
            let old = std::mem::replace(&mut self.buf[self.pos], cur);
            for k in 0..4 { self.sums[k] += cur[k] - old[k]; }
            self.pos = (self.pos + 1) % self.n;
            self.acc += 1.0;
            if self.acc >= self.step {
                self.acc -= self.step;
                let m1 = self.sums[0].hypot(self.sums[1]);
                let m0 = self.sums[2].hypot(self.sums[3]);
                if let Some(c) = self.ms_step(m1, m0) { found = Some(c); }
            }
        }
        found
    }

    /// ein Schritt im 1-ms-Raster; das Fenster (22 ms) endet jeweils am aktuellen Zeitpunkt
    fn ms_step(&mut self, m1: f64, m0: f64) -> Option<String> {
        self.ms += 1;
        match &mut self.state {
            State::Idle { guard_ms } => {
                // 2100-Hz-Vorlauf: deutlich stärker als 1900 Hz
                if m0 > 3.0 * m1 && m0 > 1e-3 * self.n as f64 {
                    *guard_ms += 1;
                    self.guard_level = 0.9 * self.guard_level + 0.1 * m0;
                } else if *guard_ms >= 60 && m1 > m0 {
                    // Wechsel zum Startbit: die Fenstermitte liegt jetzt ungefähr am Bitanfang + 11 ms
                    self.state = State::Bits { t: 0, bits: vec![], level: self.guard_level };
                } else if m1 > m0 || m0 + m1 < 0.2 * self.guard_level {
                    *guard_ms = 0; // 1900 Hz ohne Vorlauf oder Stille
                } // dazwischen (Übergang im Fenster): abwarten
                None
            }
            State::Bits { t, bits, level } => {
                *t += 1;
                // Bit j (Startbit = −1) ist ganz im Fenster bei t = 11 + 22·(j+1)
                if *t >= 11 && (*t - 11) % 22 == 0 && *t > 11 {
                    let present = m0 + m1 > 0.3 * *level;
                    if !present { return self.finish(false); }
                    if let State::Bits { bits, .. } = &mut self.state { bits.push((m1 > m0) as u8); }
                    return self.check();
                }
                let _ = bits;
                None
            }
        }
    }

    /// nach jedem Bit: vollständige Symbole auswerten
    fn check(&mut self) -> Option<String> {
        let State::Bits { bits, .. } = &self.state else { return None };
        if bits.len() % 6 != 0 { return None; }
        let syms: Vec<u8> = bits.chunks(6).map(|c| c.iter().enumerate().fold(0, |a, (k, b)| a | (b << k))).collect();
        if syms[0] != 0x2A || syms.len() > 24 { self.state = State::Idle { guard_ms: 0 }; return None; }
        // nach $01 kommt noch die Prüfsumme
        if let Some(end) = syms.iter().position(|&s| s == 0x01) {
            if syms.len() == end + 2 { return self.finish(true); }
        }
        None
    }

    /// Ende: Rufzeichen prüfen (mit Prüfsumme, wenn sie gesendet wurde)
    fn finish(&mut self, with_sum: bool) -> Option<String> {
        let st = std::mem::replace(&mut self.state, State::Idle { guard_ms: 0 });
        let State::Bits { bits, .. } = st else { return None };
        let syms: Vec<u8> = bits.chunks_exact(6).map(|c| c.iter().enumerate().fold(0, |a, (k, b)| a | (b << k))).collect();
        let end = syms.iter().position(|&s| s == 0x01)?;
        if syms.first() != Some(&0x2A) || end < 4 { return None; }
        let codes = &syms[1..end];
        if with_sum && codes.iter().fold(0u8, |a, c| a ^ c) != syms[end + 1] { return None; }
        let call: String = codes.iter().map(|c| (c + 0x20) as char).collect();
        // Rufzeichen: Buchstaben, Ziffern, Schrägstrich, Bindestrich (SSID)
        (call.chars().all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '-') && call.chars().any(|c| c.is_ascii_digit())).then_some(call)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(x: &[f32], fs: f64) -> Option<String> {
        let mut d = Decoder::new(fs);
        let mut r = None;
        let tail = vec![0.0f32; (fs * 0.3) as usize]; // Nachlauf wie im laufenden Empfang
        for c in x.chunks(700).chain([tail.as_slice()]) { if let Some(s) = d.push(c) { r = Some(s); } }
        r
    }

    #[test]
    fn hin_und_zurueck() {
        for fs in [48000.0, 44100.0, 11025.0] {
            let mut x = vec![0.0f32; 4000];
            x.extend(encode("dl1abc/p", fs, 0.5));
            x.extend(vec![0.0; 4000]);
            assert_eq!(run(&x, fs).as_deref(), Some("DL1ABC/P"), "fs {fs}");
        }
    }

    #[test]
    fn mit_rauschen_und_ohne_falschmeldung() {
        let fs = 48000.0;
        let mut r = 1u32;
        let mut noise = |s: f64| { r = r.wrapping_mul(1103515245).wrapping_add(12345); ((r >> 8) as f64 / 8388608.0 - 1.0) * s };
        let mut x = vec![0.0f32; 4800];
        x.extend(encode("DK0TST", fs, 0.4));
        x.extend(vec![0.0; 4800]);
        let y: Vec<f32> = x.iter().map(|&v| (v as f64 + noise(0.25)) as f32).collect();
        assert_eq!(run(&y, fs).as_deref(), Some("DK0TST"));
        // reines Rauschen und ein Dauerton 2100 Hz: nichts
        let z: Vec<f32> = (0..48000 * 20).map(|_| noise(0.3) as f32).collect();
        assert_eq!(run(&z, fs), None);
        let mut t = vec![];
        tone(&mut t, &mut 0.0, 2100.0, 3.0, fs, 0.5);
        assert_eq!(run(&t, fs), None);
    }

    /// Gegenprobe mit pySSTV (unabhängiger Kodierer, sendet ohne Prüfsumme):
    /// FSKID_WAV=datei.wav cargo test --release fskid_wav -- --ignored --nocapture
    #[test]
    #[ignore]
    fn fskid_wav() {
        let p = std::env::var("FSKID_WAV").unwrap();
        let mut r = hound::WavReader::open(&p).unwrap();
        let fs = r.spec().sample_rate as f64;
        let x: Vec<f32> = r.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();
        println!("FSKID: {:?}", run(&x, fs));
    }
}
