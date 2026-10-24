//! Milling tool families: the fixed parameter schema of each family, and its validation.
//!
//! Every length parameter is millimetres and every angle parameter degrees, on input and in the
//! stored entry (callers convert other units before validation). Ranges are generous shop bounds,
//! not physical limits: they exist to refuse hostile or nonsense input early, and cross-parameter
//! rules encode the geometry each family's profile builder can actually build.

use std::collections::BTreeMap;

use crate::{Result, invalid};

/// What a parameter is measured in; drives unit conversion on import and command input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamUnit {
    Length,
    Angle,
}

/// One parameter of a family schema. `None` default makes the parameter required.
pub struct ParamSpec {
    pub name: &'static str,
    pub min: f64,
    pub max: f64,
    pub default: Option<f64>,
    pub unit: ParamUnit,
    pub doc: &'static str,
}

const fn length_opt(name: &'static str, min: f64, max: f64, default: f64, doc: &'static str) -> ParamSpec {
    ParamSpec { name, min, max, default: Some(default), unit: ParamUnit::Length, doc }
}
const fn angle(name: &'static str, min: f64, max: f64, default: f64, doc: &'static str) -> ParamSpec {
    ParamSpec { name, min, max, default: Some(default), unit: ParamUnit::Angle, doc }
}

/// The milling families built in. Turning items (inserts, turning holders) are deliberately
/// absent until their geometry is designed: importing one reports "not supported yet".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    EndMillFlat,
    EndMillBall,
    EndMillBull,
    Drill,
    CenterDrill,
    ChamferMill,
    Countersink,
    Tap,
    Keyway,
    TSlot,
    CornerRadius,
    Counterbore,
}

pub const FAMILIES: [Family; 12] = [
    Family::EndMillFlat,
    Family::EndMillBall,
    Family::EndMillBull,
    Family::Drill,
    Family::CenterDrill,
    Family::ChamferMill,
    Family::Countersink,
    Family::Tap,
    Family::Keyway,
    Family::TSlot,
    Family::CornerRadius,
    Family::Counterbore,
];

impl Family {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EndMillFlat => "end_mill_flat",
            Self::EndMillBall => "end_mill_ball",
            Self::EndMillBull => "end_mill_bull",
            Self::Drill => "drill",
            Self::CenterDrill => "center_drill",
            Self::ChamferMill => "chamfer_mill",
            Self::Countersink => "countersink",
            Self::Tap => "tap",
            Self::Keyway => "keyway",
            Self::TSlot => "t_slot",
            Self::CornerRadius => "corner_radius",
            Self::Counterbore => "counterbore",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        FAMILIES.iter().copied().find(|f| f.as_str() == value)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::EndMillFlat => "Flat end mill",
            Self::EndMillBall => "Ball nose end mill",
            Self::EndMillBull => "Bull nose end mill",
            Self::Drill => "Drill",
            Self::CenterDrill => "Centre drill",
            Self::ChamferMill => "Chamfer mill",
            Self::Countersink => "Countersink",
            Self::Tap => "Tap",
            Self::Keyway => "Keyway cutter",
            Self::TSlot => "T-slot cutter",
            Self::CornerRadius => "Corner rounding end mill",
            Self::Counterbore => "Counterbore",
        }
    }
    /// The family's parameter schema, in display order.
    pub fn schema(self) -> &'static [ParamSpec] {
        use Family::*;
        match self {
            EndMillFlat | EndMillBall => FLAT_BALL,
            EndMillBull => BULL,
            Drill => DRILL,
            CenterDrill => CENTER_DRILL,
            ChamferMill => CHAMFER_MILL,
            Countersink => COUNTERSINK,
            Tap => TAP,
            Keyway => KEYWAY,
            TSlot => T_SLOT,
            CornerRadius => CORNER_RADIUS,
            Counterbore => COUNTERBORE,
        }
    }
}

const FLAT_BALL: &[ParamSpec] = &[
    length_opt("cutting_diameter", 0.1, 63.0, 10.0, "cutting diameter"),
    length_opt("flute_length", 0.1, 300.0, 20.0, "axial length of the cutting flutes"),
    length_opt("overall_length", 1.0, 600.0, 70.0, "tip to top of the tool"),
    length_opt("shank_diameter", 0.1, 100.0, 10.0, "clamp (arbor) diameter"),
    angle("taper_angle", 0.0, 20.0, 0.0, "included taper angle of the cutting body, widening toward the shank (0 = straight)"),
];
const BULL: &[ParamSpec] = &[
    length_opt("cutting_diameter", 0.1, 63.0, 10.0, "cutting diameter"),
    length_opt("corner_radius", 0.05, 30.0, 1.0, "corner fillet radius"),
    length_opt("flute_length", 0.1, 300.0, 20.0, "axial length of the cutting flutes"),
    length_opt("overall_length", 1.0, 600.0, 70.0, "tip to top of the tool"),
    length_opt("shank_diameter", 0.1, 100.0, 10.0, "clamp (arbor) diameter"),
    angle("taper_angle", 0.0, 20.0, 0.0, "included taper angle of the cutting body, widening toward the shank (0 = straight)"),
];
const DRILL: &[ParamSpec] = &[
    length_opt("cutting_diameter", 0.05, 63.0, 6.0, "drill diameter"),
    length_opt("flute_length", 0.1, 300.0, 30.0, "flute length below the neck"),
    length_opt("overall_length", 1.0, 600.0, 60.0, "tip to top of the shank"),
    length_opt("shank_diameter", 0.05, 100.0, 6.0, "shank diameter"),
    angle("point_angle", 60.0, 179.9, 118.0, "included point angle"),
];
const CENTER_DRILL: &[ParamSpec] = &[
    length_opt("pilot_diameter", 0.1, 20.0, 2.0, "pilot (centre) hole diameter"),
    length_opt("pilot_length", 0.1, 30.0, 5.0, "pilot cylinder length"),
    length_opt("major_diameter", 0.2, 63.0, 6.0, "countersink (major) diameter"),
    angle("point_angle", 60.0, 179.9, 118.0, "included angle of the countersink cone"),
    length_opt("overall_length", 1.0, 300.0, 50.0, "tip to top of the shank"),
    length_opt("shank_diameter", 0.1, 63.0, 6.0, "shank diameter"),
];
const CHAMFER_MILL: &[ParamSpec] = &[
    length_opt("cutting_diameter", 0.5, 63.0, 10.0, "maximum cutting diameter"),
    angle("included_angle", 30.0, 179.9, 90.0, "included chamfer angle"),
    length_opt("pilot_diameter", 0.0, 31.0, 0.0, "flat tip diameter below the cone"),
    length_opt("flute_length", 0.1, 100.0, 8.0, "length of the cone plus its relief"),
    length_opt("overall_length", 1.0, 600.0, 50.0, "tip to top of the tool"),
    length_opt("shank_diameter", 0.1, 100.0, 10.0, "clamp (arbor) diameter"),
];
const COUNTERSINK: &[ParamSpec] = &[
    length_opt("cutting_diameter", 0.5, 63.0, 10.0, "maximum cutting diameter"),
    angle("included_angle", 30.0, 179.9, 90.0, "included cone angle"),
    length_opt("flute_length", 0.1, 100.0, 8.0, "length of the cone plus its relief"),
    length_opt("overall_length", 1.0, 600.0, 50.0, "tip to top of the tool"),
    length_opt("shank_diameter", 0.1, 100.0, 10.0, "clamp (arbor) diameter"),
];
const TAP: &[ParamSpec] = &[
    length_opt("major_diameter", 0.5, 63.0, 6.0, "thread major diameter"),
    length_opt("pitch", 0.2, 6.0, 1.0, "thread pitch"),
    length_opt("thread_length", 0.5, 200.0, 20.0, "threaded length"),
    length_opt("overall_length", 1.0, 600.0, 60.0, "tip to top of the shank"),
    length_opt("shank_diameter", 0.1, 100.0, 4.0, "shank (tang) diameter"),
];
const KEYWAY: &[ParamSpec] = &[
    length_opt("cutting_diameter", 1.0, 63.0, 6.0, "cutter diameter"),
    length_opt("width", 0.5, 30.0, 3.0, "cut width (the key width)"),
    length_opt("overall_length", 1.0, 600.0, 60.0, "tip to top of the shank"),
    length_opt("shank_diameter", 0.1, 100.0, 4.0, "shank diameter"),
];
const T_SLOT: &[ParamSpec] = &[
    length_opt("crossbar_diameter", 2.0, 63.0, 10.0, "diameter of the T crossbar"),
    length_opt("crossbar_width", 1.0, 30.0, 4.0, "thickness of the crossbar"),
    length_opt("shank_diameter", 0.5, 63.0, 6.0, "neck (shank) diameter"),
    length_opt("overall_length", 1.0, 600.0, 60.0, "tip to top of the shank"),
];
const CORNER_RADIUS: &[ParamSpec] = &[
    length_opt("cutting_diameter", 0.5, 63.0, 6.0, "body diameter"),
    length_opt("corner_radius", 0.05, 30.0, 1.0, "concave corner radius"),
    length_opt("flute_length", 0.1, 300.0, 15.0, "axial length of the cutting flutes"),
    length_opt("overall_length", 1.0, 600.0, 60.0, "tip to top of the tool"),
    length_opt("shank_diameter", 0.1, 100.0, 6.0, "clamp (arbor) diameter"),
];
const COUNTERBORE: &[ParamSpec] = &[
    length_opt("pilot_diameter", 0.1, 63.0, 4.0, "pilot diameter"),
    length_opt("pilot_length", 0.1, 100.0, 5.0, "pilot length ahead of the counterbore"),
    length_opt("counterbore_diameter", 0.5, 100.0, 9.0, "counterbore diameter"),
    length_opt("counterbore_depth", 0.1, 100.0, 6.0, "depth of the counterbore"),
    length_opt("overall_length", 1.0, 600.0, 60.0, "tip to top of the shank"),
    length_opt("shank_diameter", 0.1, 100.0, 6.0, "shank diameter"),
];

/// The tangent of half an included angle given in degrees; validation keeps angles off 0 and 180.
pub(crate) fn half_tan(included_deg: f64) -> f64 {
    (included_deg.to_radians() / 2.0).tan()
}

/// The value of a parameter: the given one, or the schema default when absent.
pub fn value(family: Family, params: &BTreeMap<String, f64>, name: &str) -> Option<f64> {
    if let Some(v) = params.get(name) {
        return Some(*v);
    }
    family.schema().iter().find(|s| s.name == name).and_then(|s| s.default)
}

/// Rejects anything the family's profile builder could not build from: unknown or missing
/// parameters, non-finite or out-of-range values, and geometrically impossible combinations.
pub fn validate_params(family: Family, params: &BTreeMap<String, f64>) -> Result<()> {
    let schema = family.schema();
    for key in params.keys() {
        if !schema.iter().any(|s| s.name == key) {
            return Err(invalid(format!("unknown parameter `{key}` for `{}`", family.as_str())));
        }
    }
    for spec in schema {
        match params.get(spec.name) {
            Some(v) => {
                if !v.is_finite() || *v < spec.min || *v > spec.max {
                    return Err(invalid(format!(
                        "`{}` must be a finite number between {} and {} for `{}`",
                        spec.name,
                        spec.min,
                        spec.max,
                        family.as_str()
                    )));
                }
            }
            None if spec.default.is_some() => {}
            None => return Err(invalid(format!("`{}` is required for `{}`", spec.name, family.as_str()))),
        }
    }
    let get = |name: &str| value(family, params, name).unwrap_or(f64::NAN);
    let ok = |b: bool, why: &str| if b { Ok(()) } else { Err(invalid(format!("invalid `{}` parameters: {why}", family.as_str()))) };
    match family {
        Family::EndMillFlat => ok(get("flute_length") < get("overall_length"), "flute_length must be less than overall_length"),
        Family::EndMillBall => ok(get("flute_length") >= get("cutting_diameter") / 2.0, "flute_length must reach at least the ball radius")
            .and_then(|()| ok(get("flute_length") < get("overall_length"), "flute_length must be less than overall_length")),
        Family::EndMillBull => ok(2.0 * get("corner_radius") < get("cutting_diameter"), "corner_radius must be less than the cutting radius")
            .and_then(|()| ok(get("flute_length") >= get("corner_radius"), "flute_length must reach at least the corner radius"))
            .and_then(|()| ok(get("flute_length") < get("overall_length"), "flute_length must be less than overall_length")),
        Family::Drill => {
            let depth = get("cutting_diameter") / 2.0 / half_tan(get("point_angle"));
            ok(get("flute_length") >= depth, "flute_length must reach at least the point cone")
                .and_then(|()| ok(get("flute_length") < get("overall_length"), "flute_length must be less than overall_length"))
        }
        Family::CenterDrill => {
            let cone = (get("major_diameter") - get("pilot_diameter")) / 2.0 / half_tan(get("point_angle"));
            ok(get("pilot_diameter") < get("major_diameter"), "pilot_diameter must be less than major_diameter")
                .and_then(|()| ok(get("pilot_length") + cone < get("overall_length"), "pilot plus cone must fit within overall_length"))
        }
        Family::ChamferMill => {
            let cone = (get("cutting_diameter") - get("pilot_diameter")) / 2.0 / half_tan(get("included_angle"));
            ok(get("pilot_diameter") < get("cutting_diameter"), "pilot_diameter must be less than cutting_diameter")
                .and_then(|()| ok(cone < get("flute_length"), "flute_length must reach past the chamfer cone"))
                .and_then(|()| ok(get("flute_length") < get("overall_length"), "flute_length must be less than overall_length"))
        }
        Family::Countersink => {
            let cone = get("cutting_diameter") / 2.0 / half_tan(get("included_angle"));
            ok(cone < get("flute_length"), "flute_length must reach past the cone")
                .and_then(|()| ok(get("flute_length") < get("overall_length"), "flute_length must be less than overall_length"))
        }
        Family::Tap => ok(get("thread_length") < get("overall_length"), "thread_length must be less than overall_length"),
        Family::Keyway => ok(get("width") < get("overall_length"), "width must be less than overall_length"),
        Family::TSlot => ok(get("shank_diameter") < get("crossbar_diameter"), "the neck must be thinner than the crossbar")
            .and_then(|()| ok(get("crossbar_width") < get("overall_length"), "crossbar_width must be less than overall_length")),
        Family::CornerRadius => ok(2.0 * get("corner_radius") < get("cutting_diameter"), "corner_radius must be less than the body radius")
            .and_then(|()| ok(get("flute_length") >= get("corner_radius"), "flute_length must reach at least the corner radius"))
            .and_then(|()| ok(get("flute_length") < get("overall_length"), "flute_length must be less than overall_length")),
        Family::Counterbore => ok(get("pilot_diameter") < get("counterbore_diameter"), "pilot_diameter must be less than counterbore_diameter")
            .and_then(|()| {
                ok(get("pilot_length") + get("counterbore_depth") < get("overall_length"), "pilot plus counterbore must fit within overall_length")
            }),
    }
}

#[cfg(test)]
#[path = "family_tests.rs"]
mod tests;
