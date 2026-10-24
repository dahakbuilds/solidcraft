//! The CAM simulation window: pick a tool from the library, describe the stock and the
//! finished part, run a pocket toolpath through `cam.simulate`, and look at the result —
//! the machined surface with gouges painted red, and the report beside it. The toolbar's
//! CAM tab opens it (`ui.start` of `cam.simulate`); every run is the plain engine command.

use std::sync::Arc;

use egui::{Color32, ColorImage, RichText, Sense, TextureHandle, Vec2 as EVec2};
use serde_json::{Value, json};
use solidcraft_engine::geom::{Mesh, Vec3};
use solidcraft_engine::render::{Camera, Rgb, Scene, SceneMesh, render};
use solidcraft_engine::tooling::sim::{self, FACE_GOUGED, FACE_SIDES, FACE_TOP, ZMap};

use crate::SolveApp;
use crate::i18n::tr_label;
use crate::theme::Tokens;
use crate::ui_kit;

const WINDOW_PT: f32 = 560.0;
const PREVIEW_PT: f32 = 340.0;

/// Machined steel, gouged cells in honest red, the untouched blank a shade darker.
const STEEL: Rgb = Rgb(150, 156, 164);
const GOUGE_RED: Rgb = Rgb(204, 68, 58);
const BLANK: Rgb = Rgb(104, 110, 120);

/// Everything the window remembers between frames.
#[derive(Clone, Debug)]
pub struct CamSimPanel {
    /// The library entries the tool picker lists.
    entries: Vec<Value>,
    /// The picked entry (dropdown index, 0 = none).
    tool: usize,
    stock_x: String,
    stock_y: String,
    stock_z: String,
    part_top: String,
    pocket_w: String,
    pocket_l: String,
    floor_z: String,
    stepover: String,
    report: Option<Value>,
    error: Option<String>,
    cam: Camera,
    /// Frame the camera to the stock size once a run has happened.
    scene_radius: f64,
    preview: Option<(TextureHandle, u64)>,
    mesh: Option<Arc<Mesh>>,
}

impl Default for CamSimPanel {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            tool: 0,
            stock_x: "60".into(),
            stock_y: "60".into(),
            stock_z: "10".into(),
            part_top: "5".into(),
            pocket_w: "30".into(),
            pocket_l: "20".into(),
            floor_z: "5".into(),
            stepover: "60".into(),
            report: None,
            error: None,
            cam: Camera { target: Vec3::new(30.0, 30.0, 0.0), distance: 130.0, ..Camera::default() },
            scene_radius: 90.0,
            preview: None,
            mesh: None,
        }
    }
}

/// Open the window on the user's default tool library.
pub fn open(app: &mut SolveApp) {
    let mut d = CamSimPanel::default();
    d.load_tools(app);
    app.camsim = Some(d);
}

fn number(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite() && v.abs() < 1e6)
}

impl CamSimPanel {
    /// Read the default library again so freshly added or imported tools show up.
    fn load_tools(&mut self, app: &mut SolveApp) {
        if let Ok(v) = app.session.execute("tool.library.open", &json!({})) {
            self.entries = v["library"]["entries"].as_array().cloned().unwrap_or_default();
            if self.tool == 0 && !self.entries.is_empty() {
                self.tool = 1;
            }
        }
    }

    fn tool_entry(&self) -> Option<&Value> {
        self.entries.get(self.tool.saturating_sub(1))
    }

    fn tool_label(&self, entry: &Value) -> String {
        entry["label"].as_str().unwrap_or("?").to_string()
    }

    /// The cutting diameter the stepover is a percentage of; taps and T-slots fall back
    /// to their widest named diameter.
    fn tool_diameter(entry: &Value) -> f64 {
        let p = &entry["params"];
        ["cutting_diameter", "major_diameter", "crossbar_diameter", "counterbore_diameter"]
            .iter()
            .filter_map(|k| p[*k].as_f64())
            .next()
            .unwrap_or(6.0)
    }

    /// A zigzag pocket path: plunge at one corner, then passes along X at `stepover`
    /// percent of the tool diameter, each at the floor height.
    fn pocket_path(&self, sx: f64, sy: f64, sz: f64, tool_d: f64) -> Option<Vec<Value>> {
        let (w, l, floor, stepover) = (number(&self.pocket_w)?, number(&self.pocket_l)?, number(&self.floor_z)?, number(&self.stepover)?);
        if w <= 0.0 || l <= 0.0 || w > sx || l > sy {
            return None;
        }
        let step = (tool_d * (stepover.clamp(1.0, 100.0) / 100.0)).clamp(0.2, l);
        let (x0, x1) = ((sx - w) / 2.0, (sx + w) / 2.0);
        let y0 = (sy - l) / 2.0;
        let mut pts: Vec<Value> = Vec::new();
        let mut up = true;
        let mut y = y0 + step * 0.5;
        let mut first = true;
        while y <= y0 + l + 1e-9 {
            let (a, b) = if up { (x0, x1) } else { (x1, x0) };
            if first {
                pts.push(json!([a, y, sz + 2.0]));
                first = false;
            }
            pts.push(json!([a, y, floor]));
            pts.push(json!([b, y, floor]));
            y += step;
            up = !up;
        }
        Some(pts)
    }

    fn run(&mut self, app: &mut SolveApp) {
        self.error = None;
        self.report = None;
        self.mesh = None;
        let Some(entry) = self.tool_entry().cloned() else {
            self.error = Some(tr_label("Pick a tool first").to_string());
            return;
        };
        let (Some(sx), Some(sy), Some(sz), Some(part_top)) = (number(&self.stock_x), number(&self.stock_y), number(&self.stock_z), number(&self.part_top)) else {
            self.error = Some(tr_label("Stock sizes and part top must be numbers").to_string());
            return;
        };
        if sx <= 0.0 || sy <= 0.0 || sz <= 0.0 {
            self.error = Some(tr_label("Stock sizes must be positive").to_string());
            return;
        }
        let Some(path) = self.pocket_path(sx, sy, sz, Self::tool_diameter(&entry)) else {
            self.error = Some(tr_label("Check the pocket sizes and stepover").to_string());
            return;
        };
        let request = json!({
            "family": entry["family"],
            "params": entry["params"],
            "path": path,
            "stock": {"min": [0.0, 0.0, 0.0], "max": [sx, sy, sz]},
            "part_top": part_top,
        });
        match app.session.execute("cam.simulate", &request) {
            Ok(v) => {
                self.scene_radius = sx.max(sy).max(sz) * 0.8;
                self.cam.target = Vec3::new(sx / 2.0, sy / 2.0, sz / 2.0);
                self.report = Some(v);
                self.build_mesh();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Rebuild the display mesh from the report's machined grid.
    fn build_mesh(&mut self) {
        let Some(r) = self.report.clone() else { return };
        let (Some(rows), Some(origin), Some(cell), Some(part_top)) = (
            r["stock_rows"].as_array(),
            r["origin"].as_array(),
            r["cell"].as_f64(),
            r["part_top"].as_f64(),
        ) else {
            return;
        };
        let grid: Vec<Vec<f64>> = rows.iter().filter_map(|row| row.as_array().map(|r| r.iter().filter_map(|v| v.as_f64()).collect())).collect();
        if grid.iter().any(|row| row.is_empty()) {
            return;
        }
        let (Some(x0), Some(y0)) = (origin.first().and_then(Value::as_f64), origin.get(1).and_then(Value::as_f64)) else { return };
        let Ok(stock) = ZMap::from_rows((x0, y0), cell, &grid) else { return };
        let (cols, rows_n) = stock.shape();
        let Ok(part) = ZMap::flat((x0, y0), (x0 + cols as f64 * cell, y0 + rows_n as f64 * cell), part_top, cell) else { return };
        let bottom = stock.extent().0.min(0.0) - 2.0;
        if let Ok(mesh) = sim::surface(&stock, &part, bottom, 0.05) {
            self.mesh = Some(Arc::new(mesh));
        }
    }

    fn render_preview(&self, ctx: &egui::Context) -> Option<ColorImage> {
        let mesh = self.mesh.as_ref()?;
        let t = Tokens::get();
        let mut scene = Scene { background: Some((rgb(t.viewport_top), rgb(t.viewport_bottom))), ..Default::default() };
        let face_colors = vec![(FACE_TOP, STEEL), (FACE_GOUGED, GOUGE_RED), (FACE_SIDES, BLANK)];
        scene.meshes.push(SceneMesh { mesh: mesh.clone(), color: STEEL, face_colors, opacity: 1.0 });
        scene.radius = self.scene_radius.max(1.0);
        let ppp = ctx.pixels_per_point().clamp(1.0, 2.0);
        let side = ((PREVIEW_PT * ppp) as usize).clamp(8, 1024);
        let canvas = render(&scene, &self.cam, side, side);
        Some(ColorImage::from_rgba_unmultiplied([canvas.w, canvas.h], &canvas.rgba))
    }

    fn refresh_preview(&mut self, ctx: &egui::Context) {
        if self.mesh.is_none() {
            self.preview = None;
            return;
        }
        let image = self.render_preview(ctx).unwrap_or_else(|| ColorImage::new([1, 1], vec![Color32::TRANSPARENT]));
        match &mut self.preview {
            Some((tex, _)) => tex.set(image, egui::TextureOptions::LINEAR),
            None => self.preview = Some((ctx.load_texture("sc_camsim_preview", image, egui::TextureOptions::LINEAR), 0)),
        }
    }
}

fn rgb(c: [u8; 3]) -> Rgb {
    Rgb(c[0], c[1], c[2])
}

/// Draw the window when it is open. Called once per frame.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut d) = app.camsim.take() else { return };
    let closed = ui_kit::Window::new(egui::Id::new("sc_cam_simulate"), "inspect", tr_label("CAM Simulation")).cards(4.5).height(WINDOW_PT).show_body(
        ctx,
        |ui| {
            let t = Tokens::get();
            let body_h = ctx.content_rect().height() - t.form_header_height - t.form_footer_height - t.space_24 * 2.0;
            let col_h = (body_h - 30.0).clamp(200.0, 520.0);
            let preview_side = (col_h - 30.0).clamp(160.0, PREVIEW_PT);
            ui.horizontal_wrapped(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width() * 0.46);
                    ui.set_max_height(col_h);
                    let names: Vec<String> = std::iter::once(tr_label("— no tool —"))
                        .chain(d.entries.iter().map(|e| d.tool_label(e)))
                        .collect();
                    let names_ref: Vec<&str> = names.iter().map(String::as_str).collect();
                    ui.label(RichText::new(tr_label("Tool")).color(t.text_dim).small());
                    ui_kit::dropdown(ui, "sc_camsim_tool", &mut d.tool, &names_ref);
                    if ui_kit::button(ui, tr_label("Reload tools"), ui_kit::Button::Normal).clicked() {
                        d.load_tools(app);
                    }
                    ui.separator();
                    ui.label(RichText::new(tr_label("Stock")).color(t.text_dim).small());
                    ui.horizontal(|ui| {
                        ui.label(tr_label("X"));
                        ui_kit::text_field(ui, &mut d.stock_x, "");
                        ui.label(tr_label("Y"));
                        ui_kit::text_field(ui, &mut d.stock_y, "");
                        ui.label(tr_label("Z"));
                        ui_kit::text_field(ui, &mut d.stock_z, "");
                    });
                    ui.horizontal(|ui| {
                        ui.label(tr_label("Part top"));
                        ui_kit::text_field(ui, &mut d.part_top, "");
                    });
                    ui.separator();
                    ui.label(RichText::new(tr_label("Pocket")).color(t.text_dim).small());
                    ui.horizontal(|ui| {
                        ui.label(tr_label("Width"));
                        ui_kit::text_field(ui, &mut d.pocket_w, "");
                        ui.label(tr_label("Length"));
                        ui_kit::text_field(ui, &mut d.pocket_l, "");
                    });
                    ui.horizontal(|ui| {
                        ui.label(tr_label("Floor Z"));
                        ui_kit::text_field(ui, &mut d.floor_z, "");
                        ui.label(tr_label("Stepover %"));
                        ui_kit::text_field(ui, &mut d.stepover, "");
                    });
                    ui.separator();
                    if ui_kit::button(ui, tr_label("Simulate"), ui_kit::Button::Primary).clicked() {
                        d.run(app);
                    }
                    if let Some(e) = &d.error {
                        ui.label(RichText::new(e).color(t.error));
                    }
                    if let Some(r) = &d.report {
                        let mm3 = r["removed_mm3"].as_f64().unwrap_or(0.0);
                        let gouges = r["gouges"].as_u64().unwrap_or(0);
                        let worst = r["gouge_worst_mm"].as_f64().unwrap_or(0.0);
                        let left = r["remaining_cells"].as_u64().unwrap_or(0);
                        let collisions = r["collisions"].as_u64().unwrap_or(0);
                        ui.label(RichText::new(format!("{} {:.0} mm³", tr_label("Removed"), mm3)).color(t.text));
                        let gouge_text = if gouges == 0 { tr_label("No gouges").to_string() } else { format!("{gouges} {} ({worst:.2} mm)", tr_label("gouges")) };
                        ui.label(RichText::new(gouge_text).color(if gouges == 0 { t.accent } else { t.error }));
                        if collisions > 0 {
                            ui.label(RichText::new(format!("{collisions} {}", tr_label("rapid collisions"))).color(t.error));
                        }
                        if left > 0 {
                            ui.label(RichText::new(format!("{left} {} ({:.2} mm)", tr_label("cells left above the part"), r["remaining_worst_mm"].as_f64().unwrap_or(0.0))).color(t.text_dim));
                        }
                    }
                });
                ui.vertical(|ui| {
                    d.refresh_preview(ui.ctx());
                    let resp = ui.allocate_response(EVec2::splat(preview_side), Sense::drag());
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
                            d.cam.distance *= 1.15;
                        }
                        if ui_kit::button(ui, "+", ui_kit::Button::Normal).clicked() {
                            d.cam.distance = (d.cam.distance / 1.15).max(1.0);
                        }
                        ui.label(RichText::new(tr_label("drag to turn")).color(t.text_dim).small());
                    });
                });
            });
        },
    );
    if !closed {
        app.camsim = Some(d);
    }
}

#[cfg(test)]
#[path = "dialogs_camsim_tests.rs"]
mod tests;
