//! RTTY: Baudot/ITA2 (US-Ziffernbelegung wie fldigi), AFSK-Kodierer und Dekoder.
//! Konvention für LSB: Mark = tiefere Audiofrequenz (2125 Hz), Space = Mark + Shift.

use crate::sstv::Biquad;
use std::collections::VecDeque;
use std::f64::consts::TAU;

const LTRS: [char; 32] = ['\0', 'E', '\n', 'A', ' ', 'S', 'I', 'U', '\r', 'D', 'R', 'J', 'N', 'F', 'C', 'K',
    'T', 'Z', 'L', 'W', 'H', 'Y', 'P', 'Q', 'O', 'B', 'G', '\0', 'M', 'X', 'V', '\0'];
const FIGS: [char; 32] = ['\0', '3', '\n', '-', ' ', '\x07', '8', '7', '\r', '$', '4', '\'', ',', '!', ':', '(',
    '5', '"', ')', '2', '#', '6', '0', '1', '9', '?', '&', '\0', '.', '/', ';', '\0'];
const C_FIGS: u8 = 0x1B;
const C_LTRS: u8 = 0x1F;
const C_SPACE: u8 = 0x04;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Params {
    /// Mitte zwischen Mark und Space (Audio, Hz)
    pub center: f64,
    pub shift: f64,
    pub baud: f64,
    /// Mark/Space vertauscht (z. B. bei Betrieb in USB)
    pub reverse: bool,
    /// Rauschsperre 0..1: mittlere Trennschärfe, ab der Zeichen angezeigt werden
    pub squelch: f32,
}
impl Default for Params {
    fn default() -> Self { Params { center: 2210.0, shift: 170.0, baud: 45.45, reverse: false, squelch: 0.45 } }
}
impl Params {
    pub fn mark(&self) -> f64 { if self.reverse { self.center + self.shift / 2.0 } else { self.center - self.shift / 2.0 } }
    pub fn space(&self) -> f64 { if self.reverse { self.center - self.shift / 2.0 } else { self.center + self.shift / 2.0 } }
}

// ---------------- Baudot ----------------

/// Text -> Baudot-Codes mit automatischem Buchstaben/Ziffern-Umschalten.
/// Nach einem Leerzeichen gilt beim Empfänger wieder "Buchstaben" (Unshift on Space).
pub fn to_baudot(text: &str) -> Vec<u8> {
    let mut out = vec![];
    let mut figs: Option<bool> = None; // unbekannt -> erstes Zeichen erzwingt die Umschaltung
    for c in text.to_uppercase().chars() {
        match c {
            '\n' => { out.extend([0x08, 0x02]); continue; }
            '\r' => continue,
            ' ' => { out.push(C_SPACE); figs = Some(false); continue; }
            _ => {}
        }
        let (code, is_fig) = match LTRS.iter().position(|&l| l == c) {
            Some(i) if c != '\0' => (i as u8, false),
            _ => match FIGS.iter().position(|&f| f == c) {
                Some(i) if c != '\0' => (i as u8, true),
                _ => continue, // nicht darstellbar
            },
        };
        if figs != Some(is_fig) { out.push(if is_fig { C_FIGS } else { C_LTRS }); figs = Some(is_fig); }
        out.push(code);
    }
    out
}

/// Baudot-Zustand des Empfängers
#[derive(Default)]
pub struct BaudotRx { figs: bool }
impl BaudotRx {
    pub fn decode(&mut self, code: u8) -> Option<char> {
        match code {
            C_LTRS => { self.figs = false; None }
            C_FIGS => { self.figs = true; None }
            C_SPACE => { self.figs = false; Some(' ') }
            0x00 | 0x08 => None, // NUL, CR (Zeilenvorschub kommt mit LF)
            c => {
                let ch = if self.figs { FIGS[c as usize] } else { LTRS[c as usize] };
                (ch != '\0' && ch != '\x07').then_some(ch)
            }
        }
    }
}

// ---------------- Kodierer ----------------

/// Text als AFSK-Audio (mono, volle Aussteuerung): kurzer Vorlauf, CR LF, Text, CR LF, Nachlauf
pub fn encode(text: &str, p: &Params, fs: f64) -> Vec<f32> {
    let mut codes = vec![C_LTRS, C_LTRS, 0x08, 0x02];
    codes.extend(to_baudot(text));
    codes.extend([0x08, 0x02, C_LTRS]);
    let sp = fs / p.baud;
    // Bitfolge: (Mark?, Dauer in Bits)
    let mut bits: Vec<(bool, f64)> = vec![(true, 0.3 * p.baud)]; // 0,3 s Mark als Vorlauf
    for c in codes {
        bits.push((false, 1.0));
        for k in 0..5 { bits.push(((c >> k) & 1 == 1, 1.0)); }
        bits.push((true, 1.5));
    }
    bits.push((true, 0.15 * p.baud));
    let (fm, fsp) = (p.mark(), p.space());
    let total: f64 = bits.iter().map(|b| b.1).sum();
    let mut out = Vec::with_capacity((total * sp) as usize + 1);
    let (mut t, mut ph) = (0.0f64, 0.0f64);
    for (mark, dur) in bits {
        let end = ((t + dur) * sp).round() as usize;
        let f = if mark { fm } else { fsp };
        while out.len() < end {
            ph += TAU * f / fs;
            if ph > TAU { ph -= TAU; }
            out.push(ph.sin() as f32);
        }
        t += dur;
    }
    out
}

// ---------------- Dekoder ----------------

/// Zwei Filter (Mark/Space), normierte Differenz d = (M - S)/(M + S), Start-Bit-Synchronisation,
/// Mittelung über die Bitmitte. Die mittlere |d| eines Zeichens dient als Rauschsperre.
pub struct Decoder {
    fs: f64,
    pub p: Params,
    ph: [f64; 2],
    lp: [Biquad; 8],
    hist: VecDeque<f32>,
    base: u64,
    n: u64,
    search: u64,
    start: Option<u64>,
    baudot: BaudotRx,
    /// zuletzt gemessene Signalgüte (0..1) für die Anzeige
    pub quality: f32,
}

impl Decoder {
    pub fn new(fs: f64, p: Params) -> Self {
        let bw = (p.baud * 0.9).max(20.0);
        let lp = std::array::from_fn(|i| Biquad::lowpass(fs, bw, if i % 2 == 0 { 0.5412 } else { 1.3066 }));
        Decoder { fs, p, ph: [0.0; 2], lp, hist: VecDeque::new(), base: 0, n: 0, search: 1, start: None, baudot: BaudotRx::default(), quality: 0.0 }
    }

    fn avg(&self, a: f64, b: f64) -> f32 {
        let (a, b) = ((a.round() as u64).max(self.base), (b.round() as u64).min(self.n));
        if b <= a { return 0.0; }
        let s: f32 = (a..b).map(|i| self.hist[(i - self.base) as usize]).sum();
        s / (b - a) as f32
    }

    /// Audio verarbeiten, dekodierten Text zurückgeben
    pub fn push(&mut self, x: &[f32]) -> String {
        let mut out = String::new();
        let (wm, ws) = (TAU * self.p.mark() / self.fs, TAU * self.p.space() / self.fs);
        for &v in x {
            let v = v as f64;
            self.ph[0] = (self.ph[0] + wm) % TAU;
            self.ph[1] = (self.ph[1] + ws) % TAU;
            let inp = [v * self.ph[0].cos(), v * self.ph[0].sin(), v * self.ph[1].cos(), v * self.ph[1].sin()];
            let [mi, mq, si, sq] = std::array::from_fn(|k| { let a = self.lp[2 * k].run(inp[k]); self.lp[2 * k + 1].run(a) });
            let (m, s) = ((mi * mi + mq * mq).sqrt(), (si * si + sq * sq).sqrt());
            self.hist.push_back(((m - s) / (m + s + 1e-9)) as f32);
            self.n += 1;
        }
        let sp = self.fs / self.p.baud;
        loop {
            match self.start {
                None => {
                    // Flanke Mark -> Space suchen
                    let from = self.search.max(self.base + 1);
                    let found = (from..self.n).find(|&i| {
                        let (a, b) = (self.hist[(i - 1 - self.base) as usize], self.hist[(i - self.base) as usize]);
                        a > 0.0 && b <= 0.0
                    });
                    match found { Some(i) => self.start = Some(i), None => { self.search = self.n; break; } }
                }
                Some(s) => {
                    let s0 = s as f64;
                    if (self.n as f64) < s0 + 7.0 * sp { break; }
                    let bit = |k: f64| self.avg(s0 + (k + 0.2) * sp, s0 + (k + 0.8) * sp);
                    let start_bit = bit(0.0);
                    // echtes Startbit: deutlich Space, und direkt davor deutlich Mark (Ruhelage/Stoppbit)
                    if start_bit > -0.15 || self.avg(s0 - 0.6 * sp, s0) < 0.15 { self.start = None; self.search = s + 1; continue; }
                    let data: Vec<f32> = (1..=5).map(|k| bit(k as f64)).collect();
                    let stop = self.avg(s0 + 6.1 * sp, s0 + 6.9 * sp);
                    if stop <= 0.0 { self.start = None; self.search = s + 1; continue; } // Rahmenfehler
                    let q = (start_bit.abs() + stop.abs() + data.iter().map(|d| d.abs()).sum::<f32>()) / 7.0;
                    self.quality = 0.8 * self.quality + 0.2 * q;
                    if q >= self.p.squelch {
                        let code = data.iter().enumerate().fold(0u8, |c, (k, d)| c | (((*d > 0.0) as u8) << k));
                        if let Some(ch) = self.baudot.decode(code) { out.push(ch); }
                    }
                    self.start = None;
                    self.search = (s0 + 6.5 * sp) as u64;
                }
            }
        }
        // Verlauf kürzen (einige Bits Reserve)
        let keep_from = self.start.unwrap_or(self.search).saturating_sub((2.0 * sp) as u64).max(self.base);
        while self.base < keep_from { self.hist.pop_front(); self.base += 1; }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_text(codes: &[u8]) -> String {
        let mut rx = BaudotRx::default();
        codes.iter().filter_map(|&c| rx.decode(c)).collect()
    }
    struct Rng(u32);
    impl Rng {
        fn u(&mut self) -> f64 { self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345); self.0 as f64 / 4294967296.0 }
        fn gauss(&mut self) -> f64 { (-2.0 * (self.u() + 1e-12).ln()).sqrt() * (TAU * self.u()).cos() }
    }
    const TEXT: &str = "CQ CQ DE DK0XY DK0XY PSE K\nUR RST 599 599 NAME ROMAN QTH BERLIN, LOC JO62QM.\n73 TU (14.085 MHZ) 1/2 ?";

    fn roundtrip(p: &Params, rx_p: &Params, noise: f64, fs: f64) -> String {
        let mut x = vec![0f32; fs as usize / 2];
        x.extend(encode(TEXT, p, fs).iter().map(|v| v * 0.5));
        x.extend(vec![0f32; fs as usize / 2]);
        let mut rng = Rng(3);
        for v in x.iter_mut() { *v += (noise * rng.gauss()) as f32; }
        let mut d = Decoder::new(fs, *rx_p);
        x.chunks(1000).map(|c| d.push(c)).collect()
    }
    /// Zeichenfehlerquote über Levenshtein-Distanz
    fn cer(want: &str, got: &str) -> f64 {
        let (a, b): (Vec<char>, Vec<char>) = (want.chars().collect(), got.trim().chars().collect());
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        for i in 1..=a.len() {
            let mut cur = vec![i; b.len() + 1];
            for j in 1..=b.len() {
                cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + (a[i - 1] != b[j - 1]) as usize);
            }
            prev = cur;
        }
        prev[b.len()] as f64 / a.len() as f64
    }

    #[test]
    fn baudot_mit_ziffern_und_unshift_on_space() {
        let codes = to_baudot("abc 123 x1 2,3?");
        assert_eq!(decode_text(&codes), "ABC 123 X1 2,3?");
        // nach Leerzeichen muss FIGS erneut gesendet werden ("123 456")
        let c = to_baudot("1 2");
        assert_eq!(c.iter().filter(|&&x| x == C_FIGS).count(), 2);
    }

    #[test]
    fn audio_roundtrip_sauber_und_mit_rauschen() {
        let p = Params::default();
        // sauber: exakt
        assert_eq!(roundtrip(&p, &p, 0.0, 48000.0).trim(), TEXT);
        // Rauschen σ=0.25 über 24 kHz (im 170-Hz-Bereich ca. +10 dB SNR): kaum Fehler
        let got = roundtrip(&p, &p, 0.25, 48000.0);
        println!("Rauschen 0.25: CER {:.3} {:?}", cer(TEXT, &got), got);
        assert!(cer(TEXT, &got) < 0.02);
        // 44,1 kHz Soundkarte
        assert_eq!(roundtrip(&p, &p, 0.0, 44100.0).trim(), TEXT);
    }

    #[test]
    fn verstimmung_umkehr_und_baudraten() {
        let p = Params::default();
        let off = Params { center: p.center + 15.0, ..p };
        let got = roundtrip(&p, &off, 0.05, 48000.0);
        println!("15 Hz verstimmt: CER {:.3}", cer(TEXT, &got));
        assert!(cer(TEXT, &got) < 0.02);
        let rev = Params { reverse: true, ..p };
        assert_eq!(roundtrip(&rev, &rev, 0.05, 48000.0).trim(), TEXT);
        assert!(cer(TEXT, &roundtrip(&p, &rev, 0.05, 48000.0)) > 0.5, "falsche Lage darf nicht dekodieren");
        for (baud, shift) in [(50.0, 170.0), (75.0, 170.0), (45.45, 425.0), (45.45, 850.0)] {
            let q = Params { baud, shift, ..p };
            assert_eq!(roundtrip(&q, &q, 0.05, 48000.0).trim(), TEXT, "{baud} Bd / {shift} Hz");
        }
    }

    #[test]
    fn rauschen_ergibt_kaum_zeichen() {
        let mut d = Decoder::new(48000.0, Params::default());
        let mut rng = Rng(11);
        let mut n = 0;
        for _ in 0..(60 * 48000 / 4800) {
            let x: Vec<f32> = (0..4800).map(|_| (0.3 * rng.gauss()) as f32).collect();
            n += d.push(&x).chars().filter(|c| !c.is_whitespace()).count();
        }
        println!("60 s Rauschen: {n} Zeichen");
        assert!(n < 15, "{n} Zeichen aus reinem Rauschen");
    }

    /// Vergleich mit anderen Programmen: schreibt Testsignale bzw. dekodiert eine WAV-Datei
    /// RTTY_DIR=/ordner cargo test --release rtty_vergleich -- --ignored --nocapture
    #[test]
    #[ignore]
    fn rtty_vergleich() {
        let dir = std::path::PathBuf::from(std::env::var("RTTY_DIR").unwrap());
        let p = Params::default();
        let spec = hound::WavSpec { channels: 1, sample_rate: 48000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        for noise in [0.0, 0.7, 1.3, 1.6, 2.0] {
            let mut x = vec![0f32; 24000];
            x.extend(encode(TEXT, &p, 48000.0).iter().map(|v| v * 0.3));
            x.extend(vec![0f32; 24000]);
            let mut rng = Rng(5);
            for v in x.iter_mut() { *v += (noise * 0.6 * rng.gauss()) as f32; }
            let mut w = hound::WavWriter::create(dir.join(format!("ours_{noise}.wav")), spec).unwrap();
            for v in &x { w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16).unwrap(); }
            w.finalize().unwrap();
            let mut d = Decoder::new(48000.0, p);
            let got: String = x.chunks(1000).map(|c| d.push(c)).collect();
            println!("eigen  σ={noise}: CER {:.3}", cer(TEXT, &got));
        }
        if let Ok(mut r) = hound::WavReader::open(dir.join("fremd.wav")) {
            let fs = r.spec().sample_rate as f64;
            let x: Vec<f32> = r.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();
            let mut d = Decoder::new(fs, p);
            let got: String = x.chunks(1000).map(|c| d.push(c)).collect();
            println!("fremder Sender dekodiert: CER {:.3} {:?}", cer(TEXT, &got), got);
        }
    }
}
