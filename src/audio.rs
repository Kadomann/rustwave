//! Audio-Ein-/Ausgabe über cpal (unter Windows WASAPI).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering::Relaxed};
use std::sync::mpsc::Sender;
use std::sync::Arc;

fn dev_name(d: &cpal::Device) -> String {
    d.description().map(|x| x.name().to_string()).unwrap_or_else(|_| "?".into())
}
pub fn input_names() -> Vec<String> {
    cpal::default_host().input_devices().map(|it| it.map(|d| dev_name(&d)).collect()).unwrap_or_default()
}
pub fn output_names() -> Vec<String> {
    cpal::default_host().output_devices().map(|it| it.map(|d| dev_name(&d)).collect()).unwrap_or_default()
}
/// Gerät nach Name, leerer Name = Standardgerät
fn find(name: &str, input: bool) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    let dev = if name.is_empty() {
        if input { host.default_input_device() } else { host.default_output_device() }
    } else {
        let mut it = if input { host.input_devices() } else { host.output_devices() }.map_err(|e| e.to_string())?;
        it.find(|d| dev_name(d) == name)
    };
    dev.ok_or_else(|| format!("Audiogerät „{}“ nicht gefunden", if name.is_empty() { "Standard" } else { name }))
}

/// Laufende Aufnahme; Samples (mono) gehen als Blöcke an `tx`.
pub struct Input { _stream: cpal::Stream, pub fs: f64 }

pub fn start_input(name: &str, tx: Sender<Vec<f32>>) -> Result<Input, String> {
    let dev = find(name, true)?;
    let sc = dev.default_input_config().map_err(|e| e.to_string())?;
    let cfg = sc.config();
    let ch = cfg.channels as usize;
    fn build<T: SizedSample>(dev: &cpal::Device, cfg: cpal::StreamConfig, ch: usize, tx: Sender<Vec<f32>>) -> Result<cpal::Stream, String>
    where f32: FromSample<T> {
        dev.build_input_stream(cfg, move |data: &[T], _: &_| {
            let _ = tx.send(data.chunks(ch).map(|f| f32::from_sample(f[0])).collect()); // Kanal 1
        }, |e| eprintln!("Audio-Eingang: {e}"), None).map_err(|e| e.to_string())
    }
    let stream = match sc.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&dev, cfg.clone(), ch, tx),
        cpal::SampleFormat::I16 => build::<i16>(&dev, cfg.clone(), ch, tx),
        cpal::SampleFormat::I32 => build::<i32>(&dev, cfg.clone(), ch, tx),
        cpal::SampleFormat::U16 => build::<u16>(&dev, cfg.clone(), ch, tx),
        f => Err(format!("Sampleformat {f} nicht unterstützt")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Input { _stream: stream, fs: cfg.sample_rate as f64 })
}

/// Zustand einer laufenden Wiedergabe, vom GUI-Thread les- und steuerbar.
pub struct PlayState {
    gain: AtomicU32,
    pub stop: AtomicBool,
    pub done: AtomicBool,
    pub pos: AtomicUsize,
    pub len: usize,
}
impl PlayState {
    pub fn new(len: usize, gain: f32) -> Arc<PlayState> {
        Arc::new(PlayState { gain: AtomicU32::new(gain.to_bits()), stop: AtomicBool::new(false), done: AtomicBool::new(false), pos: AtomicUsize::new(0), len })
    }
    pub fn gain(&self) -> f32 { f32::from_bits(self.gain.load(Relaxed)) }
    pub fn set_gain(&self, g: f32) { self.gain.store(g.to_bits(), Relaxed) }
    pub fn progress(&self) -> f32 { self.pos.load(Relaxed) as f32 / self.len.max(1) as f32 }
}

pub struct Output { _stream: cpal::Stream, pub state: Arc<PlayState> }

/// Abtastrate des Ausgabegeräts (zum passenden Kodieren vor dem Abspielen)
pub fn output_rate(name: &str) -> Result<f64, String> {
    let dev = find(name, false)?;
    Ok(dev.default_output_config().map_err(|e| e.to_string())?.sample_rate() as f64)
}

/// Spielt `samples` (mono, volle Aussteuerung) mit Pegel `gain` auf allen Kanälen ab.
pub fn play(name: &str, samples: Vec<f32>, gain: f32) -> Result<Output, String> {
    let dev = find(name, false)?;
    let sc = dev.default_output_config().map_err(|e| e.to_string())?;
    let cfg = sc.config();
    let state = PlayState::new(samples.len(), gain);
    fn build<T: SizedSample + FromSample<f32>>(dev: &cpal::Device, cfg: cpal::StreamConfig, s: Vec<f32>, st: Arc<PlayState>) -> Result<cpal::Stream, String> {
        let ch = cfg.channels as usize;
        let mut i = 0;
        dev.build_output_stream(cfg, move |out: &mut [T], _: &_| {
            let g = st.gain();
            for frame in out.chunks_mut(ch) {
                let v = if i < s.len() && !st.stop.load(Relaxed) { i += 1; s[i - 1] * g } else { st.done.store(true, Relaxed); 0.0 };
                frame.fill(T::from_sample(v));
            }
            st.pos.store(i, Relaxed);
        }, |e| eprintln!("Audio-Ausgang: {e}"), None).map_err(|e| e.to_string())
    }
    let st = state.clone();
    let stream = match sc.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&dev, cfg, samples, st),
        cpal::SampleFormat::I16 => build::<i16>(&dev, cfg, samples, st),
        cpal::SampleFormat::I32 => build::<i32>(&dev, cfg, samples, st),
        cpal::SampleFormat::U16 => build::<u16>(&dev, cfg, samples, st),
        f => Err(format!("Sampleformat {f} nicht unterstützt")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Output { _stream: stream, state })
}
