//! The Tool Library window: browse, add, edit and remove milling cutters in a library file,
//! with a live 3D preview of the revolved profile in the classic CAM colours (gold cutting
//! surfaces, silver shank, gray holder). The toolbar's Tool Library (`ui.start` of
//! `tool.library.open`) opens it; every action runs a `tool.*` command — the window never
//! touches the library file itself.

use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Color32, ColorImage, Image, RichText, Sense, TextureHandle, Vec2};
use serde_json::{Value, json};
use solidcraft_engine::geom::Vec3;
use solidcraft_engine::render::{Camera, Rgb, Scene, SceneMesh, render};
use solidcraft_engine::tooling::{Family, assembly, mesh, profile};

use crate::SolveApp;
use crate::i18n::tr_label;
use crate::theme::Tokens;
use crate::ui_kit;

/// The classic CAM look: coated cutter gold, bare steel silver, anodized gray for holders.
const GOLD: Rgb = Rgb(201, 162, 39);
const SILVER: Rgb = Rgb(184, 188, 192);
const GRAY: Rgb = Rgb(138, 143, 152);
/// Preview resolution in logical points, the thumbnail edge, and the angular steps of the revolve.
const PREVIEW_PT: f32 = 400.0;
const THUMB_PT: f32 = 44.0;
const REVOLVE_STEPS: usize = 48;

/// One family as `tool.families` describes it.
pub struct FamilyInfo {
    pub id: String,
    pub label: String,
    /// The parameter schema: name, min, max, default, unit, doc.
    pub params: Vec<Value>,
}

/// One catalogue holder as `tool.holders` describes it.
pub struct HolderInfo {
    pub id: String,
    pub label: String,
    pub nose_len: f64,
    pub overall_length: f64,
}

/// The entry form. Parameters are free text until Apply, so typing never fights the user.
#[derive(Default)]
pub struct Form {
    pub label: String,
    pub family: usize,
    pub params: Vec<String>,
    /// 0 = no holder, otherwise an index + 1 into the holder catalogue.
    pub holder: usize,
    pub stickout: String,
    pub vendor: String,
    pub product_id: String,
    pub notes: String,
}

/// The window's state; the library comes from the engine.
pub struct ToolingPanel {
    pub path: String,
    library: Value,
    families: Vec<FamilyInfo>,
    holders: Vec<HolderInfo>,
    search: String,
    /// The picked entry's id, or None while composing a new tool.
    selected: Option<String>,
    form: Form,
    adding: bool,
    error: Option<String>,
    cam: Camera,
    preview: Option<(TextureHandle, u64)>,
    /// One mini preview per entry id (content-addressed, so it invalidates with the content).
    thumbs: BTreeMap<String, TextureHandle>,
}

impl Default for ToolingPanel {
    fn default() -> Self {
        Self {
            path: String::new(),
            library: json!({"entries": []}),
            families: Vec::new(),
            holders: Vec::new(),
            search: String::new(),
            selected: None,
            form: Form::default(),
            adding: false,
            error: None,
            cam: Camera { target: Vec3::new(0.0, 0.0, 25.0), distance: 150.0, ..Camera::default() },
            preview: None,
            thumbs: BTreeMap::new(),
        }
    }
}

/// Open the window on the default user library.
pub fn open(app: &mut SolveApp) {
    let mut d = ToolingPanel { families: families(app), holders: holders(app), ..Default::default() };
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

fn holders(app: &mut SolveApp) -> Vec<HolderInfo> {
    let Ok(v) = app.session.execute("tool.holders", &json!({})) else { return Vec::new() };
    v["holders"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|h| HolderInfo {
            id: h["id"].as_str().unwrap_or_default().to_string(),
            label: h["label"].as_str().unwrap_or_default().to_string(),
            nose_len: h["nose_length"].as_f64().unwrap_or_default(),
            overall_length: h["overall_length"].as_f64().unwrap_or_default(),
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
        // drop the mini previews of entries that are gone (ids are content-addressed)
        let ids: Vec<String> = self.entries().iter().filter_map(|e| e["id"].as_str().map(str::to_string)).collect();
        self.thumbs.retain(|k, _| ids.iter().any(|i| i == k));
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

    /// Frame the tool so it fills the preview: the camera distance scales with the tool's
    /// overall length, so a 40 mm tool is not a speck next to a 150 mm one.
    fn fit_camera(&mut self, overall: f64) {
        if overall.is_finite() && overall > 0.0 {
            self.cam.distance = (overall * 1.3).clamp(30.0, 4000.0);
        }
    }

    /// When the form carries a holder, frame the whole assembly instead of the bare tool.
    fn fit_assembly(&mut self) {
        let taper = self.holders.get(self.form.holder.wrapping_sub(1)).map(|h| h.overall_length - h.nose_len);
        let stickout: Option<f64> = self.form.stickout.trim().parse().ok().filter(|v: &f64| v.is_finite() && *v > 0.0);
        if let (Some(taper), Some(stickout)) = (taper, stickout) {
            self.fit_camera(stickout + taper);
        }
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
        self.form.holder = match entry["holder_type"].as_str() {
            Some(type_id) => self.holders.iter().position(|h| h.id == type_id).map(|i| i + 1).unwrap_or(0),
            None => 0,
        };
        self.form.stickout = entry["stickout"].as_f64().map(|v| format!("{v}")).unwrap_or_default();
        if let Some(overall) = entry["params"]["overall_length"].as_f64() {
            self.fit_camera(overall);
        }
        self.fit_assembly();
    }

    /// Start composing a new tool with the family's defaults.
    fn start_add(&mut self) {
        self.selected = None;
        self.adding = true;
        self.form.label = String::new();
        self.form.vendor = String::new();
        self.form.product_id = String::new();
        self.form.notes = String::new();
        self.form.holder = 0;
        self.form.stickout = String::new();
        self.reset_params();
    }

    fn reset_params(&mut self) {
        let (texts, overall) = match self.family() {
            Some(f) => (
                f.params.iter().map(|p| p["default"].to_string()).collect::<Vec<String>>(),
                f.params.iter().find(|p| p["name"].as_str() == Some("overall_length")).and_then(|p| p["default"].as_f64()),
            ),
            None => (Vec::new(), None),
        };
        self.form.params = texts;
        if let Some(overall) = overall {
            self.fit_camera(overall);
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
        if self.form.holder > 0 {
            let holder = self.holders.get(self.form.holder - 1).ok_or("holder is no longer in the catalogue")?;
            let stickout: f64 = self.form.stickout.trim().parse().map_err(|_| "stickout must be a number".to_string())?;
            if !stickout.is_finite() || stickout <= 0.0 {
                return Err("stickout must be a positive length".into());
            }
            req["holder_type"] = json!(holder.id);
            req["stickout"] = json!(stickout);
        } else if !self.adding {
            // clearing the holder also clears the stickout on the engine side
            req["holder_type"] = json!("");
        }
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

    /// The preview's cache key: family, parameters, holder, stickout and camera.
    fn preview_key(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.form.family.hash(&mut h);
        for p in &self.form.params {
            p.trim().hash(&mut h);
        }
        self.form.holder.hash(&mut h);
        self.form.stickout.trim().hash(&mut h);
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
        let holder_id = self.holders.get(self.form.holder.wrapping_sub(1)).map(|h| h.id.as_str());
        let stickout: Option<f64> = self.form.stickout.trim().parse().ok().filter(|v: &f64| v.is_finite() && *v > 0.0);
        let (revolved, overall, radius) = self.view_of(&profile, &params, holder_id, stickout, REVOLVE_STEPS)?;
        let t = Tokens::get();
        let mut scene = Scene { background: Some((rgb(t.viewport_top), rgb(t.viewport_bottom))), ..Default::default() };
        let face_colors: Vec<(u32, Rgb)> = revolved.face_roles.iter().enumerate().map(|(i, role)| (i as u32, color_for(role))).collect();
        scene.meshes.push(SceneMesh { mesh: Arc::new(revolved.mesh), color: SILVER, face_colors, opacity: 1.0 });
        scene.radius = overall.max(radius * 4.0).max(1.0);
        let mut cam = self.cam;
        cam.target = Vec3::new(0.0, 0.0, overall / 2.0);
        let ppp = ctx.pixels_per_point().clamp(1.0, 2.0);
        let side = ((PREVIEW_PT * ppp) as usize).clamp(8, 1024);
        let canvas = render(&scene, &cam, side, side);
        Some(ColorImage::from_rgba_unmultiplied([canvas.w, canvas.h], &canvas.rgba))
    }

    /// The mesh to draw for a profile: the holder assembly when one is given and composes,
    /// otherwise the bare tool; with the overall length and radius that frame the camera.
    fn view_of(
        &self,
        profile: &profile::CutterProfile,
        params: &BTreeMap<String, f64>,
        holder_id: Option<&str>,
        stickout: Option<f64>,
        steps: usize,
    ) -> Option<(mesh::Revolved, f64, f64)> {
        if let (Some(holder_id), Some(stickout)) = (holder_id, stickout) {
            let bore = params.get("shank_diameter").copied().unwrap_or(0.0);
            if let Ok(built) = assembly::compose(profile, holder_id, bore, stickout, steps) {
                return Some((built.mesh, built.overall_length, built.max_radius));
            }
        }
        let revolved = mesh::revolve(profile, steps).ok()?;
        Some((revolved, profile.overall_length, profile.max_radius))
    }

    /// The mini preview for one library row, at a fixed three-quarter view.
    fn thumbnail(&self, ctx: &egui::Context, entry: &Value) -> Option<ColorImage> {
        let family = Family::parse(entry["family"].as_str()?)?;
        let params: BTreeMap<String, f64> = entry["params"].as_object()?.iter().filter_map(|(k, v)| Some((k.clone(), v.as_f64()?))).collect();
        let profile = profile::of(family, &params).ok()?;
        let (revolved, overall, radius) = self.view_of(&profile, &params, entry["holder_type"].as_str(), entry["stickout"].as_f64(), 16)?;
        let t = Tokens::get();
        let mut scene = Scene { background: Some((rgb(t.viewport_top), rgb(t.viewport_bottom))), ..Default::default() };
        let face_colors: Vec<(u32, Rgb)> = revolved.face_roles.iter().enumerate().map(|(i, role)| (i as u32, color_for(role))).collect();
        scene.meshes.push(SceneMesh { mesh: Arc::new(revolved.mesh), color: SILVER, face_colors, opacity: 1.0 });
        scene.radius = overall.max(radius * 4.0).max(1.0);
        let cam = Camera { target: Vec3::new(0.0, 0.0, overall / 2.0), distance: (overall * 1.35).clamp(30.0, 4000.0), ..Camera::default() };
        let ppp = ctx.pixels_per_point().clamp(1.0, 2.0);
        let side = ((THUMB_PT * ppp) as usize).clamp(8, 1024);
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
        mesh::Role::Holder => GRAY,
    }
}

/// Draw the window when it is open. Called once per frame.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut d) = app.tooling.take() else { return };
    if d.form.params.is_empty() {
        // a fresh window shows a valid default tool, not an empty grid
        d.reset_params();
    }
    let closed = ui_kit::Window::new(egui::Id::new("sc_tool_library"), "library", tr_label("Tool Library")).cards(4.5).show_body(ctx, |ui| {
        let t = Tokens::get();
        // Grow with the window: the columns take the body height the ui_kit window allows its
        // scrollable body, minus the path row, the search/button rows and the zoom row, so the
        // whole form fits without nested scrolling on a tall screen and still fits a short one.
        let body_h = ctx.content_rect().height() - t.form_header_height - t.form_footer_height - t.space_24 * 2.0;
        let col_h = (body_h - 96.0).clamp(170.0, 400.0);
        let preview_side = (col_h - 30.0).clamp(140.0, PREVIEW_PT);
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
                ui.set_width(280.0);
                ui.add(egui::TextEdit::singleline(&mut d.search).hint_text("Search tools").desired_width(270.0));
                egui::ScrollArea::vertical().max_height((col_h - 60.0).max(100.0)).auto_shrink([false, false]).show(ui, |ui| {
                    let rows = d.filtered();
                    if rows.is_empty() {
                        ui.label(RichText::new("no tools match").color(t.text_dim).italics());
                    }
                    for entry in rows {
                        let id = entry["id"].as_str().unwrap_or_default().to_string();
                        let label = entry["label"].as_str().unwrap_or_default().to_string();
                        let family_label = d.families.iter().find(|f| f.id == entry["family"].as_str().unwrap_or_default()).map(|f| f.label.clone());
                        let picked = d.selected.as_deref() == Some(id.as_str());
                        if !d.thumbs.contains_key(&id)
                            && let Some(image) = d.thumbnail(ui.ctx(), &entry)
                        {
                            let tex = ui.ctx().load_texture(format!("sc_tool_thumb_{}", id.replace('/', "_")), image, egui::TextureOptions::LINEAR);
                            d.thumbs.insert(id.clone(), tex);
                        }
                        ui.horizontal(|ui| {
                            if let Some(tex) = d.thumbs.get(&id) {
                                ui.add(Image::new((tex.id(), Vec2::splat(THUMB_PT))).sense(Sense::hover()));
                            }
                            ui.vertical(|ui| {
                                if ui_kit::list_picker(ui, &id, &label, picked).clicked() {
                                    d.selected = Some(id.clone());
                                    d.adding = false;
                                    d.fill(&entry);
                                }
                                if let Some(family_label) = family_label {
                                    ui.label(RichText::new(format!("    {family_label}")).color(t.text_dim).small());
                                }
                            });
                        });
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui_kit::button(ui, "Add", ui_kit::Button::Normal).clicked() {
                        d.start_add();
                    }
                    if ui_kit::button(ui, "Duplicate", ui_kit::Button::Normal).clicked() {
                        let (label, family, params, vendor, product, holder, stickout) = (
                            format!("{} copy", d.form.label),
                            d.form.family,
                            d.form.params.clone(),
                            d.form.vendor.clone(),
                            d.form.product_id.clone(),
                            d.form.holder,
                            d.form.stickout.clone(),
                        );
                        d.start_add();
                        d.form.label = label;
                        d.form.family = family;
                        d.form.params = params;
                        d.form.vendor = vendor;
                        d.form.product_id = product;
                        d.form.holder = holder;
                        d.form.stickout = stickout;
                    }
                    if d.selected.is_some() && ui_kit::button(ui, "Delete tool", ui_kit::Button::Normal).clicked() {
                        d.remove_selected(app);
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(380.0);
                // cap the form height: with the holder rows the body would otherwise grow
                // past the window frame and the Apply button below would lose its clicks
                egui::ScrollArea::vertical().id_salt("sc_tool_form_scroll").max_height((col_h - 30.0).max(140.0)).show(ui, |ui| {
                    egui::Grid::new("sc_tool_form").num_columns(2).spacing([10.0, 4.0]).min_col_width(150.0).show(ui, |ui| {
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
                            ui.label(capitalize(&label));
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
                        let holder_names: Vec<String> = std::iter::once("None".to_string()).chain(d.holders.iter().map(|h| h.label.clone())).collect();
                        let holder_names: Vec<&str> = holder_names.iter().map(String::as_str).collect();
                        let before_holder = d.form.holder;
                        ui.label("Holder");
                        egui::ComboBox::from_id_salt("sc_tool_holder")
                            .selected_text(holder_names.get(d.form.holder).copied().unwrap_or("?"))
                            .show_ui(ui, |ui| {
                                for (i, name) in holder_names.iter().enumerate() {
                                    ui.selectable_value(&mut d.form.holder, i, *name);
                                }
                            });
                        ui.end_row();
                        if d.form.holder != before_holder {
                            d.fit_assembly();
                        }
                        if d.form.holder > 0 {
                            ui.label("Stickout (mm)");
                            ui.add(egui::TextEdit::singleline(&mut d.form.stickout).desired_width(90.0));
                            ui.end_row();
                        }
                    });
                });
                ui.horizontal(|ui| {
                    let label = if d.adding { "Add tool" } else { "Apply" };
                    if ui_kit::button(ui, label, ui_kit::Button::Primary).clicked() {
                        d.commit(app);
                    }
                });
                if let Some(e) = &d.error {
                    // inside the form column: a long message wraps here instead of growing
                    // the body past the window frame
                    ui.label(RichText::new(e).color(t.error));
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                d.refresh_preview(ui.ctx());
                let resp = ui.allocate_response(Vec2::splat(preview_side), Sense::drag());
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
