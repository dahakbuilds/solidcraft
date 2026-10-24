use super::*;

fn params(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn flat_end_mill_envelope_is_a_cylinder_then_the_shank() {
    let p = params(&[("cutting_diameter", 10.0), ("flute_length", 20.0), ("overall_length", 50.0), ("shank_diameter", 8.0)]);
    let prof = of(Family::EndMillFlat, &p).unwrap();
    assert!(approx(prof.max_radius, 5.0));
    assert!(approx(prof.cutting_length, 20.0));
    assert!(approx(prof.radius_at(0.0).unwrap(), 5.0));
    assert!(approx(prof.radius_at(10.0).unwrap(), 5.0));
    assert!(approx(prof.radius_at(20.0).unwrap(), 5.0));
    assert!(approx(prof.radius_at(30.0).unwrap(), 4.0));
    assert!(approx(prof.radius_at(50.0).unwrap(), 4.0));
    assert!(prof.radius_at(50.1).is_none());
    assert!(prof.radius_at(-0.1).is_none());
    // the end face and flank cut; the shank does not
    assert!(prof.segments.iter().filter(|s| s.cutting()).count() >= 2);
    assert!(!prof.segments.last().unwrap().cutting());
}

#[test]
fn ball_nose_envelope_follows_the_sphere() {
    let p = params(&[("cutting_diameter", 10.0), ("flute_length", 20.0), ("overall_length", 50.0), ("shank_diameter", 10.0)]);
    let prof = of(Family::EndMillBall, &p).unwrap();
    // r(z) = sqrt(R^2 - (R - z)^2) for z <= R
    for z in [0.5_f64, 1.0, 3.0, 4.9] {
        let want = (25.0 - (5.0 - z) * (5.0 - z)).sqrt();
        assert!(approx(prof.radius_at(z).unwrap(), want), "z={z}");
    }
    assert!(approx(prof.radius_at(5.0).unwrap(), 5.0));
    assert!(approx(prof.radius_at(12.0).unwrap(), 5.0));
}

#[test]
fn bull_nose_envelope_is_flat_then_torus_then_cylinder() {
    let p = params(&[("cutting_diameter", 12.0), ("corner_radius", 2.0), ("flute_length", 20.0), ("overall_length", 60.0), ("shank_diameter", 12.0)]);
    let prof = of(Family::EndMillBull, &p).unwrap();
    assert!(approx(prof.radius_at(0.0).unwrap(), 4.0)); // flat to R - r = 4
    // torus branch: r = (R - r) + sqrt(r^2 - (z - r)^2)
    let z = 1.0_f64;
    let want = 4.0 + (4.0 - (z - 2.0) * (z - 2.0)).sqrt();
    assert!(approx(prof.radius_at(z).unwrap(), want));
    assert!(approx(prof.radius_at(2.0).unwrap(), 6.0));
    assert!(approx(prof.radius_at(10.0).unwrap(), 6.0));
}

#[test]
fn drill_point_cone_matches_the_point_angle() {
    let p =
        params(&[("cutting_diameter", 10.0), ("flute_length", 33.0), ("overall_length", 100.0), ("shank_diameter", 10.0), ("point_angle", 118.0)]);
    let prof = of(Family::Drill, &p).unwrap();
    let tan = (59.0_f64).to_radians().tan();
    let zp = 5.0 / tan;
    assert!(prof.radius_at(zp / 2.0).unwrap() < 5.0);
    assert!(approx(prof.radius_at(zp).unwrap(), 5.0));
    assert!(approx(prof.radius_at(60.0).unwrap(), 5.0));
}

#[test]
fn chamfer_mill_cone_and_relief() {
    let p = params(&[
        ("cutting_diameter", 16.0),
        ("included_angle", 90.0),
        ("pilot_diameter", 0.0),
        ("flute_length", 10.0),
        ("overall_length", 50.0),
        ("shank_diameter", 10.0),
    ]);
    let prof = of(Family::ChamferMill, &p).unwrap();
    // 90° included, zero pilot: cone height = R = 8
    assert!(approx(prof.cutting_length, 8.0));
    assert!(approx(prof.radius_at(4.0).unwrap(), 4.0));
    assert!(approx(prof.radius_at(8.0).unwrap(), 8.0));
    // the relief above the cone is not a cutting surface
    let relief = &prof.segments[1];
    assert!(!relief.cutting());
}

#[test]
fn countersink_and_tap_build() {
    let p = params(&[("cutting_diameter", 10.0), ("included_angle", 90.0), ("flute_length", 6.0), ("overall_length", 40.0), ("shank_diameter", 6.0)]);
    let prof = of(Family::Countersink, &p).unwrap();
    assert!(approx(prof.cutting_length, 5.0));
    let p = params(&[("major_diameter", 6.0), ("pitch", 1.0), ("thread_length", 18.0), ("overall_length", 60.0), ("shank_diameter", 4.5)]);
    let prof = of(Family::Tap, &p).unwrap();
    assert!(approx(prof.max_radius, 3.0));
    assert!(approx(prof.cutting_length, 18.0));
}

#[test]
fn hostile_parameters_never_build_a_profile() {
    let p = params(&[("cutting_diameter", 10.0), ("flute_length", f64::NAN), ("overall_length", 70.0), ("shank_diameter", 10.0)]);
    assert!(of(Family::EndMillFlat, &p).is_err());
    let empty = BTreeMap::new();
    assert!(of(Family::EndMillBall, &empty).is_err());
}
