//! Empfang: geteilter Zustand, Empfangs-Thread (Dekoder, Wasserfall, Mitschnitt) und Dekodieren von WAV-Dateien.

use super::*;

#[derive(Default)]
pub(crate) struct RxShared {
    pub(crate) paused: AtomicBool,
    pub(crate) ctl: AtomicU8,
    /// Some = RTTY-Betrieb mit diesen Parametern, None = SSTV
    pub(crate) rtty: std::sync::Mutex<Option<rtty::Params>>,
    /// APRS-Betrieb
    pub(crate) aprs: AtomicBool,
    /// POCSAG-Betrieb mit diesen Dekoder-Einstellungen; Baudraten im Sync (Bit 0 = 512, 1 = 1200, 2 = 2400)
    pub(crate) pocsag: std::sync::Mutex<Option<pocsag::Params>>,
    pub(crate) pocsag_sync: AtomicU8,
    /// Eingang ist die 12-kHz-ZF des Funkgeräts: vor allen Dekodern FM-demodulieren
    pub(crate) if_rx: AtomicBool,
    /// Eingang sieht nach ZF aus (für einen Hinweis, wenn der Schalter aus ist)
    pub(crate) if_seen: AtomicBool,
    /// Anteil übersteuerter Eingangssamples (|x| ≥ 0,97) in den letzten 2 s, in Promille (f32-Bits)
    pub(crate) clip: std::sync::atomic::AtomicU32,
    /// Zähler des POCSAG-Dekoders je Baudrate (Diagnose)
    pub(crate) pocsag_stats: std::sync::Mutex<[pocsag::Stats; 3]>,
    /// Packet Radio mit dieser Geschwindigkeit; Kanal belegt (DCD)
    pub(crate) packet: std::sync::Mutex<Option<ax25::Speed>>,
    pub(crate) packet_dcd: AtomicBool,
    /// APRS auf Kurzwelle (300 Baud) statt UKW
    pub(crate) aprs_hf: AtomicBool,
    /// KW-APRS über das ganze Durchlassband (Skimmer)
    pub(crate) aprs_skim: AtomicBool,
    /// Signalgüte des RTTY-Dekoders (f32-Bits)
    pub(crate) rtty_quality: std::sync::atomic::AtomicU32,
    /// CW-Betrieb: Parameter, Signal/Rauschen (dB) und Tempo (WpM) als f32-Bits
    pub(crate) cw: std::sync::Mutex<Option<cw::Params>>,
    pub(crate) cw_snr: std::sync::atomic::AtomicU32,
    pub(crate) cw_wpm: std::sync::atomic::AtomicU32,
    /// CW-Skimmer (alle Signale im Durchlass) einschalten
    pub(crate) skim: AtomicBool,
    /// Mitschnitt des Empfangs: gewünschte Datei (None = keine Aufnahme) und aufgenommene Samples/Rate
    pub(crate) rec_path: std::sync::Mutex<Option<PathBuf>>,
    pub(crate) rec_n: std::sync::atomic::AtomicU64,
    pub(crate) rec_fs: std::sync::atomic::AtomicU32,
    /// bisher empfangene Audio-Samples (Selbsttest misst damit die tatsächliche Rate)
    pub(crate) rx_samples: std::sync::atomic::AtomicU64,
    /// Wasserfallzeilen pro Sekunde, die die Oberfläche gerade braucht (0 = Wasserfall nicht sichtbar)
    pub(crate) wf_rate: std::sync::atomic::AtomicU32,
    /// Mithören: Empfang (nach ZF-Demodulation) auf den PC-Lautsprecher
    pub(crate) monitor: std::sync::Mutex<Option<Arc<crate::voice::Ring>>>,
}

pub(crate) fn forward(dec: &mut sstv::Decoder, tx: &mpsc::Sender<UiEv>) {
    for e in dec.events.drain(..) {
        let _ = tx.send(match e {
            sstv::Event::Start(m) => UiEv::Start(m),
            sstv::Event::Line { y, rgb } => UiEv::Line(y, rgb),
            sstv::Event::End { mode, img, complete } => UiEv::End(mode, img, complete),
        });
    }
}

/// Dekodiert den Live-Eingang und berechnet Wasserfall und Pegel.
pub(crate) fn rx_thread(fs: f64, rx: mpsc::Receiver<Vec<f32>>, tx: mpsc::Sender<UiEv>, sh: Arc<RxShared>, _ctx: egui::Context) {
    use rustfft::num_complex::Complex;
    let mut dec = sstv::Decoder::new(fs);
    let mut fdec = fskid::Decoder::new(fs);
    let mut rdec: Option<rtty::Decoder> = None;
    let mut adec: Option<aprs::Receiver> = None;
    let mut askim: Option<aprs::Skimmer> = None;
    let mut cdec: Option<cw::Decoder> = None;
    let mut pdec: Option<pocsag::Receiver> = None;
    let mut kdec: Option<(ax25::Speed, ax25::Receiver)> = None;
    let mut skim: Option<skimmer::Skimmer> = None;
    let n = 2048;
    let fft = rustfft::FftPlanner::<f32>::new().plan_fft_forward(n);
    let win: Vec<f32> = (0..n).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos()).collect();
    // Neuzeichnen nur im Takt des Wasserfalls (früher nach jedem Audioblock, bis zu 100× pro Sekunde)
    let mut hop = (fs / 25.0) as usize;
    let (mut ring, mut since, mut sumsq) = (VecDeque::with_capacity(n + 1), 0usize, 0f32);
    let mut rec: Option<(PathBuf, hound::WavWriter<std::io::BufWriter<std::fs::File>>)> = None;
    let mut ifd: Option<ifdemod::IfDemod> = None;
    // Erkennung ZF/NF und Übersteuerung über je 2 s gesammeltes Roh-Audio (WLAN liefert 20-ms-Pakete)
    let mut probe: Vec<f32> = Vec::with_capacity(9600);
    let (mut win_n, mut win_clip) = (0usize, 0usize);
    while let Ok(raw) = rx.recv() {
        sh.rx_samples.fetch_add(raw.len() as u64, Relaxed);
        // ab und zu prüfen, ob das Funkgerät ZF statt NF liefert
        win_n += raw.len();
        win_clip += raw.iter().filter(|v| v.abs() >= 0.97).count();
        if probe.len() < 9600 { probe.extend_from_slice(&raw[..raw.len().min(9600 - probe.len())]); }
        if win_n >= fs as usize * 2 {
            if probe.len() >= 4096 { sh.if_seen.store(ifdemod::looks_like_if(&probe, fs), Relaxed); }
            sh.clip.store((1000.0 * win_clip as f32 / win_n as f32).to_bits(), Relaxed);
            probe.clear();
            win_n = 0;
            win_clip = 0;
        }
        // Mitschnitt (16 Bit mono, Rate des Eingangs)
        let want = sh.rec_path.lock().unwrap().clone();
        if rec.as_ref().map(|r| &r.0) != want.as_ref() {
            if let Some((p, w)) = rec.take() {
                let r = w.finalize().map_err(|e| e.to_string());
                let _ = tx.send(UiEv::Log(match r { Ok(()) => format!("Aufnahme gespeichert: {}", p.display()), Err(e) => format!("Aufnahme: {e}") }));
            }
            if let Some(p) = want {
                let spec = hound::WavSpec { channels: 1, sample_rate: fs as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
                match std::fs::create_dir_all(p.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())
                    .and_then(|_| hound::WavWriter::create(&p, spec).map_err(|e| e.to_string())) {
                    Ok(w) => { sh.rec_n.store(0, Relaxed); sh.rec_fs.store(fs as u32, Relaxed); rec = Some((p, w)); }
                    Err(e) => { let _ = tx.send(UiEv::Log(format!("Aufnahme nicht möglich: {e}"))); *sh.rec_path.lock().unwrap() = None; }
                }
            }
        }
        if let Some((_, w)) = &mut rec {
            for &v in &raw { let _ = w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16); }
            sh.rec_n.fetch_add(raw.len() as u64, Relaxed);
        }
        // ZF: selbst FM-demodulieren, danach geht alles wie gewohnt weiter (Aufnahme bleibt roh)
        let demod = if sh.if_rx.load(Relaxed) { Some(ifd.get_or_insert_with(|| ifdemod::IfDemod::new(fs)).push(&raw)) } else { ifd = None; None };
        let x: &[f32] = demod.as_deref().unwrap_or(&raw);
        if let Some(m) = &*sh.monitor.lock().unwrap() { if !sh.paused.load(Relaxed) { m.set_src_fs(fs); m.push(x); } }
        match sh.ctl.swap(0, Relaxed) {
            CTL_RESET => dec = sstv::Decoder::new(fs),
            CTL_ABORT => dec.abort(),
            _ => {}
        }
        if sh.aprs.load(Relaxed) && sh.aprs_skim.load(Relaxed) {
            adec = None;
            let d = askim.get_or_insert_with(|| aprs::Skimmer::new(fs));
            if !sh.paused.load(Relaxed) { for (f, c) in d.push(&x) { let _ = tx.send(UiEv::Aprs(f, Some(c))); } }
        } else if sh.aprs.load(Relaxed) {
            askim = None;
            let m = if sh.aprs_hf.load(Relaxed) { aprs::HF } else { aprs::VHF };
            if adec.as_ref().is_none_or(|d: &aprs::Receiver| d.modem != m) { adec = Some(aprs::Receiver::new(fs, m)); }
            let d = adec.as_mut().unwrap();
            if !sh.paused.load(Relaxed) { for f in d.push(&x) { let _ = tx.send(UiEv::Aprs(f, None)); } }
        } else { adec = None; askim = None; }
        let poc_p = *sh.pocsag.lock().unwrap();
        match poc_p {
            Some(p) => {
                if pdec.as_ref().is_none_or(|d| d.p != p) { pdec = Some(pocsag::Receiver::new(fs, p)); }
                let d = pdec.as_mut().unwrap();
                if !sh.paused.load(Relaxed) { for m in d.push(&x) { let _ = tx.send(UiEv::Pocsag(m)); } }
                let bits = d.synced().iter().filter_map(|b| pocsag::BAUDS.iter().position(|x| x == b)).fold(0u8, |a, i| a | 1 << i);
                sh.pocsag_sync.store(bits, Relaxed);
                *sh.pocsag_stats.lock().unwrap() = d.stats();
            }
            None => { pdec = None; sh.pocsag_sync.store(0, Relaxed); }
        }
        let pk = *sh.packet.lock().unwrap();
        match pk {
            Some(s) => {
                if kdec.as_ref().is_none_or(|d| d.0 != s) { kdec = Some((s, ax25::Receiver::new(fs, s))); }
                let d = &mut kdec.as_mut().unwrap().1;
                if !sh.paused.load(Relaxed) { for b in d.push(&x) { let _ = tx.send(UiEv::Packet(b, true)); } }
                sh.packet_dcd.store(d.busy(), Relaxed);
            }
            None => { kdec = None; sh.packet_dcd.store(false, Relaxed); }
        }
        let cw_p = *sh.cw.lock().unwrap();
        match cw_p {
            Some(p) => {
                if cdec.is_none() { cdec = Some(cw::Decoder::new(fs, p)); }
                let d = cdec.as_mut().unwrap();
                d.p = p; // Tonhöhe und Sperre ohne Neustart übernehmen
                if !sh.paused.load(Relaxed) {
                    let t = d.push(&x);
                    if !t.is_empty() { let _ = tx.send(UiEv::Cw(t)); }
                }
                sh.cw_snr.store(d.snr_db.to_bits(), Relaxed);
                sh.cw_wpm.store(d.wpm.to_bits(), Relaxed);
                if sh.skim.load(Relaxed) {
                    let s = skim.get_or_insert_with(|| skimmer::Skimmer::new(fs, p.squelch));
                    if !sh.paused.load(Relaxed) { if let Some(v) = s.push(&x) { let _ = tx.send(UiEv::Skim(v)); } }
                } else { skim = None; }
            }
            None => { cdec = None; skim = None; }
        }
        let other = sh.aprs.load(Relaxed) || cw_p.is_some() || poc_p.is_some() || pk.is_some(); // anderer Dekoder aktiv: kein SSTV/RTTY
        let want = if other { None } else { *sh.rtty.lock().unwrap() };
        match want {
            Some(p) => {
                if rdec.as_ref().is_none_or(|d| d.p != p) { rdec = Some(rtty::Decoder::new(fs, p)); }
                let d = rdec.as_mut().unwrap();
                if !sh.paused.load(Relaxed) {
                    let t = d.push(&x);
                    if !t.is_empty() { let _ = tx.send(UiEv::Rtty(t)); }
                }
                sh.rtty_quality.store(d.quality.to_bits(), Relaxed);
            }
            None if !other => {
                rdec = None;
                if !sh.paused.load(Relaxed) {
                    dec.push(&x);
                    if let Some(c) = fdec.push(&x) { let _ = tx.send(UiEv::FskId(c)); }
                }
                forward(&mut dec, &tx);
            }
            None => {}
        }
        // Pegel immer vom Roh-Eingang (bei ZF sonst das Rauschen des Demodulators), Wasserfall vom Dekoder-Audio
        for (&v, &r) in x.iter().zip(raw.iter()) {
            ring.push_back(v);
            if ring.len() > n { ring.pop_front(); }
            sumsq += r * r;
            since += 1;
            if since >= hop && ring.len() == n {
                let level = (sumsq / since as f32).sqrt();
                since = 0; sumsq = 0.0;
                let rate = sh.wf_rate.load(Relaxed);
                hop = (fs / rate.clamp(5, 25) as f64) as usize;
                // kein Anstoß zum Neuzeichnen von hier: die Oberfläche zeichnet bei laufenden Daten selbst im festen Takt
                if rate == 0 { let _ = tx.send(UiEv::Wf(vec![], level)); continue; } // nur Pegel
                let mut buf: Vec<Complex<f32>> = ring.iter().zip(&win).map(|(a, w)| Complex::new(a * w, 0.0)).collect();
                fft.process(&mut buf);
                let row = (0..WF_W).map(|c| {
                    let bin = (c as f32 / WF_W as f32 * WF_MAX_HZ / fs as f32 * n as f32) as usize;
                    let db = 20.0 * (buf[bin].norm() / n as f32 + 1e-9).log10();
                    ((db + 100.0) / 80.0 * 255.0).clamp(0.0, 255.0) as u8
                }).collect();
                let _ = tx.send(UiEv::Wf(row, level));
            }
        }
    }
}

/// WAV-Datei dekodieren (eigener Thread, gleiche Ereignisse wie live)
/// womit eine WAV-Datei dekodiert wird
#[derive(Clone, Copy)]
pub(crate) enum FileMode { Sstv, Rtty(rtty::Params), Cw(cw::Params), Aprs(aprs::Modem), AprsSkim, Pocsag(pocsag::Params), Packet(ax25::Speed) }

pub(crate) fn decode_file(path: PathBuf, tx: mpsc::Sender<UiEv>, ctx: Option<egui::Context>, mode: FileMode, if_rx: bool) {
    let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let r = (|| -> Result<(), String> {
        let mut rd = hound::WavReader::open(&path).map_err(|e| e.to_string())?;
        let spec = rd.spec();
        let ch = spec.channels as usize;
        let x: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => rd.samples::<f32>().step_by(ch).map(|s| s.unwrap_or(0.0)).collect(),
            hound::SampleFormat::Int => {
                let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
                rd.samples::<i32>().step_by(ch).map(|s| s.unwrap_or(0) as f32 / max).collect()
            }
        };
        let fs = spec.sample_rate as f64;
        // ZF-Aufnahmen (Output Select = IF) erkennt man am Spektrum – dann unabhängig vom Schalter selbst demodulieren
        let probe = &x[..x.len().min(fs as usize * 4)];
        let if_rx = if_rx || (fs >= 24000.0 && ifdemod::looks_like_if(probe, fs));
        let x = if if_rx { ifdemod::IfDemod::new(fs).push(&x) } else { x };
        if let FileMode::AprsSkim = mode {
            let mut d = aprs::Skimmer::new(fs);
            let mut n = 0;
            for c in x.chunks(fs as usize) { for (f, t) in d.push(c) { n += 1; let _ = tx.send(UiEv::Aprs(f, Some(t))); } }
            let _ = tx.send(UiEv::Log(format!("{n} APRS-Pakete in {name} (Skimmer)")));
            if let Some(c) = &ctx { c.request_repaint(); }
            return Ok(());
        }
        if let FileMode::Packet(s) = mode {
            let mut d = ax25::Receiver::new(fs, s);
            let mut n = 0;
            for c in x.chunks(fs as usize) { for b in d.push(c) { n += 1; let _ = tx.send(UiEv::Packet(b, false)); } }
            let _ = tx.send(UiEv::Log(format!("{n} Packet-Rahmen in {name}")));
            if let Some(c) = &ctx { c.request_repaint(); }
            return Ok(());
        }
        if let FileMode::Pocsag(p) = mode {
            let mut d = pocsag::Receiver::new(fs, p);
            let mut n = 0;
            let tail = vec![0.0; fs as usize];
            for c in x.chunks(fs as usize).chain([tail.as_slice()]) { for m in d.push(c) { n += 1; let _ = tx.send(UiEv::Pocsag(m)); } }
            for m in d.flush() { n += 1; let _ = tx.send(UiEv::Pocsag(m)); }
            let st: Vec<String> = pocsag::BAUDS.iter().zip(d.stats()).filter(|(_, s)| s.preambles + s.syncs > 0)
                .map(|(b, s)| format!("{b} Bd: {} Präambeln, {} Sync, Wörter {} gut/{} korrigiert/{} defekt", s.preambles, s.syncs, s.ok, s.fixed, s.bad)).collect();
            let _ = tx.send(UiEv::Log(format!("{n} Funkrufe in {name}{}", if st.is_empty() { " · kein POCSAG-Signal erkannt".into() } else { format!(" · {}", st.join(" · ")) })));
            if let Some(c) = &ctx { c.request_repaint(); }
            return Ok(());
        }
        if let FileMode::Aprs(m) = mode {
            let mut d = aprs::Receiver::new(fs, m);
            let mut n = 0;
            for c in x.chunks(fs as usize) { for f in d.push(c) { n += 1; let _ = tx.send(UiEv::Aprs(f, None)); } }
            let _ = tx.send(UiEv::Log(format!("{n} APRS-Pakete in {name}")));
            if let Some(c) = &ctx { c.request_repaint(); }
            return Ok(());
        }
        if let FileMode::Cw(p) = mode {
            let mut d = cw::Decoder::new(fs, p);
            for c in x.chunks(fs as usize).chain([vec![0.0; 2 * fs as usize].as_slice()]) {
                let t = d.push(c);
                if !t.is_empty() { let _ = tx.send(UiEv::Cw(t)); }
            }
            return Ok(());
        }
        if let FileMode::Rtty(p) = mode {
            let mut d = rtty::Decoder::new(fs, p);
            for c in x.chunks(fs as usize) { let t = d.push(c); if !t.is_empty() { let _ = tx.send(UiEv::Rtty(t)); } }
            return Ok(());
        }
        let mut dec = sstv::Decoder::new(fs);
        let mut fdec = fskid::Decoder::new(fs);
        for c in x.chunks(fs as usize).chain([vec![0.0; 2 * fs as usize].as_slice()]) { // + Nachlauf
            dec.push(c);
            if let Some(call) = fdec.push(c) { let _ = tx.send(UiEv::FskId(call)); }
            forward(&mut dec, &tx);
            if let Some(c) = &ctx { c.request_repaint(); }
        }
        dec.abort();
        forward(&mut dec, &tx);
        Ok(())
    })();
    let _ = tx.send(UiEv::Log(match r { Ok(()) => format!("{name} dekodiert"), Err(e) => format!("{name}: {e}") }));
    if let Some(c) = &ctx { c.request_repaint(); }
}

// ---------------- Hilfen ----------------

