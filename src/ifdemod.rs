//! FM-Demodulator für die 12-kHz-ZF-Ausgabe von Icom-Geräten (IC-705, IC-7300, IC-9700 …: SET > Connectors >
//! USB/WLAN AF/IF Output > Output Select = IF). Die ZF enthält das FM-Signal noch vor Deemphasis und dem Hochpass
//! gegen CTCSS – demoduliert ergibt sie flaches Basisband wie ein SDR (wichtig für POCSAG und 9600-Bd-Packet).

use std::f64::consts::TAU;

/// Mittenfrequenz der ZF-Ausgabe
pub const IF_HZ: f64 = 12000.0;
/// Ausgang: Frequenzhub / SCALE (±4,5 kHz Hub -> ±0,5)
const SCALE: f64 = 9000.0;

struct Bq { b0: f64, b1: f64, b2: f64, a1: f64, a2: f64, z1: f64, z2: f64 }
impl Bq {
    fn lowpass(fs: f64, fc: f64, q: f64) -> Self {
        let w = TAU * fc.min(fs * 0.45) / fs;
        let (c, a) = (w.cos(), w.sin() / (2.0 * q));
        let a0 = 1.0 + a;
        Bq { b0: (1.0 - c) / 2.0 / a0, b1: (1.0 - c) / a0, b2: (1.0 - c) / 2.0 / a0, a1: -2.0 * c / a0, a2: (1.0 - a) / a0, z1: 0.0, z2: 0.0 }
    }
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

pub struct IfDemod {
    fs: f64,
    /// Zeiger des Mischers und seine Drehung je Sample
    ph: (f64, f64),
    rot: (f64, f64),
    /// Kanalfilter ±8 kHz (je 4. Ordnung für I und Q)
    lp_i: [Bq; 2],
    lp_q: [Bq; 2],
    prev: (f64, f64),
    /// Tiefpass nach dem Diskriminator (7 kHz, reicht auch für 9600 Bd)
    post: [Bq; 2],
}

impl IfDemod {
    pub fn new(fs: f64) -> Self {
        let w = -TAU * IF_HZ / fs;
        let lp = |fc: f64| [Bq::lowpass(fs, fc, 0.5412), Bq::lowpass(fs, fc, 1.3066)];
        IfDemod { fs, ph: (1.0, 0.0), rot: (w.cos(), w.sin()), lp_i: lp(8000.0), lp_q: lp(8000.0), prev: (1e-9, 0.0), post: lp(7000.0) }
    }
    pub fn push(&mut self, x: &[f32]) -> Vec<f32> {
        let k = self.fs / TAU / SCALE;
        x.iter().map(|&v| {
            let v = v as f64;
            let (c, s) = self.ph;
            let (rc, rs) = self.rot;
            let (c, s) = (c * rc - s * rs, c * rs + s * rc);
            let g = 1.5 - 0.5 * (c * c + s * s); // Betrag auf 1 halten
            self.ph = (c * g, s * g);
            let i0 = self.lp_i[0].run(v * self.ph.0);
            let i = self.lp_i[1].run(i0);
            let q0 = self.lp_q[0].run(v * self.ph.1);
            let q = self.lp_q[1].run(q0);
            // Phasendifferenz zum vorigen Sample = Momentanfrequenz
            let (pi, pq) = self.prev;
            let d = (q * pi - i * pq).atan2(i * pi + q * pq);
            self.prev = (i, q);
            let y0 = self.post[0].run(d * k);
            let y = self.post[1].run(y0);
            y.clamp(-1.0, 1.0) as f32
        }).collect()
    }
}

/// Liefert das Audio offenbar ZF statt NF? NF vom Funkgerät endet bei etwa 3 kHz; die ZF hat ihre Energie
/// um 12 kHz (6–18 kHz). Verglichen wird die Energie über 5 kHz mit der unter 3 kHz.
pub fn looks_like_if(x: &[f32], fs: f64) -> bool {
    if fs < 32000.0 || x.len() < 2048 { return false; }
    let mut lo = [Bq::lowpass(fs, 3000.0, std::f64::consts::FRAC_1_SQRT_2), Bq::lowpass(fs, 3000.0, std::f64::consts::FRAC_1_SQRT_2)];
    let mut hl = [Bq::lowpass(fs, 5000.0, std::f64::consts::FRAC_1_SQRT_2), Bq::lowpass(fs, 5000.0, std::f64::consts::FRAC_1_SQRT_2)];
    let (mut el, mut eh) = (0.0, 0.0);
    for &v in x {
        let v = v as f64;
        let l0 = lo[0].run(v);
        let l = lo[1].run(l0);
        let h0 = hl[0].run(v);
        let h = v - hl[1].run(h0); // Hochpass = Signal minus Tiefpass
        el += l * l;
        eh += h * h;
    }
    eh > 2.0 * el && eh > 1e-8 * x.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    /// FM-modulierte ZF erzeugen und zurückdemodulieren: POCSAG muss lesbar sein
    #[test]
    fn zf_pocsag_rundlauf() {
        let fs = 48000.0;
        let pg = crate::pocsag::Page { addr: 1081, func: 3, content: crate::pocsag::Content::Alpha("20.11.26 03:01 - 17:00z: DAUERREGEN".into()) };
        let bb = crate::pocsag::modulate(&[pg], crate::pocsag::Charset::Us, 1200, fs, false);
        let mut ph = 0.0f64;
        let mut seed = 1u32;
        let ifs: Vec<f32> = std::iter::repeat_n(0.0f32, 4800).chain(bb).chain(std::iter::repeat_n(0.0, 9600)).map(|m| {
            ph += TAU * (IF_HZ + 4500.0 / 0.9 * m as f64) / fs;
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            (0.3 * ph.sin() + 0.02 * ((seed >> 16) as f64 / 32768.0 - 1.0)) as f32
        }).collect();
        assert!(looks_like_if(&ifs[4800..14400], fs));
        assert!(!looks_like_if(&crate::pocsag::modulate(&[crate::pocsag::Page { addr: 8, func: 3, content: crate::pocsag::Content::Alpha("NF".into()) }],
            crate::pocsag::Charset::Us, 1200, fs, false)[..9600], fs));
        let y = IfDemod::new(fs).push(&ifs);
        let mut rx = crate::pocsag::Receiver::new(fs, crate::pocsag::Params::default());
        let mut out = vec![];
        for c in y.chunks(4096) { out.extend(rx.push(c)); }
        out.extend(rx.flush());
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].clean_text(), "20.11.26 03:01 - 17:00z: DAUERREGEN");
    }

    /// echte Aufnahme prüfen: IFDEMOD_WAV=/pfad.wav cargo test --release zf_erkennen -- --ignored --nocapture
    #[test]
    #[ignore]
    fn zf_erkennen() {
        let mut rd = hound::WavReader::open(std::env::var("IFDEMOD_WAV").unwrap()).unwrap();
        let fs = rd.spec().sample_rate as f64;
        let x: Vec<f32> = rd.samples::<i16>().map(|v| v.unwrap() as f32 / 32768.0).collect();
        let n = x.chunks(9600).filter(|c| looks_like_if(c, fs)).count();
        println!("ZF-ERKANNT {n} von {} Blöcken", x.len().div_ceil(9600));
    }

    /// Demodulat schreiben: IFDEMOD_WAV=in.wav IFDEMOD_OUT=out.wav cargo test --release zf_schreiben -- --ignored
    #[test]
    #[ignore]
    fn zf_schreiben() {
        let mut rd = hound::WavReader::open(std::env::var("IFDEMOD_WAV").unwrap()).unwrap();
        let fs = rd.spec().sample_rate;
        let x: Vec<f32> = rd.samples::<i16>().map(|v| v.unwrap() as f32 / 32768.0).collect();
        let y = IfDemod::new(fs as f64).push(&x);
        let mut w = hound::WavWriter::create(std::env::var("IFDEMOD_OUT").unwrap(), hound::WavSpec { channels: 1, sample_rate: fs, bits_per_sample: 16, sample_format: hound::SampleFormat::Int }).unwrap();
        for v in y { w.write_sample((v * 32767.0) as i16).unwrap(); }
        w.finalize().unwrap();
    }
}
