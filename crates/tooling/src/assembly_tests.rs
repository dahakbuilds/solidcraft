use std::collections::BTreeMap;

use super::*;
use crate::profile;
use crate::{Family, Role};

fn tool() -> CutterProfile {
    let mut params = BTreeMap::new();
    params.insert("cutting_diameter".to_string(), 10.0);
    params.insert("flute_length".to_string(), 20.0);
    params.insert("overall_length".to_string(), 70.0);
    params.insert("shank_diameter".to_string(), 10.0);
    profile::of(Family::EndMillFlat, &params).unwrap()
}

#[test]
fn assembly_stacks_tool_below_holder() {
    // er32_bt40: nose 30 mm below the girth, taper 55 mm above; stickout 60 from the tip.
    let a = compose(&tool(), "er32_bt40", 10.0, 60.0, 16).unwrap();
    assert!((a.reach - 30.0).abs() < 1e-9);
    assert!((a.overall_length - 115.0).abs() < 1e-9);
    assert!(a.max_radius >= 31.0);
    // the holder faces sit above the girth line and carry the holder role
    let holder_faces: Vec<usize> = a.mesh.face_roles.iter().enumerate().filter(|(_, r)| **r == Role::Holder).map(|(i, _)| i).collect();
    assert!(!holder_faces.is_empty());
    for (ti, tri) in a.mesh.mesh.triangles.iter().enumerate() {
        let face = a.mesh.mesh.tri_face[ti] as usize;
        let role = a.mesh.face_roles[face];
        if role == Role::Holder {
            let z = a.mesh.mesh.positions[tri[0] as usize].z;
            assert!(z >= 60.0 - 30.0 - 1e-9, "holder face dipped below the nose: z {z} face {face} tri {tri:?}");
        }
    }
    // the tool's cutting faces still start at the tip
    let mut zmin_tool = f64::MAX;
    for (ti, tri) in a.mesh.mesh.triangles.iter().enumerate() {
        if a.mesh.face_roles[a.mesh.mesh.tri_face[ti] as usize] == Role::Cutting {
            for v in tri {
                zmin_tool = zmin_tool.min(a.mesh.mesh.positions[*v as usize].z);
            }
        }
    }
    assert!(zmin_tool.abs() < 1e-9);
}

#[test]
fn hostile_assemblies_are_refused() {
    // stickout inside the nose
    assert!(compose(&tool(), "er32_bt40", 10.0, 20.0, 16).is_err());
    assert!(compose(&tool(), "er32_bt40", 10.0, f64::NAN, 16).is_err());
    // unknown holder or a bore the holder cannot grip
    assert!(compose(&tool(), "nope", 10.0, 60.0, 16).is_err());
    assert!(compose(&tool(), "er32_bt40", 32.0, 60.0, 16).is_err());
}
