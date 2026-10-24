//! The revolved cutter profile: the lightweight geometric representation CAM math uses.
//!
//! A milling tool's cutting and shank geometry is a surface of revolution about the tool axis.
//! The profile is the ordered chain of line and arc segments in the (radius, height) half-plane,
//! from the tip at z = 0 upward toward the holder. A line revolves to a cone or cylinder, an arc
//! to a torus or sphere. Each segment carries whether it is a cutting surface or a non-cutting
//! one (shank, neck, relief), so later collision and reach math can tell them apart.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::{
    Result,
    family::{self, Family},
    invalid,
};

/// One segment of the profile, from (r0, z0) to (r1, z1) with z increasing upward from the tip.
/// An arc is a circular arc with centre (cr, cz) and `radius`, traversed from its first point to
/// its second.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Segment {
    Line { r0: f64, z0: f64, r1: f64, z1: f64, cutting: bool },
    Arc { cr: f64, cz: f64, radius: f64, r0: f64, z0: f64, r1: f64, z1: f64, cutting: bool },
}

impl Segment {
    pub fn line(r0: f64, z0: f64, r1: f64, z1: f64, cutting: bool) -> Self {
        Self::Line { r0, z0, r1, z1, cutting }
    }
    pub fn arc(cr: f64, cz: f64, radius: f64, r0: f64, z0: f64, r1: f64, z1: f64, cutting: bool) -> Self {
        Self::Arc { cr, cz, radius, r0, z0, r1, z1, cutting }
    }
    pub fn cutting(&self) -> bool {
        match self {
            Self::Line { cutting, .. } | Self::Arc { cutting, .. } => *cutting,
        }
    }
    fn z_range(&self) -> (f64, f64) {
        let (z0, z1) = match self {
            Self::Line { z0, z1, .. } | Self::Arc { z0, z1, .. } => (*z0, *z1),
        };
        (z0.min(z1), z0.max(z1))
    }
    fn radii(&self) -> (f64, f64) {
        let (r0, r1) = match self {
            Self::Line { r0, r1, .. } | Self::Arc { r0, r1, .. } => (*r0, *r1),
        };
        (r0.min(r1), r0.max(r1))
    }
    fn max_radius(&self) -> f64 {
        self.radii().1
    }
}

/// A tool's revolved profile plus the lengths every CAM consumer derives from it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CutterProfile {
    pub segments: Vec<Segment>,
    /// Highest z reached by a cutting segment.
    pub cutting_length: f64,
    /// Tip-to-top height, as given by `overall_length`.
    pub overall_length: f64,
    /// Largest radius anywhere on the profile.
    pub max_radius: f64,
}

impl CutterProfile {
    /// The envelope radius at height `z` above the tip (the widest profile point at that height);
    /// `None` outside the tool's z range.
    pub fn radius_at(&self, z: f64) -> Option<f64> {
        if !z.is_finite() || z < -1e-9 || z > self.overall_length + 1e-9 {
            return None;
        }
        let mut best: Option<f64> = None;
        for seg in &self.segments {
            let (zmin, zmax) = seg.z_range();
            if z < zmin - 1e-9 || z > zmax + 1e-9 {
                continue;
            }
            let r = match *seg {
                Segment::Line { r0, z0, r1, z1, .. } => {
                    if (z1 - z0).abs() < 1e-12 {
                        r0.max(r1)
                    } else {
                        let t = ((z - z0) / (z1 - z0)).clamp(0.0, 1.0);
                        r0 + t * (r1 - r0)
                    }
                }
                Segment::Arc { cr, cz, radius, .. } => {
                    let dz = z - cz;
                    if dz.abs() > radius * (1.0 + 1e-9) {
                        continue;
                    }
                    let s = (radius * radius - dz * dz).max(0.0).sqrt();
                    let (rmin, rmax) = seg.radii();
                    let (lo, hi) = (cr - s, cr + s);
                    // Pick the branch that lies on this arc's radial span.
                    let lo_ok = lo >= rmin - 1e-9 && lo <= rmax + 1e-9;
                    let hi_ok = hi >= rmin - 1e-9 && hi <= rmax + 1e-9;
                    match (lo_ok, hi_ok) {
                        (true, true) => lo.max(hi),
                        (true, false) => lo,
                        (false, true) => hi,
                        (false, false) => continue,
                    }
                }
            };
            if best.is_none_or(|b| r > b) {
                best = Some(r);
            }
        }
        best
    }

    /// The lowest cutting-surface height at radial distance `r` from the axis — the inverse of
    /// [`radius_at`](Self::radius_at) restricted to cutting segments. With the tool tip at
    /// height `z`, material at that radius is cut down to `z + height`; `None` when no cutting
    /// segment reaches the radius.
    pub fn cut_height_at(&self, r: f64) -> Option<f64> {
        self.lowest_at(r, true)
    }

    /// The lowest point of the whole envelope (cutting or not) at radius `r`; rapid-move
    /// shank-collision tests use this.
    pub fn envelope_height_at(&self, r: f64) -> Option<f64> {
        self.lowest_at(r, false)
    }

    fn lowest_at(&self, r: f64, cutting_only: bool) -> Option<f64> {
        if !r.is_finite() || r < 0.0 {
            return None;
        }
        let mut best: Option<f64> = None;
        let mut consider = |z: f64| {
            if best.is_none_or(|b| z < b) {
                best = Some(z);
            }
        };
        for seg in &self.segments {
            if cutting_only && !seg.cutting() {
                continue;
            }
            match *seg {
                Segment::Line { r0, z0, r1, z1, .. } => {
                    let (rmin, rmax) = (r0.min(r1), r0.max(r1));
                    if r < rmin - 1e-9 || r > rmax + 1e-9 {
                        continue;
                    }
                    let z = if (r1 - r0).abs() < 1e-12 {
                        z0.min(z1)
                    } else {
                        let t = ((r - r0) / (r1 - r0)).clamp(0.0, 1.0);
                        z0 + t * (z1 - z0)
                    };
                    consider(z);
                }
                Segment::Arc { cr, cz, radius, .. } => {
                    let dr = r - cr;
                    if dr.abs() > radius * (1.0 + 1e-9) {
                        continue;
                    }
                    let dz = (radius * radius - dr * dr).max(0.0).sqrt();
                    let (zmin, zmax) = seg.z_range();
                    for z in [cz - dz, cz + dz] {
                        if z >= zmin - 1e-9 && z <= zmax + 1e-9 {
                            consider(z);
                        }
                    }
                }
            }
        }
        best
    }
}

/// Builds the revolved profile of `family` from its (validated) parameters.
pub fn of(family: Family, params: &BTreeMap<String, f64>) -> Result<CutterProfile> {
    family::validate_params(family, params)?;
    let get = |name: &str| -> Result<f64> { family::value(family, params, name).ok_or_else(|| invalid(format!("missing `{name}`"))) };
    let overall = get("overall_length")?;
    let shank_r = get("shank_diameter")? / 2.0;
    let mut segs: Vec<Segment> = Vec::new();
    let cutting_length: f64;
    match family {
        Family::EndMillFlat => {
            let r = get("cutting_diameter")? / 2.0;
            let fl = get("flute_length")?;
            let top = r + fl * family::half_tan(get("taper_angle")?);
            segs.push(Segment::line(0.0, 0.0, r, 0.0, true));
            segs.push(Segment::line(r, 0.0, top, fl, true));
            cutting_length = fl;
            push_tail(&mut segs, top, fl, shank_r, overall);
        }
        Family::EndMillBall => {
            let r = get("cutting_diameter")? / 2.0;
            let fl = get("flute_length")?;
            let top = r + (fl - r) * family::half_tan(get("taper_angle")?);
            segs.push(Segment::arc(0.0, r, r, 0.0, 0.0, r, r, true));
            segs.push(Segment::line(r, r, top, fl, true));
            cutting_length = fl;
            push_tail(&mut segs, top, fl, shank_r, overall);
        }
        Family::EndMillBull => {
            let r = get("cutting_diameter")? / 2.0;
            let cr = get("corner_radius")?;
            let fl = get("flute_length")?;
            let top = r + (fl - cr) * family::half_tan(get("taper_angle")?);
            segs.push(Segment::line(0.0, 0.0, r - cr, 0.0, true));
            segs.push(Segment::arc(r - cr, cr, cr, r - cr, 0.0, r, cr, true));
            segs.push(Segment::line(r, cr, top, fl, true));
            cutting_length = fl;
            push_tail(&mut segs, top, fl, shank_r, overall);
        }
        Family::Drill => {
            let r = get("cutting_diameter")? / 2.0;
            let fl = get("flute_length")?;
            let zp = r / family::half_tan(get("point_angle")?);
            segs.push(Segment::line(0.0, 0.0, r, zp, true));
            segs.push(Segment::line(r, zp, r, fl, true));
            cutting_length = fl;
            push_tail(&mut segs, r, fl, shank_r, overall);
        }
        Family::CenterDrill => {
            let rp = get("pilot_diameter")? / 2.0;
            let r = get("major_diameter")? / 2.0;
            let pl = get("pilot_length")?;
            let cone = (r - rp) / family::half_tan(get("point_angle")?);
            segs.push(Segment::line(0.0, 0.0, rp, 0.0, true));
            segs.push(Segment::line(rp, 0.0, rp, pl, true));
            segs.push(Segment::line(rp, pl, r, pl + cone, true));
            cutting_length = pl + cone;
            push_tail(&mut segs, r, pl + cone, shank_r, overall);
        }
        Family::ChamferMill => {
            let r = get("cutting_diameter")? / 2.0;
            let rp = get("pilot_diameter")? / 2.0;
            let fl = get("flute_length")?;
            let cone = (r - rp) / family::half_tan(get("included_angle")?);
            if rp > 1e-9 {
                segs.push(Segment::line(0.0, 0.0, rp, 0.0, true));
            }
            segs.push(Segment::line(rp, 0.0, r, cone, true));
            segs.push(Segment::line(r, cone, r, fl, false));
            cutting_length = cone;
            push_tail(&mut segs, r, fl, shank_r, overall);
        }
        Family::Countersink => {
            let r = get("cutting_diameter")? / 2.0;
            let fl = get("flute_length")?;
            let cone = r / family::half_tan(get("included_angle")?);
            segs.push(Segment::line(0.0, 0.0, r, cone, true));
            segs.push(Segment::line(r, cone, r, fl, false));
            cutting_length = cone;
            push_tail(&mut segs, r, fl, shank_r, overall);
        }
        Family::Tap => {
            let r = get("major_diameter")? / 2.0;
            let tl = get("thread_length")?;
            segs.push(Segment::line(0.0, 0.0, r, 0.0, true));
            segs.push(Segment::line(r, 0.0, r, tl, true));
            cutting_length = tl;
            push_tail(&mut segs, r, tl, shank_r, overall);
        }
        Family::Keyway => {
            let r = get("cutting_diameter")? / 2.0;
            let w = get("width")?;
            segs.push(Segment::line(0.0, 0.0, r, 0.0, true));
            segs.push(Segment::line(r, 0.0, r, w, true));
            cutting_length = w;
            push_tail(&mut segs, r, w, shank_r, overall);
        }
        Family::TSlot => {
            let rb = get("crossbar_diameter")? / 2.0;
            let w = get("crossbar_width")?;
            segs.push(Segment::line(0.0, 0.0, rb, 0.0, true));
            segs.push(Segment::line(rb, 0.0, rb, w, true));
            cutting_length = w;
            push_tail(&mut segs, rb, w, shank_r, overall);
        }
        Family::CornerRadius => {
            let r = get("cutting_diameter")? / 2.0;
            let cr = get("corner_radius")?;
            let fl = get("flute_length")?;
            segs.push(Segment::line(0.0, 0.0, r - cr, 0.0, true));
            segs.push(Segment::arc(r, 0.0, cr, r - cr, 0.0, r, cr, true));
            segs.push(Segment::line(r, cr, r, fl, true));
            cutting_length = fl;
            push_tail(&mut segs, r, fl, shank_r, overall);
        }
        Family::Counterbore => {
            let rp = get("pilot_diameter")? / 2.0;
            let r = get("counterbore_diameter")? / 2.0;
            let pl = get("pilot_length")?;
            let cb = get("counterbore_depth")?;
            segs.push(Segment::line(0.0, 0.0, rp, 0.0, true));
            segs.push(Segment::line(rp, 0.0, rp, pl, true));
            segs.push(Segment::line(rp, pl, r, pl, true));
            segs.push(Segment::line(r, pl, r, pl + cb, true));
            cutting_length = pl + cb;
            push_tail(&mut segs, r, pl + cb, shank_r, overall);
        }
    }
    let max_radius = segs.iter().map(Segment::max_radius).fold(0.0_f64, f64::max);
    Ok(CutterProfile { segments: segs, cutting_length, overall_length: overall, max_radius })
}

/// Appends the non-cutting shoulder and shank from the last cutting point (r_end, z_end) up to
/// the top of the tool.
fn push_tail(segs: &mut Vec<Segment>, r_end: f64, z_end: f64, shank_r: f64, overall: f64) {
    if z_end >= overall {
        return;
    }
    if (shank_r - r_end).abs() > 1e-9 {
        segs.push(Segment::line(r_end, z_end, shank_r, z_end, false));
    }
    if overall > z_end + 1e-9 {
        segs.push(Segment::line(shank_r, z_end, shank_r, overall, false));
    }
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
