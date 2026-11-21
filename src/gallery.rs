//! SSTV-Galerie: alle empfangenen Bilder mit Suche (Rufzeichen, Modus), Zeitraum, Modus und Band.
//! Zu jedem Bild liegt eine kleine JSON-Datei mit Zeit, Modus, Frequenz und (per FSKID) Rufzeichen.
//! Ältere Bilder ohne JSON: Zeit und Modus aus dem Dateinamen.

use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
pub(crate) struct Meta { pub utc: i64, pub mode: String, pub freq: Option<u64>, pub call: Option<String> }

#[derive(Clone)]
pub(crate) struct Item { pub path: PathBuf, pub meta: Meta }

#[derive(Default)]
pub(crate) struct Gallery {
    items: Vec<Item>,
    pub stale: bool,
    loaded: bool,
    search: String,
    days: u32,
    mode: String,
    band: String,
    thumbs: HashMap<PathBuf, TextureHandle>,
    sel: Option<PathBuf>,
    big: Option<(PathBuf, TextureHandle)>,
}

pub(crate) fn meta_path(png: &Path) -> PathBuf { png.with_extension("json") }

/// Metadaten lesen; ohne JSON aus dem Dateinamen „sstv_JJJJMMTT_hhmmss_Modus.png“ (Ortszeit)
pub(crate) fn read_meta(png: &Path) -> Option<Meta> {
    if let Some(m) = std::fs::read_to_string(meta_path(png)).ok().and_then(|s| serde_json::from_str(&s).ok()) { return Some(m); }
    let stem = png.file_stem()?.to_str()?;
    let mut p = stem.strip_prefix("sstv_")?.splitn(3, '_');
    let (d, t, mode) = (p.next()?, p.next()?, p.next().unwrap_or(""));
    let dt = chrono::NaiveDateTime::parse_from_str(&format!("{d}{t}"), "%Y%m%d%H%M%S").ok()?;
    let utc = dt.and_local_timezone(chrono::Local).single()?.timestamp();
    // Dateiname ohne Leerzeichen („Robot36“) -> Name wie in neuen Bildern („Robot 36“)
    let mode = crate::sstv::MODES.iter().find(|m| m.name.replace(' ', "") == mode).map_or(mode.to_string(), |m| m.name.to_string());
    Some(Meta { utc, mode, freq: None, call: None })
}

pub(crate) fn write_meta(png: &Path, m: &Meta) -> Result<(), String> {
    std::fs::write(meta_path(png), serde_json::to_string_pretty(m).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn band_name(f: u64) -> &'static str { crate::scope::BANDS.iter().find(|b| (b.1..=b.2).contains(&f)).map_or("", |b| b.0) }

fn scan(dir: &Path) -> Vec<Item> {
    let mut v: Vec<Item> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "png") && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("sstv_")))
        .filter_map(|p| read_meta(&p).map(|meta| Item { path: p, meta })).collect();
    v.sort_by_key(|i| -i.meta.utc);
    v
}

fn thumb(path: &Path) -> Option<ColorImage> {
    let img = image::open(path).ok()?.thumbnail(160, 128).to_rgb8();
    Some(ColorImage::from_rgb([img.width() as usize, img.height() as usize], img.as_raw()))
}

impl App {
    pub(crate) fn ui_gallery(&mut self, ctx: &egui::Context) {
        let g = &mut self.gal;
        if !g.loaded || g.stale { g.items = scan(&self.save_dir); g.loaded = true; g.stale = false; }
        let now = chrono::Utc::now().timestamp();
        let q = g.search.trim().to_uppercase();
        let shown: Vec<Item> = g.items.iter().filter(|i| {
            let m = &i.meta;
            (g.days == 0 || now - m.utc <= g.days as i64 * 86400)
                && (g.mode.is_empty() || m.mode == g.mode)
                && (g.band.is_empty() || m.freq.is_some_and(|f| band_name(f) == g.band))
                && (q.is_empty() || m.call.as_deref().is_some_and(|c| c.contains(&q)) || m.mode.to_uppercase().contains(&q))
        }).cloned().collect();
        let mut modes: Vec<String> = g.items.iter().map(|i| i.meta.mode.clone()).filter(|m| !m.is_empty()).collect();
        modes.sort(); modes.dedup();
        let mut bands: Vec<&str> = g.items.iter().filter_map(|i| i.meta.freq.map(band_name)).filter(|b| !b.is_empty()).collect();
        bands.sort(); bands.dedup();

        let mut pick_pip = None;
        egui::TopBottomPanel::top("gal_bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{} von {} Bildern", shown.len(), g.items.len())).strong());
                ui.separator();
                ui.add(egui::TextEdit::singleline(&mut g.search).desired_width(140.0).hint_text("Rufzeichen oder Modus"));
                for (d, l) in [(1, "Heute"), (7, "7 Tage"), (30, "30 Tage"), (0, "Alle")] { ui.selectable_value(&mut g.days, d, l); }
                egui::ComboBox::from_id_salt("gal_mode").selected_text(if g.mode.is_empty() { "alle Modi" } else { g.mode.as_str() }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut g.mode, String::new(), "alle Modi");
                    for m in &modes { ui.selectable_value(&mut g.mode, m.clone(), m); }
                });
                egui::ComboBox::from_id_salt("gal_band").selected_text(if g.band.is_empty() { "alle Bänder" } else { g.band.as_str() }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut g.band, String::new(), "alle Bänder");
                    for b in &bands { ui.selectable_value(&mut g.band, b.to_string(), *b); }
                });
                if ui.button("⟳").on_hover_text("neu einlesen").clicked() { g.stale = true; }
                if ui.button("Ordner öffnen").clicked() { open_path(&self.save_dir); }
            });
        });
        // Vorschau des gewählten Bildes
        if let Some(sel) = g.sel.clone() {
            if let Some(it) = g.items.iter().find(|i| i.path == sel) {
                egui::SidePanel::right("gal_big").resizable(true).default_width(420.0).show(ctx, |ui| {
                    if g.big.as_ref().is_none_or(|b| b.0 != sel) {
                        if let Ok(img) = image::open(&sel) {
                            let img = img.to_rgb8();
                            let ci = ColorImage::from_rgb([img.width() as usize, img.height() as usize], img.as_raw());
                            g.big = Some((sel.clone(), ctx.load_texture("gal_big", ci, TextureOptions::LINEAR)));
                        }
                    }
                    if let Some((_, t)) = &g.big {
                        let w = ui.available_width();
                        let s = t.size_vec2();
                        ui.add(egui::Image::new(t).fit_to_exact_size(egui::vec2(w, w * s.y / s.x)));
                    }
                    let m = &it.meta;
                    egui::Grid::new("gal_meta").num_columns(2).show(ui, |ui| {
                        let t = chrono::DateTime::<chrono::Utc>::from_timestamp(m.utc, 0).map_or(String::new(), |d| d.with_timezone(&chrono::Local).format("%d.%m.%Y %H:%M:%S").to_string());
                        for (k, v) in [("Zeit", t), ("Modus", m.mode.clone()),
                                       ("Frequenz", m.freq.map_or("–".into(), |f| format!("{} {}", crate::rigbar::fmt_freq(f), band_name(f)))),
                                       ("Rufzeichen", m.call.clone().unwrap_or("– (kein FSKID)".into()))] {
                            ui.label(RichText::new(k).weak()); ui.label(v); ui.end_row();
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Als Bild im Bild").on_hover_text("für deine Antwort in der Vorlage verwenden").clicked() { pick_pip = Some(sel.clone()); }
                        if ui.button("Datei öffnen").clicked() { open_path(&sel); }
                        if ui.button("Schließen").clicked() { g.sel = None; }
                    });
                });
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            if shown.is_empty() {
                ui.label(RichText::new(if g.items.is_empty() { "Noch keine empfangenen SSTV-Bilder." } else { "Keine Bilder für diese Auswahl." }).weak());
                return;
            }
            // Vorschaubilder nach und nach laden, damit die Oberfläche flüssig bleibt
            let mut budget = 6;
            let cols = ((ui.available_width() + 8.0) / 172.0).floor().max(1.0) as usize;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("gal_grid").spacing([8.0, 10.0]).show(ui, |ui| {
                    for (n, it) in shown.iter().enumerate() {
                        if n > 0 && n % cols == 0 { ui.end_row(); }
                        if !g.thumbs.contains_key(&it.path) && budget > 0 {
                            budget -= 1;
                            if let Some(ci) = thumb(&it.path) { g.thumbs.insert(it.path.clone(), ctx.load_texture(it.path.to_string_lossy(), ci, TextureOptions::LINEAR)); }
                        }
                        ui.vertical(|ui| {
                            ui.set_width(164.0);
                            let resp = match g.thumbs.get(&it.path) {
                                Some(t) => ui.add(egui::Image::new(t).fit_to_exact_size(egui::vec2(160.0, 128.0)).sense(egui::Sense::click())),
                                None => ui.add(egui::Button::new("…").min_size(egui::vec2(160.0, 128.0))),
                            };
                            if g.sel.as_ref() == Some(&it.path) { ui.painter().rect_stroke(resp.rect.expand(2.0), 2.0, egui::Stroke::new(2.0_f32, ORANGE), egui::StrokeKind::Outside); }
                            if resp.clicked() { g.sel = Some(it.path.clone()); }
                            let m = &it.meta;
                            let t = chrono::DateTime::<chrono::Utc>::from_timestamp(m.utc, 0).map_or(String::new(), |d| d.with_timezone(&chrono::Local).format("%d.%m. %H:%M").to_string());
                            ui.label(RichText::new(format!("{t} · {}", m.mode)).small());
                            let sub = [m.call.clone(), m.freq.map(|f| band_name(f).to_string()).filter(|b| !b.is_empty())].into_iter().flatten().collect::<Vec<_>>().join(" · ");
                            if !sub.is_empty() { ui.label(RichText::new(sub).small().strong()); }
                        });
                    }
                });
            });
            if budget == 0 { ctx.request_repaint(); }
        });
        if let Some(p) = pick_pip {
            if let Ok(img) = image::open(&p) {
                self.received = Some(img.to_rgb8());
                self.received_ver += 1;
                self.log("Bild aus der Galerie als „Bild im Bild“ bereit");
            }
        }
    }

    /// nach dem Speichern eines empfangenen Bildes: Metadaten schreiben und ggf. in den Kopierordner
    pub(crate) fn gallery_stored(&mut self, png: &Path, mode: &str) {
        let m = Meta { utc: chrono::Utc::now().timestamp(), mode: mode.to_string(), freq: self.freq_shown, call: None };
        if let Err(e) = write_meta(png, &m) { self.log(format!("Galerie: Metadaten nicht gespeichert ({e})")); }
        self.last_rx_img = Some((png.to_path_buf(), Instant::now()));
        self.gal.stale = true;
        let dir = self.copy_dir.trim().to_string();
        if !dir.is_empty() {
            let to = Path::new(&dir).join(png.file_name().unwrap_or_default());
            match std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(png, &to)) {
                Ok(_) => {}
                Err(e) => self.log(format!("Kopie nach {dir} fehlgeschlagen: {e}")),
            }
        }
    }

    /// FSKID kurz nach einem Bild: Rufzeichen in dessen Metadaten eintragen
    pub(crate) fn gallery_fskid(&mut self, call: &str) {
        let Some((p, t)) = self.last_rx_img.clone() else { return };
        if t.elapsed() > Duration::from_secs(20) { return; }
        if let Some(mut m) = read_meta(&p) {
            m.call = Some(call.to_string());
            let _ = write_meta(&p, &m);
            self.gal.stale = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadaten_und_dateiname() {
        let d = std::env::temp_dir().join(format!("sstv_gal_test_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        // altes Bild ohne JSON: aus dem Dateinamen
        let old = d.join("sstv_20261117_143012_Robot36.png");
        std::fs::write(&old, b"x").unwrap();
        let m = read_meta(&old).unwrap();
        assert_eq!(m.mode, "Robot 36");
        let local = chrono::DateTime::<chrono::Utc>::from_timestamp(m.utc, 0).unwrap().with_timezone(&chrono::Local);
        assert_eq!(local.format("%Y%m%d %H%M%S").to_string(), "20261117 143012");
        // mit JSON: Frequenz und Rufzeichen
        let new = d.join("sstv_20261117_150000_MartinM1.png");
        std::fs::write(&new, b"x").unwrap();
        write_meta(&new, &Meta { utc: 1_800_000_000, mode: "Martin M1".into(), freq: Some(14_230_000), call: Some("DL1ABC".into()) }).unwrap();
        let items = scan(&d);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].meta.call.as_deref(), Some("DL1ABC")); // neuestes zuerst
        assert_eq!(band_name(14_230_000), "20 m");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
