//! Reiter „Logbuch“: Liste, Suche, Bearbeiten, ADIF-Export.

use super::*;

impl App {
    // ================= Reiter „Logbuch“ =================
    pub(crate) fn ui_logbook(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(format!("Logbuch – {} QSOs", self.logbook.qsos.len()));
            ui.label("Suche");
            ui.add(egui::TextEdit::singleline(&mut self.log_filter).desired_width(120.0).hint_text("Rufzeichen"));
            if ui.button("ADIF exportieren").on_hover_text("für Logprogramme, LoTW, eQSL").clicked() { self.export_adif(); }
            if let Some((i, q)) = self.log_undo.clone() {
                if ui.button(format!("Löschen von {} rückgängig", q.call)).clicked() {
                    self.logbook.qsos.insert(i.min(self.logbook.qsos.len()), q);
                    let _ = self.logbook.save();
                    self.log_undo = None;
                }
            }
        });
        let filter = self.log_filter.trim().to_uppercase();
        let mut edit = None;
        let mut delete = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("log").striped(true).num_columns(11).spacing([14.0, 4.0]).show(ui, |ui| {
                for h in ["Datum (UTC)", "Zeit", "Rufzeichen", "Name", "QTH", "RST ges.", "RST empf.", "Band", "Modus", "Notiz", ""] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for (i, q) in self.logbook.qsos.iter().enumerate().rev() {
                    if !filter.is_empty() && !q.call.contains(&filter) { continue; }
                    let t = chrono::DateTime::<chrono::Utc>::from_timestamp(q.start, 0).unwrap_or_default();
                    ui.label(t.format("%d.%m.%Y").to_string());
                    ui.label(t.format("%H:%M").to_string());
                    ui.label(RichText::new(&q.call).monospace().strong());
                    ui.label(&q.name);
                    ui.label(&q.qth);
                    ui.label(&q.rsv_sent);
                    ui.label(&q.rsv_rcvd);
                    ui.label(q.freq_hz.map_or("", logbook::band));
                    ui.label(format!("{} {}", q.mode, q.sstv_mode));
                    ui.label(&q.note);
                    ui.horizontal(|ui| {
                        if ui.small_button("✏").on_hover_text("In die QSO-Leiste laden und bearbeiten").clicked() { edit = Some(i); }
                        if ui.small_button("🗑").on_hover_text("Löschen").clicked() { delete = Some(i); }
                    });
                    ui.end_row();
                }
            });
        });
        if let Some(i) = edit {
            self.qso = self.logbook.qsos[i].clone();
            self.known = self.logbook.lookup(&self.qso.call);
            self.logged = true;
            self.tab = Tab::Operate;
        }
        if let Some(i) = delete {
            let q = self.logbook.qsos.remove(i);
            let _ = self.logbook.save();
            self.log_undo = Some((i, q));
        }
    }
    pub(crate) fn export_adif(&mut self) {
        #[cfg(windows)]
        let target = rfd::FileDialog::new().set_file_name("sstv_log.adi").add_filter("ADIF", &["adi"]).save_file();
        #[cfg(not(windows))]
        let target = Some(self.save_dir.join("sstv_log.adi"));
        if let Some(p) = target {
            match self.logbook.export_adif(&p) {
                Ok(n) => self.log(format!("{n} QSOs als ADIF gespeichert: {}", p.display())),
                Err(e) => self.log(format!("ADIF-Export fehlgeschlagen: {e}")),
            }
        }
    }
}
