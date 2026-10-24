use super::*;

fn params(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

#[test]
fn every_family_parses_back_from_its_id() {
    for f in FAMILIES {
        assert_eq!(Family::parse(f.as_str()), Some(f), "{}", f.as_str());
        assert!(!f.schema().is_empty());
    }
    assert_eq!(Family::parse("turn_holder"), None);
}

#[test]
fn flat_end_mill_accepts_a_sane_definition() {
    let p = params(&[("cutting_diameter", 10.0), ("flute_length", 20.0), ("overall_length", 70.0), ("shank_diameter", 10.0)]);
    assert!(validate_params(Family::EndMillFlat, &p).is_ok());
}

#[test]
fn missing_parameters_use_defaults_and_unknown_are_refused() {
    // every built-in parameter carries a default, so a partial set validates and fills in
    let partial = params(&[("cutting_diameter", 10.0), ("flute_length", 20.0)]);
    validate_params(Family::EndMillFlat, &partial).unwrap();
    assert_eq!(value(Family::EndMillFlat, &partial, "overall_length"), Some(70.0));
    assert_eq!(value(Family::EndMillFlat, &partial, "shank_diameter"), Some(10.0));
    let mut extra = params(&[("cutting_diameter", 10.0), ("flute_length", 20.0), ("overall_length", 70.0), ("shank_diameter", 10.0)]);
    extra.insert("magic".into(), 1.0);
    assert!(validate_params(Family::EndMillFlat, &extra).unwrap_err().to_string().contains("magic"));
}

#[test]
fn hostile_numbers_are_refused() {
    let base = [("cutting_diameter", 10.0), ("flute_length", 20.0), ("overall_length", 70.0), ("shank_diameter", 10.0)];
    for bad in [f64::NAN, f64::INFINITY, -10.0, 1e300] {
        let p = params(&[("cutting_diameter", bad), ("flute_length", 20.0), ("overall_length", 70.0), ("shank_diameter", 10.0)]);
        assert!(validate_params(Family::EndMillFlat, &p).is_err(), "{bad}");
    }
    // out of the schema range
    let p = params(&base).into_iter().map(|(k, v)| if k == "flute_length" { (k, 500.0) } else { (k, v) }).collect();
    assert!(validate_params(Family::EndMillFlat, &p).is_err());
}

#[test]
fn cross_parameter_rules() {
    // flute longer than the whole tool
    let p = params(&[("cutting_diameter", 10.0), ("flute_length", 80.0), ("overall_length", 70.0), ("shank_diameter", 10.0)]);
    assert!(validate_params(Family::EndMillFlat, &p).is_err());
    // ball whose flute does not reach the ball radius
    let p = params(&[("cutting_diameter", 10.0), ("flute_length", 4.0), ("overall_length", 70.0), ("shank_diameter", 10.0)]);
    assert!(validate_params(Family::EndMillBall, &p).is_err());
    // bull corner radius reaching the centreline
    let p = params(&[("cutting_diameter", 10.0), ("corner_radius", 5.0), ("flute_length", 20.0), ("overall_length", 70.0), ("shank_diameter", 10.0)]);
    assert!(validate_params(Family::EndMillBull, &p).is_err());
    // centre drill whose pilot exceeds the major diameter
    let p = params(&[("pilot_diameter", 8.0), ("pilot_length", 5.0), ("major_diameter", 6.0), ("overall_length", 60.0), ("shank_diameter", 10.0)]);
    assert!(validate_params(Family::CenterDrill, &p).is_err());
    // a sound centre drill (118° default): cone height (6-6.0... check with real numbers)
    let p = params(&[("pilot_diameter", 3.15), ("pilot_length", 3.2), ("major_diameter", 6.3), ("overall_length", 63.0), ("shank_diameter", 6.3)]);
    assert!(validate_params(Family::CenterDrill, &p).is_ok());
}

#[test]
fn optional_parameters_use_their_defaults() {
    // drill without point_angle: the 118° default applies and the cone fits
    let p = params(&[("cutting_diameter", 10.0), ("flute_length", 33.0), ("overall_length", 100.0), ("shank_diameter", 10.0)]);
    assert!(validate_params(Family::Drill, &p).is_ok());
    assert_eq!(value(Family::Drill, &p, "point_angle"), Some(118.0));
    // a drill whose flute stops short of its point cone is refused
    let p = params(&[("cutting_diameter", 10.0), ("flute_length", 2.0), ("overall_length", 100.0), ("shank_diameter", 10.0)]);
    assert!(validate_params(Family::Drill, &p).is_err());
}
