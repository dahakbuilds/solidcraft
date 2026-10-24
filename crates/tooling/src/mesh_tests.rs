use super::*;
use crate::{Family, profile};
use std::collections::BTreeMap;

fn params(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn flat_end_mill() -> CutterProfile {
    profile::of(
        Family::EndMillFlat,
        &params(&[("cutting_diameter", 10.0), ("flute_length", 20.0), ("overall_length", 70.0), ("shank_diameter", 10.0)]),
    )
    .unwrap()
}

/// Every triangle's geometric normal (CCW seen from outside) agrees with its stored vertex normals.
fn assert_wound_outward(mesh: &Mesh) {
    for t in &mesh.triangles {
        let [a, b, c] = *t;
        let (pa, pb, pc) = (mesh.positions[a as usize], mesh.positions[b as usize], mesh.positions[c as usize]);
        let geometric = (pb - pa).cross(pc - pa);
        let averaged = mesh.normals[a as usize] + mesh.normals[b as usize] + mesh.normals[c as usize];
        assert!(geometric.dot(averaged) > 0.0, "triangle {t:?} is wound inward");
    }
}

#[test]
fn flat_end_mill_revolve_has_the_expected_band_counts() {
    let steps = 64;
    let r = revolve(&flat_end_mill(), steps).unwrap();
    // bottom disc + flute side + shank side + top disc
    assert_eq!(r.mesh.triangles.len(), 6 * steps);
    assert_eq!(r.face_roles, vec![Role::Cutting, Role::Cutting, Role::Shank]);
    assert_eq!(r.mesh.tri_face.last().copied(), Some(2));
    // the side band above the flute (equal diameters, so collinear with the cutting wall) is
    // painted Shank, not merged into the cutting role
    for (i, face) in r.mesh.tri_face.iter().enumerate() {
        let z = r.mesh.positions[r.mesh.triangles[i][0] as usize].z;
        if z > 20.0 + 1e-9 && z < 70.0 - 1e-9 {
            assert_eq!(r.face_roles[*face as usize], Role::Shank, "band above the flute is the shank");
        }
    }
    assert_wound_outward(&r.mesh);
}

#[test]
fn cylinder_band_normals_point_radially_outward() {
    let r = revolve(&flat_end_mill(), 32).unwrap();
    for (i, t) in r.mesh.tri_face.iter().enumerate() {
        if *t != 1 {
            continue;
        }
        for v in r.mesh.triangles[i] {
            let p = r.mesh.positions[v as usize];
            let n = r.mesh.normals[v as usize];
            let radial = Vec3::new(p.x, p.y, 0.0).normalized().unwrap();
            assert!(n.dot(radial) > 0.99, "side normal {n:?} at {p:?} is not radial");
        }
    }
}

#[test]
fn cone_apex_collapses_to_triangles_at_the_axis() {
    let drill = profile::of(
        Family::Drill,
        &params(&[("cutting_diameter", 8.0), ("flute_length", 30.0), ("overall_length", 60.0), ("shank_diameter", 8.0), ("point_angle", 118.0)]),
    )
    .unwrap();
    let steps = 48;
    let r = revolve(&drill, steps).unwrap();
    // cone side + flute side + shank side + top disc
    assert_eq!(r.mesh.triangles.len(), 6 * steps);
    let on_axis = r.mesh.positions.iter().filter(|p| p.x.abs() < 1e-12 && p.y.abs() < 1e-12).count();
    assert!(on_axis >= steps, "apex ring should sit on the axis, found {on_axis}");
    assert_wound_outward(&r.mesh);
}

#[test]
fn ball_nose_arc_stays_on_its_sphere() {
    let ball = profile::of(
        Family::EndMillBall,
        &params(&[("cutting_diameter", 12.0), ("flute_length", 25.0), ("overall_length", 60.0), ("shank_diameter", 12.0)]),
    )
    .unwrap();
    let r = revolve(&ball, 48).unwrap();
    for p in &r.mesh.positions {
        if p.z <= 6.0 + 1e-9 {
            let d = p.x.hypot(p.y).hypot(p.z - 6.0);
            assert!((d - 6.0).abs() < 1e-6, "point {p:?} left the ball sphere (r = {d})");
        }
    }
    assert_wound_outward(&r.mesh);
}

#[test]
fn hostile_profiles_and_steps_are_refused_not_panicked() {
    assert!(revolve(&CutterProfile { segments: vec![], cutting_length: 0.0, overall_length: 0.0, max_radius: 0.0 }, 32).is_err());
    assert!(revolve(&flat_end_mill(), 2).is_err());
    assert!(revolve(&flat_end_mill(), 513).is_err());
    let broken = CutterProfile {
        segments: vec![Segment::Line { r0: f64::NAN, z0: 0.0, r1: 5.0, z1: 0.0, cutting: true }],
        cutting_length: 0.0,
        overall_length: 1.0,
        max_radius: 5.0,
    };
    assert!(revolve(&broken, 32).is_err());
    let off_circle = CutterProfile {
        segments: vec![Segment::Arc { cr: 0.0, cz: 5.0, radius: 5.0, r0: 0.0, z0: 0.0, r1: 9.0, z1: 5.0, cutting: true }],
        cutting_length: 5.0,
        overall_length: 5.0,
        max_radius: 9.0,
    };
    assert!(revolve(&off_circle, 32).is_err());
}

#[test]
fn every_family_revolves_at_preview_quality() {
    let defaults: &[Family] = &[
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
    for family in defaults {
        let schema = family.schema();
        let filled: BTreeMap<String, f64> = schema.iter().map(|s| (s.name.to_string(), s.default.unwrap_or(1.0))).collect();
        let Ok(profile) = profile::of(*family, &filled) else { continue };
        let r = revolve(&profile, 64).unwrap_or_else(|e| panic!("{family:?}: {e}"));
        assert!(!r.mesh.triangles.is_empty(), "{family:?} produced no triangles");
        assert!(r.mesh.triangles.len() <= MAX_TRIANGLES);
        assert_wound_outward(&r.mesh);
    }
}
