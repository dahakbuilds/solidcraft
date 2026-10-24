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

const fn length(name: &'static str, min: f64, max: f64, doc: &'static str) -> ParamSpec {
    ParamSpec { name, min, max, default: None, unit: ParamUnit::Length, doc }
}
const fn length_opt(name: &'static str, min: f64, max: f64, default: f64, doc: &'static str) -> ParamSpec {
    ParamSpec { name, min, max: max, default: Some(default), unit: ParamUnit::Length, doc }
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
}

pub const FAMILIES: [Family; 8] = [
    Family::EndMillFlat,
    Family::EndMillBall,
    Family::EndMillBull,
    Family::Drill,
    Family::CenterDrill,
    Family::ChamferMill,
    Family::Countersink,
    Family::Tap,
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
        }
    }
}

const FLAT_BALL: &[ParamSpec] = &[
    length("cutting_diameter", 0.1, 63.0, "cutting diameter"),
    length("flute_length", 0.1, 300.0, "axial length of the cutting flutes"),
    length("overall_length", 1.0, 600.0, "tip to top of the tool"),
    length("shank_diameter", 0.1, 100.0, "clamp (arbor) diameter"),
];
const BULL: &[ParamSpec] = &[
    length("cutting_diameter", 0.1, 63.0, "cutting diameter"),
    length("corner_radius", 0.05, 30.0, "corner fillet radius"),
    length("flute_length", 0.1, 300.0, "axial length of the cutting flutes"),
    length("overall_length", 1.0, 600.0, "tip to top of the tool"),
    length("shank_diameter", 0.1, 100.0, "clamp (arbor) diameter"),
];
const DRILL: &[ParamSpec] = &[
    length("cutting_diameter", 0.05, 63.0, "drill diameter"),
    length("flute_length", 0.1, 300.0, "flute length below the neck"),
    length("overall_length", 1.0, 600.0, "tip to top of the shank"),
    length("shank_diameter", 0.05, 100.0, "shank diameter"),
    angle("point_angle", 60.0, 179.9, 118.0, "included point angle"),
];
const CENTER_DRILL: &[ParamSpec] = &[
    length("pilot_diameter", 0.1, 20.0, "pilot (centre) hole diameter"),
    length("pilot_length", 0.1, 30.0, "pilot cylinder length"),
    length("major_diameter", 0.2, 63.0, "countersink (major) diameter"),
    angle("point_angle", 60.0, 179.9, 118.0, "included angle of the countersink cone"),
    length("overall_length", 1.0, 300.0, "tip to top of the shank"),
    length("shank_diameter", 0.1, 63.0, "shank diameter"),
];
const CHAMFER_MILL: &[ParamSpec] = &[
    length("cutting_diameter", 0.5, 63.0, "maximum cutting diameter"),
    angle("included_angle", 30.0, 179.9, 90.0, "included chamfer angle"),
    length_opt("pilot_diameter", 0.0, 31.0, 0.0, "flat tip diameter below the cone"),
    length("flute_length", 0.1, 100.0, "length of the cone plus its relief"),
    length("overall_length", 1.0, 600.0, "tip to top of the tool"),
    length("shank_diameter", 0.1, 100.0, "clamp (arbor) diameter"),
];
const COUNTERSINK: &[ParamSpec] = &[
    length("cutting_diameter", 0.5, 63.0, "maximum cutting diameter"),
    angle("included_angle", 30.0, 179.9, 90.0, "included cone angle"),
    length("flute_length", 0.1, 100.0, "length of the cone plus its relief"),
    length("overall_length", 1.0, 600.0, "tip to top of the tool"),
    length("shank_diameter", 0.1, 100.0, "clamp (arbor) diameter"),
];
const TAP: &[ParamSpec] = &[
    length("major_diameter", 0.5, 63.0, "thread major diameter"),
    length("pitch", 0.2, 6.0, "thread pitch"),
    length("thread_length", 0.5, 200.0, "threaded length"),
    length("overall_length", 1.0, 600.0, "tip to top of the shank"),
    length("shank_diameter", 0.1, 100.0, "shank (tang) diameter"),
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
    }
}

#[cfg(test)]
#[path = "family_tests.rs"]
mod tests;
