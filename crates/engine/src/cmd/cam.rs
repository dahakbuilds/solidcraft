//! CAM: the setup saved with the design (operations, stock, fixtures, toolpaths and the
//! work origin, in `solidcraft-cam`) and the simulation that replays a toolpath against
//! a stock block (the math in `solidcraft-tooling::sim`). This module is the
//! hostile-parameter boundary and the unit conversion; state changes go through the
//! document so undo and the design file carry them.
use super::CommandSpec;
use super::tooling::{Unit, parse_family, parse_params, path, unit};
use crate::{Result, Session, params::bad};
use serde::Deserialize;
use serde_json::{Value, json};
use solidcraft_cam::{self as cam, Allowance, Fixture, Operation, Stock, ToolRef, Toolpath, WorkOrigin};
use solidcraft_tooling::{self as tooling, sim};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("cam.simulate", "Simulate Toolpath", simulate_toolpath).icon("inspect").noundo().doc_only()
        .params("family, params {…}, path [[x,y,z] | {x,y,z,cut?}, …] or file (a G-code toolpath, format gcode: G0/G1 linear moves, G90/G91, G20/G21, T/M6; arcs refused); stock {min:[x,y,z], max:[x,y,z]}, part_top? (finished height, default the stock top), cell? mm (default footprint/200), tolerance? mm (default 0.05), unit? mm|inch; Z-map replay: removed volume, gouges below the part, material left above it, and rapid collisions"),
    CommandSpec::new("cam.machine", "Machine Stock to Part", machine_stock).at("MANUFACTURE", "SIMULATE").icon("cut")
        .params("as cam.simulate (family, params, path or file, stock {min, max}, part_top?, cell?, tolerance?, unit?), plus body? (name for the finished component); replays the toolpath over the stock and makes the machined surface a body of the design"),
    CommandSpec::new("cam.setup", "CAM Setup", setup).icon("list").noundo().doc_only()
        .params("→ operations [{id, name, tool?, stock?, toolpath?, comment?}], fixtures [{id, name, path, origin}], origin {position, x_axis, z_axis}, tool_library?"),
    CommandSpec::new("cam.operation.add", "Add CAM Operation", add_operation).at("MANUFACTURE", "SETUP").icon("new").doc_only()
        .params("name?, tool? {number?, entry?, name?}, stock? {kind: box|x,y,z | cylinder|radius,height | body|body, origin?, allowance?}, comment? → id, name"),
    CommandSpec::new("cam.operation.rename", "Rename CAM Operation", rename_operation).icon("edit").doc_only()
        .params("id, name → id, name"),
    CommandSpec::new("cam.operation.remove", "Remove CAM Operation", remove_operation).icon("delete").doc_only()
        .params("id → removed"),
    CommandSpec::new("cam.tool.assign", "Assign CAM Tool", assign_tool).icon("gear").doc_only()
        .params("id, number?, entry?, name? → tool"),
    CommandSpec::new("cam.stock.set", "Set CAM Stock", set_stock).at("MANUFACTURE", "SETUP").icon("box").doc_only()
        .params("id, kind: box (x, y, z) | cylinder (radius, height) | body (body), origin? [x,y,z], allowance? {plus_x, minus_x, plus_y, minus_y, plus_z, minus_z} → stock"),
    CommandSpec::new("cam.toolpath.set", "Set CAM Toolpath", set_toolpath).icon("line").doc_only()
        .params("id, segments [{type: line, from, to, cut?} | {type: arc, from, to, center, normal, clockwise?, cut?}] → segments"),
    CommandSpec::new("cam.toolpath.load", "Load G-code Toolpath", load_toolpath).at("MANUFACTURE", "SETUP").icon("import").doc_only()
        .params("file (G-code), format? (gcode), id? (existing operation; a new one by the file's name otherwise), allowance? (stock margin, default 0) → id, segments, tools, warnings, stock"),
    CommandSpec::new("cam.toolpath.remove", "Remove CAM Toolpath", remove_toolpath).icon("delete").doc_only()
        .params("id → removed"),
    CommandSpec::new("cam.fixture.add", "Add CAM Fixture", add_fixture).icon("import").doc_only()
        .params("path (a model file), name?, origin? [x,y,z] → id, name"),
    CommandSpec::new("cam.fixture.remove", "Remove CAM Fixture", remove_fixture).icon("delete").doc_only()
        .params("id → removed"),
    CommandSpec::new("cam.fixture.place", "Place CAM Fixture", place_fixture).icon("move").doc_only()
        .params("id, origin [x,y,z] → id, origin"),
    CommandSpec::new("cam.origin.set", "Set Work Origin", set_origin).icon("origin").doc_only()
        .params("position? [x,y,z], x_axis? [x,y,z], z_axis? [x,y,z] → origin"),
];

/// Parse a command's JSON into its request struct, reporting at the command boundary.
fn req<T: serde::de::DeserializeOwned>(cmd: &str, p: &Value) -> Result<T> {
    serde_json::from_value(p.clone()).map_err(|e| bad(cmd, e.to_string()))
}

/// Fail the same way for a value the `solidcraft-cam` crate refused.
fn cam_err(cmd: &str, e: cam::CamError) -> crate::EngineError {
    bad(cmd, e.to_string())
}

fn setup(s: &mut Session, _p: &Value) -> Result<Value> {
    serde_json::to_value(&s.doc.cam).map_err(|e| bad("cam.setup", e.to_string()))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct OpAdd {
    name: Option<String>,
    tool: Option<ToolRef>,
    stock: Option<Stock>,
    comment: Option<String>,
}

fn add_operation(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.operation.add";
    let r: OpAdd = req(CMD, p)?;
    let id = s.doc_mut().allocate_id().map_err(|e| bad(CMD, e.to_string()))?;
    let name = r.name.unwrap_or_else(|| format!("Operation {}", s.doc.cam.operations.len() + 1));
    let op = Operation { id, name, tool: r.tool, stock: r.stock, toolpath: None, comment: r.comment };
    op.check().map_err(|e| cam_err(CMD, e))?;
    let name = op.name.clone();
    s.doc_mut().cam.operations.push(op);
    Ok(json!({"id": id, "name": name}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdName {
    id: u64,
    name: String,
}

fn rename_operation(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.operation.rename";
    let r: IdName = req(CMD, p)?;
    let op = s.doc_mut().cam.find_operation_mut(r.id).ok_or_else(|| bad(CMD, format!("no CAM operation with id {}", r.id)))?;
    op.name = r.name;
    op.check().map_err(|e| cam_err(CMD, e))?;
    Ok(json!({"id": op.id, "name": op.name}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdOnly {
    id: u64,
}

fn remove_operation(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.operation.remove";
    let r: IdOnly = req(CMD, p)?;
    let doc = s.doc_mut();
    let before = doc.cam.operations.len();
    doc.cam.operations.retain(|o| o.id != r.id);
    if doc.cam.operations.len() == before {
        return Err(bad(CMD, format!("no CAM operation with id {}", r.id)));
    }
    Ok(json!({"removed": r.id}))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct ToolAssign {
    id: u64,
    number: Option<u32>,
    entry: Option<String>,
    name: Option<String>,
}

fn assign_tool(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.tool.assign";
    let r: ToolAssign = req(CMD, p)?;
    if r.number.is_none() && r.entry.is_none() && r.name.is_none() {
        return Err(bad(CMD, "give number, entry or name"));
    }
    let op = s.doc_mut().cam.find_operation_mut(r.id).ok_or_else(|| bad(CMD, format!("no CAM operation with id {}", r.id)))?;
    let mut tool = op.tool.clone().unwrap_or_default();
    if let Some(number) = r.number {
        tool.number = number;
    }
    if let Some(entry) = r.entry {
        tool.entry = entry;
    }
    if let Some(name) = r.name {
        tool.name = name;
    }
    tool.check().map_err(|e| cam_err(CMD, e))?;
    let out = serde_json::to_value(&tool).map_err(|e| bad(CMD, e.to_string()))?;
    op.tool = Some(tool);
    Ok(json!({"id": r.id, "tool": out}))
}

fn set_stock(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.stock.set";
    #[derive(Deserialize)]
    struct StockSet {
        id: u64,
        #[serde(flatten)]
        stock: Stock,
    }
    let r: StockSet = req(CMD, p)?;
    r.stock.check().map_err(|e| cam_err(CMD, e))?;
    let op = s.doc_mut().cam.find_operation_mut(r.id).ok_or_else(|| bad(CMD, format!("no CAM operation with id {}", r.id)))?;
    let out = serde_json::to_value(&r.stock).map_err(|e| bad(CMD, e.to_string()))?;
    op.stock = Some(r.stock);
    Ok(json!({"id": r.id, "stock": out}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathSet {
    id: u64,
    segments: Vec<cam::Segment>,
}

fn set_toolpath(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.toolpath.set";
    let r: PathSet = req(CMD, p)?;
    let path = Toolpath { segments: r.segments };
    path.check().map_err(|e| cam_err(CMD, e))?;
    let count = path.len();
    let op = s.doc_mut().cam.find_operation_mut(r.id).ok_or_else(|| bad(CMD, format!("no CAM operation with id {}", r.id)))?;
    op.toolpath = Some(path);
    Ok(json!({"id": r.id, "segments": count}))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct PathLoad {
    file: String,
    format: Option<String>,
    id: Option<u64>,
    allowance: Option<f64>,
}

/// `T7` → 7; anything else → 0 (the name is kept as it was).
fn tool_number(name: &str) -> u32 {
    name.strip_prefix('T').and_then(|n| n.parse::<u32>().ok()).unwrap_or(0)
}

fn load_toolpath(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.toolpath.load";
    let r: PathLoad = req(CMD, p)?;
    let format = r.format.as_deref().unwrap_or("gcode").to_ascii_lowercase();
    if format != "gcode" {
        return Err(bad(CMD, format!("unknown path format `{format}` (gcode)")));
    }
    path(CMD, &r.file)?;
    let len = solidcraft_io::vfs::len(&r.file).map_err(|e| bad(CMD, format!("could not read the toolpath: {e}")))?;
    if len > tooling::gcode::MAX_BYTES as u64 {
        return Err(bad(CMD, "the G-code file exceeds 4 MiB"));
    }
    let bytes = solidcraft_io::vfs::read(&r.file, tooling::gcode::MAX_BYTES).map_err(|e| bad(CMD, format!("could not read the toolpath: {e}")))?;
    let decoded = tooling::gcode::read(&String::from_utf8_lossy(&bytes)).map_err(|e| bad(CMD, e.to_string()))?;
    if decoded.points.is_empty() {
        return Err(bad(CMD, "the G-code file carries no moves"));
    }
    if decoded.points.len() > cam::MAX_SEGMENTS + 1 {
        return Err(bad(CMD, format!("the G-code file carries more than {} moves", cam::MAX_SEGMENTS)));
    }
    let points: Vec<solidcraft_geom::Vec3> = decoded.points.iter().map(|c| solidcraft_geom::Vec3::new(c.x, c.y, c.z)).collect();
    let cuts: Vec<bool> = decoded.points.iter().map(|c| c.cut).collect();
    let toolpath = Toolpath::from_points(&points, &cuts).map_err(|e| cam_err(CMD, e))?;
    let (min, max) = toolpath.bounds().ok_or_else(|| bad(CMD, "the toolpath has no bounds"))?;
    let margin = r.allowance.unwrap_or(0.0);
    if !margin.is_finite() || margin < 0.0 || margin > 1000.0 {
        return Err(bad(CMD, "allowance must be between 0 and 1000 mm"));
    }
    let allowance = Allowance { plus_x: margin, minus_x: margin, plus_y: margin, minus_y: margin, plus_z: 0.0, minus_z: 0.0 };
    let stock = Stock {
        kind: cam::StockKind::Box {
            x: (max[0] - min[0]).max(1.0),
            y: (max[1] - min[1]).max(1.0),
            z: (max[2] - min[2]).max(1.0),
        },
        origin: [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0, min[2]],
        allowance,
    };
    stock.check().map_err(|e| cam_err(CMD, e))?;
    let tool = decoded.tools.first().map(|name| ToolRef { number: tool_number(name), entry: String::new(), name: name.clone() });
    let warnings: Vec<String> = decoded.warnings.clone();
    let segments = toolpath.len();

    let id = match r.id {
        Some(id) => {
            let op = s.doc_mut().cam.find_operation_mut(id).ok_or_else(|| bad(CMD, format!("no CAM operation with id {id}")))?;
            op.toolpath = Some(toolpath);
            if op.stock.is_none() {
                op.stock = Some(stock.clone());
            }
            if op.tool.is_none() {
                op.tool = tool.clone();
            }
            id
        }
        None => {
            let id = s.doc_mut().allocate_id().map_err(|e| bad(CMD, e.to_string()))?;
            let stem = r.file.rsplit(['/', '\\']).next().unwrap_or(r.file.as_str());
            let stem = stem.rsplit_once('.').map(|(s, _)| s).unwrap_or(stem);
            let op = Operation {
                id,
                name: if stem.is_empty() { "Toolpath".into() } else { stem.to_string() },
                tool: tool.clone(),
                stock: Some(stock.clone()),
                toolpath: Some(toolpath),
                comment: None,
            };
            op.check().map_err(|e| cam_err(CMD, e))?;
            s.doc_mut().cam.operations.push(op);
            id
        }
    };
    Ok(json!({
        "id": id,
        "segments": segments,
        "tools": decoded.tools,
        "warnings": warnings,
        "stock": serde_json::to_value(&stock).map_err(|e| bad(CMD, e.to_string()))?,
    }))
}

fn remove_toolpath(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.toolpath.remove";
    let r: IdOnly = req(CMD, p)?;
    let op = s.doc_mut().cam.find_operation_mut(r.id).ok_or_else(|| bad(CMD, format!("no CAM operation with id {}", r.id)))?;
    if op.toolpath.take().is_none() {
        return Err(bad(CMD, format!("operation {} has no toolpath", r.id)));
    }
    Ok(json!({"removed": r.id}))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct FixtureAdd {
    path: String,
    name: Option<String>,
    origin: Option<[f64; 3]>,
}

fn add_fixture(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.fixture.add";
    let r: FixtureAdd = req(CMD, p)?;
    path(CMD, &r.path)?;
    let id = s.doc_mut().allocate_id().map_err(|e| bad(CMD, e.to_string()))?;
    let stem = r.path.rsplit(['/', '\\']).next().unwrap_or(r.path.as_str());
    let name = r.name.unwrap_or_else(|| stem.to_string());
    let fixture = Fixture { id, name: name.clone(), path: r.path, origin: r.origin.unwrap_or([0.0; 3]) };
    fixture.check().map_err(|e| cam_err(CMD, e))?;
    s.doc_mut().cam.fixtures.push(fixture);
    Ok(json!({"id": id, "name": name}))
}

fn remove_fixture(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.fixture.remove";
    let r: IdOnly = req(CMD, p)?;
    let doc = s.doc_mut();
    let before = doc.cam.fixtures.len();
    doc.cam.fixtures.retain(|f| f.id != r.id);
    if doc.cam.fixtures.len() == before {
        return Err(bad(CMD, format!("no fixture with id {}", r.id)));
    }
    Ok(json!({"removed": r.id}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixturePlace {
    id: u64,
    origin: [f64; 3],
}

fn place_fixture(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.fixture.place";
    let r: FixturePlace = req(CMD, p)?;
    let fixture = s.doc_mut().cam.fixtures.iter_mut().find(|f| f.id == r.id).ok_or_else(|| bad(CMD, format!("no fixture with id {}", r.id)))?;
    fixture.origin = r.origin;
    fixture.check().map_err(|e| cam_err(CMD, e))?;
    Ok(json!({"id": r.id, "origin": r.origin}))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct OriginSet {
    position: Option<[f64; 3]>,
    x_axis: Option<[f64; 3]>,
    z_axis: Option<[f64; 3]>,
}

fn set_origin(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.origin.set";
    let r: OriginSet = req(CMD, p)?;
    if r.position.is_none() && r.x_axis.is_none() && r.z_axis.is_none() {
        return Err(bad(CMD, "give position, x_axis or z_axis"));
    }
    let origin = &mut s.doc_mut().cam.origin;
    let next = WorkOrigin {
        position: r.position.unwrap_or(origin.position),
        x_axis: r.x_axis.unwrap_or(origin.x_axis),
        z_axis: r.z_axis.unwrap_or(origin.z_axis),
    };
    next.check().map_err(|e| cam_err(CMD, e))?;
    *origin = next;
    Ok(json!({"origin": serde_json::to_value(&*origin).map_err(|e| bad(CMD, e.to_string()))?}))
}

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
            let bytes =
                solidcraft_io::vfs::read(file, tooling::gcode::MAX_BYTES).map_err(|e| bad(cmd, format!("could not read the toolpath: {e}")))?;
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

/// The simulation of one request: the machined stock grid, the part target and the
/// report. Shared by `cam.simulate` (which reports) and `cam.machine` (which models).
struct SimRun {
    warnings: Vec<String>,
    stock: sim::ZMap,
    part: sim::ZMap,
    lo: [f64; 3],
    hi: [f64; 3],
    part_top: f64,
    tolerance: f64,
    report: sim::SimReport,
}

fn run_sim(cmd: &str, request: &Sim) -> Result<SimRun> {
    let (points, warnings) = resolve_points(cmd, request)?;
    let u = unit(cmd, request.unit.as_deref())?;
    let scale = match u {
        Unit::Mm => 1.0,
        Unit::Inch => 25.4,
    };
    let family = parse_family(cmd, &request.family)?;
    let empty = json!({});
    let params_in = if request.params.is_null() { &empty } else { &request.params };
    let params = parse_params(cmd, params_in, family, &u)?;
    let profile = tooling::profile::of(family, &params).map_err(|e| bad(cmd, e.to_string()))?;
    let lo = request.stock.min;
    let hi = request.stock.max;
    if lo.iter().chain(hi.iter()).any(|v| !v.is_finite() || v.abs() > 1e6) || hi[0] <= lo[0] || hi[1] <= lo[1] || hi[2] <= lo[2] {
        return Err(bad(cmd, "stock min/max must be finite with max above min"));
    }
    let (lo, hi) = ([lo[0] * scale, lo[1] * scale, lo[2] * scale], [hi[0] * scale, hi[1] * scale, hi[2] * scale]);
    let cell = match request.cell {
        Some(c) if c.is_finite() && c > 0.0 => c * scale,
        None => (((hi[0] - lo[0]).max(hi[1] - lo[1])) / 200.0).clamp(sim::MIN_CELL, 5.0),
        _ => return Err(bad(cmd, "cell must be a positive number")),
    };
    let part_top = match request.part_top {
        Some(z) if z.is_finite() => z * scale,
        Some(_) => return Err(bad(cmd, "part_top must be finite")),
        None => hi[2],
    };
    let tolerance = match request.tolerance {
        Some(t) if t.is_finite() && t >= 0.0 => t * scale,
        Some(_) => return Err(bad(cmd, "tolerance must be a non-negative number")),
        None => 0.05 * scale,
    };
    let mut stock = sim::ZMap::flat((lo[0], lo[1]), (hi[0], hi[1]), hi[2], cell).map_err(|e| bad(cmd, e.to_string()))?;
    let part = sim::ZMap::flat((lo[0], lo[1]), (hi[0], hi[1]), part_top, cell).map_err(|e| bad(cmd, e.to_string()))?;
    let opts = sim::SimOptions { gouge_tol: tolerance, step: 0.0 };
    let report = sim::simulate(&profile, &points, &mut stock, &part, &opts).map_err(|e| bad(cmd, e.to_string()))?;
    Ok(SimRun { warnings, stock, part, lo, hi, part_top, tolerance, report })
}

fn simulate_toolpath(_s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.simulate";
    let request: Sim = serde_json::from_value(p.clone()).map_err(|e| bad(CMD, e.to_string()))?;
    let run = run_sim(CMD, &request)?;
    let SimRun { warnings, stock, hi, part_top, report, .. } = run;
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
        "origin": [hi[0], hi[1]],
        "cell": stock.cell(),
        "stock_rows": stock_rows,
        "stock_top": hi[2],
        "part_top": part_top,
    }))
}

#[derive(Deserialize)]
struct Machine {
    #[serde(flatten)]
    sim: Sim,
    /// Name for the finished component and body (the file's stem otherwise).
    #[serde(default)]
    body: Option<String>,
}

/// Machine a stock block by a toolpath and make the result a body of the design: the
/// Z-map replay's finished surface is meshed, inserted as a mesh, and converted — the
/// same path a scanned STL takes. This is how a G-code file with no model becomes one.
fn machine_stock(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.machine";
    let r: Machine = req(CMD, p)?;
    let run = run_sim(CMD, &r.sim)?;
    let mesh = sim::surface(&run.stock, &run.part, run.lo[2], run.tolerance.max(1e-4)).map_err(|e| bad(CMD, e.to_string()))?;
    let name = r.body.clone().unwrap_or_else(|| match r.sim.file.as_deref() {
        Some(f) => {
            let stem = f.rsplit(['/', '\\']).next().unwrap_or(f);
            stem.rsplit_once('.').map(|(s, _)| s).unwrap_or(stem).to_string()
        }
        None => "Machined".to_string(),
    });
    if name.is_empty() || name.len() > 120 {
        return Err(bad(CMD, "body name must be between 1 and 120 bytes"));
    }
    let stl = solidcraft_io::stl_binary(&[mesh], &name);
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = solidcraft_io::vfs::temp_dir().join(format!("solidcraft-machine-{}-{}.stl", std::process::id(), SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let tmp = tmp.to_string_lossy().into_owned();
    solidcraft_io::vfs::write(&tmp, &stl).map_err(|e| bad(CMD, format!("could not stage the machined surface: {e}")))?;
    let before: Vec<String> = s.model.state().bodies.iter().map(|b| b.name.clone()).collect();
    s.execute("file.insert_mesh", &json!({"path": tmp, "name": name})).map_err(|e| bad(CMD, format!("the machined surface could not be inserted: {e}")))?;
    let new_body = s
        .model
        .state()
        .bodies
        .iter()
        .map(|b| b.name.clone())
        .find(|n| !before.contains(n))
        .ok_or_else(|| bad(CMD, "the machined surface came in with no body"))?;
    // Best effort: a watertight surface converts to a CAD solid; a Z-map surface with
    // a ragged rim stays a mesh body (a failed conversion leaves the design alone).
    let (body, converted) = match s.execute("mesh.convert", &json!({"body": new_body})) {
        Ok(out) => (out.get("body").cloned().unwrap_or(json!(new_body)), true),
        Err(_) => (json!(new_body), false),
    };
    Ok(json!({
        "body": body,
        "converted": converted,
        "removed_mm3": run.report.removed_mm3,
        "gouges": run.report.gouge_count,
        "collisions": run.report.collisions,
        "warnings": run.warnings,
    }))
}

#[cfg(test)]
#[path = "cam_tests.rs"]
mod tests;
