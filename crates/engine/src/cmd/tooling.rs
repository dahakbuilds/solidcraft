//! Tool libraries: milling families, cutter profiles and portable library files.
//!
//! These commands read and write library files (like `library.*` does for scalar libraries);
//! none of them touches the design, so all are `noundo` and `doc_only`. Geometry lives in
//! `solidcraft_tooling`; this module is the hostile-parameter boundary and the file access.
use super::CommandSpec;
use crate::{Result, Session, params::bad};
use serde::Deserialize;
use serde_json::{Value, json};
use solidcraft_tooling::{self as tooling, Family, ItemKind, Provenance, ToolEntry, ToolLibrary};
use std::collections::BTreeMap;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("tool.families", "Tool Families", families).icon("inspect").noundo().doc_only()
        .params("no parameters; lists the milling tool families with their parameter schema"),
    CommandSpec::new("tool.profile", "Cutter Profile", profile).icon("inspect").noundo().doc_only()
        .params("family, params {…}, unit? mm|inch; the revolved cutter profile (segments from the tip upward, cutting flags) and derived lengths; reads no file"),
    CommandSpec::new("tool.library.open", "Tool Library", open).icon("inspect").noundo().doc_only()
        .params("path? (default: the user library in the data folder); opens the library file, creating it empty on first use; returns {path, library}; the toolbar opens the Tool Library window on this command"),
    CommandSpec::new("tool.library.create", "Create Tool Library", create).icon("export").noundo().doc_only()
        .params("path (.json); id? (default user.tools), name? (default My tools), overwrite? false"),
    CommandSpec::new("tool.library.inspect", "Inspect Tool Library", inspect).icon("inspect").noundo().doc_only()
        .params("path; returns the validated library with entry ids and content revisions"),
    CommandSpec::new("tool.library.add", "Add Tool To Library", add).icon("import").noundo().doc_only()
        .params("path, family, label, params {…}, unit? mm|inch, kind? cutting, vendor?, product_id?, product_url?, machine_connection? {standard,size}, work_connection?, notes?, provenance?; all lengths stored in mm"),
    CommandSpec::new("tool.library.edit", "Edit Tool Library Entry", edit).icon("import").noundo().doc_only()
        .params("path, entry (current id), plus any add fields to replace (params replaces the whole set); the entry keeps its content revision, so its id may change"),
    CommandSpec::new("tool.library.remove", "Remove Tool From Library", remove).icon("delete").noundo().doc_only()
        .params("path, entry"),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnReq {
    standard: String,
    size: String,
}
impl ConnReq {
    fn to_conn(&self) -> tooling::Connection {
        tooling::Connection { standard: self.standard.clone(), size: self.size.clone() }
    }
}

/// The fields that define an entry's content, shared by add and edit. `None` means "leave as is"
/// (edit) or "not given" (add).
#[derive(Default)]
struct Fields<'a> {
    family: Option<&'a str>,
    label: Option<&'a str>,
    params: Option<&'a Value>,
    unit: Option<&'a str>,
    vendor: Option<&'a String>,
    product_id: Option<&'a String>,
    product_url: Option<&'a String>,
    machine_connection: Option<&'a ConnReq>,
    work_connection: Option<&'a ConnReq>,
    notes: Option<&'a String>,
    provenance: Option<&'a Provenance>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    path: String,
    #[serde(default = "default_id")]
    id: String,
    #[serde(default = "default_name")]
    name: String,
    #[serde(default)]
    overwrite: bool,
}
fn default_id() -> String {
    "user.tools".into()
}
fn default_name() -> String {
    "My tools".into()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inspect {
    path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Add {
    path: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    unit: Option<String>,
    #[serde(default)]
    vendor: Option<String>,
    #[serde(default)]
    product_id: Option<String>,
    #[serde(default)]
    product_url: Option<String>,
    #[serde(default)]
    machine_connection: Option<ConnReq>,
    #[serde(default)]
    work_connection: Option<ConnReq>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    provenance: Option<Provenance>,
}
impl Add {
    fn fields(&self) -> Fields<'_> {
        Fields {
            family: self.family.as_deref(),
            label: self.label.as_deref(),
            params: self.params.as_ref(),
            unit: self.unit.as_deref(),
            vendor: self.vendor.as_ref(),
            product_id: self.product_id.as_ref(),
            product_url: self.product_url.as_ref(),
            machine_connection: self.machine_connection.as_ref(),
            work_connection: self.work_connection.as_ref(),
            notes: self.notes.as_ref(),
            provenance: self.provenance.as_ref(),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    path: String,
    entry: String,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    unit: Option<String>,
    #[serde(default)]
    vendor: Option<String>,
    #[serde(default)]
    product_id: Option<String>,
    #[serde(default)]
    product_url: Option<String>,
    #[serde(default)]
    machine_connection: Option<ConnReq>,
    #[serde(default)]
    work_connection: Option<ConnReq>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    provenance: Option<Provenance>,
}
impl Edit {
    fn fields(&self) -> Fields<'_> {
        Fields {
            family: self.family.as_deref(),
            label: self.label.as_deref(),
            params: self.params.as_ref(),
            unit: self.unit.as_deref(),
            vendor: self.vendor.as_ref(),
            product_id: self.product_id.as_ref(),
            product_url: self.product_url.as_ref(),
            machine_connection: self.machine_connection.as_ref(),
            work_connection: self.work_connection.as_ref(),
            notes: self.notes.as_ref(),
            provenance: self.provenance.as_ref(),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Remove {
    path: String,
    entry: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileReq {
    family: String,
    params: Value,
    #[serde(default)]
    unit: Option<String>,
}

fn path(cmd: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(bad(cmd, "invalid tool library path"));
    }
    solidcraft_io::vfs::check(value).map_err(|e| bad(cmd, e.to_string()))
}

fn read_library(cmd: &str, path: &str) -> Result<ToolLibrary> {
    let len = solidcraft_io::vfs::len(path).map_err(|e| bad(cmd, format!("could not read tool library: {e}")))?;
    if len > tooling::model::MAX_BYTES as u64 {
        return Err(bad(cmd, "tool library exceeds 8 MiB"));
    }
    let bytes = solidcraft_io::vfs::read(path, tooling::model::MAX_BYTES).map_err(|e| bad(cmd, format!("could not read tool library: {e}")))?;
    ToolLibrary::decode(&bytes).map_err(|e| bad(cmd, e.to_string()))
}

fn write_library(cmd: &str, path: &str, library: &ToolLibrary, replace: bool) -> Result<()> {
    let bytes = library.encode().map_err(|e| bad(cmd, e.to_string()))?;
    #[cfg(not(target_arch = "wasm32"))]
    let result = if replace {
        solidcraft_io::write_atomic(std::path::Path::new(path), &bytes, false)
    } else {
        solidcraft_io::safe_write::create_atomic(std::path::Path::new(path), &bytes)
    };
    #[cfg(target_arch = "wasm32")]
    let result = if !replace && solidcraft_io::vfs::exists(path) {
        Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "tool library already exists"))
    } else {
        solidcraft_io::vfs::write(path, &bytes)
    };
    result.map_err(|e| bad(cmd, format!("could not write tool library: {e}")))
}

/// The unit lengths arrive in; stored parameters are always millimetres.
enum Unit {
    Mm,
    Inch,
}
fn unit(cmd: &str, value: Option<&str>) -> Result<Unit> {
    match value.unwrap_or("mm") {
        "mm" => Ok(Unit::Mm),
        "inch" => Ok(Unit::Inch),
        other => Err(bad(cmd, format!("unit must be mm or inch, not `{other}`"))),
    }
}

/// Parses a hostile parameter object, converts lengths from the request unit, and validates
/// against the family schema.
fn parse_params(cmd: &str, value: &Value, family: Family, unit: &Unit) -> Result<BTreeMap<String, f64>> {
    let object = value.as_object().ok_or_else(|| bad(cmd, "params must be an object of numbers"))?;
    if object.len() > tooling::model::MAX_PARAMS {
        return Err(bad(cmd, "params has too many entries"));
    }
    let mut out = BTreeMap::new();
    for (key, raw) in object {
        let number =
            raw.as_f64().filter(|v| v.is_finite() && v.abs() < 1e9).ok_or_else(|| bad(cmd, format!("params.{key} must be a finite number")))?;
        let scaled = match unit {
            Unit::Mm => number,
            Unit::Inch => match family.schema().iter().find(|s| s.name == key) {
                Some(spec) if spec.unit == tooling::ParamUnit::Length => number * 25.4,
                _ => number,
            },
        };
        out.insert(key.clone(), scaled);
    }
    tooling::family::validate_params(family, &out).map_err(|e| bad(cmd, e.to_string()))?;
    Ok(out)
}

fn parse_family(cmd: &str, value: &str) -> Result<Family> {
    Family::parse(value).ok_or_else(|| bad(cmd, format!("unknown tool family `{value}` (see tool.families)")))
}

fn families(_s: &mut Session, _p: &Value) -> Result<Value> {
    let list: Vec<Value> = tooling::family::FAMILIES
        .iter()
        .map(|f| {
            json!({
                "id": f.as_str(),
                "label": f.label(),
                "params": f.schema().iter().map(|s| json!({
                    "name": s.name,
                    "min": s.min,
                    "max": s.max,
                    "default": s.default,
                    "unit": match s.unit { tooling::ParamUnit::Length => "mm", tooling::ParamUnit::Angle => "deg" },
                    "doc": s.doc,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({"families": list, "kinds": ["cutting"]}))
}

fn profile(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "tool.profile";
    let request: ProfileReq = serde_json::from_value(p.clone()).map_err(|e| bad(cmd, e.to_string()))?;
    let family = parse_family(cmd, &request.family)?;
    let params = parse_params(cmd, &request.params, family, &unit(cmd, request.unit.as_deref())?)?;
    let profile = tooling::profile::of(family, &params).map_err(|e| bad(cmd, e.to_string()))?;
    serde_json::to_value(profile).map_err(|e| bad(cmd, e.to_string()))
}

/// The default user tool library file: `<data_dir>/libraries/tools.solidcraft-tools.json`;
/// in the browser build, the same name inside the virtual filesystem.
fn default_path(cmd: &str) -> Result<std::path::PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    let dir = crate::recovery::data_dir().ok_or_else(|| bad(cmd, "no per-user data folder is available"))?.join("libraries");
    #[cfg(target_arch = "wasm32")]
    let dir = std::path::PathBuf::from("libraries");
    Ok(dir.join("tools.solidcraft-tools.json"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Open {
    #[serde(default)]
    path: Option<String>,
}

fn open(_s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "tool.library.open";
    let request: Open = serde_json::from_value(p.clone()).map_err(|e| bad(CMD, e.to_string()))?;
    let text = match request.path {
        Some(text) => {
            path(CMD, &text)?;
            if !text.to_ascii_lowercase().ends_with(".json") {
                return Err(bad(CMD, "tool library path must end in .json"));
            }
            text
        }
        None => default_path(CMD)?.to_string_lossy().into_owned(),
    };
    let file = std::path::PathBuf::from(&text);
    if !solidcraft_io::vfs::exists(&text) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(parent) = file.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return Err(bad(CMD, format!("could not create the tool library folder: {e}")));
        }
        let library = ToolLibrary::new(&default_id(), &default_name()).map_err(|e| bad(CMD, e.to_string()))?;
        if let Err(e) = write_library(CMD, &text, &library, false)
            && !solidcraft_io::vfs::exists(&text)
        {
            return Err(e);
        }
    }
    let library = read_library(CMD, &text)?;
    Ok(json!({"path": text, "library": library}))
}

fn create(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "tool.library.create";
    let request: Create = serde_json::from_value(p.clone()).map_err(|e| bad(cmd, e.to_string()))?;
    path(cmd, &request.path)?;
    if !request.path.to_ascii_lowercase().ends_with(".json") {
        return Err(bad(cmd, "tool library path must end in .json"));
    }
    let library = ToolLibrary::new(&request.id, &request.name).map_err(|e| bad(cmd, e.to_string()))?;
    write_library(cmd, &request.path, &library, request.overwrite)?;
    Ok(json!({"path": request.path, "id": library.id, "name": library.name, "entries": 0}))
}

fn inspect(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "tool.library.inspect";
    let request: Inspect = serde_json::from_value(p.clone()).map_err(|e| bad(cmd, e.to_string()))?;
    path(cmd, &request.path)?;
    let library = read_library(cmd, &request.path)?;
    Ok(json!({"library": library}))
}

/// Applies the request fields to an entry (edit: starting from the stored one; add: from empty).
fn apply_fields(cmd: &str, entry: &mut ToolEntry, fields: &Fields) -> Result<()> {
    if let Some(family) = fields.family {
        entry.family = parse_family(cmd, family)?.as_str().to_string();
    }
    if let Some(label) = fields.label {
        entry.label = label.to_string();
    }
    if let Some(params) = fields.params {
        let family = Family::parse(&entry.family).ok_or_else(|| bad(cmd, format!("unknown tool family `{}`", entry.family)))?;
        entry.params = parse_params(cmd, params, family, &unit(cmd, fields.unit)?)?;
    }
    if fields.vendor.is_some() {
        entry.vendor = fields.vendor.cloned();
    }
    if fields.product_id.is_some() {
        entry.product_id = fields.product_id.cloned();
    }
    if fields.product_url.is_some() {
        entry.product_url = fields.product_url.cloned();
    }
    if fields.machine_connection.is_some() {
        entry.machine_connection = fields.machine_connection.map(ConnReq::to_conn);
    }
    if fields.work_connection.is_some() {
        entry.work_connection = fields.work_connection.map(ConnReq::to_conn);
    }
    if fields.notes.is_some() {
        entry.notes = fields.notes.cloned();
    }
    if fields.provenance.is_some() {
        entry.provenance = fields.provenance.cloned().unwrap_or_default();
    }
    Ok(())
}

fn add(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "tool.library.add";
    let request: Add = serde_json::from_value(p.clone()).map_err(|e| bad(cmd, e.to_string()))?;
    path(cmd, &request.path)?;
    let kind = match request.kind.as_deref().unwrap_or("cutting") {
        "cutting" => ItemKind::Cutting,
        other if ItemKind::parse(other).is_some() => return Err(bad(cmd, format!("not supported yet: tool kind `{other}`"))),
        other => return Err(bad(cmd, format!("unknown tool kind `{other}`"))),
    };
    if request.family.is_none() {
        return Err(bad(cmd, "`family` is required (see tool.families)"));
    }
    if request.label.is_none() || request.params.is_none() {
        return Err(bad(cmd, "`label` and `params` are required"));
    }
    let mut library = read_library(cmd, &request.path)?;
    let mut entry = ToolEntry {
        id: String::new(),
        revision: String::new(),
        kind,
        family: String::new(),
        label: String::new(),
        vendor: None,
        product_id: None,
        product_url: None,
        params: BTreeMap::new(),
        machine_connection: None,
        work_connection: None,
        notes: None,
        provenance: Provenance::default(),
        vendor_properties: BTreeMap::new(),
    };
    apply_fields(cmd, &mut entry, &request.fields())?;
    entry.assign_id(&library.id).map_err(|e| bad(cmd, e.to_string()))?;
    if library.find(&entry.id).is_some() {
        return Err(bad(cmd, "an identical entry is already in the library"));
    }
    library.entries.push(entry.clone());
    library.validate().map_err(|e| bad(cmd, e.to_string()))?;
    write_library(cmd, &request.path, &library, true)?;
    Ok(json!({"entry": entry.id, "revision": entry.revision, "entries": library.entries.len()}))
}

fn edit(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "tool.library.edit";
    let request: Edit = serde_json::from_value(p.clone()).map_err(|e| bad(cmd, e.to_string()))?;
    path(cmd, &request.path)?;
    let mut library = read_library(cmd, &request.path)?;
    let index = library.entries.iter().position(|e| e.id == request.entry).ok_or_else(|| bad(cmd, "entry is not in the library"))?;
    let mut entry = library.entries[index].clone();
    apply_fields(cmd, &mut entry, &request.fields())?;
    entry.assign_id(&library.id).map_err(|e| bad(cmd, e.to_string()))?;
    if entry.id != request.entry && library.entries.iter().any(|e| e.id == entry.id) {
        return Err(bad(cmd, "the edited entry would collide with an identical existing entry"));
    }
    library.entries[index] = entry.clone();
    library.validate().map_err(|e| bad(cmd, e.to_string()))?;
    write_library(cmd, &request.path, &library, true)?;
    Ok(json!({"entry": entry.id, "previous": request.entry, "revision": entry.revision, "entries": library.entries.len()}))
}

fn remove(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "tool.library.remove";
    let request: Remove = serde_json::from_value(p.clone()).map_err(|e| bad(cmd, e.to_string()))?;
    path(cmd, &request.path)?;
    let mut library = read_library(cmd, &request.path)?;
    let before = library.entries.len();
    library.entries.retain(|e| e.id != request.entry);
    if library.entries.len() == before {
        return Err(bad(cmd, "entry is not in the library"));
    }
    write_library(cmd, &request.path, &library, true)?;
    Ok(json!({"removed": request.entry, "entries": library.entries.len()}))
}

#[cfg(test)]
#[path = "tooling_tests.rs"]
mod tests;
