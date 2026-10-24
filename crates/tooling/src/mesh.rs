//! The display/collision mesh of a revolved cutter profile.
//!
//! Revolving the profile is pure math: each line becomes a cone or cylinder band, each arc a
//! torus or sphere band, sampled at `steps` angles about the tool axis. Every triangle carries a
//! face id, and each face id maps to the [Role] of the profile segment it came from, so a viewer
//! can paint cutting surfaces, shank and (later) holders differently without knowing the families.

use serde::Serialize;
use solidcraft_geom::{Mesh, Vec3};

use crate::{
    Result,
    invalid,
    profile::{CutterProfile, Segment},
};

/// What a band of the revolved surface is, derived from the profile segment's cutting flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Cutting,
    Shank,
    /// Reserved for tool assemblies (phase 3); the profile math never emits it yet.
    Holder,
}

/// A revolved profile: the triangle mesh plus the role of each face id.
pub struct Revolved {
    pub mesh: Mesh,
    /// One role per face id (`tri_face` indexes into this).
    pub face_roles: Vec<Role>,
}

/// Smallest radius treated as off-axis; a ring at or below this collapses to the axis point.
const AXIS_EPS: f64 = 1e-9;
/// Most triangles one revolve may produce; a request beyond this is refused, not truncated.
const MAX_TRIANGLES: usize = 200_000;

/// Revolves `profile` about its z axis at `steps` angular steps (3..=512).
pub fn revolve(profile: &CutterProfile, steps: usize) -> Result<Revolved> {
    if profile.segments.is_empty() {
        return Err(invalid("cutter profile has no segments"));
    }
    if !(3..=512).contains(&steps) {
        return Err(invalid("revolve steps must be between 3 and 512"));
    }
    // One polyline of (r, 0, z) points through all segments; consecutive segments share their
    // junction point. face_of[j] is the segment the band above point j belongs to.
    let mut polyline: Vec<Vec3> = Vec::new();
    let mut face_of: Vec<u32> = Vec::new();
    for (i, seg) in profile.segments.iter().enumerate() {
        for p in sample(seg)? {
            if polyline.last().is_some_and(|last| (last.x - p.x).abs() < 1e-12 && (last.z - p.z).abs() < 1e-12) {
                continue;
            }
            polyline.push(p);
            face_of.push(i as u32);
        }
    }
    let face_roles: Vec<Role> = profile.segments.iter().map(|s| if s.cutting() { Role::Cutting } else { Role::Shank }).collect();
    if polyline.len() < 2 {
        return Err(invalid("cutter profile is degenerate"));
    }

    let mut mesh = Mesh::default();
    // Side bands between consecutive profile points. Wound CCW seen from outside: around the
    // axis the ring runs +azimuth, so the quad is lower@k, lower@k+1, upper@k+1, upper@k.
    for j in 0..polyline.len() - 1 {
        let face = face_of[j];
        let (p0, p1) = (polyline[j], polyline[j + 1]);
        if p0.x <= AXIS_EPS && p1.x <= AXIS_EPS {
            continue;
        }
        let ring = |p: Vec3, angle: f64| Vec3::new(p.x * angle.cos(), p.x * angle.sin(), p.z);
        let (dr, dz) = (p1.x - p0.x, p1.z - p0.z);
        // Outward surface normal of the band at angle a: the revolution of tangent (dr, dz).
        let normal = |a: f64| Vec3::new(dz * a.cos(), dz * a.sin(), -dr).normalized().unwrap_or(Vec3::X);
        for k in 0..steps {
            let (a0, a1) = (std::f64::consts::TAU * (k as f64) / (steps as f64), std::f64::consts::TAU * ((k + 1) as f64) / (steps as f64));
            let (n0, n1) = (normal(a0), normal(a1));
            // A corner on the axis collapses the quad to a triangle (a cone tip or apex ring).
            let corners: Vec<(Vec3, Vec3)> = match (p0.x <= AXIS_EPS, p1.x <= AXIS_EPS) {
                (false, false) => vec![(ring(p0, a0), n0), (ring(p0, a1), n1), (ring(p1, a1), n1), (ring(p1, a0), n0)],
                (true, false) => vec![(Vec3::ZERO, n0), (ring(p1, a1), n1), (ring(p1, a0), n0)],
                (false, true) => vec![(ring(p0, a0), n0), (ring(p0, a1), n1), (Vec3::ZERO, n1)],
                _ => vec![],
            };
            let base = mesh.positions.len() as u32;
            for (pos, normal) in corners.iter() {
                mesh.positions.push(*pos);
                mesh.normals.push(*normal);
            }
            mesh.triangles.push([base, base + 1, base + 2]);
            mesh.tri_face.push(face);
            if corners.len() == 4 {
                mesh.triangles.push([base, base + 2, base + 3]);
                mesh.tri_face.push(face);
            }
        }
    }
    // Flat end caps where the profile starts or ends off-axis (a flat end or a top face).
    if polyline[0].x > AXIS_EPS {
        cap(&mut mesh, polyline[0].x, polyline[0].z, steps, false, face_of[0]);
    }
    let top = polyline[polyline.len() - 1];
    if top.x > AXIS_EPS {
        cap(&mut mesh, top.x, top.z, steps, true, *face_of.last().unwrap_or(&0));
    }
    if mesh.triangles.len() > MAX_TRIANGLES {
        return Err(invalid(format!("revolve would produce {} triangles (limit {MAX_TRIANGLES})", mesh.triangles.len())));
    }
    let clean = mesh.positions.iter().chain(mesh.normals.iter()).all(|v| v.x.is_finite() && v.y.is_finite() && v.z.is_finite());
    if !clean {
        return Err(invalid("revolve produced non-finite geometry"));
    }
    Ok(Revolved { mesh, face_roles })
}

/// A flat disc at radius `r` and height `z`, wound outward (−z bottom, +z top).
fn cap(mesh: &mut Mesh, r: f64, z: f64, steps: usize, top: bool, face: u32) {
    let n = if top { Vec3::Z } else { Vec3::new(0.0, 0.0, -1.0) };
    let base = mesh.positions.len() as u32;
    for k in 0..steps {
        let a = std::f64::consts::TAU * (k as f64) / (steps as f64);
        mesh.positions.push(Vec3::new(r * a.cos(), r * a.sin(), z));
        mesh.normals.push(n);
    }
    for k in 0..steps {
        let a = base + k as u32;
        let b = base + ((k + 1) % steps) as u32;
        let c = mesh.positions.len() as u32;
        mesh.positions.push(Vec3::new(0.0, 0.0, z));
        mesh.normals.push(n);
        mesh.triangles.push(if top { [a, b, c] } else { [b, a, c] });
        mesh.tri_face.push(face);
    }
}

/// The (r, z) points a segment revolves through, from its first point to its last.
fn sample(seg: &Segment) -> Result<Vec<Vec3>> {
    match *seg {
        Segment::Line { r0, z0, r1, z1, .. } => {
            if ![r0, z0, r1, z1].iter().all(|v| v.is_finite()) || r0 < 0.0 || r1 < 0.0 {
                return Err(invalid("profile line segment has invalid coordinates"));
            }
            Ok(vec![Vec3::new(r0, 0.0, z0), Vec3::new(r1, 0.0, z1)])
        }
        Segment::Arc { cr, cz, radius, r0, z0, r1, z1, .. } => {
            if ![cr, cz, radius, r0, z0, r1, z1].iter().all(|v| v.is_finite()) || radius <= AXIS_EPS {
                return Err(invalid("profile arc segment has invalid coordinates"));
            }
            let on = |r: f64, z: f64| ((r - cr).powi(2) + (z - cz).powi(2) - radius * radius).abs() <= 1e-6 * radius.max(1.0);
            if !on(r0, z0) || !on(r1, z1) {
                return Err(invalid("profile arc endpoints are not on its circle"));
            }
            let a0 = (z0 - cz).atan2(r0 - cr);
            let mut sweep = (z1 - cz).atan2(r1 - cr) - a0;
            while sweep > std::f64::consts::PI {
                sweep -= std::f64::consts::TAU;
            }
            while sweep < -std::f64::consts::PI {
                sweep += std::f64::consts::TAU;
            }
            if sweep.abs() > std::f64::consts::PI + 1e-9 {
                return Err(invalid("profile arc sweeps more than a half circle"));
            }
            let n = ((sweep.abs() / 0.1).ceil() as usize).clamp(2, 48);
            Ok((0..=n)
                .map(|i| {
                    let a = a0 + sweep * (i as f64) / (n as f64);
                    Vec3::new(cr + radius * a.cos(), 0.0, cz + radius * a.sin())
                })
                .collect())
        }
    }
}

#[cfg(test)]
#[path = "mesh_tests.rs"]
mod tests;
