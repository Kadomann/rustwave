//! Funkgerät-Leiste, Zusätze für Icom: Tuner-Knopf, Versorgungsspannung/Strom, Empfangsschalter
//! (Filter, Vorverstärker, Abschwächer, NB, NR, AGC) und das Stellen der PC-Uhr nach dem GPS des Funkgeräts.

use super::*;
use crate::rigpoll::Kind;
use egui::containers::menu::{MenuButton, MenuConfig};

/// Befehlskennung für den Tuner-Start (Ablehnung = kein Tuner)
const TAG_TUNE: u8 = 10;

impl App {
    fn civ_rig(&self) -> Option<&cat::RigModel> { self.rig.as_ref().map(|r| &r.model).filter(|m| m.proto == cat::Proto::Civ) }

    /// Schalter setzen: Befehl in die Warteschlange, Anzeige sofort umstellen
    fn set_ctl(&mut self, k: Kind, v: u8) {
        let Some(m) = self.civ_rig() else { return };
        let b = cat::civ(m, &[rigpoll::ctl_body(k), &[v]].concat());
        self.poll.cmdq.push_back((if k == Kind::Tuner && v == 2 { TAG_TUNE } else { 0 }, b));
        self.poll.ctl[k as usize] = Some(v);
    }

    /// je Bild: Abstimmvorgang verfolgen (das Gerät meldet 02, solange es abstimmt)
    pub(crate) fn tune_tick(&mut self) {
        let Some(t0) = self.tuning else { return };
        if self.poll.cmd_ng.contains(&TAG_TUNE) {
            self.poll.cmd_ng.retain(|&t| t != TAG_TUNE);
            self.tuning = None;
            self.log("Tuner: vom Funkgerät abgelehnt – ist ein Tuner angeschlossen und unter SET > Function > Tuner gewählt?");
            return;
        }
        let el = t0.elapsed();
        let done = el > Duration::from_millis(1500) && self.poll.ctl[Kind::Tuner as usize].is_some_and(|v| v != 2);
        if done || el > Duration::from_secs(20) {
            self.tuning = None;
            let swr = self.poll.meters.swr.map_or(String::new(), |s| format!(" – SWR {s:.1}"));
            self.log(if done { format!("Tuner: abgestimmt{swr}") } else { format!("Tuner: keine Rückmeldung nach 20 s{swr}") });
        }
    }

    /// TUNE-Knopf, Spannung/Strom und Empfangsschalter in der Funkgerät-Leiste
    pub(crate) fn ui_rig_extras(&mut self, ui: &mut egui::Ui) {
        let Some(model) = self.civ_rig().cloned() else { return };
        // Tuner
        if self.poll.supported(Kind::Tuner) && self.poll.ctl[Kind::Tuner as usize].is_some() {
            if self.tuning.is_some() {
                ui.label(RichText::new("stimmt ab …").color(ORANGE).strong());
                ui.ctx().request_repaint_after(Duration::from_millis(200));
            } else {
                let on = self.poll.ctl[Kind::Tuner as usize] == Some(1);
                let idle = self.tx.is_none() && self.cw_tx.is_none();
                let r = ui.add_enabled(idle, egui::Button::new(RichText::new("TUNE").strong()).selected(on))
                    .on_hover_text("Antennentuner abstimmen (sendet kurz einen Träger). Rechtsklick: Tuner ein/aus. \
                                    IC-705: mit AH-705 (SET > Function > Tuner > Tuner Select).");
                if r.clicked() { self.set_ctl(Kind::Tuner, 2); self.tuning = Some(Instant::now()); self.log("Tuner: stimmt ab …"); }
                if r.secondary_clicked() { self.set_ctl(Kind::Tuner, if on { 0 } else { 1 }); }
            }
        }
        // Empfangsschalter
        let hf = self.freq_shown.is_none_or(|f| f < 100_000_000);
        let c = self.poll.ctl;
        let mut sum = vec![];
        if let Some((_, f)) = self.poll.filter { sum.push(format!("FIL{f}")); }
        match c[Kind::Pre as usize] { Some(1) => sum.push(if hf { "P1".into() } else { "PRE".into() }), Some(2) => sum.push("P2".into()), _ => {} }
        if c[Kind::Att as usize].is_some_and(|v| v != 0) { sum.push("ATT".into()); }
        if c[Kind::Nb as usize] == Some(1) { sum.push("NB".into()); }
        if c[Kind::Nr as usize] == Some(1) { sum.push("NR".into()); }
        if !sum.is_empty() || c[Kind::Agc as usize].is_some() {
            let label = if sum.is_empty() { "RX".to_string() } else { format!("RX {}", sum.join(" ")) };
            MenuButton::from_button(egui::Button::new(RichText::new(label).monospace()))
                .config(MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
                .ui(ui, |ui| self.ui_rx_menu(ui, &model, hf)).0
                .on_hover_text("Empfangseinstellungen des Funkgeräts: Filter, Vorverstärker, Abschwächer, Störaustaster, Rauschminderung, AGC");
        }
        // Versorgung
        if let Some(v) = self.poll.vd {
            // unter 9 V: Akkubetrieb (IC-705 mit BP-272, 7,4 V)
            let batt = v < 9.0;
            let (warn, low) = if batt { (7.0, 6.6) } else { (12.0, 11.5) };
            let col = if v < low { RED } else if v < warn { ORANGE } else { ui.visuals().weak_text_color() };
            let tx = self.poll.tx;
            let txt = match (tx, self.poll.id) { (true, Some(a)) => format!("{v:.1} V {a:.1} A"), _ => format!("{v:.1} V") };
            ui.label(RichText::new(txt).monospace().color(col)).on_hover_text(format!(
                "Versorgungsspannung{} – {}", if batt { " (Akku)" } else { "" },
                if v < low { "Akku fast leer!" } else if v < warn { "wird knapp" } else { "in Ordnung" }));
            if v < low && !self.low_volt_warned { self.low_volt_warned = true; self.log(format!("Achtung: Versorgungsspannung nur {v:.1} V")); }
            if v >= warn { self.low_volt_warned = false; }
        }
    }

    fn ui_rx_menu(&mut self, ui: &mut egui::Ui, model: &cat::RigModel, hf: bool) {
        let c = self.poll.ctl;
        egui::Grid::new("rxctl").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            if let Some((m, f)) = self.poll.filter {
                ui.label("Filter");
                ui.horizontal(|ui| for k in 1..=3u8 {
                    if ui.selectable_label(f == k, format!("FIL{k}")).on_hover_text("Filterbreiten wie am Gerät eingestellt (FIL1 breit … FIL3 schmal)").clicked() {
                        self.poll.cmdq.push_back((0, cat::set_mode_filter(model, m, k)));
                        self.poll.filter = Some((m, k));
                    }
                });
                ui.end_row();
            }
            let rows: [(Kind, &str, Vec<(u8, &str)>); 5] = [
                (Kind::Pre, "Vorverstärker", if hf { vec![(0, "aus"), (1, "P.AMP1"), (2, "P.AMP2")] } else { vec![(0, "aus"), (1, "an")] }),
                (Kind::Att, "Abschwächer", vec![(0x00, "aus"), (0x20, "20 dB")]),
                (Kind::Nb, "Störaustaster (NB)", vec![(0, "aus"), (1, "an")]),
                (Kind::Nr, "Rauschminderung (NR)", vec![(0, "aus"), (1, "an")]),
                (Kind::Agc, "AGC", vec![(1, "FAST"), (2, "MID"), (3, "SLOW")]),
            ];
            for (k, label, opts) in rows {
                let Some(cur) = c[k as usize] else { continue };
                ui.label(label);
                ui.horizontal(|ui| for (v, l) in opts {
                    if ui.selectable_label(cur == v, l).clicked() && cur != v { self.set_ctl(k, v); }
                });
                ui.end_row();
            }
        });
        ui.label(RichText::new("Für die Skimmer das breite Filter (FIL1) wählen.").small().weak());
    }

    /// Hinweis, wenn die PC-Uhr von der GPS-Zeit des Funkgeräts abweicht; Klick stellt sie
    pub(crate) fn ui_clock(&mut self, ui: &mut egui::Ui) {
        let Some(off) = self.gps.clock_offset else { return };
        MenuButton::new(RichText::new(format!("Uhr {off:+} s")).color(ORANGE)).ui(ui, |ui| {
            ui.label(format!("Die PC-Uhr geht laut GPS des Funkgeräts {} s {}.", off.abs(), if off > 0 { "nach" } else { "vor" }));
            ui.label(RichText::new("Das Logbuch nimmt ohnehin die GPS-Zeit. Für APRS und FT8 & Co. sollte die PC-Uhr aber stimmen. \
                Das Stellen braucht Administratorrechte – Windows fragt dann nach.").small().weak());
            if ui.button("PC-Uhr nach GPS stellen").clicked() { self.set_pc_clock(off); }
        }).0.on_hover_text("PC-Uhr weicht von der GPS-Zeit ab");
    }

    fn set_pc_clock(&mut self, off: i64) {
        #[cfg(windows)]
        {
            // erst direkt (falls das Programm schon Administratorrechte hat), sonst sich selbst mit Rechten starten
            if clock::set_offset(off).is_ok() { self.log(format!("PC-Uhr um {off:+} s gestellt")); self.gps.clock_offset = None; return; }
            let Ok(exe) = std::env::current_exe() else { return };
            let tx = self.ev_tx.clone();
            self.log("PC-Uhr: Windows fragt nach Administratorrechten …");
            std::thread::spawn(move || {
                let arg = format!("Start-Process -FilePath '{}' -ArgumentList '--uhr-stellen','{off}' -Verb RunAs -Wait", exe.display().to_string().replace('\'', "''"));
                use std::os::windows::process::CommandExt;
                let r = std::process::Command::new("powershell").args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &arg])
                    .creation_flags(0x0800_0000).status();
                let _ = tx.send(UiEv::Log(match r {
                    Ok(s) if s.success() => format!("PC-Uhr um {off:+} s gestellt"),
                    _ => "PC-Uhr nicht gestellt (abgebrochen oder keine Rechte)".into(),
                }));
            });
            self.gps.clock_offset = None; // neu messen
        }
        #[cfg(not(windows))]
        { let _ = off; self.log("PC-Uhr stellen geht nur unter Windows"); }
    }
}

/// Uhr stellen (Windows): auch als eigener Aufruf `rustwave.exe --uhr-stellen <Sekunden>` mit Administratorrechten
#[cfg(windows)]
pub(crate) mod clock {
    use chrono::{Datelike, Timelike};
    pub fn set_offset(off: i64) -> Result<(), String> {
        use windows::Win32::Foundation::SYSTEMTIME;
        use windows::Win32::System::SystemInformation::SetSystemTime;
        let t = chrono::Utc::now() + chrono::Duration::seconds(off);
        let st = SYSTEMTIME { wYear: t.year() as u16, wMonth: t.month() as u16, wDayOfWeek: 0, wDay: t.day() as u16,
            wHour: t.hour() as u16, wMinute: t.minute() as u16, wSecond: t.second() as u16, wMilliseconds: t.timestamp_subsec_millis() as u16 };
        unsafe { SetSystemTime(&st) }.map_err(|e| e.message().to_string())
    }
}
