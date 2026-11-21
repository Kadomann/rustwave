//! Morse-Dekoder (CW): Ton bei `pitch` per I/Q-Mischung und Tiefpass (≈100 Hz Bandbreite), Hüllkurve im
//! 1-ms-Raster, Schwelle zwischen Rauschboden und Spitze, Tempo aus den Längen von Punkten und Strichen.

use crate::sstv::Biquad;
use std::collections::VecDeque;
use std::f64::consts::TAU;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Params {
    /// Tonhöhe des Signals im Audio (Hz) – bei Icom im CW-Betrieb standardmäßig 600 Hz
    pub pitch: f64,
    /// Rauschsperre: Mindestabstand Signal/Rauschen in dB
    pub squelch: f64,
}
impl Default for Params { fn default() -> Self { Params { pitch: 600.0, squelch: 10.0 } } }

const TABLE: &[(&str, &str)] = &[
    (".-", "A"), ("-...", "B"), ("-.-.", "C"), ("-..", "D"), (".", "E"), ("..-.", "F"), ("--.", "G"), ("....", "H"),
    ("..", "I"), (".---", "J"), ("-.-", "K"), (".-..", "L"), ("--", "M"), ("-.", "N"), ("---", "O"), (".--.", "P"),
    ("--.-", "Q"), (".-.", "R"), ("...", "S"), ("-", "T"), ("..-", "U"), ("...-", "V"), (".--", "W"), ("-..-", "X"),
    ("-.--", "Y"), ("--..", "Z"), ("-----", "0"), (".----", "1"), ("..---", "2"), ("...--", "3"), ("....-", "4"),
    (".....", "5"), ("-....", "6"), ("--...", "7"), ("---..", "8"), ("----.", "9"), (".-.-.-", "."), ("--..--", ","),
    ("..--..", "?"), ("-..-.", "/"), ("-...-", "="), (".-.-.", "+"), ("-....-", "-"), ("---...", ":"), (".----.", "'"),
    ("-.--.", "<KN>"), ("...-.-", "<SK>"), ("...-.", "<SN>"), (".-...", "<AS>"), ("........", "<HH>"), ("-.-.-", "<KA>"),
    (".--.-.", "@"), ("-.-.--", "!"), ("-.--.-", ")"), (".-..-.", "\""), ("..--.-", "_"), ("-.-.-.", ";"),
];
pub fn decode_symbol(s: &str) -> Option<&'static str> { TABLE.iter().find(|(c, _)| *c == s).map(|(_, t)| *t) }

/// Text für die CW-Ausgabe des Funkgeräts: Großbuchstaben, Umlaute umschreiben, nur Zeichen,
/// die Icom per CI-V senden kann (A–Z, 0–9, / ? . - , : ' ( ) = + " @, „^“ verbindet Zeichen zu Betriebszeichen)
pub fn sanitize(t: &str) -> String {
    let mut out = String::new();
    for c in t.to_uppercase().chars() {
        match c {
            'Ä' => out.push_str("AE"), 'Ö' => out.push_str("OE"), 'Ü' => out.push_str("UE"), 'ß' => out.push_str("SS"),
            'A'..='Z' | '0'..='9' | '/' | '?' | '.' | '-' | ',' | ':' | '\'' | '(' | ')' | '=' | '+' | '"' | '@' | '^' => out.push(c),
            _ if c.is_whitespace() => if !out.ends_with(' ') && !out.is_empty() { out.push(' ') },
            _ => {}
        }
    }
    out.trim_end().to_string()
}
/// in Stücke von höchstens `max` Zeichen teilen, möglichst an Wortgrenzen (die Pause zwischen zwei Stücken fällt dann in eine Wortpause)
pub fn chunks(t: &str, max: usize) -> Vec<String> {
    let mut v: Vec<String> = vec![];
    for w in t.split(' ').filter(|w| !w.is_empty()) {
        let mut w = w.to_string();
        while w.len() > max { let rest = w.split_off(max); v.push(std::mem::replace(&mut w, rest)); }
        match v.last_mut() {
            Some(l) if l.len() + 1 + w.len() <= max => { l.push(' '); l.push_str(&w); }
            _ => v.push(w),
        }
    }
    v
}
/// Sendedauer in Sekunden (PARIS-Zeitmaß: Punkt 1, Strich 3, Pause im Zeichen 1, zwischen Zeichen 3, zwischen Wörtern 7)
pub fn duration_s(t: &str, wpm: f64) -> f64 {
    let mut units = 0.0;
    for (wi, w) in t.split(' ').filter(|w| !w.is_empty()).enumerate() {
        if wi > 0 { units += 7.0; }
        let mut first = true;
        let mut join = false;
        for c in w.chars() {
            if c == '^' { join = true; continue; }
            let Some(code) = TABLE.iter().find(|(_, t)| t.len() == 1 && t.starts_with(c)).map(|(c, _)| *c) else { continue };
            if !first { units += if join { 1.0 } else { 3.0 }; }
            first = false;
            units += code.chars().map(|e| if e == '-' { 3.0 } else { 1.0 }).sum::<f64>() + (code.len() - 1) as f64;
        }
    }
    units * 1.2 / wpm
}

pub struct Decoder {
    pub p: Params,
    fs: f64,
    ph: f64,
    lp: [Biquad; 4],
    step: f64,
    t: f64,
    peak: f64,
    hold: f64,
    noise: f64,
    on: bool,
    /// Dauer des aktuellen Zustands (ms) und Zähler für die Entprellung
    dur: f64,
    cand: f64,
    dit: f64,
    marks: VecDeque<f64>,
    sym: String,
    word_open: bool,
    warm: u32,
    /// geschätztes Tempo (Wörter pro Minute, PARIS) und Signal/Rauschen in dB
    pub wpm: f32,
    pub snr_db: f32,
}

impl Decoder {
    pub fn new(fs: f64, p: Params) -> Self {
        let lp = || Biquad::lowpass(fs, 50.0, 0.7071);
        Decoder {
            p, fs, ph: 0.0, lp: [lp(), lp(), lp(), lp()], step: fs / 1000.0, t: 0.0,
            peak: 1e-9, hold: 1e-9, noise: 1e-9, on: false, dur: 0.0, cand: 0.0, dit: 60.0, marks: VecDeque::new(),
            sym: String::new(), word_open: false, warm: 0, wpm: 20.0, snr_db: 0.0,
        }
    }

    /// Audio verarbeiten, neu erkannten Text zurückgeben
    pub fn push(&mut self, x: &[f32]) -> String {
        let mut out = String::new();
        let w = TAU * self.p.pitch / self.fs;
        for &v in x {
            self.ph = (self.ph + w) % TAU;
            let i = self.lp[0].run(v as f64 * self.ph.cos());
            let i = self.lp[1].run(i);
            let q = self.lp[2].run(v as f64 * self.ph.sin());
            let q = self.lp[3].run(q);
            self.t += 1.0;
            if self.t >= self.step { self.t -= self.step; self.ms(i.hypot(q), &mut out); }
        }
        out
    }

    /// ein Schritt im 1-ms-Raster
    fn ms(&mut self, env: f64, out: &mut String) {
        // Rauschboden folgt schnell nach unten, langsam nach oben; Spitze schnell nach oben, langsam nach unten
        // Rauschboden = mittlere Hüllkurve in den Pausen (nicht während eines Tons); die ersten 300 ms nur einmessen
        self.warm += 1;
        if self.warm < 300 { self.noise += (env - self.noise) * 0.05; self.peak = self.noise; self.hold = self.noise; return; }
        if !self.on && self.cand == 0.0 { self.noise += (env - self.noise) * 0.004; }
        // Spitze fällt in etwa 8 Punktlängen ab: folgt Schwund (QSB), hält aber über Wortpausen
        self.peak += (env - self.peak) * if env > self.peak { 0.3 } else { 1.0 / (8.0 * self.dit).clamp(200.0, 1500.0) };
        self.peak = self.peak.max(self.noise);
        // für die Rauschsperre eine langsamer fallende Spitze: kurze Schwundlöcher schließen die Sperre nicht
        self.hold += (env - self.hold) * if env > self.hold { 0.3 } else { 0.0007 };
        self.hold = self.hold.max(self.peak);
        // Schwelle nicht beliebig tief: sonst löst in langen Pausen das Rauschen aus
        self.peak = self.peak.max(self.noise + 0.25 * (self.hold - self.noise));
        self.snr_db = (20.0 * (self.hold / self.noise.max(1e-12)).log10()) as f32;
        let active = self.snr_db as f64 >= self.p.squelch;
        let span = self.peak - self.noise;
        let level = if self.on { self.noise + 0.35 * span } else { self.noise + 0.5 * span };
        let raw = active && env > level;

        self.dur += 1.0;
        // Entprellen: ein Zustandswechsel muss eine Weile anhalten
        let deb = (self.dit * 0.3).clamp(4.0, 15.0);
        if raw != self.on {
            self.cand += 1.0;
            if self.cand >= deb {
                let len = self.dur - self.cand;
                        if self.on { self.mark(len); }
                self.on = raw;
                self.dur = self.cand;
                self.cand = 0.0;
            }
        } else { self.cand = 0.0; }

        if !self.on {
            if self.dur > 2.0 * self.dit && !self.sym.is_empty() {
                out.push_str(decode_symbol(&self.sym).unwrap_or("*"));
                self.sym.clear();
                self.word_open = true;
            }
            if self.dur > 5.0 * self.dit && self.word_open && self.sym.is_empty() {
                out.push(' ');
                self.word_open = false;
            }
        }
    }

    /// Ende eines Tons der Länge `len` ms: Punkt oder Strich, Tempo nachführen
    fn mark(&mut self, len: f64) {
        if len < 0.25 * self.dit { return; } // Störspitze
        self.marks.push_back(len);
        if self.marks.len() > 24 { self.marks.pop_front(); }
        self.estimate();
        let dash = len > 2.0 * self.dit;
        if self.sym.len() < 9 { self.sym.push(if dash { '-' } else { '.' }); }
    }

    /// Punktlänge aus den letzten Tönen: in zwei Gruppen teilen (k-Means); liegen die Mittel mindestens
    /// Faktor 2 auseinander, sind es Punkte und Striche (Strich = 3 Punkte)
    fn estimate(&mut self) {
        let m: Vec<f64> = self.marks.iter().copied().collect();
        // Start bei den Quantilen 15 % / 85 %: einzelne Ausreißer bilden keine eigene Gruppe
        let mut srt = m.clone();
        srt.sort_by(|x, y| x.total_cmp(y));
        let q = |f: f64| srt[((srt.len() - 1) as f64 * f).round() as usize];
        let (mut a, mut b) = (q(0.15), q(0.85));
        let (mut na, mut nb) = (0, 0);
        for _ in 0..10 {
            let mid = (a * b).sqrt();
            let (s, l): (Vec<f64>, Vec<f64>) = m.iter().partition(|&&v| v < mid);
            if s.is_empty() || l.is_empty() { break; }
            (na, nb) = (s.len(), l.len());
            a = s.iter().sum::<f64>() / na as f64;
            b = l.iter().sum::<f64>() / nb as f64;
        }
        let est = if na > 0 && nb > 0 && b / a >= 2.0 {
            (a * na as f64 + b / 3.0 * nb as f64) / (na + nb) as f64
        } else {
            // nur eine Längenklasse: an der bisherigen Schätzung messen, ob es Punkte oder Striche sind
            let avg = m.iter().sum::<f64>() / m.len() as f64;
            if avg > 2.0 * self.dit { avg / 3.0 } else { avg }
        };
        self.dit = (0.5 * self.dit + 0.5 * est).clamp(20.0, 240.0);
        self.wpm = (1200.0 / self.dit) as f32;
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn encode_char(c: char) -> Option<&'static str> {
        let u = c.to_ascii_uppercase().to_string();
        TABLE.iter().find(|(_, t)| *t == u).map(|(c, _)| *c)
    }

    /// CW-Signal: weiche Flanken (5 ms), Hand-Tastung als Zufallsabweichung der Längen
    pub fn keyed(text: &str, wpm: f64, pitch: f64, fs: f64, jitter: f64, seed: u32) -> Vec<f32> {
        let dit = 1.2 / wpm;
        let mut rng = seed;
        let mut rnd = || { rng = rng.wrapping_mul(1103515245).wrapping_add(12345); (rng >> 8) as f64 / 16777216.0 * 2.0 - 1.0 };
        let mut seg: Vec<(bool, f64)> = vec![(false, 0.3)];
        for word in text.split(' ') {
            for (ci, c) in word.chars().enumerate() {
                if ci > 0 { seg.push((false, 3.0 * dit)); }
                for (k, s) in encode_char(c).unwrap().chars().enumerate() {
                    if k > 0 { seg.push((false, dit * (1.0 + jitter * rnd()))); }
                    seg.push((true, if s == '-' { 3.0 } else { 1.0 } * dit * (1.0 + jitter * rnd())));
                }
            }
            seg.push((false, 7.0 * dit));
        }
        seg.push((false, 0.5));
        let mut out = vec![];
        let ramp = 0.005 * fs;
        let mut n = 0usize;
        for (on, d) in seg {
            let len = (d * fs) as usize;
            for k in 0..len {
                let env = if on { ((k as f64 / ramp).min(1.0)).min((len - k) as f64 / ramp).min(1.0) } else { 0.0 };
                out.push((0.5 * env * (TAU * pitch * n as f64 / fs).sin()) as f32);
                n += 1;
            }
        }
        out
    }
    fn noisy(x: &[f32], sigma: f64, seed: u32) -> Vec<f32> {
        let mut r = seed;
        let mut u = || { r = r.wrapping_mul(1103515245).wrapping_add(12345); (r as f64 + 1.0) / 4294967297.0 };
        x.iter().map(|&v| (v as f64 + sigma * (-2.0 * u().ln()).sqrt() * (TAU * u()).cos()) as f32).collect()
    }
    fn run(x: &[f32], fs: f64, p: Params) -> (String, f32) {
        let mut d = Decoder::new(fs, p);
        let t: String = x.chunks(960).map(|c| d.push(c)).collect();
        (t.trim().to_string(), d.wpm)
    }
    const TXT: &str = "CQ CQ DE DL1ABC DL1ABC PSE K 599 TU 73";

    /// Vergleich mit anderen Dekodern: CW_WAV=datei.wav cargo test --release cw_wav -- --ignored --nocapture
    #[test]
    #[ignore]
    fn cw_wav() {
        let p = std::env::var("CW_WAV").unwrap();
        let mut r = hound::WavReader::open(&p).unwrap();
        let fs = r.spec().sample_rate as f64;
        let x: Vec<f32> = r.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();
        let pitch = std::env::var("CW_PITCH").ok().and_then(|v| v.parse().ok()).unwrap_or(600.0);
        let (t, w) = run(&x, fs, Params { pitch, ..Params::default() });
        println!("TEXT: {t}\nWPM: {w:.1}");
    }

    #[test]
    fn senden_vorbereiten() {
        assert_eq!(sanitize("cq de dk0tst, grüße\nAus Köln & mehr ^sk"), "CQ DE DK0TST, GRUESSE AUS KOELN MEHR ^SK");
        let c = chunks("CQ CQ CQ DE DK0TST DK0TST DK0TST PSE K", 30);
        assert_eq!(c, vec!["CQ CQ CQ DE DK0TST DK0TST", "DK0TST PSE K"]);
        assert!(chunks(&"A".repeat(70), 30).iter().all(|c| c.len() <= 30));
        // PARIS = 50 Einheiten ohne Wortpause am Ende: 43 + (bei 20 WpM 60 ms je Einheit)
        assert!((duration_s("PARIS", 20.0) - 43.0 * 0.06).abs() < 1e-9);
        assert!((duration_s("PARIS PARIS", 20.0) - (43.0 * 2.0 + 7.0) * 0.06).abs() < 1e-9);
        // ^SK: S und K ohne Zeichenpause
        assert!((duration_s("^SK", 20.0) - (5.0 + 1.0 + 9.0) * 0.06).abs() < 1e-9);
    }

    #[test]
    fn tabelle() {
        assert_eq!(decode_symbol("...-.-"), Some("<SK>"));
        assert_eq!(encode_char('q'), Some("--.-"));
        for (c, _) in TABLE { assert_eq!(TABLE.iter().filter(|(d, _)| d == c).count(), 1, "doppelt: {c}"); }
    }

    #[test]
    fn tempi_und_tonablage() {
        let fs = 48000.0;
        for (wpm, off) in [(12.0, 0.0), (20.0, 30.0), (28.0, -30.0), (40.0, 0.0)] {
            // Vorlauf, damit sich das Tempo einpendelt
            let x = keyed(&format!("VVV {TXT}"), wpm, 600.0 + off, fs, 0.0, 1);
            let (t, w) = run(&noisy(&x, 0.05, 2), fs, Params::default());
            println!("{wpm} WpM, Ablage {off} Hz: {t:?} ({w:.1} WpM)");
            assert!(t.ends_with(TXT), "{t}");
            assert!((w as f64 - wpm).abs() < wpm * 0.12);
        }
    }

    #[test]
    fn tempowechsel_zwischen_stationen() {
        let fs = 48000.0;
        let mut x = keyed(&format!("VVV {TXT}"), 14.0, 600.0, fs, 0.05, 9);
        x.extend(keyed(TXT, 32.0, 600.0, fs, 0.0, 10));
        let (t, w) = run(&noisy(&x, 0.05, 11), fs, Params::default());
        // nach dem Tempowechsel darf das erste Wort verloren gehen, dann muss es wieder stimmen
        assert!(t.starts_with(&format!("VVV {TXT}")) && t.ends_with(&TXT[6..]), "{t}");
        assert!((w - 32.0).abs() < 4.0, "{w}");
    }

    #[test]
    fn handtastung_und_rauschen() {
        let fs = 48000.0;
        let x = keyed(&format!("VVV {TXT}"), 18.0, 600.0, fs, 0.15, 3);
        let (t, _) = run(&noisy(&x, 0.35, 4), fs, Params::default());
        println!("{t:?}");
        assert!(t.ends_with(TXT), "{t}");
    }

    #[test]
    fn nachbarsignal_und_reines_rauschen() {
        let fs = 48000.0;
        let a = keyed(&format!("VVV {TXT}"), 22.0, 600.0, fs, 0.0, 5);
        let b = keyed("TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST", 30.0, 850.0, fs, 0.0, 6);
        let mix: Vec<f32> = (0..a.len()).map(|i| a[i] + 1.5 * b.get(i).copied().unwrap_or(0.0)).collect();
        let (t, _) = run(&noisy(&mix, 0.05, 7), fs, Params::default());
        assert!(t.ends_with(TXT), "{t}");
        // nur Rauschen: nichts ausgeben
        let (t, _) = run(&noisy(&vec![0.0; 48000 * 20], 0.3, 8), fs, Params::default());
        assert!(t.is_empty(), "{t:?}");
    }
}
