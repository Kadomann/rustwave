//! Reiter „Vorlagen“: Vorlagen verwalten, Ebenen bearbeiten, direkt im Bild ziehen/skalieren/drehen.

use crate::template::{self, Align, Fill, Fit, Kind, Layer, Phase, Rgba, Source, Template};
use crate::{App, ORANGE};
use eframe::egui::{self, pos2, vec2, Color32, Pos2, RichText, Sense, Stroke};

#[derive(Clone, Copy, PartialEq)]
enum DragKind { Move, Scale, Rotate }

struct Drag { kind: DragKind, start: [f32; 2], layer0: Layer }

pub struct EdState {
    pub sel_layer: Option<usize>,
    drag: Option<Drag>,
    /// Vorschau mit Beispieldaten, wenn kein QSO läuft
    pub sample: bool,
}
impl Default for EdState {
    fn default() -> Self { EdState { sel_layer: None, drag: None, sample: true } }
}

const HANDLE: f32 = 7.0; // Griffradius in Bildschirmpixeln

fn new_layer(name: &str, kind: Kind, x: f32, y: f32, w: f32, h: f32) -> Layer {
    Layer { name: name.into(), kind, x, y, w, h, rotation: 0.0, keystone: 0.0, opacity: 1.0, visible: true }
}
fn kind_icon(k: &Kind) -> &'static str {
    match k {
        Kind::Text { .. } => "T",
        Kind::Picture { source: Source::Photo, .. } => "📷",
        Kind::Picture { source: Source::Received, .. } => "📥",
        Kind::Picture { .. } => "🖼",
        Kind::Rect { .. } => "▭",
    }
}

/// Farbe bearbeiten (mit Deckkraft)
fn color(ui: &mut egui::Ui, c: &mut Rgba) -> bool { ui.color_edit_button_srgba_unmultiplied(&mut c.0).changed() }

fn ui_fill(ui: &mut egui::Ui, fill: &mut Fill) -> bool {
    let mut ch = false;
    ui.horizontal_wrapped(|ui| {
        let grad = matches!(fill, Fill::Gradient { .. });
        if ui.selectable_label(!grad, "Einfarbig").clicked() && grad {
            if let Fill::Gradient { a, .. } = *fill { *fill = Fill::Solid(a); ch = true; }
        }
        if ui.selectable_label(grad, "Verlauf").clicked() && !grad {
            if let Fill::Solid(a) = *fill { *fill = Fill::Gradient { a, b: Rgba([255, 140, 0, 255]), angle: 90.0 }; ch = true; }
        }
    });
    ui.horizontal(|ui| match fill {
        Fill::Solid(c) => { ch |= color(ui, c); }
        Fill::Gradient { a, b, angle } => {
            ch |= color(ui, a);
            ui.label("→");
            ch |= color(ui, b);
            ch |= ui.add(egui::Slider::new(angle, 0.0..=360.0).suffix("°").text("Richtung")).changed();
        }
    });
    ch
}

/// optionaler Rand/Schatten: Haken, Farbe, Stärke
fn ui_opt(ui: &mut egui::Ui, label: &str, v: &mut Option<(Rgba, f32)>, default: (Rgba, f32), max: f32, unit: &str) -> bool {
    let mut ch = false;
    ui.horizontal_wrapped(|ui| {
        let mut on = v.is_some();
        if ui.checkbox(&mut on, label).changed() { *v = if on { Some(default) } else { None }; ch = true; }
        if let Some((c, w)) = v {
            ch |= color(ui, c);
            ch |= ui.add(egui::Slider::new(w, 0.0..=max).text(unit)).changed();
        }
    });
    ch
}

impl App {
    pub(crate) fn add_layer(&mut self, l: Layer) {
        let t = &mut self.templates[self.sel_tpl];
        t.layers.push(l);
        self.ed.sel_layer = Some(t.layers.len() - 1);
        self.tpl_changed();
    }
    pub(crate) fn add_file_layer(&mut self, path: String) {
        let name = std::path::Path::new(&path).file_name().map_or("Bild".into(), |n| n.to_string_lossy().to_string());
        self.add_layer(new_layer(&name, Kind::Picture { source: Source::File, path, fit: Fit::Contain, border: None }, 0.5, 0.5, 0.5, 0.5));
    }

    pub(crate) fn ui_editor(&mut self, ctx: &egui::Context) {
        if self.ed.sel_layer.is_some_and(|i| i >= self.templates[self.sel_tpl].layers.len()) { self.ed.sel_layer = None; }
        // kleine Bildschirme: schmalere Seitenleisten, damit die Arbeitsfläche Platz behält
        let compact = ctx.content_rect().width() < 1150.0;
        let (lw, rw) = if compact { (190.0, 250.0) } else { (250.0, 330.0) };
        egui::SidePanel::left("ed_left").exact_width(lw).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.ui_templates_list(ui));
        });
        egui::SidePanel::right("ed_props").exact_width(rw).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.ui_props(ui));
        });
        egui::CentralPanel::default().show(ctx, |ui| self.ui_canvas(ui));
        // Tastatur, solange kein Textfeld den Fokus hat
        if ctx.memory(|m| m.focused().is_none()) {
            if let Some(li) = self.ed.sel_layer {
                let m = &crate::sstv::MODES[self.tx_mode];
                let (dx, dy, del) = ctx.input(|i| {
                    let step = if i.modifiers.shift { 10.0 } else { 1.0 };
                    let mut d = (0.0, 0.0);
                    if i.key_pressed(egui::Key::ArrowLeft) { d.0 -= step; }
                    if i.key_pressed(egui::Key::ArrowRight) { d.0 += step; }
                    if i.key_pressed(egui::Key::ArrowUp) { d.1 -= step; }
                    if i.key_pressed(egui::Key::ArrowDown) { d.1 += step; }
                    (d.0, d.1, i.key_pressed(egui::Key::Delete))
                });
                if del {
                    self.templates[self.sel_tpl].layers.remove(li);
                    self.ed.sel_layer = None;
                    self.tpl_changed();
                } else if dx != 0.0 || dy != 0.0 {
                    let l = &mut self.templates[self.sel_tpl].layers[li];
                    l.x += dx / m.w as f32;
                    l.y += dy / m.h as f32;
                    self.tpl_changed();
                }
            }
        }
    }

    fn ui_templates_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Vorlagen");
        let mut pick = None;
        for (i, t) in self.templates.iter().enumerate() {
            let tag = t.phase.map_or(String::new(), |p| format!("[{}] ", p.label()));
            if ui.selectable_label(i == self.sel_tpl, format!("{tag}{}", t.name)).clicked() { pick = Some(i); }
        }
        if let Some(i) = pick { self.select_template(i); self.ed.sel_layer = None; }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Neu").clicked() {
                self.templates.push(Template { name: "Neue Vorlage".into(), phase: None, layers: vec![
                    new_layer("Sendefoto", Kind::Picture { source: Source::Photo, path: String::new(), fit: Fit::Cover, border: None }, 0.5, 0.5, 1.0, 1.0)] });
                self.select_template(self.templates.len() - 1);
                self.ed.sel_layer = None;
                self.tpl_changed();
            }
            if ui.button("Duplizieren").clicked() {
                let mut t = self.templates[self.sel_tpl].clone();
                t.name += " (Kopie)";
                t.phase = None; // jede Phase nur einmal
                self.templates.insert(self.sel_tpl + 1, t);
                self.select_template(self.sel_tpl + 1);
                self.tpl_changed();
            }
            if ui.add_enabled(self.templates.len() > 1, egui::Button::new("Löschen")).clicked() {
                self.templates.remove(self.sel_tpl);
                self.select_template(self.sel_tpl.saturating_sub(1));
                self.ed.sel_layer = None;
                self.tpl_changed();
            }
        });
        if ui.button("Standardvorlagen hinzufügen").on_hover_text("Fügt die mitgelieferten Vorlagen erneut hinzu (ohne Phasenzuordnung, falls belegt)").clicked() {
            for mut t in template::defaults() {
                if t.phase.is_some_and(|p| self.templates.iter().any(|x| x.phase == Some(p))) { t.phase = None; }
                self.templates.push(t);
            }
            self.tpl_changed();
        }
        ui.separator();
        let mut ch = false;
        let mut phase_set = None;
        {
            let t = &mut self.templates[self.sel_tpl];
            ui.horizontal_wrapped(|ui| { ui.label("Name"); ch |= ui.text_edit_singleline(&mut t.name).changed(); });
            ui.horizontal_wrapped(|ui| {
                ui.label("Phase");
                let mut p = t.phase;
                egui::ComboBox::from_id_salt("phase").selected_text(p.map_or("keine (nur Ablage)", |p| p.label())).show_ui(ui, |ui| {
                    ui.selectable_value(&mut p, None, "keine (nur Ablage)");
                    for x in Phase::ALL { ui.selectable_value(&mut p, Some(x), x.label()); }
                });
                if p != t.phase { phase_set = Some(p); }
            });
        }
        if let Some(p) = phase_set {
            // jede Phase gehört genau einer Vorlage: bisherige Zuordnung lösen
            if let Some(p) = p { for t in self.templates.iter_mut() { if t.phase == Some(p) { t.phase = None; } } }
            self.templates[self.sel_tpl].phase = p;
            if let Some(p) = p { self.phase = p; }
            ch = true;
        }
        if ch { self.tpl_changed(); }

        ui.separator();
        ui.heading("Ebenen");
        ui.label(RichText::new("oben = vorne").small().weak());
        let n = self.templates[self.sel_tpl].layers.len();
        let mut sel = self.ed.sel_layer;
        let mut vis_ch = false;
        for i in (0..n).rev() {
            let l = &mut self.templates[self.sel_tpl].layers[i];
            ui.horizontal_wrapped(|ui| {
                vis_ch |= ui.checkbox(&mut l.visible, "").on_hover_text("sichtbar").changed();
                if ui.selectable_label(sel == Some(i), format!("{}  {}", kind_icon(&l.kind), l.name)).clicked() { sel = Some(i); }
            });
        }
        self.ed.sel_layer = sel;
        if vis_ch { self.tpl_changed(); }
        ui.horizontal_wrapped(|ui| {
            if ui.button("+ Text").clicked() {
                let mut l = new_layer("Text", Kind::Text { text: "%c de %m".into(), font: 0, size: 0.1, fill: Fill::Solid(Rgba::WHITE),
                    outline: Some((Rgba::BLACK, 0.08)), shadow: None, align: Align::Center }, 0.5, 0.5, 0.0, 0.0);
                l.name = "Text".into();
                self.add_layer(l);
            }
            if ui.button("+ Sendefoto").clicked() {
                self.add_layer(new_layer("Sendefoto", Kind::Picture { source: Source::Photo, path: String::new(), fit: Fit::Cover, border: None }, 0.5, 0.5, 1.0, 1.0));
            }
            if ui.button("+ Empfangsbild").on_hover_text("Bild im Bild: zuletzt empfangenes bzw. in der Galerie markiertes Bild").clicked() {
                self.add_layer(new_layer("Empfangsbild", Kind::Picture { source: Source::Received, path: String::new(), fit: Fit::Cover,
                    border: Some((Rgba::WHITE, 0.008)) }, 0.75, 0.4, 0.38, 0.38));
            }
            #[cfg(windows)]
            if ui.button("+ Bilddatei").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("Bilder", &["png", "jpg", "jpeg", "bmp"]).pick_file() { self.add_file_layer(p.to_string_lossy().into()); }
            }
            if ui.button("+ Rechteck").clicked() {
                self.add_layer(new_layer("Rechteck", Kind::Rect { fill: Fill::Solid(Rgba([0, 0, 0, 160])), border: None, radius: 0.02 }, 0.5, 0.5, 0.6, 0.2));
            }
        });
        ui.label(RichText::new("Bilddateien auch einfach ins Fenster ziehen").small().weak());
        if let Some(li) = self.ed.sel_layer {
            ui.horizontal_wrapped(|ui| {
                let layers = &mut self.templates[self.sel_tpl].layers;
                let mut ch = false;
                if ui.add_enabled(li + 1 < layers.len(), egui::Button::new("⬆ nach vorn")).clicked() { layers.swap(li, li + 1); self.ed.sel_layer = Some(li + 1); ch = true; }
                if ui.add_enabled(li > 0, egui::Button::new("⬇ nach hinten")).clicked() { layers.swap(li, li - 1); self.ed.sel_layer = Some(li - 1); ch = true; }
                if ui.button("Kopie").on_hover_text("Ebene duplizieren").clicked() {
                    let mut c = layers[li].clone();
                    c.x += 0.03; c.y += 0.03;
                    layers.insert(li + 1, c);
                    self.ed.sel_layer = Some(li + 1);
                    ch = true;
                }
                if ui.button("🗑").on_hover_text("Löschen (Entf)").clicked() { layers.remove(li); self.ed.sel_layer = None; ch = true; }
                if ch { self.tpl_changed(); }
            });
        }
    }

    fn ui_canvas(&mut self, ui: &mut egui::Ui) {
        let m = &crate::sstv::MODES[self.tx_mode];
        ui.horizontal_wrapped(|ui| {
            ui.heading(&self.templates[self.sel_tpl].name);
            ui.label(RichText::new(format!("{} · {}×{}", m.name, m.w, m.h)).weak());
            ui.checkbox(&mut self.ed.sample, "Beispieldaten, wo QSO-Felder leer sind");
        });
        ui.label(RichText::new("Klicken wählt eine Ebene · Ziehen verschiebt · runder Griff unten rechts skaliert · Griff oben dreht (Umschalt: 15°-Schritte) · Pfeiltasten, Entf").small().weak());
        let Some(tex) = self.tx_tex.clone() else { return };
        // Touch-Bedienung (große Klickflächen): Griffe doppelt so groß
        let hr = if ui.spacing().interact_size.y >= 30.0 { 2.0 * HANDLE } else { HANDLE };
        let (w, h) = (m.w as f32, m.h as f32);
        let avail = ui.available_size();
        let scale = (avail.x / w).min((avail.y - 10.0) / h).max(0.2);
        let (resp, painter) = ui.allocate_painter(vec2(w * scale, h * scale), Sense::click_and_drag());
        let rect = resp.rect;
        painter.image(tex.id(), rect, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        let to_screen = |p: [f32; 2]| pos2(rect.min.x + p[0] * scale, rect.min.y + p[1] * scale);
        let to_img = |p: Pos2| [(p.x - rect.min.x) / scale, (p.y - rect.min.y) / scale];

        // Griffe der gewählten Ebene (in Bildpixeln)
        let handles = |q: &template::Quad| {
            let c = [(q[0][0] + q[2][0]) / 2.0, (q[0][1] + q[2][1]) / 2.0];
            let top = [(q[0][0] + q[1][0]) / 2.0, (q[0][1] + q[1][1]) / 2.0];
            let d = [top[0] - c[0], top[1] - c[1]];
            let len = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-3);
            let rot = [top[0] + d[0] / len * 22.0 / scale, top[1] + d[1] / len * 22.0 / scale];
            (q[2], rot, top)
        };
        let sel_quad = self.ed.sel_layer.and_then(|i| self.quads.get(i).copied().flatten());

        if resp.drag_started() || resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let p = to_img(pos);
                let near = |a: [f32; 2]| ((a[0] - p[0]).powi(2) + (a[1] - p[1]).powi(2)).sqrt() * scale <= hr + 3.0;
                let mut kind = None;
                if let Some(q) = sel_quad {
                    let (sc, rot, _) = handles(&q);
                    if near(sc) { kind = Some(DragKind::Scale); } else if near(rot) { kind = Some(DragKind::Rotate); }
                }
                if kind.is_none() {
                    // oberste Ebene unter dem Mauszeiger
                    let hit = (0..self.quads.len()).rev().find(|&i| self.quads[i].is_some_and(|q| template::quad_contains(&q, p)));
                    self.ed.sel_layer = hit;
                    if hit.is_some() { kind = Some(DragKind::Move); }
                }
                if let (Some(kind), Some(li), true) = (kind, self.ed.sel_layer, resp.drag_started()) {
                    self.ed.drag = Some(Drag { kind, start: p, layer0: self.templates[self.sel_tpl].layers[li].clone() });
                }
            }
        }
        if resp.dragged() {
            if let (Some(d), Some(li), Some(pos)) = (&self.ed.drag, self.ed.sel_layer, resp.interact_pointer_pos()) {
                let p = to_img(pos);
                let l0 = &d.layer0;
                let c = [l0.x * w, l0.y * h];
                let shift = ui.input(|i| i.modifiers.shift);
                let mut l = l0.clone();
                match d.kind {
                    DragKind::Move => { l.x = l0.x + (p[0] - d.start[0]) / w; l.y = l0.y + (p[1] - d.start[1]) / h; }
                    DragKind::Scale => {
                        let dist = |a: [f32; 2]| ((a[0] - c[0]).powi(2) + (a[1] - c[1]).powi(2)).sqrt();
                        let f = (dist(p) / dist(d.start).max(1.0)).clamp(0.05, 20.0);
                        match &mut l.kind {
                            Kind::Text { size, .. } => { if let Kind::Text { size: s0, .. } = &l0.kind { *size = (s0 * f).clamp(0.01, 1.0); } }
                            _ => { l.w = (l0.w * f).clamp(0.01, 3.0); l.h = (l0.h * f).clamp(0.01, 3.0); }
                        }
                    }
                    DragKind::Rotate => {
                        let ang = |a: [f32; 2]| (a[1] - c[1]).atan2(a[0] - c[0]).to_degrees();
                        let mut r = l0.rotation + ang(p) - ang(d.start);
                        r = (r + 180.0).rem_euclid(360.0) - 180.0;
                        if shift { r = (r / 15.0).round() * 15.0; }
                        l.rotation = r;
                    }
                }
                self.templates[self.sel_tpl].layers[li] = l;
                self.tpl_changed();
            }
        }
        if resp.drag_stopped() { self.ed.drag = None; }

        // Auswahl zeichnen
        if let Some(q) = sel_quad {
            let pts: Vec<Pos2> = q.iter().map(|&p| to_screen(p)).collect();
            painter.add(egui::Shape::closed_line(pts, Stroke::new(1.5_f32, ORANGE)));
            let (sc, rot, top) = handles(&q);
            painter.line_segment([to_screen(top), to_screen(rot)], Stroke::new(1.0_f32, ORANGE));
            painter.circle(to_screen(sc), hr, Color32::WHITE, Stroke::new(1.5_f32, ORANGE));
            painter.circle(to_screen(rot), hr, ORANGE, Stroke::new(1.5_f32, Color32::WHITE));
        }
    }

    fn ui_props(&mut self, ui: &mut egui::Ui) {
        ui.heading("Eigenschaften");
        let Some(li) = self.ed.sel_layer else {
            ui.label("Ebene im Bild anklicken oder links in der Liste wählen.");
            ui.add_space(10.0);
            ui.label(RichText::new("Makros für Texte").strong());
            for (k, d) in template::MACROS { ui.label(format!("%{k}  {d}")); }
            return;
        };
        let mut ch = false;
        let mut pick_file = false;
        {
            let l = &mut self.templates[self.sel_tpl].layers[li];
            ui.horizontal_wrapped(|ui| { ui.label("Name"); ch |= ui.text_edit_singleline(&mut l.name).changed(); });
            ui.separator();
            match &mut l.kind {
                Kind::Text { text, font, size, fill, outline, shadow, align } => {
                    ui.label("Text (Zeilenumbruch mit Enter)");
                    ch |= ui.add(egui::TextEdit::multiline(text).desired_rows(3).desired_width(f32::INFINITY)).changed();
                    ui.label(RichText::new("Makro anhängen:").small());
                    ui.horizontal_wrapped(|ui| {
                        for (k, d) in template::MACROS {
                            if ui.small_button(format!("%{k}")).on_hover_text(d).clicked() { text.push('%'); text.push(k); ch = true; }
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Schrift");
                        egui::ComboBox::from_id_salt("font").selected_text(template::FONTS[*font].0).show_ui(ui, |ui| {
                            for (i, f) in template::FONTS.iter().enumerate() { ch |= ui.selectable_value(font, i, f.0).changed(); }
                        });
                    });
                    ch |= ui.add(egui::Slider::new(size, 0.02..=0.5).text("Größe")).changed();
                    ui.horizontal_wrapped(|ui| {
                        for (a, t) in [(Align::Left, "links"), (Align::Center, "mittig"), (Align::Right, "rechts")] {
                            ch |= ui.selectable_value(align, a, t).changed();
                        }
                    });
                    ui.label(RichText::new("Füllung").strong());
                    ch |= ui_fill(ui, fill);
                    ch |= ui_opt(ui, "Rand", outline, (Rgba::BLACK, 0.08), 0.3, "Stärke");
                    ch |= ui_opt(ui, "Schatten", shadow, (Rgba([0, 0, 0, 160]), 0.06), 0.3, "Versatz");
                }
                Kind::Picture { source, path, fit, border } => {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Quelle");
                        for (s, t) in [(Source::Photo, "Sendefoto"), (Source::Received, "Empfangsbild"), (Source::File, "Datei")] {
                            ch |= ui.selectable_value(source, s, t).changed();
                        }
                    });
                    if *source == Source::File {
                        ui.add(egui::Label::new(RichText::new(if path.is_empty() { "(keine Datei)" } else { path.as_str() }).small()).truncate());
                        if cfg!(windows) && ui.button("Datei wählen").clicked() { pick_file = true; }
                    }
                    if *source == Source::Received { ui.label(RichText::new("Klick auf ein Galeriebild im Reiter Betrieb wählt es aus").small().weak()); }
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Einpassen");
                        for (f, t) in [(Fit::Cover, "füllen"), (Fit::Contain, "ganz zeigen"), (Fit::Stretch, "strecken")] {
                            ch |= ui.selectable_value(fit, f, t).changed();
                        }
                    });
                    ch |= ui_opt(ui, "Rahmen", border, (Rgba::WHITE, 0.008), 0.05, "Stärke");
                }
                Kind::Rect { fill, border, radius } => {
                    ui.label(RichText::new("Füllung").strong());
                    ch |= ui_fill(ui, fill);
                    ch |= ui_opt(ui, "Rahmen", border, (Rgba::WHITE, 0.006), 0.05, "Stärke");
                    ch |= ui.add(egui::Slider::new(radius, 0.0..=0.25).text("Ecken")).changed();
                }
            }
            ui.separator();
            ui.label(RichText::new("Lage").strong());
            egui::Grid::new("lage").num_columns(2).show(ui, |ui| {
                ui.label("Position");
                ui.horizontal_wrapped(|ui| {
                    ch |= ui.add(egui::DragValue::new(&mut l.x).speed(0.002).range(-0.5..=1.5).prefix("x ").fixed_decimals(3)).changed();
                    ch |= ui.add(egui::DragValue::new(&mut l.y).speed(0.002).range(-0.5..=1.5).prefix("y ").fixed_decimals(3)).changed();
                });
                ui.end_row();
                if !matches!(l.kind, Kind::Text { .. }) {
                    ui.label("Größe");
                    ui.horizontal_wrapped(|ui| {
                        ch |= ui.add(egui::DragValue::new(&mut l.w).speed(0.002).range(0.01..=3.0).prefix("b ").fixed_decimals(3)).changed();
                        ch |= ui.add(egui::DragValue::new(&mut l.h).speed(0.002).range(0.01..=3.0).prefix("h ").fixed_decimals(3)).changed();
                    });
                    ui.end_row();
                }
                ui.label("Drehung");
                ch |= ui.add(egui::Slider::new(&mut l.rotation, -180.0..=180.0).suffix("°")).changed();
                ui.end_row();
                ui.label("Perspektive");
                ch |= ui.add(egui::Slider::new(&mut l.keystone, -0.9..=0.9)).on_hover_text("> 0: oben schmaler (Star-Wars-Effekt), < 0: unten schmaler").changed();
                ui.end_row();
                ui.label("Deckkraft");
                ch |= ui.add(egui::Slider::new(&mut l.opacity, 0.0..=1.0)).changed();
                ui.end_row();
            });
        }
        #[cfg(windows)]
        if pick_file {
            if let Some(p) = rfd::FileDialog::new().add_filter("Bilder", &["png", "jpg", "jpeg", "bmp"]).pick_file() {
                if let Kind::Picture { path, .. } = &mut self.templates[self.sel_tpl].layers[li].kind { *path = p.to_string_lossy().into(); ch = true; }
            }
        }
        let _ = pick_file;
        if ch { self.tpl_changed(); }
    }
}
