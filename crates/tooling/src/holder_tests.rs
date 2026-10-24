use super::*;

#[test]
fn catalog_is_consistent() {
    let catalog = catalog();
    assert!(catalog.len() >= 12);
    for h in &catalog {
        assert!(h.bore_min < h.bore_max, "{}", h.id);
        assert!(h.nose_len > 0.0 && h.overall_length > h.nose_len, "{}", h.id);
        assert_eq!(find(&h.id).map(|x| x.id), Some(h.id.clone()));
    }
    assert!(find("er32_bt40").is_some());
    assert!(find("nope").is_none());
}

#[test]
fn holder_profiles_stand_on_the_nose_and_revolve() {
    for h in catalog() {
        let bore = (h.bore_min + h.bore_max) / 2.0;
        let p = profile_of(&h.id, bore).unwrap();
        assert_eq!(p.cutting_length, 0.0);
        assert!((p.overall_length - h.overall_length).abs() < 1e-9, "{}", h.id);
        assert!(p.max_radius > 0.0);
        // the profile spans -nose_len .. taper_len about the girth line
        let zmin = p
            .segments
            .iter()
            .map(|s| match *s {
                Segment::Line { z0, z1, .. } | Segment::Arc { z0, z1, .. } => z0.min(z1),
            })
            .fold(0.0_f64, f64::min);
        assert!((zmin + h.nose_len).abs() < 1e-9, "{}", h.id);
        let revolved = crate::mesh::revolve_with(&p, 16, crate::Role::Holder).unwrap();
        assert!(!revolved.mesh.triangles.is_empty());
        assert!(revolved.face_roles.iter().all(|r| *r == crate::Role::Holder));
    }
}

#[test]
fn every_catalog_entry_revolves_at_both_bore_extremes() {
    for h in catalog() {
        for bore in [h.bore_min, h.bore_max] {
            let p = profile_of(&h.id, bore).unwrap();
            let r = crate::mesh::revolve_with(&p, 8, crate::Role::Holder).unwrap();
            assert!(!r.mesh.triangles.is_empty(), "{} @ bore {bore}", h.id);
        }
    }
}

#[test]
fn hostile_bores_are_refused() {
    assert!(profile_of("er32_bt40", f64::NAN).is_err());
    assert!(profile_of("er32_bt40", 0.0).is_err());
    assert!(profile_of("er32_bt40", 999.0).is_err());
    assert!(profile_of("unknown", 8.0).is_err());
    // bore-sized bodies accept any bore in range
    assert!(profile_of("shrink_bt40", 10.0).is_ok());
}
