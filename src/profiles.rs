//! Profile: alle Einstellungen unter einem Namen speichern und mit einem Klick laden
//! (z. B. „Zuhause“: IC-7300 per USB, „Portabel“: IC-705 als Access Point mit Touch-Bedienung).

use crate::App;
use eframe::egui::{self, RichText};
use std::collections::BTreeMap;
use std::path::PathBuf;

type Settings = BTreeMap<String, String>;

/// fängt `App::save` ab, damit Profile genau dieselben Schlüssel enthalten wie die normale Speicherung
#[derive(Default)]
struct MapStorage(Settings);
impl eframe::Storage for MapStorage {
    fn get_string(&self, key: &str) -> Option<String> { self.0.get(key).cloned() }
    fn set_string(&mut self, key: &str, value: String) { self.0.insert(key.to_string(), value); }
    fn flush(&mut self) {}
}

/// gehören nicht zum Profil: Sitzungsdaten
const SKIP: [&str; 3] = ["photo", "aprs_msgno", "profile"];

#[derive(Default)]
pub struct Profiles {
    pub all: BTreeMap<String, Settings>,
    pub name: String,
    pub active: String,
    path: PathBuf,
}

impl Profiles {
    pub fn load(path: PathBuf, active: String) -> Self {
        let all = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Profiles { all, name: active.clone(), active, path }
    }
    fn store(&self) -> Result<(), String> {
        if let Some(d) = self.path.parent() { std::fs::create_dir_all(d).map_err(|e| e.to_string())?; }
        std::fs::write(&self.path, serde_json::to_string_pretty(&self.all).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
}

impl App {
    fn current_settings(&mut self) -> Settings {
        let mut m = MapStorage::default();
        eframe::App::save(self, &mut m);
        m.0
    }

    pub(crate) fn profile_save(&mut self, name: &str) {
        let name = name.trim().to_string();
        if name.is_empty() { self.log("Profil: Namen eingeben"); return; }
        let mut s = self.current_settings();
        s.retain(|k, _| !SKIP.contains(&k.as_str()));
        self.profiles.all.insert(name.clone(), s);
        match self.profiles.store() {
            Ok(()) => { self.profiles.active = name.clone(); self.log(format!("Profil „{name}“ gespeichert")); }
            Err(e) => self.log(format!("Profil nicht gespeichert: {e}")),
        }
    }

    /// Profil laden: Verbindung trennen, Einstellungen übernehmen, wie im Profil wieder verbinden
    pub(crate) fn profile_load(&mut self, name: &str, ctx: &egui::Context) {
        if self.tx.is_some() || self.cw_tx.is_some() { self.log("Profil: erst nach dem Senden wechseln"); return; }
        let Some(p) = self.profiles.all.get(name).cloned() else { return };
        let cur = self.current_settings();
        let get = |k: &str| p.get(k).or_else(|| cur.get(k)).cloned(); // fehlende Schlüssel (ältere Profile): wie bisher
        self.reconnect = None;
        if self.rig.is_some() || self.net.is_some() { self.disconnect(); }
        self.input = None; // Audio aus, Empfangs-Thread endet
        self.apply_settings(&get);
        self.touch_applied = None; // Stil neu setzen
        self.profiles.active = name.to_string();
        self.log(format!("Profil „{name}“ geladen"));
        if get("audio_on").as_deref() == Some("1") && !self.use_net { self.start_audio(ctx); }
        if get("cat_on").as_deref() == Some("1") { self.connect(); }
    }

    pub(crate) fn ui_profiles(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(RichText::new("Alle Einstellungen (Funkgerät, Verbindung, Audio, Pegel, APRS, Darstellung) unter einem Namen. Ein Klick lädt das Profil und verbindet wie gespeichert.").small().weak());
        let names: Vec<String> = self.profiles.all.keys().cloned().collect();
        ui.horizontal_wrapped(|ui| {
            for n in &names {
                let active = *n == self.profiles.active;
                if ui.selectable_label(active, n).on_hover_text("dieses Profil laden").clicked() {
                    let n = n.clone();
                    self.profile_load(&n, ctx);
                    self.profiles.name = n;
                }
            }
            if names.is_empty() { ui.label(RichText::new("noch keine Profile").weak()); }
        });
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.profiles.name).desired_width(110.0).hint_text("z. B. Portabel"));
            let exists = self.profiles.all.contains_key(self.profiles.name.trim());
            if ui.button(if exists { "Überschreiben" } else { "Speichern" }).on_hover_text("aktuelle Einstellungen unter diesem Namen speichern").clicked() {
                let n = self.profiles.name.clone();
                self.profile_save(&n);
            }
            if exists && ui.button("Löschen").clicked() {
                let n = self.profiles.name.trim().to_string();
                self.profiles.all.remove(&n);
                if self.profiles.active == n { self.profiles.active.clear(); }
                if let Err(e) = self.profiles.store() { self.log(format!("Profil: {e}")); } else { self.log(format!("Profil „{n}“ gelöscht")); }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn speicher_ablage() {
        let mut m = MapStorage::default();
        eframe::Storage::set_string(&mut m, "a", "1".into());
        assert_eq!(eframe::Storage::get_string(&m, "a").as_deref(), Some("1"));
        let path = std::env::temp_dir().join("sstv_profile_test.json");
        let mut p = Profiles::load(path.clone(), String::new());
        p.all.insert("Portabel".into(), BTreeMap::from([("net_ap".to_string(), "1".to_string())]));
        p.store().unwrap();
        let q = Profiles::load(path, "Portabel".into());
        assert_eq!(q.all["Portabel"]["net_ap"], "1");
    }
}
