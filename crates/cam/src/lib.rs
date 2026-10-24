//! SolidCraft CAM setup: the manufacturing side of a design. A [CamSetup] holds the
//! operations (each with its tool, stock and toolpath), the fixtures clamping the work,
//! and the work origin the machine counts from. Toolpaths are 3D [LINE and ARC](Segment)
//! segments; the machining math (cutter profiles, Z-map simulation, G-code decoding)
//! lives in `solidcraft-tooling`, which converts its cutter-location points into these
//! segments. All lengths are millimetres and all angles degrees, whatever an import
//! started from; Z is up. The crate never touches the filesystem, the network or the
//! UI — the engine commands own file access.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod model;
mod toolpath;

pub use model::{Allowance, CamSetup, Fixture, Operation, Stock, StockKind, ToolRef, WorkOrigin};
pub use toolpath::{DEFAULT_TOLERANCE, MAX_SEGMENTS, Polyline, Segment, Toolpath};

#[derive(Debug, thiserror::Error)]
pub enum CamError {
    #[error("{0}")]
    Invalid(String),
    #[error("not supported yet: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, CamError>;

pub(crate) fn invalid(message: impl Into<String>) -> CamError {
    CamError::Invalid(message.into())
}
