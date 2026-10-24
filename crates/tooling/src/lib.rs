//! Tool libraries: milling cutters, holders and tool assemblies.
//!
//! A tool is a typed parametric definition: a [family](Family) fixes the parameter schema and the
//! revolved [cutter profile](profile) the CAM math will later use; a [library](ToolLibrary) is a
//! portable JSON file of entries with content revisions, provenance and vendor identity. All
//! lengths are millimetres and all angles degrees, whatever unit an import started from.
//! The [import](import) module maps vendor data (GTC/p21 CAM tool files, tool JSON) onto
//! these types and the [starter](starter) module seeds a fresh library; the
//! crate never touches the filesystem, the network or the UI — the engine commands own file access.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod assembly;
pub mod family;
pub mod gcode;
pub mod holder;
pub mod import;
pub mod mesh;
pub mod model;
pub mod profile;
pub mod sim;
pub mod starter;

pub use assembly::Assembly;
pub use family::{Family, ParamSpec, ParamUnit};
pub use holder::HolderType;
pub use import::{ImportReport, ImportedTool, Skipped};
pub use mesh::{Revolved, Role};
pub use model::{Connection, ItemKind, Provenance, ToolEntry, ToolLibrary};
pub use profile::{CutterProfile, Segment};
pub use sim::{ClPoint, FACE_GOUGED, FACE_SIDES, FACE_TOP, SimOptions, SimReport, ZMap, simulate, surface};
pub use starter::starter_pack;

#[derive(Debug, thiserror::Error)]
pub enum ToolingError {
    #[error("{0}")]
    Invalid(String),
    #[error("not supported yet: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, ToolingError>;

pub(crate) fn invalid(message: impl Into<String>) -> ToolingError {
    ToolingError::Invalid(message.into())
}
