//! Sprechfunk über WLAN bzw. USB: PC-Mikrofon zum Funkgerät (mit PTT), Empfang über den PC-Lautsprecher (Mithören).
//! Ein Ringpuffer je Richtung entkoppelt die Takte; Abtastraten werden linear umgerechnet.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

/// Ringpuffer mit bekannter Quellrate; höchstens `max_s` Sekunden, ältere Samples fallen weg
pub struct Ring { q: Mutex<VecDeque<f32>>, src_fs: AtomicU32, max_s: f32, pub gain: AtomicU32 }

impl Ring {
    pub fn new(src_fs: f64, max_s: f32) -> Arc<Ring> {
        Arc::new(Ring { q: Mutex::new(VecDeque::new()), src_fs: AtomicU32::new(src_fs as u32), max_s, gain: AtomicU32::new(1f32.to_bits()) })
    }
    pub fn set_src_fs(&self, fs: f64) { self.src_fs.store(fs as u32, Relaxed); }
    pub fn src_fs(&self) -> f64 { self.src_fs.load(Relaxed) as f64 }
    pub fn gain(&self) -> f32 { f32::from_bits(self.gain.load(Relaxed)) }
    pub fn set_gain(&self, g: f32) { self.gain.store(g.to_bits(), Relaxed) }
    pub fn push(&self, x: &[f32]) {
        let mut q = self.q.lock().unwrap();
        q.extend(x.iter().copied());
        let max = (self.src_fs() * self.max_s as f64) as usize;
        if q.len() > max { let n = q.len() - max; q.drain(..n); }
    }
    #[cfg(test)]
    pub fn len(&self) -> usize { self.q.lock().unwrap().len() }
    /// `n` Samples mit Zielrate `out_fs` holen (fehlende als Stille); `phase` merkt den Bruchteil zwischen Aufrufen
    pub fn pull(&self, n: usize, out_fs: f64, phase: &mut f64) -> Vec<f32> {
        let step = self.src_fs() / out_fs;
        let mut q = self.q.lock().unwrap();
        let g = self.gain();
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let i = *phase as usize;
            let fr = (*phase - i as f64) as f32;
            let (a, b) = (q.get(i).copied(), q.get(i + 1).copied());
            out.push(match (a, b) { (Some(a), Some(b)) => (a + (b - a) * fr) * g, (Some(a), None) => a * g, _ => 0.0 });
            *phase += step;
        }
        let used = (*phase as usize).min(q.len());
        q.drain(..used);
        *phase -= used as f64;
        if q.is_empty() { *phase = 0.0; }
        out
    }
}

/// Ausgabe auf ein Audiogerät aus einem Ring (Mithören über den PC-Lautsprecher bzw. Sprache über USB zum Funkgerät)
pub struct RingOut { _stream: cpal::Stream }

pub fn ring_out(name: &str, ring: Arc<Ring>) -> Result<RingOut, String> {
    let host = cpal::default_host();
    let dev = if name.is_empty() { host.default_output_device() } else {
        host.output_devices().map_err(|e| e.to_string())?.find(|d| d.description().map(|x| x.name().to_string()).unwrap_or_default() == name)
    }.ok_or_else(|| format!("Audiogerät „{}“ nicht gefunden", if name.is_empty() { "Standard" } else { name }))?;
    let sc = dev.default_output_config().map_err(|e| e.to_string())?;
    let cfg = sc.config();
    let fs = cfg.sample_rate as f64;
    fn build<T: SizedSample + FromSample<f32>>(dev: &cpal::Device, cfg: cpal::StreamConfig, ring: Arc<Ring>) -> Result<cpal::Stream, String> {
        let ch = cfg.channels as usize;
        let fs = cfg.sample_rate as f64;
        let mut phase = 0.0;
        dev.build_output_stream(cfg, move |out: &mut [T], _: &_| {
            let x = ring.pull(out.len() / ch, fs, &mut phase);
            for (frame, v) in out.chunks_mut(ch).zip(x) { frame.fill(T::from_sample(v.clamp(-1.0, 1.0))); }
        }, |e| eprintln!("Audio-Ausgang: {e}"), None).map_err(|e| e.to_string())
    }
    let stream = match sc.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&dev, cfg, ring),
        cpal::SampleFormat::I16 => build::<i16>(&dev, cfg, ring),
        cpal::SampleFormat::I32 => build::<i32>(&dev, cfg, ring),
        cpal::SampleFormat::U16 => build::<u16>(&dev, cfg, ring),
        f => Err(format!("Sampleformat {f} nicht unterstützt")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    let _ = fs;
    Ok(RingOut { _stream: stream })
}

/// laufende Sprachaussendung: Mikrofon → Ring (→ WLAN bzw. USB-Ausgang)
pub struct Talk {
    _mic: crate::audio::Input,
    _out: Option<RingOut>,
    pub ring: Arc<Ring>,
    pub level: Arc<AtomicU32>,
    pub since: std::time::Instant,
}

impl Talk {
    /// `usb_out`: Name des Ausgangs zum Funkgerät (USB-Codec); None = über WLAN (Ring liest der Netzwerk-Thread mit 48 kHz)
    pub fn start(mic: &str, usb_out: Option<&str>, gain: f32) -> Result<Talk, String> {
        let (tx, rx) = std::sync::mpsc::channel::<Vec<f32>>();
        let input = crate::audio::start_input(mic, tx)?;
        let ring = Ring::new(input.fs, 0.4);
        ring.set_gain(gain);
        let level = Arc::new(AtomicU32::new(0));
        let (r2, l2) = (ring.clone(), level.clone());
        std::thread::spawn(move || {
            // einfacher Hochpass gegen Brummen/Gleichanteil und Pegelanzeige
            let (mut px, mut py) = (0f32, 0f32);
            while let Ok(mut x) = rx.recv() {
                let mut peak = 0f32;
                for v in x.iter_mut() { let y = 0.995 * (py + *v - px); px = *v; py = y; *v = y; peak = peak.max(y.abs()); }
                l2.store(peak.to_bits(), Relaxed);
                r2.push(&x);
            }
        });
        let out = match usb_out { Some(n) => Some(ring_out(n, ring.clone())?), None => None };
        Ok(Talk { _mic: input, _out: out, ring, level, since: std::time::Instant::now() })
    }
    pub fn level(&self) -> f32 { f32::from_bits(self.level.load(Relaxed)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ring_umrechnen() {
        // 1 kHz bei 44,1 kHz rein, 48 kHz raus: Länge und Frequenz bleiben
        let r = Ring::new(44_100.0, 2.0);
        let x: Vec<f32> = (0..44_100).map(|i| (std::f32::consts::TAU * 1000.0 * i as f32 / 44_100.0).sin()).collect();
        r.push(&x);
        let mut ph = 0.0;
        let mut y = vec![];
        for _ in 0..100 { y.extend(r.pull(480, 48_000.0, &mut ph)); }
        assert_eq!(y.len(), 48_000);
        let zc = y[100..47_000].windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((zc as i32 - 978).abs() <= 3, "{zc}");
        // leer: Stille
        assert!(r.pull(100, 48_000.0, &mut ph)[5..].iter().all(|v| *v == 0.0));
        // Begrenzung: höchstens max_s Sekunden
        r.push(&vec![0.5; 200_000]);
        assert_eq!(r.len(), 88_200);
    }
}

// ---------------- Bedienung ----------------

use crate::App;
use eframe::egui::{self, Color32, RichText};

/// längste Sprechzeit am Stück (Schutz vor hängender Sendetaste)
const MAX_TALK_S: u64 = 180;

impl App {
    /// Sendetaste drücken: Mikrofon zum Funkgerät
    pub(crate) fn talk_start(&mut self) {
        if self.talk.is_some() || self.tx.is_some() || self.cw_tx.is_some() { return; }
        self.talk_release = None;
        let usb = if self.net.is_some() { None } else { Some(self.out_dev.clone()) };
        match Talk::start(&self.mic_dev, usb.as_deref(), self.mic_gain) {
            Ok(t) => {
                if let Some(n) = &self.net { *n.shared.live.lock().unwrap() = Some(t.ring.clone()); }
                if let Some(r) = &mut self.rig { if let Err(e) = r.ptt(true) { self.log(format!("PTT: {e}")); } }
                self.rx_sh.paused.store(true, Relaxed);
                self.talk = Some(t);
            }
            Err(e) => self.log(format!("Mikrofon: {e}")),
        }
    }
    /// Sendetaste los: PTT aus, Empfang weiter
    pub(crate) fn talk_stop(&mut self) {
        let Some(_t) = self.talk.take() else { return };
        if let Some(n) = &self.net { *n.shared.live.lock().unwrap() = None; }
        // über WLAN puffert das Funkgerät noch etwas Audio – PTT erst danach lösen, sonst fehlt das letzte Wort
        let tail = if self.net.is_some() { 100 + crate::icomnet::TX_BUFFER_MS as u64 } else { 100 };
        self.talk_release = Some(std::time::Instant::now() + std::time::Duration::from_millis(tail));
    }

    /// je Bild: Sprechtaste (Leertaste halten bzw. umschalten), Zeitbegrenzung, Mithören
    pub(crate) fn voice_tick(&mut self, ctx: &egui::Context) {
        // Leertaste nur, wenn kein Textfeld den Fokus hat
        let typing = ctx.wants_keyboard_input();
        let (down, pressed) = ctx.input(|i| (i.key_down(egui::Key::Space), i.key_pressed(egui::Key::Space)));
        if !typing && self.mon_on {
            if self.ptt_toggle {
                if pressed { if self.talk.is_some() { self.talk_stop(); } else { self.talk_start(); } }
            } else if (down || self.talk_btn) && self.talk.is_none() { self.talk_start(); }
            else if !down && !self.talk_btn && self.talk.is_some() { self.talk_stop(); }
        } else if self.talk.is_some() && !self.talk_btn && !self.ptt_toggle {
            self.talk_stop();
        }
        if self.talk_release.is_some_and(|t| std::time::Instant::now() >= t) {
            self.talk_release = None;
            if let Some(r) = &mut self.rig { let _ = r.ptt(false); }
            self.rx_sh.paused.store(false, Relaxed);
        } else if self.talk_release.is_some() { ctx.request_repaint_after(std::time::Duration::from_millis(50)); }
        if let Some(t) = &self.talk {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            if t.since.elapsed().as_secs() >= MAX_TALK_S {
                self.talk_stop();
                self.log(format!("Sprechfunk: nach {MAX_TALK_S} s automatisch auf Empfang"));
            }
        }
        // Mithören an/aus, Lautsprecher gewechselt
        let want = self.mon_on.then(|| self.spk_dev.clone());
        if want.as_ref() != self.mon_out.as_ref().map(|m| &m.2) {
            *self.rx_sh.monitor.lock().unwrap() = None;
            self.mon_out = None;
            if let Some(dev) = want {
                let ring = Ring::new(48_000.0, 0.5);
                match ring_out(&dev, ring.clone()) {
                    Ok(o) => { *self.rx_sh.monitor.lock().unwrap() = Some(ring.clone()); self.mon_out = Some((o, ring, dev)); }
                    Err(e) => { self.log(format!("Lautsprecher: {e}")); self.mon_on = false; }
                }
            }
        }
        if let Some((_, r, _)) = &self.mon_out { r.set_gain(self.mon_vol); }
        if let Some(t) = &self.talk { t.ring.set_gain(self.mic_gain); }
    }

    /// Funkgerät-Leiste: Mithören und Sprechtaste
    pub(crate) fn ui_voice_bar(&mut self, ui: &mut egui::Ui) {
        let on = self.mon_on;
        let b = egui::Button::new(RichText::new("🔊").color(if on { crate::GREEN } else { ui.visuals().weak_text_color() }));
        let r = ui.add(b).on_hover_text("Sprechfunk: Empfang über den PC-Lautsprecher mithören, mit Leertaste bzw. „Sprechen“ senden");
        if r.clicked() { self.mon_on = !self.mon_on; }
        if !on { return; }
        ui.add(egui::Slider::new(&mut self.mon_vol, 0.0..=2.0).show_value(false)).on_hover_text("Lautstärke");
        let talking = self.talk.is_some();
        let lvl = self.talk.as_ref().map_or(0.0, |t| t.level());
        let txt = RichText::new(if talking { format!("Sprechen {:.0} %", (lvl * 100.0).min(100.0)) } else { "Sprechen".into() }).strong();
        let b = if talking { egui::Button::new(txt.color(Color32::WHITE)).fill(crate::RED) } else { egui::Button::new(txt) };
        let r = ui.add(b.sense(egui::Sense::click_and_drag())).on_hover_text(if self.ptt_toggle { "Sprechen: Klick oder Leertaste schaltet um" }
            else { "Sprechen: gedrückt halten (oder Leertaste halten)" });
        if self.ptt_toggle { if r.clicked() { if talking { self.talk_stop(); } else { self.talk_start(); } } }
        else { self.talk_btn = r.is_pointer_button_down_on(); }
    }

    /// Einstellungen › Audio: Sprechfunk
    pub(crate) fn ui_set_voice(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.label(RichText::new("Sprechfunk (Mikrofon und Lautsprecher am PC)").strong());
        let names_in = crate::audio::input_names();
        let names_out = crate::audio::output_names();
        let combo = |ui: &mut egui::Ui, id: &str, val: &mut String, names: &[String]| {
            egui::ComboBox::from_id_salt(id).width(ui.available_width().min(360.0)).selected_text(if val.is_empty() { "Standard" } else { val.as_str() })
                .show_ui(ui, |ui| { ui.selectable_value(val, String::new(), "Standard"); for n in names { ui.selectable_value(val, n.clone(), n); } });
        };
        ui.label("Mikrofon");
        combo(ui, "mic", &mut self.mic_dev, &names_in);
        ui.horizontal(|ui| {
            ui.label("Mikrofonpegel");
            ui.add(egui::Slider::new(&mut self.mic_gain, 0.1..=4.0).logarithmic(true));
        });
        ui.label("Lautsprecher (Mithören)");
        combo(ui, "spk", &mut self.spk_dev, &names_out);
        ui.checkbox(&mut self.ptt_toggle, "Leertaste/Taste schaltet um (statt gedrückt halten)");
        ui.label(RichText::new(format!("Ein 🔊 in der Funkgerät-Leiste schaltet das Mithören ein; gesprochen wird mit gehaltener Leertaste oder „Sprechen“. \
            Nach {MAX_TALK_S} s geht Rustwave von selbst auf Empfang. Das Funkgerät muss die Sprache vom PC nehmen: beim IC-705 \
            unter SET › Connectors › MOD Input › DATA OFF MOD „WLAN“ (bzw. „USB“ über Kabel) wählen und in FM/SSB ohne DATA senden.")).small().weak());
    }
}

