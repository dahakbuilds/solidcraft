//! The Tool Library window: browse, add, edit and remove milling cutters in a library file,
//! with a live 3D preview of the revolved profile in the classic CAM colours (gold cutting
//! surfaces, silver shank, teal holders when assemblies arrive). The toolbar's Tool Library
//! (`ui.start` of `tool.library.open`) opens it; every action runs a `tool.*` command — the
//! window never touches the library file itself.

use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Color32, ColorImage, RichText, Sense, TextureHandle, Vec2};
use serde_json::{Value, json};
use solidcraft_engine::geom::Vec3;
use solidcraft_engine::render::{Camera, Rgb, Scene, SceneMesh, render};
use solidcraft_engine::tooling::{Family, mesh, profile};

use crate::SolveApp;
use crate::i18n::tr_label;
use crate::theme::Tokens;
use crate::ui_kit;

/// The classic CAM look: coated cutter gold, bare steel silver, Windows-95 teal for holders.
const GOLD: Rgb = Rgb(201, 162, 39);
const SILVER: Rgb = Rgb(184, 188, 192);
const TEAL: Rgb = Rgb(0, 128, 128);
/// Preview resolution in logical points and the angular steps of the revolve.
const PREVIEW_PT: f32 = 220.0;
const REVOLVE_STEPS: usize = 48;

/// One family as `tool.families` describes it.
pub struct FamilyInfo {
    pub id: String,
    pub label: String,
    /// The parameter schema: name, min, max, default, unit, doc.
    pub params: Vec<Value>,
}

/// The entry form. Parameters are free text until Apply, so typing never fights the user.
#[derive(Default)]
pub struct Form {
    pub label: String,
    pub family: usize,
    pub params: Vec<String>,
    pub vendor: String,
    pub product_id: String,
    pub notes: String,
}

/// The window's state; the library comes from the engine.
pub struct ToolingPanel {
    pub path: String,
    library: Value,
    families: Vec<FamilyInfo>,
    search: String,
    /// The picked entry's id, or None while composing a new tool.
    selected: Option<String>,
    form: Form,
    adding: bool,
    error: Option<String>,
    cam: Camera,
    preview: Option<(TextureHandle, u64)>,
}

impl Default for ToolingPanel {
    fn default() -> Self {
        Self {
            path: String::new(),
            library: json!({"entries": []}),
            families: Vec::new(),
            search: String::new(),
            selected: None,
            form: Form::default(),
            adding: false,
            error: None,
            cam: Camera { target: Vec3::new(0.0, 0.0, 25.0), distance: 150.0, ..Camera::default() },
            preview: None,
        }
    }
}

/// Open the window on the default user library.
pub fn open(app: &mut SolveApp) {
    let mut d = ToolingPanel { families: families(app), ..Default::default() };
    d.reload(app);
    app.tooling = Some(d);
}

fn families(app: &mut SolveApp) -> Vec<FamilyInfo> {
    let Ok(v) = app.session.execute("tool.families", &json!({})) else { return Vec::new() };
    v["families"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| FamilyInfo {
            id: f["id"].as_str().unwrap_or_default().to_string(),
            label: f["label"].as_str().unwrap_or_default().to_string(),
            params: f["params"].as_array().cloned().unwrap_or_default(),
        })
        .collect()
}

impl ToolingPanel {
    /// Read the current library file again (the default one when no path is set yet).
    fn reload(&mut self, app: &mut SolveApp) {
        let params = if self.path.is_empty() { json!({}) } else { json!({"path": self.path}) };
        match app.session.execute("tool.library.open", &params) {
            Ok(v) => {
                self.path = v["path"].as_str().unwrap_or_default().to_string();
                self.library = v["library"].clone();
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        self.selected = None;
        self.form = Form::default();
        self.adding = false;
    }

    fn entries(&self) -> Vec<Value> {
        self.library["entries"].as_array().cloned().unwrap_or_default()
    }

    /// The entries the search box leaves (label, vendor or family name, case-insensitive).
    fn filtered(&self) -> Vec<Value> {
        let needle = self.search.trim().to_lowercase();
        if needle.is_empty() {
            return self.entries();
        }
        self.entries()
            .into_iter()
            .filter(|e| {
                let family = self.families.iter().find(|f| f.id == e["family"].as_str().unwrap_or_default());
                let family = family.map(|f| f.label.clone()).unwrap_or_default();
                [e["label"].as_str(), e["vendor"].as_str(), Some(family.as_str())].into_iter().flatten().any(|t| t.to_lowercase().contains(&needle))
            })
            .collect()
    }

    fn family(&self) -> Option<&FamilyInfo> {
        self.families.get(self.form.family)
    }

    /// Fill the form from a stored entry (parameters in the schema's order).
    fn fill(&mut self, entry: &Value) {
        self.form.family = self.families.iter().position(|f| f.id == entry["family"].as_str().unwrap_or_default()).unwrap_or(0);
        let fam = self.family();
        self.form.params = match fam {
            Some(f) => f
                .params
                .iter()
                .map(|p| {
                    let name = p["name"].as_str().unwrap_or_default();
                    match entry["params"][name].as_f64() {
                        Some(v) => format!("{v}"),
                        None => p["default"].to_string(),
                    }
                })
                .collect(),
            None => Vec::new(),
        };
        self.form.label = entry["label"].as_str().unwrap_or_default().to_string();
        self.form.vendor = entry["vendor"].as_str().unwrap_or_default().to_string();
        self.form.product_id = entry["product_id"].as_str().unwrap_or_default().to_string();
        self.form.notes = entry["notes"].as_str().unwrap_or_default().to_string();
    }

    /// Start composing a new tool with the family's defaults.
    fn start_add(&mut self) {
        self.selected = None;
        self.adding = true;
        self.form.label = String::new();
        self.form.vendor = String::new();
        self.form.product_id = String::new();
        self.form.notes = String::new();
        self.reset_params();
    }

    fn reset_params(&mut self) {
        if let Some(f) = self.family() {
            self.form.params = f.params.iter().map(|p| p["default"].to_string()).collect();
        }
    }

    /// The form's parameter text as numbers, in the family's schema.
    fn params_json(&self) -> std::result::Result<Value, String> {
        let fam = self.family().ok_or("no tool family is selected")?;
        let mut map = serde_json::Map::new();
        for (spec, text) in fam.params.iter().zip(self.form.params.iter()) {
            let name = spec["name"].as_str().ok_or("broken family schema")?;
            let value: f64 = text.trim().parse().map_err(|_| format!("`{text}` is not a number for {name}"))?;
            if !value.is_finite() {
                return Err(format!("{name} must be a finite number"));
            }
            map.insert(name.to_string(), json!(value));
        }
        Ok(Value::Object(map))
    }

    fn form_request(&self) -> std::result::Result<Value, String> {
        let fam = self.family().ok_or("no tool family is selected")?;
        let mut req = json!({"family": fam.id, "label": self.form.label.trim(), "params": self.params_json()?});
        if !self.form.vendor.trim().is_empty() {
            req["vendor"] = json!(self.form.vendor.trim());
        }
        if !self.form.product_id.trim().is_empty() {
            req["product_id"] = json!(self.form.product_id.trim());
        }
        if !self.form.notes.trim().is_empty() {
            req["notes"] = json!(self.form.notes.trim());
        }
        Ok(req)
    }

    fn commit(&mut self, app: &mut SolveApp) {
        let cmd = if self.adding { "tool.library.add" } else { "tool.library.edit" };
        let mut req = match self.form_request() {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        if !self.adding {
            req["entry"] = json!(self.selected.clone().unwrap_or_default());
        }
        req["path"] = json!(self.path);
        match app.session.execute(cmd, &req) {
            Ok(v) => {
                let id = v["entry"].as_str().unwrap_or_default().to_string();
                self.error = None;
                self.reload(app);
                self.select_id(&id);
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Pick a freshly written entry (its id can move when its content changes).
    fn select_id(&mut self, id: &str) {
        if self.entries().iter().any(|e| e["id"].as_str() == Some(id)) {
            self.selected = Some(id.to_string());
            self.adding = false;
            if let Some(entry) = self.entries().into_iter().find(|e| e["id"].as_str() == Some(id)) {
                self.fill(&entry);
            }
        }
    }

    fn remove_selected(&mut self, app: &mut SolveApp) {
        let Some(id) = self.selected.clone() else { return };
        match app.session.execute("tool.library.remove", &json!({"path": self.path, "entry": id})) {
            Ok(_) => {
                self.error = None;
                self.reload(app);
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// The preview's cache key: family, parameters and camera.
    fn preview_key(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.form.family.hash(&mut h);
        for p in &self.form.params {
            p.trim().hash(&mut h);
        }
        for v in [self.cam.yaw, self.cam.pitch, self.cam.distance] {
            v.to_bits().hash(&mut h);
        }
        h.finish()
    }

    /// Revolve the form's tool and rasterise it into the preview texture.
    fn refresh_preview(&mut self, ctx: &egui::Context) {
        let key = self.preview_key();
        if self.preview.as_ref().is_some_and(|(_, k)| *k == key) {
            return;
        }
        let image = self.render_preview(ctx).unwrap_or_else(|| ColorImage::new([1, 1], vec![Color32::TRANSPARENT]));
        match &mut self.preview {
            Some((tex, k)) => {
                tex.set(image, egui::TextureOptions::LINEAR);
                *k = key;
            }
            None => self.preview = Some((ctx.load_texture("sc_tool_preview", image, egui::TextureOptions::LINEAR), key)),
        }
    }

    fn render_preview(&self, ctx: &egui::Context) -> Option<ColorImage> {
        let fam = self.family()?;
        let family = Family::parse(&fam.id)?;
        let params: BTreeMap<String, f64> =
            self.params_json().ok()?.as_object()?.iter().map(|(k, v)| (k.clone(), v.as_f64().unwrap_or_default())).collect();
        let profile = profile::of(family, &params).ok()?;
        let revolved = mesh::revolve(&profile, REVOLVE_STEPS).ok()?;
        let t = Tokens::get();
        let mut scene = Scene { background: Some((rgb(t.viewport_top), rgb(t.viewport_bottom))), ..Default::default() };
        let face_colors: Vec<(u32, Rgb)> = revolved.face_roles.iter().enumerate().map(|(i, role)| (i as u32, color_for(role))).collect();
        scene.meshes.push(SceneMesh { mesh: Arc::new(revolved.mesh), color: SILVER, face_colors, opacity: 1.0 });
        scene.radius = profile.overall_length.max(profile.max_radius * 4.0).max(1.0);
        let mut cam = self.cam;
        cam.target = Vec3::new(0.0, 0.0, profile.overall_length / 2.0);
        let ppp = ctx.pixels_per_point().clamp(1.0, 2.0);
        let side = ((PREVIEW_PT * ppp) as usize).clamp(8, 1024);
        let canvas = render(&scene, &cam, side, side);
        Some(ColorImage::from_rgba_unmultiplied([canvas.w, canvas.h], &canvas.rgba))
    }
}

fn rgb(c: Color32) -> Rgb {
    let [r, g, b, _] = c.to_array();
    Rgb(r, g, b)
}

fn color_for(role: &mesh::Role) -> Rgb {
    match role {
        mesh::Role::Cutting => GOLD,
        mesh::Role::Shank => SILVER,
        mesh::Role::Holder => TEAL,
    }
}

/// Draw the window when it is open. Called once per frame.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut d) = app.tooling.take() else { return };
    if d.form.params.is_empty() {
        // a fresh window shows a valid default tool, not an empty grid
        d.reset_params();
    }
    let closed = ui_kit::Window::new(egui::Id::new("sc_tool_library"), "library", tr_label("Tool Library")).cards(4.0).show_body(ctx, |ui| {
        let t = Tokens::get();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&d.path).color(t.text_dim).small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui_kit::button(ui, "Reload", ui_kit::Button::Normal).clicked() {
                    d.reload(app);
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    if ui_kit::button(ui, "Open…", ui_kit::Button::Normal).clicked() {
                        app.pick_open(move |app, picked| {
                            if let Some(picked) = picked
                                && let Some(mut d) = app.tooling.take()
                            {
                                d.path = picked;
                                d.reload(app);
                                app.tooling = Some(d);
                            }
                        });
                    }
                }
            });
        });
        ui.separator();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(240.0);
                ui.add(egui::TextEdit::singleline(&mut d.search).hint_text("Search tools").desired_width(230.0));
                egui::ScrollArea::vertical().max_height(220.0).auto_shrink([false, false]).show(ui, |ui| {
                    let rows = d.filtered();
                    if rows.is_empty() {
                        ui.label(RichText::new("no tools match").color(t.text_dim).italics());
                    }
                    for entry in rows {
                        let id = entry["id"].as_str().unwrap_or_default().to_string();
                        let label = entry["label"].as_str().unwrap_or_default().to_string();
                        let family_label = d.families.iter().find(|f| f.id == entry["family"].as_str().unwrap_or_default()).map(|f| f.label.clone());
                        let picked = d.selected.as_deref() == Some(id.as_str());
                        if ui_kit::list_picker(ui, &id, &label, picked).clicked() {
                            d.selected = Some(id.clone());
                            d.adding = false;
                            d.fill(&entry);
                        }
                        if let Some(family_label) = family_label {
                            ui.label(RichText::new(format!("    {family_label}")).color(t.text_dim).small());
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui_kit::button(ui, "Add", ui_kit::Button::Normal).clicked() {
                        d.start_add();
                    }
                    if ui_kit::button(ui, "Duplicate", ui_kit::Button::Normal).clicked() {
                        let (label, family, params, vendor, product) = (
                            format!("{} copy", d.form.label),
                            d.form.family,
                            d.form.params.clone(),
                            d.form.vendor.clone(),
                            d.form.product_id.clone(),
                        );
                        d.start_add();
                        d.form.label = label;
                        d.form.family = family;
                        d.form.params = params;
                        d.form.vendor = vendor;
                        d.form.product_id = product;
                    }
                    if d.selected.is_some() && ui_kit::button(ui, "Delete tool", ui_kit::Button::Normal).clicked() {
                        d.remove_selected(app);
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(360.0);
                eprintln!("tooling-layout: middle avail {:.1}", ui.available_width());
                let grid = egui::Grid::new("sc_tool_form").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                    ui.label("Label");
                    ui.add(egui::TextEdit::singleline(&mut d.form.label).desired_width(200.0));
                    ui.end_row();
                    let names: Vec<String> = d.families.iter().map(|f| f.label.clone()).collect();
                    let names: Vec<&str> = names.iter().map(String::as_str).collect();
                    let before = d.form.family;
                    ui.label("Family");
                    egui::ComboBox::from_id_salt("sc_tool_family").selected_text(names.get(d.form.family).copied().unwrap_or("?")).show_ui(
                        ui,
                        |ui| {
                            for (i, name) in names.iter().enumerate() {
                                ui.selectable_value(&mut d.form.family, i, *name);
                            }
                        },
                    );
                    ui.end_row();
                    if d.form.family != before {
                        d.reset_params();
                    }
                    let specs: Vec<Value> = d.family().map(|f| f.params.clone()).unwrap_or_default();
                    for (i, spec) in specs.iter().enumerate() {
                        let name = spec["name"].as_str().unwrap_or_default();
                        let unit = spec["unit"].as_str().unwrap_or("");
                        let mut label = name.replace('_', " ");
                        if !unit.is_empty() {
                            label.push_str(&format!(" ({unit})"));
                        }
                        let resp = ui.label(capitalize(&label));
                        eprintln!("tooling-layout: label `{label}` rect {:?} avail {:.1}", resp.rect, ui.available_width());
                        if let Some(text) = d.form.params.get_mut(i) {
                            ui.add(egui::TextEdit::singleline(text).desired_width(90.0));
                        }
                        ui.end_row();
                    }
                    ui.label("Vendor");
                    ui.add(egui::TextEdit::singleline(&mut d.form.vendor).desired_width(200.0));
                    ui.end_row();
                    ui.label("Product code");
                    ui.add(egui::TextEdit::singleline(&mut d.form.product_id).desired_width(200.0));
                    ui.end_row();
                });
                eprintln!("tooling-layout: grid rect {:?}", grid.response.rect);
                ui.horizontal(|ui| {
                    let label = if d.adding { "Add tool" } else { "Apply" };
                    if ui_kit::button(ui, label, ui_kit::Button::Primary).clicked() {
                        d.commit(app);
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                d.refresh_preview(ui.ctx());
                let resp = ui.allocate_response(Vec2::new(PREVIEW_PT, PREVIEW_PT), Sense::drag());
                if resp.dragged() {
                    let dd = resp.drag_delta();
                    d.cam.yaw += dd.x as f64 * 0.01;
                    d.cam.pitch = (d.cam.pitch + dd.y as f64 * 0.01).clamp(-1.45, 1.45);
                }
                if let Some((tex, _)) = &d.preview {
                    ui.painter().image(tex.id(), resp.rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                }
                ui.horizontal(|ui| {
                    if ui_kit::button(ui, "−", ui_kit::Button::Normal).clicked() {
                        d.cam.distance = (d.cam.distance * 1.25).clamp(10.0, 4000.0);
                    }
                    if ui_kit::button(ui, "+", ui_kit::Button::Normal).clicked() {
                        d.cam.distance = (d.cam.distance * 0.8).clamp(10.0, 4000.0);
                    }
                    ui.label(RichText::new("drag to turn").color(t.text_dim).small());
                });
            });
        });
        if let Some(e) = &d.error {
            ui.separator();
            ui.label(RichText::new(e).color(t.error));
        }
    });
    if !closed {
        app.tooling = Some(d);
    }
}

fn capitalize(s: &str) -> String {
    let mut out = s.to_string();
    if let Some(first) = out.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    out
}

#[cfg(test)]
#[path = "dialogs_tooling_tests.rs"]
mod tests;
