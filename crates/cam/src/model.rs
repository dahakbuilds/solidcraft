//! The CAM setup saved with a design: operations with their tool, stock and toolpath,
//! the fixtures clamping the work, and the work origin the machine counts from.
//!
//! Placement convention: a stock sits on its `origin` — a box is centred on it in XY
//! with its bottom at its height, a cylinder stands on it along +Z. Allowances add
//! stock beyond the nominal size, per face. Everything here is hostile input once it
//! comes from a file or a command, so every number is checked before use.

use crate::{Result, invalid};
use serde::{Deserialize, Serialize};

/// Largest number of operations one setup may hold.
pub const MAX_OPERATIONS: usize = 256;
/// Largest number of fixtures one setup may hold.
pub const MAX_FIXTURES: usize = 64;
/// Largest stock size or allowance in millimetres.
pub const MAX_STOCK_SIZE: f64 = 100_000.0;
/// Longest operation, fixture or tool name.
pub const MAX_NAME: usize = 120;

fn is_zero3(v: &[f64; 3]) -> bool {
    v[0] == 0.0 && v[1] == 0.0 && v[2] == 0.0
}

fn check_point(what: &str, v: &[f64; 3]) -> Result<()> {
    if !v.iter().all(|c| c.is_finite() && c.abs() <= MAX_STOCK_SIZE) {
        return Err(invalid(format!("{what} needs finite coordinates within {MAX_STOCK_SIZE} mm")));
    }
    Ok(())
}

fn check_name(what: &str, name: &str) -> Result<()> {
    if name.is_empty() || name.len() > MAX_NAME {
        return Err(invalid(format!("{what} must be between 1 and {MAX_NAME} bytes")));
    }
    Ok(())
}

/// How an operation names its cutter: a machine tool number, a library entry, or both.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToolRef {
    /// Machine tool number (the `T` word); 0 means unassigned.
    pub number: u32,
    /// Tool library entry id this operation cuts with.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub entry: String,
    /// Display name captured when the tool was assigned (libraries change).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

impl ToolRef {
    pub fn check(&self) -> Result<()> {
        check_name("tool name", &self.name)
    }
}

/// Added stock beyond the nominal size, per face, in millimetres (an add-stock feature).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Allowance {
    pub plus_x: f64,
    pub minus_x: f64,
    pub plus_y: f64,
    pub minus_y: f64,
    /// Above the nominal top.
    pub plus_z: f64,
    /// Below the nominal bottom.
    pub minus_z: f64,
}

impl Allowance {
    pub fn is_zero(&self) -> bool {
        *self == Self::default()
    }

    pub fn check(&self) -> Result<()> {
        for (face, v) in [("plus_x", self.plus_x), ("minus_x", self.minus_x), ("plus_y", self.plus_y), ("minus_y", self.minus_y), ("plus_z", self.plus_z), ("minus_z", self.minus_z)] {
            if !v.is_finite() || v < 0.0 || v > MAX_STOCK_SIZE {
                return Err(invalid(format!("allowance {face} must be between 0 and {MAX_STOCK_SIZE} mm")));
            }
        }
        Ok(())
    }
}

/// What the operation starts cutting from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StockKind {
    /// Rectangular bar, centred on the stock origin in XY, bottom at its height.
    Box { x: f64, y: f64, z: f64 },
    /// Round bar standing on the stock origin along +Z.
    Cylinder { radius: f64, height: f64 },
    /// An existing body of the design used as the stock.
    Body { body: String },
}

impl StockKind {
    pub fn check(&self) -> Result<()> {
        let size = |v: f64| v.is_finite() && v > 0.0 && v <= MAX_STOCK_SIZE;
        match self {
            StockKind::Box { x, y, z } => {
                if !(size(*x) && size(*y) && size(*z)) {
                    return Err(invalid("box stock needs positive sizes within the limit"));
                }
            }
            StockKind::Cylinder { radius, height } => {
                if !(size(*radius) && size(*height)) {
                    return Err(invalid("cylinder stock needs a positive radius and height"));
                }
            }
            StockKind::Body { body } => check_name("stock body", body)?,
        }
        Ok(())
    }
}

/// The stock of one operation: its nominal shape, its work origin and the allowances
/// added to it. The kind is required; `deny_unknown_fields` cannot apply because the
/// kind is flattened.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stock {
    #[serde(flatten)]
    pub kind: StockKind,
    /// The stock's work origin (see the module placement convention).
    #[serde(default, skip_serializing_if = "is_zero3")]
    pub origin: [f64; 3],
    #[serde(default, skip_serializing_if = "Allowance::is_zero")]
    pub allowance: Allowance,
}

impl Stock {
    /// A box centred on the origin with the given allowances.
    pub fn box_stock(x: f64, y: f64, z: f64, allowance: Allowance) -> Self {
        Stock { kind: StockKind::Box { x, y, z }, origin: [0.0; 3], allowance }
    }

    pub fn check(&self) -> Result<()> {
        self.kind.check()?;
        self.allowance.check()?;
        check_point("stock origin", &self.origin)
    }

    /// The axis-aligned box the stock occupies (machine coordinates), or `None` for a
    /// body stock, whose extent only the evaluated model knows.
    pub fn extent(&self) -> Option<([f64; 3], [f64; 3])> {
        let (mut min, mut max) = match &self.kind {
            StockKind::Box { x, y, z } => (
                [self.origin[0] - x / 2.0, self.origin[1] - y / 2.0, self.origin[2]],
                [self.origin[0] + x / 2.0, self.origin[1] + y / 2.0, self.origin[2] + z],
            ),
            StockKind::Cylinder { radius, height } => (
                [self.origin[0] - radius, self.origin[1] - radius, self.origin[2]],
                [self.origin[0] + radius, self.origin[1] + radius, self.origin[2] + height],
            ),
            StockKind::Body { .. } => return None,
        };
        min[0] -= self.allowance.minus_x;
        min[1] -= self.allowance.minus_y;
        min[2] -= self.allowance.minus_z;
        max[0] += self.allowance.plus_x;
        max[1] += self.allowance.plus_y;
        max[2] += self.allowance.plus_z;
        Some((min, max))
    }
}

/// The work origin: where the machine counts from and how its axes sit on the work.
/// `y_axis` is `z_axis × x_axis`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkOrigin {
    pub position: [f64; 3],
    pub x_axis: [f64; 3],
    pub z_axis: [f64; 3],
}

impl Default for WorkOrigin {
    fn default() -> Self {
        WorkOrigin { position: [0.0; 3], x_axis: [1.0, 0.0, 0.0], z_axis: [0.0, 0.0, 1.0] }
    }
}

impl WorkOrigin {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn check(&self) -> Result<()> {
        check_point("work origin position", &self.position)?;
        for (what, axis) in [("x_axis", self.x_axis), ("z_axis", self.z_axis)] {
            let v = solidcraft_geom::Vec3::new(axis[0], axis[1], axis[2]);
            if (v.len() - 1.0).abs() >= 1e-6 {
                return Err(invalid(format!("work origin {what} must be a unit vector")));
            }
        }
        let x = solidcraft_geom::Vec3::new(self.x_axis[0], self.x_axis[1], self.x_axis[2]);
        let z = solidcraft_geom::Vec3::new(self.z_axis[0], self.z_axis[1], self.z_axis[2]);
        if x.dot(z).abs() > 1e-6 {
            return Err(invalid("work origin axes must be perpendicular"));
        }
        Ok(())
    }
}

/// One machining operation: what cuts (tool), what it cuts (stock), and the path it
/// follows (toolpath). All three are optional while the operation is being set up.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Operation {
    pub id: u64,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stock: Option<Stock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toolpath: Option<crate::Toolpath>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

impl Operation {
    pub fn check(&self) -> Result<()> {
        check_name("operation name", &self.name)?;
        if let Some(tool) = &self.tool {
            tool.check()?;
        }
        if let Some(stock) = &self.stock {
            stock.check()?;
        }
        if let Some(path) = &self.toolpath {
            path.check()?;
        }
        if self.comment.as_ref().is_some_and(|c| c.len() > 4 * MAX_NAME) {
            return Err(invalid("operation comment is too long"));
        }
        Ok(())
    }
}

/// A fixture holding the work: a model file placed in machine coordinates.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Fixture {
    pub id: u64,
    pub name: String,
    /// The fixture model, read the way inserts read files (relative to the design).
    pub path: String,
    /// Where the fixture sits (its own origin in machine coordinates).
    #[serde(skip_serializing_if = "is_zero3")]
    pub origin: [f64; 3],
}

impl Fixture {
    pub fn check(&self) -> Result<()> {
        check_name("fixture name", &self.name)?;
        if self.path.is_empty() || self.path.len() > 1024 {
            return Err(invalid("fixture path must be between 1 and 1024 bytes"));
        }
        check_point("fixture origin", &self.origin)
    }
}

/// Everything CAM saves with a design. Empty by default: a design without CAM never
/// writes it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CamSetup {
    pub operations: Vec<Operation>,
    pub fixtures: Vec<Fixture>,
    /// The default work origin; an operation's stock may carry its own.
    pub origin: WorkOrigin,
    /// The tool library this design cuts from (a library file path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_library: Option<String>,
}

impl CamSetup {
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty() && self.fixtures.is_empty() && self.origin.is_default() && self.tool_library.is_none()
    }

    pub fn find_operation(&self, id: u64) -> Option<&Operation> {
        self.operations.iter().find(|o| o.id == id)
    }

    pub fn find_operation_mut(&mut self, id: u64) -> Option<&mut Operation> {
        self.operations.iter_mut().find(|o| o.id == id)
    }

    /// Refuse a setup that could not have been built by the commands (and therefore
    /// came from a hostile file).
    pub fn check(&self) -> Result<()> {
        if self.operations.len() > MAX_OPERATIONS {
            return Err(invalid(format!("a design holds at most {MAX_OPERATIONS} CAM operations")));
        }
        if self.fixtures.len() > MAX_FIXTURES {
            return Err(invalid(format!("a design holds at most {MAX_FIXTURES} fixtures")));
        }
        self.origin.check()?;
        let mut ids: Vec<u64> = Vec::new();
        for op in &self.operations {
            op.check()?;
            if ids.contains(&op.id) {
                return Err(invalid(format!("two CAM operations share id {}", op.id)));
            }
            ids.push(op.id);
        }
        for fixture in &self.fixtures {
            fixture.check()?;
            if ids.contains(&fixture.id) {
                return Err(invalid(format!("a fixture and an operation share id {}", fixture.id)));
            }
            ids.push(fixture.id);
        }
        if self.tool_library.as_ref().is_some_and(|p| p.is_empty() || p.len() > 1024) {
            return Err(invalid("the tool library path must be between 1 and 1024 bytes"));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
