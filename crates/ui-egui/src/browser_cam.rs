//! The CAM tree (the browser's second tab): the design's manufacturing setup — the
//! operations with their tool, stock and path, the fixtures clamping the work, and the
//! way to the tool library. Thin by design: it reads the document's `cam` setup and
//! acts only through commands.

use crate::i18n::tr;
use crate::theme::Tokens;
use crate::SolveApp;
use egui::{pos2, Align2, FontId, RichText, ScrollArea, Sense, Ui, Vec2};
use serde_json::json;
use solidcraft_engine::cam::{Operation, StockKind};
use std::collections::BTreeSet;

/// One row's dim suffix: what the operation cuts with and how far it is set up.
fn operation_suffix(op: &Operation) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(tool) = &op.tool {
        parts.push(if !tool.name.is_empty() { tool.name.clone() } else if tool.number > 0 { format!("T{tool_number}", tool_number = tool.number) } else { String::new() });
    }
    if let Some(stock) = &op.stock {
        let kind = match &stock.kind {
            StockKind::Box { .. } => "box",
            StockKind::Cylinder { .. } => "cylinder",
            StockKind::Body { .. } => "body",
        };
        parts.push(kind.into());
    }
    if let Some(path) = &op.toolpath {
        parts.push(format!("{} segs", path.len()));
    }
    parts.retain(|p| !p.is_empty());
    parts.join(" · ")
}

/// A folder header with a disclosure triangle; returns whether it is open.
fn folder(ui: &mut Ui, collapsed: &mut BTreeSet<String>, key: &str, label: &str) -> bool {
    let t = Tokens::get();
    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), t.space_24), Sense::click());
    let open = !collapsed.contains(key);
    crate::ui_kit::disclosure(ui.painter(), pos2(r.left() + 8.0, r.center().y), open, t.text_dim);
    ui.painter().text(pos2(r.left() + 20.0, r.center().y), Align2::LEFT_CENTER, tr(label), FontId::proportional(t.font_body), t.text);
    if resp.clicked() {
        if open {
            collapsed.insert(key.into());
        } else {
            collapsed.remove(key);
        }
    }
    open
}

fn row(ui: &mut Ui, label: &str, suffix: &str) {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        ui.add_space(t.space_16);
        ui.label(label);
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(t.text_dim).small());
        }
    });
}

pub fn rows(app: &mut SolveApp, ui: &mut Ui) {
    let t = Tokens::get();
    ScrollArea::vertical().id_source("cam-tree").auto_shrink([false, false]).show(ui, |ui| {
        if ui.button(tr("ui.cam_add_operation")).clicked() {
            let _ = app.run("cam.operation.add", &json!({}));
        }
        if ui.button(tr("ui.cam_tool_library")).clicked() {
            app.start("tool.library.open");
        }
        ui.add_space(t.space_3);
        // Read the setup into owned rows first: the rows below act through commands,
        // which replace the document.
        let operations: Vec<(u64, String, String)> =
            app.session.doc.cam.operations.iter().map(|o| (o.id, o.name.clone(), operation_suffix(o))).collect();
        let fixtures: Vec<(u64, String, String)> =
            app.session.doc.cam.fixtures.iter().map(|f| (f.id, f.name.clone(), f.path.clone())).collect();
        if operations.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(t.space_16);
                ui.label(RichText::new(tr("ui.cam_empty")).color(t.text_dim).small());
            });
        } else if folder(ui, &mut app.tree.collapsed, "cam/operations", "ui.cam_operations") {
            for (_id, name, suffix) in &operations {
                row(ui, name, suffix);
            }
        }
        if folder(ui, &mut app.tree.collapsed, "cam/fixtures", "ui.cam_fixtures") {
            if fixtures.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(t.space_16);
                    ui.label(RichText::new(tr("ui.cam_no_fixtures")).color(t.text_dim).small());
                });
            } else {
                for (_id, name, path) in &fixtures {
                    row(ui, name, path);
                }
            }
        }
    });
}
