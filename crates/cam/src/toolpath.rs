//! Toolpaths as 3D LINE and ARC segments — what a post sends and a simulator replays.
//!
//! A [Toolpath] is a chain of [Segment]s, each marked `cut` (the tool is feeding) or
//! not (a rapid). Arcs are centre-form (the way G-code states them): `from`, `to`,
//! `center` and the plane `normal`, with `clockwise` read looking along `+normal`
//! toward the origin. A `from` equal to `to` is a full circle. [Toolpath::tessellate]
//! turns the segments into coloured-ready polylines within a chord tolerance, which
//! is what the viewport draws and what bounds are computed from.

use crate::{Result, invalid};
use serde::{Deserialize, Serialize};
use solidcraft_geom::Vec3;

/// Largest number of segments one toolpath may hold.
pub const MAX_SEGMENTS: usize = 100_000;
/// Largest number of points one tessellation may produce.
pub const MAX_TESSALLATED_POINTS: usize = 200_000;
/// The chord tolerance tessellation uses unless the caller asks for another.
pub const DEFAULT_TOLERANCE: f64 = 0.01;
/// Points one arc may produce, however fine the tolerance.
const MAX_ARC_POINTS: usize = 4096;
/// Smallest usable radius, in millimetres.
const MIN_RADIUS: f64 = 1e-6;

fn cut_true() -> bool {
    true
}

fn finite3(v: &[f64; 3]) -> bool {
    v[0].is_finite() && v[1].is_finite() && v[2].is_finite()
}

/// One move of the cutter tip.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Segment {
    Line {
        from: [f64; 3],
        to: [f64; 3],
        /// The move feeds material; `false` is a rapid.
        #[serde(default = "cut_true")]
        cut: bool,
    },
    Arc {
        from: [f64; 3],
        to: [f64; 3],
        center: [f64; 3],
        normal: [f64; 3],
        /// Clockwise read looking along `normal` toward the centre.
        #[serde(default)]
        clockwise: bool,
        #[serde(default = "cut_true")]
        cut: bool,
    },
}

impl Segment {
    pub fn cut(&self) -> bool {
        match self {
            Segment::Line { cut, .. } | Segment::Arc { cut, .. } => *cut,
        }
    }

    pub fn from(&self) -> [f64; 3] {
        match self {
            Segment::Line { from, .. } | Segment::Arc { from, .. } => *from,
        }
    }

    pub fn to(&self) -> [f64; 3] {
        match self {
            Segment::Line { to, .. } | Segment::Arc { to, .. } => *to,
        }
    }

    fn check(&self) -> Result<()> {
        match self {
            Segment::Line { from, to, .. } => {
                if !finite3(from) || !finite3(to) {
                    return Err(invalid("a line segment needs finite coordinates"));
                }
                if Vec3::new(to[0], to[1], to[2]).dist(Vec3::new(from[0], from[1], from[2])) <= MIN_RADIUS {
                    return Err(invalid("a line segment is empty"));
                }
            }
            Segment::Arc { from, to, center, normal, .. } => {
                for (what, v) in [("from", from), ("to", to), ("center", center), ("normal", normal)] {
                    if !finite3(v) {
                        return Err(invalid(format!("an arc segment needs finite {what} coordinates")));
                    }
                }
                let n = Vec3::new(normal[0], normal[1], normal[2])
                    .normalized()
                    .ok_or_else(|| invalid("an arc needs a non-zero normal"))?;
                let u = Vec3::new(from[0], from[1], from[2]) - Vec3::new(center[0], center[1], center[2]);
                let r = u.len();
                if !(r.is_finite() && r > MIN_RADIUS) {
                    return Err(invalid("an arc needs a radius above 1e-6 mm"));
                }
                let tol = (DEFAULT_TOLERANCE * r).max(1e-6);
                let tc = Vec3::new(to[0], to[1], to[2]) - Vec3::new(center[0], center[1], center[2]);
                if (tc.len() - r).abs() > tol {
                    return Err(invalid("an arc's end is not on its radius"));
                }
                if u.dot(n).abs() > tol || tc.dot(n).abs() > tol {
                    return Err(invalid("an arc's ends are off the arc plane"));
                }
            }
        }
        Ok(())
    }

    /// The polyline of this one segment within `tolerance` of the true path.
    fn tessellate(&self, tolerance: f64) -> Result<Vec<Vec3>> {
        match self {
            Segment::Line { from, to, .. } => Ok(vec![v(from), v(to)]),
            Segment::Arc { from, to, center, normal, clockwise, .. } => {
                let n = Vec3::new(normal[0], normal[1], normal[2])
                    .normalized()
                    .ok_or_else(|| invalid("an arc needs a non-zero normal"))?;
                let c = v(center);
                let u = v(from) - c;
                let r = u.len();
                let w = n.cross(u);
                let tc = v(to) - c;
                let mut theta = tc.dot(w).atan2(tc.dot(u));
                theta = if *clockwise {
                    if theta >= 0.0 {
                        theta - std::f64::consts::TAU
                    } else {
                        theta
                    }
                } else if theta <= 0.0 {
                    theta + std::f64::consts::TAU
                } else {
                    theta
                };
                // Chord height r(1 - cos(θ/2)) ≤ tol → θ = 2·acos(1 - tol/r).
                let step = 2.0 * (1.0 - (tolerance / r).min(1.0)).clamp(-1.0, 1.0).acos();
                let step = step.clamp(1e-4, std::f64::consts::PI);
                let steps = ((theta.abs() / step).ceil() as usize).clamp(2, MAX_ARC_POINTS);
                let mut pts = Vec::with_capacity(steps + 1);
                for i in 0..=steps {
                    let a = theta * i as f64 / steps as f64;
                    pts.push(c + u * a.cos() + w * a.sin());
                }
                Ok(pts)
            }
        }
    }
}

fn v(a: &[f64; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

fn a(p: Vec3) -> [f64; 3] {
    [p.x, p.y, p.z]
}

/// One tessellated segment: its points and whether it cuts — what the renderer colours
/// and the simulator walks.
#[derive(Clone, Debug, PartialEq)]
pub struct Polyline {
    pub points: Vec<Vec3>,
    pub cut: bool,
}

/// A toolpath: the ordered segments of one tool's path.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Toolpath {
    pub segments: Vec<Segment>,
}

impl Toolpath {
    /// Straight segments through the points; `cuts[i]` says whether the move *into*
    /// point `i + 1` feeds. Used to adopt a G-code replay, whose points carry the flag.
    pub fn from_points(points: &[Vec3], cuts: &[bool]) -> Result<Self> {
        if points.len() < 2 {
            return Err(invalid("a toolpath needs at least two points"));
        }
        if points.len() - 1 > MAX_SEGMENTS {
            return Err(invalid(format!("a toolpath holds at most {MAX_SEGMENTS} segments")));
        }
        if points.iter().any(|p| !p.is_finite()) {
            return Err(invalid("a toolpath point is not finite"));
        }
        let mut segments = Vec::with_capacity(points.len() - 1);
        for i in 0..points.len() - 1 {
            segments.push(Segment::Line {
                from: a(points[i]),
                to: a(points[i + 1]),
                // cuts[i + 1]: whether the move into point i + 1 feeds.
                cut: cuts.get(i + 1).copied().unwrap_or(true),
            });
        }
        let path = Toolpath { segments };
        path.check()?;
        Ok(path)
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn len(&self) -> usize {
        self.segments.len()
    }

    pub fn check(&self) -> Result<()> {
        if self.segments.len() > MAX_SEGMENTS {
            return Err(invalid(format!("a toolpath holds at most {MAX_SEGMENTS} segments")));
        }
        for segment in &self.segments {
            segment.check()?;
        }
        Ok(())
    }

    /// Every segment as its own polyline, so each can be drawn in its own colour.
    pub fn tessellate(&self, tolerance: f64) -> Result<Vec<Polyline>> {
        let tol = if tolerance.is_finite() && tolerance > 0.0 { tolerance } else { DEFAULT_TOLERANCE };
        let mut out = Vec::with_capacity(self.segments.len());
        let mut budget = MAX_TESSALLATED_POINTS;
        for segment in &self.segments {
            let points = segment.tessellate(tol)?;
            budget = budget.checked_sub(points.len()).ok_or_else(|| invalid("the toolpath tessellates to more points than allowed"))?;
            out.push(Polyline { points, cut: segment.cut() });
        }
        Ok(out)
    }

    /// The axis-aligned box the path sweeps, from its tessellation; `None` when empty.
    pub fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        let polylines = self.tessellate(DEFAULT_TOLERANCE).ok()?;
        let mut min = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut max = Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut any = false;
        for line in &polylines {
            for p in &line.points {
                min = min.min(*p);
                max = max.max(*p);
                any = true;
            }
        }
        any.then(|| (a(min), a(max)))
    }
}

#[cfg(test)]
#[path = "toolpath_tests.rs"]
mod tests;
