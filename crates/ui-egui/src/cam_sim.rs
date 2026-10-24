//! CAM simulation in the main window. The CAM tree carries a small player that walks
//! an operation's toolpath: a slider, a feed speed, and toggles for the cutter and the
//! stock. The viewport draws the cutter at the player's point; nothing here computes
//! material removal (that is `cam.simulate`'s replay).

use crate::i18n::tr;
use crate::SolveApp;
use egui::{RichText, Slider, Ui};
use solidcraft_engine::cam::Toolpath;
use solidcraft_engine::geom::Vec3;

/// The player state, kept between frames in the tree state.
#[derive(Clone, Debug)]
pub struct CamSim {
    /// How far along the path the cutter sits (0..=1 of its length).
    pub progress: f64,
    pub playing: bool,
    /// Feed rate the player walks the path at, in millimetres per second.
    pub speed: f64,
    pub show_cutter: bool,
    pub show_stock: bool,
}

impl Default for CamSim {
    fn default() -> Self {
        CamSim { progress: 0.0, playing: false, speed: 50.0, show_cutter: true, show_stock: true }
    }
}

/// The operation the player walks: the first with a toolpath.
pub fn active_path(app: &SolveApp) -> Option<&Toolpath> {
    app.session.doc.cam.operations.iter().find_map(|o| o.toolpath.as_ref())
}

/// The tessellated path's total length, in millimetres.
pub fn path_length(path: &Toolpath) -> f64 {
    walk(path, |_, _, l| l).iter().sum()
}

/// Collect per-chord data of the tessellated path.
fn walk(path: &Toolpath, each: impl Fn(Vec3, Vec3, f64) -> f64) -> Vec<f64> {
    let Ok(lines) = path.tessellate(0.1) else { return Vec::new() };
    let mut out = Vec::new();
    for line in &lines {
        for w in line.points.windows(2) {
            let len = (w[1] - w[0]).len();
            if len > 1e-9 {
                out.push(each(w[0], w[1], len));
            }
        }
    }
    out
}

/// Walk the tessellated path to `progress` (0..=1) of its length: the cutter tip and
/// the unit direction of travel there.
pub fn tip_at(path: &Toolpath, progress: f64) -> Option<(Vec3, Vec3)> {
    let lines = path.tessellate(0.1).ok()?;
    let mut chords: Vec<(Vec3, Vec3, f64)> = Vec::new();
    let mut total = 0.0;
    for line in &lines {
        for w in line.points.windows(2) {
            let len = (w[1] - w[0]).len();
            if len > 1e-9 {
                chords.push((w[0], w[1], len));
                total += len;
            }
        }
    }
    if chords.is_empty() || !total.is_finite() || total <= 0.0 {
        return None;
    }
    let mut want = total * progress.clamp(0.0, 1.0);
    for (a, b, len) in &chords {
        if want <= *len {
            let t = (want / len).clamp(0.0, 1.0);
            return Some((a.lerp(*b, t), (*b - *a) / *len));
        }
        want -= len;
    }
    let (a, b, len) = *chords.last()?;
    Some((b, (b - a) / len))
}

/// The player row under the CAM tree: slider, play, speed, and what to show.
pub fn controls(app: &mut SolveApp, ui: &mut Ui) {
    ui.separator();
    ui.label(RichText::new(tr("ui.simulate")).strong());
    let Some(path) = active_path(app) else {
        ui.label(RichText::new(tr("ui.cam_sim_no_path")).weak());
        return;
    };
    // Take the length before touching the state again: the path borrows the session.
    let length = path_length(path).max(1e-6);
    let slider = Slider::new(&mut app.tree.cam_sim.progress, 0.0..=1.0).show_value(false);
    if ui.add(slider).on_hover_text(format!("{:.0} mm", length * app.tree.cam_sim.progress)).changed() {
        app.tree.cam_sim.playing = false;
    }
    ui.horizontal(|ui| {
        let playing = app.tree.cam_sim.playing;
        if ui.button(if playing { "⏸" } else { "▶" }).clicked() {
            app.tree.cam_sim.playing = !playing;
            if app.tree.cam_sim.playing && app.tree.cam_sim.progress >= 1.0 {
                app.tree.cam_sim.progress = 0.0;
            }
        }
        egui::ComboBox::from_id_salt("cam-sim-speed")
            .selected_text(format!("{:.0} mm/s", app.tree.cam_sim.speed))
            .show_ui(ui, |ui| {
                for speed in [10.0, 50.0, 200.0] {
                    ui.selectable_value(&mut app.tree.cam_sim.speed, speed, format!("{speed:.0} mm/s"));
                }
            });
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut app.tree.cam_sim.show_cutter, tr("ui.cam_sim_cutter"));
        ui.checkbox(&mut app.tree.cam_sim.show_stock, tr("ui.cam_sim_stock"));
    });
    if app.tree.cam_sim.playing {
        let dt = ui.ctx().input(|i| i.stable_dt) as f64;
        app.tree.cam_sim.progress += dt * app.tree.cam_sim.speed / length;
        if app.tree.cam_sim.progress >= 1.0 {
            app.tree.cam_sim.progress = 1.0;
            app.tree.cam_sim.playing = false;
        }
        ui.ctx().request_repaint();
    }
}
