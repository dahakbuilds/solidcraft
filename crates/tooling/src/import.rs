//! Import vendor tool data: GTC/p21 CAM tool files and SolidCraft tool JSON.
//!
//! GTC/p21 ("CNC CAM tool" files: `.gtc`, `.p21`, `.tl`, `.tpt`) is the plain-text,
//! line-oriented format post-processors write for CAM systems. Posts disagree about which
//! records they emit, so the parser reads the common records, ignores what it does not
//! know, and reports per-tool skips instead of failing the whole file. Lengths arrive in
//! millimetres unless the file (or the caller) says inches.
//!
//! The JSON form is SolidCraft's own: an array of tool objects, or an object with a
//! `tools` array — the same fields `tool.library.add` takes, so a vendor sheet can be
//! edited into it by hand.
//!
//! Like the rest of the crate this is pure data work: no files, no UI. The engine command
//! owns file access and turns each [`ImportedTool`] into a validated library entry.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::{Result, family, family::Family, holder, invalid};

/// Largest number of tools one import may produce.
pub const MAX_TOOLS: usize = 1024;
/// Longest GTC line we look at, in bytes.
const MAX_LINE: usize = 4096;
/// Longest GTC file we parse, in lines.
const MAX_LINES: usize = 65536;

/// One tool as an importer understood it, before it becomes a library entry.
#[derive(Clone, Debug)]
pub struct ImportedTool {
    pub family: Family,
    pub label: String,
    /// The family's parameters in millimetres and degrees, every schema field filled.
    pub params: BTreeMap<String, f64>,
    pub holder_type: Option<String>,
    pub stickout: Option<f64>,
    pub vendor: Option<String>,
    pub product_id: Option<String>,
    pub notes: Option<String>,
    /// Raw source fields kept verbatim (`gtc.tool_type` …), never parsed further.
    pub vendor_properties: BTreeMap<String, String>,
}

/// One tool the importer could not use, and why.
#[derive(Clone, Debug)]
pub struct Skipped {
    /// How the tool was described in the file (its name, number or index).
    pub what: String,
    pub why: String,
}

/// What an import produced: usable tools and the skips beside them, never a silent drop.
#[derive(Clone, Debug, Default)]
pub struct ImportReport {
    pub tools: Vec<ImportedTool>,
    pub skipped: Vec<Skipped>,
}

// --------------------------------------------------------------------------- GTC/p21

/// One tool block of a GTC file: every `KEY/fields` record seen, first occurrence kept.
#[derive(Default)]
struct Block {
    unit_inch: bool,
    fields: BTreeMap<String, Vec<String>>,
}

impl Block {
    fn text(&self, key: &str) -> Option<&str> {
        self.fields.get(key)?.iter().map(String::as_str).find(|s| !s.is_empty()).map(str::trim)
    }
    fn number(&self, key: &str) -> Option<f64> {
        self.fields.get(key)?.iter().filter_map(|f| parse_f64(f)).next()
    }
    /// The nth field of a record, 0-based.
    fn field(&self, key: &str, index: usize) -> Option<f64> {
        self.fields.get(key)?.get(index).and_then(|f| parse_f64(f))
    }
    fn keep(&mut self, key: String, fields: Vec<String>) {
        self.fields.entry(key).or_insert(fields);
    }
}

/// A GTC number: plain decimal, tolerant of a leading sign and surrounding blanks.
fn parse_f64(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<f64>().ok().filter(|v| v.is_finite() && v.abs() < 1e9)
}

/// Uppercase and drop everything that is not a letter or digit, so `TAPER_BALL`,
/// `taper ball` and `Taper-Ball` all compare equal.
fn normalise(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace() && !matches!(c, '_' | '-' | '\'' | '.')).flat_map(char::to_uppercase).collect()
}

/// Split a GTC line into its upper-cased key and its comma-separated fields.
fn split_record(line: &str) -> Option<(String, Vec<String>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('(') {
        return None;
    }
    let (key, rest) = match line.find('/').or(line.find(',')) {
        Some(i) => (line[..i].trim(), &line[i + 1..]),
        None => (line, ""),
    };
    if key.is_empty() {
        return None;
    }
    let fields = rest.split(',').map(|f| f.trim().to_string()).collect();
    Some((key.to_ascii_uppercase(), fields))
}

/// Parse a GTC/p21 tool file. `unit_inch` is the fallback unit for files that do not
/// carry a `UNIT` record.
pub fn parse_gtc(text: &str, unit_inch: bool) -> ImportReport {
    let mut report = ImportReport::default();
    let mut current: Option<Block> = None;
    let mut global_unit_inch = unit_inch;
    for line in text.lines().take(MAX_LINES) {
        if line.len() > MAX_LINE {
            continue;
        }
        let Some((key, fields)) = split_record(line) else { continue };
        if key == "UNIT" || key == "UNITS" {
            if let Some(unit) = fields.first().map(|f| normalise(f)) {
                if unit == "INCH" || unit == "IN" {
                    global_unit_inch = true;
                } else if unit == "MM" {
                    global_unit_inch = false;
                }
            }
            continue;
        }
        if key == "TOOL" {
            if let Some(block) = current.take() {
                finish_block(block, &mut report);
            }
            if report.tools.len() + report.skipped.len() >= MAX_TOOLS {
                report.skipped.push(Skipped { what: "…".into(), why: format!("the file carries more than {MAX_TOOLS} tools") });
                break;
            }
            let mut block = Block { unit_inch: global_unit_inch, fields: BTreeMap::new() };
            block.keep("TOOL".into(), fields);
            current = Some(block);
            continue;
        }
        // Some posts omit the `TOOL,` marker; a repeated TOOL_TYPE or TOOL_NO opens the
        // next block, so a file that mixes markers still splits per tool.
        let repeats = current
            .as_ref()
            .is_some_and(|b| (key == "TOOL_TYPE" && b.fields.contains_key("TOOL_TYPE")) || (key == "TOOL_NO" && b.fields.contains_key("TOOL_NO")));
        if repeats {
            if let Some(block) = current.take() {
                finish_block(block, &mut report);
            }
            if report.tools.len() + report.skipped.len() >= MAX_TOOLS {
                report.skipped.push(Skipped { what: "…".into(), why: format!("the file carries more than {MAX_TOOLS} tools") });
                break;
            }
        }
        if current.is_none() {
            current = Some(Block { unit_inch: global_unit_inch, fields: BTreeMap::new() });
        }
        if let Some(block) = current.as_mut() {
            block.keep(key, fields);
        }
    }
    if let Some(block) = current {
        finish_block(block, &mut report);
    }
    report
}

fn finish_block(block: Block, report: &mut ImportReport) {
    match build_gtc_tool(&block) {
        Ok(tool) => report.tools.push(tool),
        Err(why) => report.skipped.push(Skipped { what: describe_block(&block), why }),
    }
}

fn describe_block(block: &Block) -> String {
    for key in ["NAME", "DESCR", "DESC", "COMMENT", "TOOL"] {
        if let Some(text) = block.text(key) {
            return text.to_string();
        }
    }
    match block.text("TOOL_NO").or_else(|| block.fields.get("TOOL_NO").and_then(|f| f.first()).map(String::as_str)) {
        Some(n) => format!("tool {n}"),
        None => "unnamed tool".into(),
    }
}

/// Map a normalised GTC tool type onto one of our families. An empty type is inferred
/// from the corner and diameter; anything not recognisable as a milling cutter is an
/// error the report shows verbatim.
fn family_of(type_key: &str, corner: f64, diameter: Option<f64>) -> std::result::Result<(Family, f64), String> {
    use Family::*;
    let exact = match type_key {
        "" => None,
        "END" | "ENDMILL" | "FLAT" | "FLATEND" | "FLATENDMILL" => Some((EndMillFlat, 0.0)),
        "BALL" | "BALLNOSE" => Some((EndMillBall, 0.0)),
        "TAPERBALL" | "TAPER" => Some((EndMillBall, 7.0)),
        "TAPEREND" => Some((EndMillFlat, 7.0)),
        "BULL" | "BULLNOSE" => Some((EndMillBull, 0.0)),
        "RADIUS" | "CORNERRADIUS" | "CORNERROUND" => Some((CornerRadius, 0.0)),
        "DRILL" | "DRILLING" => Some((Drill, 0.0)),
        "SPOT" | "CENTER" | "CENTRE" | "CENTERDRILL" | "CENTREDRILL" => Some((CenterDrill, 0.0)),
        "CHAMFER" | "CHAMFERMILL" => Some((ChamferMill, 0.0)),
        "CSINK" | "COUNTERSINK" => Some((Countersink, 0.0)),
        "CBORE" | "COUNTERBORE" => Some((Counterbore, 0.0)),
        "TAP" | "TAPPING" => Some((Tap, 0.0)),
        "KEY" | "KEYWAY" => Some((Keyway, 0.0)),
        "TSLOT" => Some((TSlot, 0.0)),
        other => return Err(format!("tool type `{other}` is not a supported milling type")),
    };
    if let Some(found) = exact {
        return Ok(found);
    }
    // No TOOL_TYPE record: guess from the geometry the file does carry.
    if let Some(d) = diameter {
        if corner > 0.0 && (d - 2.0 * corner).abs() < 1e-6 {
            return Ok((EndMillBall, 0.0));
        }
        if corner > 0.0 {
            return Ok((EndMillBull, 0.0));
        }
        return Ok((EndMillFlat, 0.0));
    }
    Err("the block carries neither a tool type nor a diameter".into())
}

fn build_gtc_tool(block: &Block) -> std::result::Result<ImportedTool, String> {
    let corner = block.number("CORNER").unwrap_or(0.0).max(0.0);
    let diameter = block.number("CUT_DIAMETER").or_else(|| block.number("DIAMETER")).or_else(|| block.field("TOOL_NO", 2));
    let type_key = block.text("TOOL_TYPE").map(normalise).unwrap_or_default();
    let (family, taper_default) = family_of(&type_key, corner, diameter)?;
    // Posts write a plain `END` with a corner radius for a bull nose.
    let (family, taper_default) = match (family, corner) {
        (Family::EndMillFlat, c) if c > 0.0 => (Family::EndMillBull, taper_default),
        _ => (family, taper_default),
    };
    // A ball is often written as just its corner radius: the diameter is twice the corner.
    let diameter = diameter.or(match family {
        Family::EndMillBall if corner > 0.0 => Some(2.0 * corner),
        _ => None,
    });

    let mut given: BTreeMap<String, f64> = BTreeMap::new();
    let mut put = |name: &str, value: Option<f64>| {
        if let Some(v) = value.filter(|v| *v > 0.0) {
            given.insert(name.to_string(), v);
        }
    };
    // The file's one diameter means different things per family; give it only to the
    // parameter the family actually cuts with.
    let diameter_key = match family {
        Family::Tap | Family::CenterDrill => "major_diameter",
        Family::TSlot => "crossbar_diameter",
        Family::Counterbore => "counterbore_diameter",
        _ => "cutting_diameter",
    };
    put(diameter_key, diameter);
    put("corner_radius", block.number("CORNER"));
    put("flute_length", block.number("CUTDEPTH").or_else(|| block.number("FLUTE_LENGTH")).or_else(|| block.number("CUT_LENGTH")));
    put("thread_length", block.number("CUTDEPTH"));
    put("counterbore_depth", block.number("CUTDEPTH"));
    put("overall_length", block.number("LENGTH").or_else(|| block.number("OVERALL")).or_else(|| block.number("TOOL_LENGTH")));
    put("shank_diameter", block.field("SHANK", 0));
    put("point_angle", block.number("POINT_ANGLE").or_else(|| block.number("ANGLE")).or_else(|| block.number("POINT")));
    put("included_angle", block.number("ANGLE"));
    put("taper_angle", block.number("TAPER").or(Some(taper_default)));
    put("pitch", block.number("PITCH"));

    let params = fill_params(family, given, block.unit_inch)?;

    let mut notes = None;
    let mut holder_type = None;
    let mut stickout = None;
    if let Some(text) = block.text("HOLDER").filter(|t| !normalise(t).is_empty()) {
        match match_holder(text) {
            Some(found) => {
                holder_type = Some(found.id.clone());
                let from_file = block.field("SHANK", 3).filter(|s| *s > found.nose_len && *s <= 500.0);
                stickout = Some(from_file.unwrap_or((found.nose_len + 10.0).min(500.0)));
            }
            None => notes = Some(format!("holder `{text}` is not in the catalogue")),
        }
    }

    let label = block
        .text("NAME")
        .or_else(|| block.text("DESCR"))
        .or_else(|| block.text("DESC"))
        .or_else(|| block.text("COMMENT"))
        .map(sanitise_label)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| match diameter {
            Some(d) => format!("{d} mm {}", family.label().to_ascii_lowercase()),
            None => format!("Tool {}", block.text("TOOL_NO").unwrap_or("?")),
        });

    let mut vendor_properties = BTreeMap::new();
    for key in ["TOOL_TYPE", "TOOL_NO", "HOLDER", "SIDE_CUT_SPEED", "FEED", "MAX_SPEED"] {
        let raw = block.fields.get(key).map(|f| f.iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(",")).filter(|s| !s.is_empty());
        if let Some(raw) = raw {
            vendor_properties.insert(format!("gtc.{}", key.to_ascii_lowercase()), raw);
        }
    }

    Ok(ImportedTool { family, label, params, holder_type, stickout, vendor: None, product_id: None, notes, vendor_properties })
}

/// Match a GTC `HOLDER` name against the catalogue: every part of a catalogue id
/// (`er32_bt40` → `ER32`, `BT40`) must appear in the name, and exactly one holder may
/// match, so `ER32` alone stays unmatched rather than guessing a spindle.
fn match_holder(text: &str) -> Option<holder::HolderType> {
    let needle = normalise(text);
    if needle.is_empty() {
        return None;
    }
    let hits: Vec<holder::HolderType> = holder::catalog()
        .into_iter()
        .filter(|h| {
            let parts: Vec<String> = h.id.split('_').map(normalise).collect();
            !parts.is_empty() && parts.iter().all(|p| needle.contains(p))
        })
        .collect();
    if hits.len() == 1 { hits.first().cloned() } else { None }
}

/// Convert the given parameters to millimetres, validate them against the family, and
/// fill every schema default the file left out.
fn fill_params(family: Family, given: BTreeMap<String, f64>, unit_inch: bool) -> std::result::Result<BTreeMap<String, f64>, String> {
    let mut params = BTreeMap::new();
    for spec in family.schema() {
        let value = match given.get(spec.name) {
            Some(v) => {
                if spec.unit == family::ParamUnit::Length && unit_inch {
                    *v * 25.4
                } else {
                    *v
                }
            }
            None => match spec.default {
                Some(d) => d,
                None => return Err(format!("`{}` is required but the file does not carry it", spec.name)),
            },
        };
        params.insert(spec.name.to_string(), value);
    }
    family::validate_params(family, &params).map_err(|e| e.to_string())?;
    Ok(params)
}

/// Trim, drop control characters and cap at the library's 128-byte label bound.
fn sanitise_label(text: &str) -> String {
    let cleaned: String = text.chars().filter(|c| !c.is_control()).take(200).collect();
    let cleaned = cleaned.trim();
    match cleaned.len() {
        0..=128 => cleaned.to_string(),
        _ => {
            let mut cut = 128;
            while cut > 0 && !cleaned.is_char_boundary(cut) {
                cut -= 1;
            }
            cleaned[..cut].trim_end().to_string()
        }
    }
}

// ---------------------------------------------------------------------- tool JSON

/// Parse SolidCraft tool JSON: an array of tool objects or `{"tools": [...]}`.
/// Structural problems are an error; a bad tool inside a good file is a skip.
pub fn parse_json(text: &str) -> Result<ImportReport> {
    let root: Value = serde_json::from_str(text).map_err(|e| invalid(format!("invalid tool JSON: {e}")))?;
    let items = if let Some(list) = root.as_array() {
        list.clone()
    } else if let Some(list) = root.get("tools").and_then(Value::as_array) {
        list.clone()
    } else {
        return Err(invalid("tool JSON must be an array of tools or an object with a `tools` array"));
    };
    if items.len() > MAX_TOOLS {
        return Err(invalid(format!("tool JSON carries more than {MAX_TOOLS} tools")));
    }
    let mut report = ImportReport::default();
    for (index, item) in items.iter().enumerate() {
        match json_tool(item) {
            Ok(tool) => report.tools.push(tool),
            Err(why) => {
                let what = item.get("label").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("#{index}"));
                report.skipped.push(Skipped { what, why });
            }
        }
    }
    Ok(report)
}

fn json_tool(item: &Value) -> std::result::Result<ImportedTool, String> {
    let family = Family::parse(item.get("family").and_then(Value::as_str).ok_or("every tool needs a `family`")?).ok_or("unknown tool family")?;
    let label = sanitise_label(item.get("label").and_then(Value::as_str).ok_or("every tool needs a `label`")?);
    if label.is_empty() {
        return Err("the label is empty".into());
    }
    let unit_inch = match item.get("unit").and_then(Value::as_str).unwrap_or("mm") {
        "mm" => false,
        "inch" => true,
        other => return Err(format!("unit must be mm or inch, not `{other}`")),
    };
    let source = item.get("params").and_then(Value::as_object).ok_or("every tool needs a `params` object")?;
    if source.len() > crate::model::MAX_PARAMS {
        return Err("too many parameters".into());
    }
    let mut given = BTreeMap::new();
    for (key, raw) in source {
        let value = raw.as_f64().filter(|v| v.is_finite() && v.abs() < 1e9).ok_or_else(|| format!("params.{key} must be a finite number"))?;
        given.insert(key.clone(), value);
    }
    let params = fill_params(family, given, unit_inch)?;

    let holder_type = match item.get("holder_type").and_then(Value::as_str) {
        Some(id) if !id.is_empty() => {
            let found = holder::find(id).ok_or_else(|| format!("unknown holder type `{id}`"))?;
            let stickout = match item.get("stickout").and_then(Value::as_f64) {
                Some(s) => {
                    let mm = if unit_inch { s * 25.4 } else { s };
                    if mm.is_finite() && mm > found.nose_len && mm <= 500.0 {
                        Some(mm)
                    } else {
                        return Err("stickout must be greater than the holder nose length and at most 500 mm".into());
                    }
                }
                None => Some((found.nose_len + 10.0).min(500.0)),
            };
            (Some(id.to_string()), stickout)
        }
        _ => (None, None),
    };
    let optional = |key: &str| item.get(key).and_then(Value::as_str).map(sanitise_label).filter(|s| !s.is_empty());
    Ok(ImportedTool {
        family,
        label,
        params,
        holder_type: holder_type.0,
        stickout: holder_type.1,
        vendor: optional("vendor"),
        product_id: optional("product_id"),
        notes: item.get("notes").and_then(Value::as_str).map(str::to_string),
        vendor_properties: BTreeMap::new(),
    })
}

#[cfg(test)]
#[path = "import_tests.rs"]
mod tests;
