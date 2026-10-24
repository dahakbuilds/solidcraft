//! CAM simulation: replay a milling toolpath against a stock block and the finished part,
//! reporting removed volume, gouges (cuts below the part), misses (material left above the
//! part) and rapid collisions. The math lives in `solidcraft_tooling::sim`; this module is
//! the hostile-parameter boundary and the unit conversion.
use super::CommandSpec;
use super::tooling::{path, unit, Unit, parse_family, parse_params};
use crate::{Result, Session, params::bad};
use serde::Deserialize;
use serde_json::{Value, json};
use solidcraft_tooling::{self as tooling, sim};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("cam.simulate", "Simulate Toolpath", simulate_toolpath).icon("inspect").noundo().doc_only()
        .params("family, params {…}, path [[x,y,z] | {x,y,z,cut?}, …] or file (a G-code toolpath, format gcode: G0/G1 linear moves, G90/G91, G20/G21, T/M6; arcs refused); stock {min:[x,y,z], max:[x,y,z]}, part_top? (finished height, default the stock top), cell? mm (default footprint/200), tolerance? mm (default 0.05), unit? mm|inch; Z-map replay: removed volume, gouges below the part, material left above it, and rapid collisions"),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Extent {
    min: [f64; 3],
    max: [f64; 3],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sim {
    family: String,
    #[serde(default)]
    params: Value,
    #[serde(default)]
    path: Option<Vec<Value>>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    format: Option<String>,
    stock: Extent,
    #[serde(default)]
    part_top: Option<f64>,
    #[serde(default)]
    cell: Option<f64>,
    #[serde(default)]
    tolerance: Option<f64>,
    #[serde(default)]
    unit: Option<String>,
}

/// One path point: `[x, y, z]` or `{x, y, z, cut?}`; the first point never cuts.
fn cl_point(cmd: &str, v: &Value, index: usize) -> Result<sim::ClPoint> {
    let (x, y, z, cut) = match v {
        Value::Array(a) if a.len() == 3 => (a[0].as_f64(), a[1].as_f64(), a[2].as_f64(), true),
        Value::Object(o) => (
            o.get("x").and_then(Value::as_f64),
            o.get("y").and_then(Value::as_f64),
            o.get("z").and_then(Value::as_f64),
            o.get("cut").and_then(Value::as_bool).unwrap_or(true),
        ),
        _ => return Err(bad(cmd, "path points are [x, y, z] or {x, y, z, cut?}")),
    };
    match (x, y, z) {
        (Some(x), Some(y), Some(z)) => {
            sim::ClPoint::new(x, y, z, index > 0 && cut).ok_or_else(|| bad(cmd, format!("path point {index} needs finite coordinates")))
        }
        _ => Err(bad(cmd, format!("path point {index} needs finite numbers"))),
    }
}

/// The tip path from either source: an inline `path` array or a `file` decoded by
/// `format` (only `gcode` for now). Returns the points and the reader's warnings.
fn resolve_points(cmd: &str, request: &Sim) -> Result<(Vec<sim::ClPoint>, Vec<String>)> {
    match (request.file.as_deref(), request.path.as_ref()) {
        (Some(_), Some(_)) => Err(bad(cmd, "give either path or file, not both")),
        (None, None) => Err(bad(cmd, "give a path or a file")),
        (None, Some(path)) => {
            if path.is_empty() || path.len() > sim::MAX_PATH {
                return Err(bad(cmd, format!("the path must carry between 1 and {} points", sim::MAX_PATH)));
            }
            Ok((path.iter().enumerate().map(|(i, v)| cl_point(cmd, v, i)).collect::<Result<Vec<_>>>()?, Vec::new()))
        }
        (Some(file), None) => {
            let format = request.format.as_deref().unwrap_or("gcode").to_ascii_lowercase();
            if format != "gcode" {
                return Err(bad(cmd, format!("unknown path format `{format}` (gcode)")));
            }
            path(cmd, file)?;
            let len = solidcraft_io::vfs::len(file).map_err(|e| bad(cmd, format!("could not read the toolpath: {e}")))?;
            if len > tooling::gcode::MAX_BYTES as u64 {
                return Err(bad(cmd, "the G-code file exceeds 4 MiB"));
            }
            let bytes = solidcraft_io::vfs::read(file, tooling::gcode::MAX_BYTES).map_err(|e| bad(cmd, format!("could not read the toolpath: {e}")))?;
            let decoded = tooling::gcode::read(&String::from_utf8_lossy(&bytes)).map_err(|e| bad(cmd, e.to_string()))?;
            if decoded.points.is_empty() {
                return Err(bad(cmd, "the G-code file carries no moves"));
            }
            if decoded.points.len() > sim::MAX_PATH {
                return Err(bad(cmd, format!("the G-code file carries more than {} moves", sim::MAX_PATH)));
            }
            Ok((decoded.points, decoded.warnings))
        }
    }
}

fn simulate_toolpath(_s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.simulate";
    let request: Sim = serde_json::from_value(p.clone()).map_err(|e| bad(CMD, e.to_string()))?;
    let (points, warnings) = resolve_points(CMD, &request)?;
    let u = unit(CMD, request.unit.as_deref())?;
    let scale = match u {
        Unit::Mm => 1.0,
        Unit::Inch => 25.4,
    };
    let family = parse_family(CMD, &request.family)?;
    let empty = json!({});
    let params_in = if request.params.is_null() { &empty } else { &request.params };
    let params = parse_params(CMD, params_in, family, &u)?;
    let profile = tooling::profile::of(family, &params).map_err(|e| bad(CMD, e.to_string()))?;
    let lo = request.stock.min;
    let hi = request.stock.max;
    if lo.iter().chain(hi.iter()).any(|v| !v.is_finite() || v.abs() > 1e6) || hi[0] <= lo[0] || hi[1] <= lo[1] || hi[2] <= lo[2] {
        return Err(bad(CMD, "stock min/max must be finite with max above min"));
    }
    let (lo, hi) = ([lo[0] * scale, lo[1] * scale, lo[2] * scale], [hi[0] * scale, hi[1] * scale, hi[2] * scale]);
    let cell = match request.cell {
        Some(c) if c.is_finite() && c > 0.0 => c * scale,
        None => (((hi[0] - lo[0]).max(hi[1] - lo[1])) / 200.0).clamp(sim::MIN_CELL, 5.0),
        _ => return Err(bad(CMD, "cell must be a positive number")),
    };
    let part_top = match request.part_top {
        Some(z) if z.is_finite() => z * scale,
        Some(_) => return Err(bad(CMD, "part_top must be finite")),
        None => hi[2],
    };
    let tolerance = match request.tolerance {
        Some(t) if t.is_finite() && t >= 0.0 => t * scale,
        Some(_) => return Err(bad(CMD, "tolerance must be a non-negative number")),
        None => 0.05 * scale,
    };
    let mut stock = sim::ZMap::flat((lo[0], lo[1]), (hi[0], hi[1]), hi[2], cell).map_err(|e| bad(CMD, e.to_string()))?;
    let part = sim::ZMap::flat((lo[0], lo[1]), (hi[0], hi[1]), part_top, cell).map_err(|e| bad(CMD, e.to_string()))?;
    let opts = sim::SimOptions { gouge_tol: tolerance, step: 0.0 };
    let report = sim::simulate(&profile, &points, &mut stock, &part, &opts).map_err(|e| bad(CMD, e.to_string()))?;
    let (cols, rows) = stock.shape();
    // The machined grid comes back so a viewer can draw it; a huge grid is left out.
    let stock_rows: Value =
        if cols * rows <= 100_000 { Value::Array(stock.heights().chunks(cols).map(|row| json!(row)).collect()) } else { Value::Null };
    Ok(json!({
        "removed_mm3": report.removed_mm3,
        "gouges": report.gouge_count,
        "gouge_worst_mm": report.gouge_worst,
        "gouge_points": report.gouges.iter().map(|g| json!([g[0], g[1], g[2]])).collect::<Vec<_>>(),
        "remaining_cells": report.remaining_cells,
        "remaining_worst_mm": report.remaining_worst,
        "collisions": report.collisions,
        "samples": report.samples,
        "warnings": warnings,
        "grid": [cols, rows],
        "origin": [lo[0], lo[1]],
        "cell": stock.cell(),
        "stock_rows": stock_rows,
        "stock_top": hi[2],
        "part_top": part_top,
    }))
}

#[cfg(test)]
#[path = "cam_tests.rs"]
mod tests;
