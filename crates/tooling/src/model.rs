//! The portable tool library file: identity, entries with content revisions, provenance.
//!
//! One JSON document per library (`.solidcraft-tools.json` by convention). An entry's `revision`
//! is the SHA-256 of its canonical content (everything except `id` and `revision`), so a design or
//! a command can pin the exact version it used; `id` embeds the library id and a revision prefix.
//! Bounds are enforced on read: hostile files are refused, never partially accepted.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    Result, ToolingError,
    family::{self, Family},
    invalid,
};

/// The format tag stored in every library file.
pub const FORMAT: &str = "solidcraft-tooling/1";
/// Largest library file we read.
pub const MAX_BYTES: usize = 8 << 20;
/// Largest number of entries in one library.
pub const MAX_ENTRIES: usize = 4096;
/// Largest number of parameters on one entry.
pub const MAX_PARAMS: usize = 64;
/// Largest number of raw vendor properties kept on one entry.
pub const MAX_VENDOR_PROPERTIES: usize = 256;

/// What kind of item an entry is. Only `Cutting` is built so far; the others are recognised so an
/// import can say "not supported yet" instead of silently mislabelling a holder as a cutter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Cutting,
    Holder,
    Adaptive,
    Assembly,
    Insert,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cutting => "cutting",
            Self::Holder => "holder",
            Self::Adaptive => "adaptive",
            Self::Assembly => "assembly",
            Self::Insert => "insert",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "cutting" => Some(Self::Cutting),
            "holder" => Some(Self::Holder),
            "adaptive" => Some(Self::Adaptive),
            "assembly" => Some(Self::Assembly),
            "insert" => Some(Self::Insert),
            _ => None,
        }
    }
}

/// A connection interface: a standard (`hsk-a`, `bt`, `cat`, `sk`, `psc`, a vendor name…) and a
/// size (`63`, `40`…). Typed interface geometry comes later; for now this records the identity
/// so assemblies and imports can reference it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub standard: String,
    pub size: String,
}

impl Connection {
    fn validate(&self) -> Result<()> {
        if !name(&self.standard, 32) || !name(&self.size, 32) {
            return Err(invalid("invalid connection (standard and size: 1–32 plain bytes)"));
        }
        Ok(())
    }
}

/// Where an entry's data came from. Claimed provenance, reviewed by the user on import.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Information only: never fetched or treated as a local path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

impl Default for Provenance {
    fn default() -> Self {
        Self { license: "LicenseRef-User-Provided".into(), author: None, source_url: None }
    }
}

impl Provenance {
    fn validate(&self) -> Result<()> {
        if !name(&self.license, 128)
            || self.author.as_ref().is_some_and(|s| !name(s, 256))
            || self.source_url.as_ref().is_some_and(|s| !text(s, 2048))
        {
            return Err(invalid("invalid tool provenance (license 128, author 256, source URL 2048 bytes)"));
        }
        Ok(())
    }
}

/// One tool (or holder, once holders are built) in a library.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolEntry {
    pub id: String,
    /// SHA-256 hex of the canonical content; see `content_revision`.
    pub revision: String,
    pub kind: ItemKind,
    /// The family id, e.g. `end_mill_bull`.
    pub family: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_url: Option<String>,
    /// The family's parameters, in millimetres and degrees.
    pub params: BTreeMap<String, f64>,
    /// The spindle-side interface (holder top, or the tool shank's interface).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_connection: Option<Connection>,
    /// The workpiece-side interface (the tool seat a holder offers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_connection: Option<Connection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub provenance: Provenance,
    /// Vendor properties kept verbatim on import (ISO 13399 names, vendor codes…), never parsed.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vendor_properties: BTreeMap<String, String>,
}

/// The content an entry's revision hashes: everything except identity fields.
#[derive(Serialize)]
struct Content<'a> {
    kind: ItemKind,
    family: &'a str,
    label: &'a str,
    vendor: &'a Option<String>,
    product_id: &'a Option<String>,
    product_url: &'a Option<String>,
    params: &'a BTreeMap<String, f64>,
    machine_connection: &'a Option<Connection>,
    work_connection: &'a Option<Connection>,
    notes: &'a Option<String>,
    provenance: &'a Provenance,
    vendor_properties: &'a BTreeMap<String, String>,
}

impl ToolEntry {
    /// The SHA-256 hex of this entry's canonical content.
    pub fn content_revision(&self) -> Result<String> {
        self.validate_content()?;
        let content = Content {
            kind: self.kind,
            family: &self.family,
            label: &self.label,
            vendor: &self.vendor,
            product_id: &self.product_id,
            product_url: &self.product_url,
            params: &self.params,
            machine_connection: &self.machine_connection,
            work_connection: &self.work_connection,
            notes: &self.notes,
            provenance: &self.provenance,
            vendor_properties: &self.vendor_properties,
        };
        let bytes = serde_json::to_vec(&content).map_err(|e| invalid(e.to_string()))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
    /// Completes an entry: validates it, computes its revision and derives its id within `library_id`.
    pub fn assign_id(&mut self, library_id: &str) -> Result<()> {
        if !identifier(library_id) {
            return Err(invalid("invalid tool library identifier"));
        }
        self.revision = self.content_revision()?;
        let prefix = self.revision.get(..8).ok_or_else(|| invalid("invalid revision"))?;
        self.id = format!("{library_id}/{}-{prefix}", self.kind.as_str());
        Ok(())
    }
    /// Validates content and identity against `library_id`.
    pub fn validate(&self, library_id: &str) -> Result<()> {
        self.validate_content()?;
        let (pack, local) = self.id.split_once('/').ok_or_else(|| invalid("entry id must be qualified by its library id"))?;
        if pack != library_id || !identifier(local) {
            return Err(invalid("entry id does not belong to this library"));
        }
        if self.revision.len() != 64 || !self.revision.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("entry revision must be a SHA-256 hex digest"));
        }
        if self.revision != self.content_revision()? {
            return Err(invalid("entry revision does not match its content"));
        }
        Ok(())
    }
    fn validate_content(&self) -> Result<()> {
        if self.kind != ItemKind::Cutting {
            return Err(ToolingError::Unsupported(format!("tool item kind `{}` (only cutting tools so far)", self.kind.as_str())));
        }
        let family = Family::parse(&self.family).ok_or_else(|| invalid(format!("unknown tool family `{}`", self.family)))?;
        if !name(&self.label, 128) {
            return Err(invalid("invalid tool label (1–128 plain bytes)"));
        }
        for value in [&self.vendor, &self.product_id].into_iter().flatten() {
            if !name(value, 128) {
                return Err(invalid("invalid vendor or product id (128 bytes)"));
            }
        }
        if self.product_url.as_ref().is_some_and(|s| !text(s, 2048)) {
            return Err(invalid("invalid product URL (2048 bytes)"));
        }
        if self.notes.as_ref().is_some_and(|s| !text(s, 4096)) {
            return Err(invalid("invalid notes (4096 bytes)"));
        }
        self.provenance.validate()?;
        for conn in self.machine_connection.iter().chain(self.work_connection.iter()) {
            conn.validate()?;
        }
        if self.params.len() > MAX_PARAMS {
            return Err(invalid(format!("at most {MAX_PARAMS} parameters per tool")));
        }
        for (key, value) in &self.params {
            if !name(key, 64) || !value.is_finite() {
                return Err(invalid("tool parameters need plain-byte names and finite values"));
            }
        }
        family::validate_params(family, &self.params)?;
        if self.vendor_properties.len() > MAX_VENDOR_PROPERTIES {
            return Err(invalid(format!("at most {MAX_VENDOR_PROPERTIES} vendor properties per tool")));
        }
        for (key, value) in &self.vendor_properties {
            if !name(key, 128) || !text(value, 1024) {
                return Err(invalid("invalid vendor property"));
            }
        }
        Ok(())
    }
}

/// A whole library file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolLibrary {
    pub format: String,
    pub id: String,
    pub name: String,
    pub entries: Vec<ToolEntry>,
}

impl ToolLibrary {
    pub fn new(id: &str, display: &str) -> Result<Self> {
        if !identifier(id) || !name(display, 128) {
            return Err(invalid("invalid tool library id or name"));
        }
        Ok(Self { format: FORMAT.into(), id: id.into(), name: display.into(), entries: Vec::new() })
    }
    pub fn find(&self, entry_id: &str) -> Option<&ToolEntry> {
        self.entries.iter().find(|e| e.id == entry_id)
    }
    pub fn validate(&self) -> Result<()> {
        if self.format != FORMAT {
            return Err(invalid(format!("not a tool library v1 (format `{}`)", self.format)));
        }
        if !identifier(&self.id) || !name(&self.name, 128) {
            return Err(invalid("invalid tool library id or name"));
        }
        if self.entries.len() > MAX_ENTRIES {
            return Err(invalid(format!("tool library exceeds {MAX_ENTRIES} entries")));
        }
        let mut ids = BTreeSet::new();
        for entry in &self.entries {
            entry.validate(&self.id)?;
            if !ids.insert(&entry.id) {
                return Err(invalid("duplicate tool entry id"));
            }
        }
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_BYTES {
            return Err(invalid("tool library exceeds 8 MiB"));
        }
        let library: Self = serde_json::from_slice(bytes).map_err(|e| invalid(format!("invalid tool library: {e}")))?;
        library.validate()?;
        Ok(library)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| invalid(e.to_string()))?;
        if bytes.len() > MAX_BYTES {
            return Err(invalid("tool library exceeds 8 MiB"));
        }
        Ok(bytes)
    }
}

/// A short display name: non-empty, trimmed, no control characters.
fn name(value: &str, max: usize) -> bool {
    !value.is_empty() && value == value.trim() && value.len() <= max && !value.chars().any(char::is_control)
}
/// Free text: like [`name`] but allowed to contain spaces inside.
fn text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
/// A library or entry id part: lower-case, digits, `._-`.
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
