//! The holder catalogue: spindle interfaces and holder bodies, with a simplified revolved
//! envelope for each combination.
//!
//! The interface dimensions are representative shop values for preview, stickout arithmetic and
//! first-pass collision checks — they are not certified interface drawings, and the bore is not
//! modelled (the envelope is solid). A holder's local profile stands on its nose: z = 0 is the
//! girth line (the spindle reference plane), the nose hangs below it and the taper rises above.

use crate::profile::{CutterProfile, Segment};
use crate::{Result, invalid};

/// A spindle taper the holder mounts into.
#[derive(Clone, Copy, Debug)]
pub struct Spindle {
    pub id: &'static str,
    pub label: &'static str,
    /// Taper diameter at the girth line.
    pub girth_d: f64,
    /// Axial length of the taper above the girth line.
    pub taper_len: f64,
    pub flange_d: f64,
    pub flange_t: f64,
    /// Radius lost per unit height climbing the taper (7:24 → 7/48, 1:10 → 0.05).
    pub slope: f64,
}

/// A holder body: the part below the girth line, plus the bore range it accepts.
#[derive(Clone, Copy, Debug)]
pub struct Body {
    pub id: &'static str,
    pub label: &'static str,
    /// Nose radius: fixed, or derived from the bore for bore-sized bodies.
    nose_r: f64,
    nut_r: f64,
    /// Height of the nose section before the step out to the nut collar.
    nut_h: f64,
    /// Nose bottom below the girth line.
    pub nose_len: f64,
    pub bore_min: f64,
    pub bore_max: f64,
    /// Some for bodies sized by the shank they grip (set screw, shrink fit, hydraulic).
    bore_sized: bool,
}

const SLOPE_724: f64 = 7.0 / 48.0;
const SLOPE_110: f64 = 0.05;

const SPINDLES: &[Spindle] = &[
    Spindle { id: "hsk_a50", label: "HSK-A50", girth_d: 50.0, taper_len: 38.0, flange_d: 68.0, flange_t: 6.0, slope: SLOPE_110 },
    Spindle { id: "hsk_a63", label: "HSK-A63", girth_d: 63.0, taper_len: 44.0, flange_d: 80.0, flange_t: 7.0, slope: SLOPE_110 },
    Spindle { id: "bt30", label: "BT30", girth_d: 30.0, taper_len: 40.0, flange_d: 46.0, flange_t: 6.0, slope: SLOPE_724 },
    Spindle { id: "bt40", label: "BT40", girth_d: 44.5, taper_len: 55.0, flange_d: 62.0, flange_t: 7.0, slope: SLOPE_724 },
    Spindle { id: "cat40", label: "CAT40", girth_d: 44.5, taper_len: 54.0, flange_d: 60.0, flange_t: 7.0, slope: SLOPE_724 },
    Spindle { id: "sk40", label: "SK40", girth_d: 44.5, taper_len: 54.0, flange_d: 61.0, flange_t: 7.0, slope: SLOPE_724 },
];

const BODIES: &[Body] = &[
    Body {
        id: "er25",
        label: "ER25 collet chuck",
        nose_r: 13.0,
        nut_r: 20.0,
        nut_h: 10.0,
        nose_len: 25.0,
        bore_min: 1.0,
        bore_max: 16.0,
        bore_sized: false,
    },
    Body {
        id: "er32",
        label: "ER32 collet chuck",
        nose_r: 16.5,
        nut_r: 25.0,
        nut_h: 12.0,
        nose_len: 30.0,
        bore_min: 3.0,
        bore_max: 20.0,
        bore_sized: false,
    },
    Body {
        id: "er40",
        label: "ER40 collet chuck",
        nose_r: 20.5,
        nut_r: 30.0,
        nut_h: 14.0,
        nose_len: 35.0,
        bore_min: 6.0,
        bore_max: 26.0,
        bore_sized: false,
    },
    Body {
        id: "weldon",
        label: "Set-screw holder",
        nose_r: 0.0,
        nut_r: 0.0,
        nut_h: 15.0,
        nose_len: 35.0,
        bore_min: 4.0,
        bore_max: 20.0,
        bore_sized: true,
    },
    Body {
        id: "shrink",
        label: "Shrink-fit holder",
        nose_r: 0.0,
        nut_r: 0.0,
        nut_h: 20.0,
        nose_len: 45.0,
        bore_min: 4.0,
        bore_max: 20.0,
        bore_sized: true,
    },
    Body {
        id: "hydraulic",
        label: "Hydraulic chuck",
        nose_r: 0.0,
        nut_r: 0.0,
        nut_h: 18.0,
        nose_len: 40.0,
        bore_min: 3.0,
        bore_max: 16.0,
        bore_sized: true,
    },
];

/// Which bodies each spindle offers, in catalogue order.
const PAIRS: &[(&str, &[&str])] = &[
    ("er25", &["hsk_a50", "bt30"]),
    ("er32", &["bt40", "cat40", "sk40", "hsk_a63"]),
    ("er40", &["bt40", "cat40"]),
    ("weldon", &["hsk_a63", "bt40", "cat40", "sk40"]),
    ("shrink", &["hsk_a63", "bt40", "cat40"]),
    ("hydraulic", &["hsk_a63", "bt40"]),
];

/// One catalogue entry: a body on a spindle, with the bore range it accepts.
#[derive(Clone, Debug, PartialEq)]
pub struct HolderType {
    pub id: String,
    pub label: String,
    pub body: &'static str,
    pub spindle: &'static str,
    pub bore_min: f64,
    pub bore_max: f64,
    /// Distance from the girth line down to the nose bottom.
    pub nose_len: f64,
    /// Nose bottom to taper top at the girth-relative profile.
    pub overall_length: f64,
}

fn body_of(id: &str) -> Option<&'static Body> {
    BODIES.iter().find(|b| b.id == id)
}
fn spindle_of(id: &str) -> Option<&'static Spindle> {
    SPINDLES.iter().find(|s| s.id == id)
}

/// The whole catalogue, generated from the body × spindle pairs.
pub fn catalog() -> Vec<HolderType> {
    let mut out = Vec::new();
    for (body_id, spindles) in PAIRS {
        let Some(body) = body_of(body_id) else { continue };
        for spindle_id in spindles.iter().copied() {
            let Some(spindle) = spindle_of(spindle_id) else { continue };
            out.push(HolderType {
                id: format!("{}_{}", body.id, spindle.id),
                label: format!("{} — {}", body.label, spindle.label),
                body: body.id,
                spindle: spindle.id,
                bore_min: body.bore_min,
                bore_max: body.bore_max,
                nose_len: body.nose_len,
                overall_length: body.nose_len + spindle.taper_len,
            });
        }
    }
    out
}

/// Look up one catalogue entry by id, e.g. `er32_bt40`.
pub fn find(id: &str) -> Option<HolderType> {
    catalog().into_iter().find(|h| h.id == id)
}

/// The bore radii a body presents for a given shank diameter.
fn body_radii(body: &Body, bore: f64) -> (f64, f64) {
    if body.bore_sized { ((bore + 4.0) / 2.0, (bore + 14.0) / 2.0) } else { (body.nose_r, body.nut_r) }
}

/// The simplified revolved envelope of a holder gripping `bore` (the tool shank diameter).
/// The profile stands on the nose: z = 0 at the girth line, the nose below, the taper above.
pub fn profile_of(type_id: &str, bore: f64) -> Result<CutterProfile> {
    let holder = find(type_id).ok_or_else(|| invalid(format!("unknown holder type `{type_id}`")))?;
    let body = body_of(holder.body).ok_or_else(|| invalid("holder catalogue is inconsistent"))?;
    let spindle = spindle_of(holder.spindle).ok_or_else(|| invalid("holder catalogue is inconsistent"))?;
    if !bore.is_finite() || bore < body.bore_min || bore > body.bore_max {
        return Err(invalid(format!("bore must be between {} and {} mm for `{type_id}`", body.bore_min, body.bore_max)));
    }
    let (nose_r, nut_r) = body_radii(body, bore);
    let flange_r = spindle.flange_d / 2.0;
    let girth_r = spindle.girth_d / 2.0;
    let nut = -body.nose_len;
    let mut segs: Vec<Segment> = Vec::new();
    // Nose bottom, nose wall, optional step out to the nut collar, nut wall up to the girth.
    segs.push(Segment::line(0.0, nut, nose_r, nut, false));
    if body.nut_h > 0.0 && nut_r > nose_r + 1e-9 {
        segs.push(Segment::line(nose_r, nut, nose_r, nut + body.nut_h, false));
        segs.push(Segment::line(nose_r, nut + body.nut_h, nut_r, nut + body.nut_h, false));
        segs.push(Segment::line(nut_r, nut + body.nut_h, nut_r, 0.0, false));
    } else {
        segs.push(Segment::line(nose_r, nut, nut_r, 0.0, false));
    }
    // Flange at the girth line, then the spindle taper rising to its small end.
    segs.push(Segment::line(nut_r, 0.0, flange_r, 0.0, false));
    segs.push(Segment::line(flange_r, 0.0, flange_r, spindle.flange_t, false));
    let taper_start = girth_r - spindle.flange_t * spindle.slope;
    segs.push(Segment::line(flange_r, spindle.flange_t, taper_start, spindle.flange_t, false));
    let taper_top = girth_r - spindle.taper_len * spindle.slope;
    segs.push(Segment::line(taper_start, spindle.flange_t, taper_top, spindle.taper_len, false));
    segs.push(Segment::line(taper_top, spindle.taper_len, 0.0, spindle.taper_len, false));
    let max_radius = [nose_r, nut_r, flange_r, girth_r].into_iter().fold(0.0_f64, f64::max);
    Ok(CutterProfile { segments: segs, cutting_length: 0.0, overall_length: body.nose_len + spindle.taper_len, max_radius })
}

#[cfg(test)]
#[path = "holder_tests.rs"]
mod tests;
