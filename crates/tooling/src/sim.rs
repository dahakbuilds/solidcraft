//! Z-map cut simulation: replay a milling toolpath against stock and the finished part,
//! reporting removed volume, gouges (material cut below the part) and misses (material
//! left above the part).
//!
//! Stock and part are heightfields (Z-maps) over one XY grid — the classic 2.5-D CAM
//! simulation: fast, analytic, and exact for the vertical-wall work this tool library's
//! families do. Each move is sampled at half a cell; at every sample the cutter profile's
//! cutting surface ([`CutterProfile::cut_height_at`]) lowers the stock inside its radius,
//! and anything cut below the part surface minus tolerance is a gouge. Rapid moves cut
//! nothing but are tested against the full envelope, so a shank crash is reported.
//!
//! Everything is bounded — grid, path length and sample budget refuse hostile input
//! before it can spend unbounded time or memory.

use crate::{
    Result, invalid,
    profile::CutterProfile,
};

/// Largest grid edge in cells.
pub const MAX_GRID: usize = 384;
/// Smallest cell size in millimetres.
pub const MIN_CELL: f64 = 0.05;
/// Largest number of path points one simulation may replay.
pub const MAX_PATH: usize = 100_000;
/// Total sample budget across all moves.
pub const MAX_SAMPLES: usize = 400_000;
/// Cell-touch budget across a whole replay (samples × cells each), so a fine grid with a
/// big tool and a long path is refused up front instead of running for minutes.
pub const MAX_WORK: u128 = 60_000_000;
/// Gouge points a report keeps.
pub const MAX_GOUGES: usize = 64;

/// One cutter-location point: the tool tip position and whether the move into it cuts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClPoint {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub cut: bool,
}

impl ClPoint {
    /// A point, or `None` when any coordinate is not finite.
    pub fn new(x: f64, y: f64, z: f64, cut: bool) -> Option<Self> {
        (x.is_finite() && y.is_finite() && z.is_finite()).then_some(Self { x, y, z, cut })
    }
}

/// A heightfield over a rectangular XY grid. Cell (i, j) covers `x0 + i·cell` by
/// `y0 + j·cell` and holds a material height (stock) or finished height (part).
#[derive(Clone, Debug)]
pub struct ZMap {
    x0: f64,
    y0: f64,
    cell: f64,
    cols: usize,
    rows: usize,
    data: Vec<f64>,
}

impl ZMap {
    /// A flat-topped map over the footprint `min .. max` at height `top`.
    pub fn flat(min: (f64, f64), max: (f64, f64), top: f64, cell: f64) -> Result<Self> {
        let cell = match cell {
            c if c.is_finite() => c.clamp(MIN_CELL, 50.0),
            _ => return Err(invalid("invalid Z-map cell size")),
        };
        if !min.0.is_finite() || !min.1.is_finite() || !max.0.is_finite() || !max.1.is_finite() || !top.is_finite() || max.0 <= min.0 || max.1 <= min.1 {
            return Err(invalid("invalid Z-map bounds"));
        }
        let cols = (((max.0 - min.0) / cell).ceil() as usize).max(1);
        let rows = (((max.1 - min.1) / cell).ceil() as usize).max(1);
        if cols > MAX_GRID || rows > MAX_GRID {
            return Err(invalid(format!("the Z-map grid would be {cols}x{rows} cells; at most {MAX_GRID} per edge — use a coarser cell")));
        }
        Ok(Self { x0: min.0, y0: min.1, cell, cols, rows, data: vec![top; cols * rows] })
    }

    pub fn cell(&self) -> f64 {
        self.cell
    }
    pub fn shape(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }
    pub fn same_grid(&self, other: &Self) -> bool {
        self.cols == other.cols && self.rows == other.rows && (self.x0 - other.x0).abs() < 1e-9 && (self.y0 - other.y0).abs() < 1e-9 && (self.cell - other.cell).abs() < 1e-12
    }
    pub fn get(&self, i: usize, j: usize) -> f64 {
        self.data[j * self.cols + i]
    }
    pub fn set(&mut self, i: usize, j: usize, value: f64) {
        self.data[j * self.cols + i] = value;
    }
    /// The cell centre of (i, j).
    pub fn centre(&self, i: usize, j: usize) -> (f64, f64) {
        (self.x0 + (i as f64 + 0.5) * self.cell, self.y0 + (j as f64 + 0.5) * self.cell)
    }
    /// The lowest and highest cell heights.
    pub fn extent(&self) -> (f64, f64) {
        let lo = self.data.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = self.data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        (lo, hi)
    }
}

/// Simulation knobs; `Default` suits a first pass over a shop toolpath.
#[derive(Clone, Copy, Debug)]
pub struct SimOptions {
    /// Cutting below the part surface by up to this much is still on purpose.
    pub gouge_tol: f64,
    /// Sample spacing along a move; `0.0` means half a cell.
    pub step: f64,
}

impl Default for SimOptions {
    fn default() -> Self {
        Self { gouge_tol: 0.05, step: 0.0 }
    }
}

/// What a replay found. `gouges` keeps the first [`MAX_GOUGES`] points as `[x, y, depth]`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimReport {
    pub removed_mm3: f64,
    pub gouge_count: usize,
    pub gouge_worst: f64,
    pub gouges: Vec<[f64; 3]>,
    pub remaining_cells: usize,
    pub remaining_worst: f64,
    pub collisions: usize,
    pub samples: usize,
}

/// Replay `path` (tip positions) over `stock`, comparing against `part`. Both maps must
/// share a grid. The first point is a position only; each later point is a move.
pub fn simulate(profile: &CutterProfile, path: &[ClPoint], stock: &mut ZMap, part: &ZMap, opts: &SimOptions) -> Result<SimReport> {
    if path.len() > MAX_PATH {
        return Err(invalid(format!("the toolpath carries more than {MAX_PATH} points")));
    }
    if !stock.same_grid(part) {
        return Err(invalid("stock and part must share the same grid"));
    }
    let tol = if opts.gouge_tol.is_finite() { opts.gouge_tol.max(0.0) } else { 0.05 };
    let step = if opts.step.is_finite() && opts.step > 0.0 { opts.step.max(MIN_CELL * 0.5) } else { (stock.cell() * 0.5).max(MIN_CELL * 0.5) };
    let max_r = profile.max_radius;
    let cell_area = stock.cell() * stock.cell();
    // Refuse work that cannot finish inside the budgets before spending any of them.
    let span = 2.0 * ((max_r / stock.cell()).ceil() + 1.0);
    let touch = (span * span) as u128;
    let samples: u128 = path
        .windows(2)
        .map(|p| {
            let d = ((p[1].x - p[0].x).powi(2) + (p[1].y - p[0].y).powi(2) + (p[1].z - p[0].z).powi(2)).sqrt();
            (d / step).ceil() as u128
        })
        .sum::<u128>()
        .max(1);
    if samples.saturating_mul(touch) > MAX_WORK {
        return Err(invalid("the toolpath is too fine for the simulation budget; use a coarser cell or a shorter path"));
    }
    let mut report = SimReport::default();
    let mut budget = MAX_SAMPLES;
    for pair in path.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let dist = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2) + (b.z - a.z).powi(2)).sqrt();
        let moves = (dist / step).ceil() as usize;
        for s in 1..=moves.max(1) {
            if budget == 0 {
                return Err(invalid("the toolpath needs more samples than the budget allows; use a coarser cell or a shorter path"));
            }
            budget -= 1;
            report.samples += 1;
            let t = s as f64 / moves.max(1) as f64;
            let (x, y, z) = (a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t, a.z + (b.z - a.z) * t);
            if b.cut {
                cut_at(profile, stock, part, x, y, z, max_r, cell_area, tol, &mut report);
            } else {
                rapid_at(profile, stock, x, y, z, max_r, &mut report);
            }
        }
    }
    for j in 0..stock.rows {
        for i in 0..stock.cols {
            let left = stock.get(i, j) - part.get(i, j);
            if left > tol {
                report.remaining_cells += 1;
                report.remaining_worst = report.remaining_worst.max(left);
            }
        }
    }
    Ok(report)
}

/// Lower the stock inside the cutter's radius to the cutting surface at `z`.
#[allow(clippy::too_many_arguments)]
fn cut_at(profile: &CutterProfile, stock: &mut ZMap, part: &ZMap, x: f64, y: f64, z: f64, max_r: f64, cell_area: f64, tol: f64, report: &mut SimReport) {
    let (i0, i1, j0, j1) = footprint(stock, x, y, max_r);
    for j in j0..=j1 {
        for i in i0..=i1 {
            let (cx, cy) = stock.centre(i, j);
            let r = ((cx - x).powi(2) + (cy - y).powi(2)).sqrt();
            let Some(h) = profile.cut_height_at(r) else { continue };
            let target = z + h;
            let current = stock.get(i, j);
            if target >= current {
                continue;
            }
            report.removed_mm3 += (current - target) * cell_area;
            stock.set(i, j, target);
            let floor = part.get(i, j);
            if target < floor - tol {
                report.gouge_count += 1;
                let depth = floor - target;
                report.gouge_worst = report.gouge_worst.max(depth);
                if report.gouges.len() < MAX_GOUGES {
                    report.gouges.push([cx, cy, depth]);
                }
            }
        }
    }
}

/// A rapid move whose envelope dips below the stock is a crash; counted once per move.
fn rapid_at(profile: &CutterProfile, stock: &ZMap, x: f64, y: f64, z: f64, max_r: f64, report: &mut SimReport) {
    let (i0, i1, j0, j1) = footprint(stock, x, y, max_r);
    for j in j0..=j1 {
        for i in i0..=i1 {
            let (cx, cy) = stock.centre(i, j);
            let r = ((cx - x).powi(2) + (cy - y).powi(2)).sqrt();
            let Some(h) = profile.envelope_height_at(r) else { continue };
            if z + h < stock.get(i, j) {
                report.collisions += 1;
                return;
            }
        }
    }
}

/// The inclusive cell window a tool of radius `max_r` centred at (x, y) can touch.
fn footprint(stock: &ZMap, x: f64, y: f64, max_r: f64) -> (usize, usize, usize, usize) {
    let span = (max_r / stock.cell()).ceil() as i64 + 1;
    let ci = ((x - stock.x0) / stock.cell()).floor() as i64;
    let cj = ((y - stock.y0) / stock.cell()).floor() as i64;
    let clamp_x = |v: i64| v.clamp(0, stock.cols as i64 - 1) as usize;
    let clamp_y = |v: i64| v.clamp(0, stock.rows as i64 - 1) as usize;
    (clamp_x(ci - span), clamp_x(ci + span), clamp_y(cj - span), clamp_y(cj + span))
}

#[cfg(test)]
#[path = "sim_tests.rs"]
mod tests;
