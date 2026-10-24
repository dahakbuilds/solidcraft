//! Plain meshes for CAM display: the stock a operation starts from. Pure geometry —
//! the viewport draws these see-through; nothing here needs the kernel.

use crate::{Result, Stock, StockKind, invalid};
use solidcraft_geom::{Mesh, Vec3};

/// Largest segment count for a round stock's rim.
pub const MAX_STOCK_SEGMENTS: usize = 128;
/// Segments a drawn cylinder rim uses when the caller does not choose.
pub const DEFAULT_STOCK_SEGMENTS: usize = 48;

fn finite(v: &[f64; 3]) -> bool {
    v[0].is_finite() && v[1].is_finite() && v[2].is_finite()
}

/// Append a quad, wound counter-clockwise seen from `outward` (the winding is
/// corrected if the corners came in backwards).
fn push_quad(mesh: &mut Mesh, corners: [Vec3; 4], outward: Vec3) {
    let [p0, p1, p2, p3] = corners;
    let face_normal = (p1 - p0).cross(p2 - p0);
    let (order, normal): ([usize; 4], Vec3) = if face_normal.dot(outward) >= 0.0 {
        ([0, 1, 2, 3], outward)
    } else {
        ([0, 2, 1, 3], -outward)
    };
    let base = mesh.positions.len() as u32;
    for i in order {
        mesh.positions.push(corners[i]);
        mesh.normals.push(normal);
    }
    mesh.triangles.push([base, base + 1, base + 2]);
    mesh.tri_face.push(0);
    mesh.triangles.push([base, base + 2, base + 3]);
    mesh.tri_face.push(0);
}

/// A closed box from `min` to `max`.
pub fn box_mesh(min: [f64; 3], max: [f64; 3]) -> Result<Mesh> {
    if !finite(&min) || !finite(&max) || !(max[0] > min[0] && max[1] > min[1] && max[2] > min[2]) {
        return Err(invalid("a stock box needs finite, ordered corners"));
    }
    let v = |x: u8, y: u8, z: u8| Vec3::new(if x == 0 { min[0] } else { max[0] }, if y == 0 { min[1] } else { max[1] }, if z == 0 { min[2] } else { max[2] });
    let mut mesh = Mesh::default();
    push_quad(&mut mesh, [v(0, 0, 0), v(1, 0, 0), v(1, 1, 0), v(0, 1, 0)], Vec3::new(0.0, 0.0, -1.0));
    push_quad(&mut mesh, [v(0, 0, 1), v(1, 0, 1), v(1, 1, 1), v(0, 1, 1)], Vec3::Z);
    push_quad(&mut mesh, [v(0, 0, 0), v(0, 0, 1), v(0, 1, 1), v(0, 1, 0)], Vec3::new(-1.0, 0.0, 0.0));
    push_quad(&mut mesh, [v(1, 0, 0), v(1, 0, 1), v(1, 1, 1), v(1, 1, 0)], Vec3::X);
    push_quad(&mut mesh, [v(0, 0, 0), v(1, 0, 0), v(1, 0, 1), v(0, 0, 1)], Vec3::new(0.0, -1.0, 0.0));
    push_quad(&mut mesh, [v(0, 1, 0), v(1, 1, 0), v(1, 1, 1), v(0, 1, 1)], Vec3::Y);
    Ok(mesh)
}

/// A closed cylinder on the axis through `centre` (XY) from `z0` up to `z1`.
pub fn cylinder_mesh(centre: [f64; 2], z0: f64, z1: f64, radius: f64, segments: usize) -> Result<Mesh> {
    if !centre[0].is_finite() || !centre[1].is_finite() || !z0.is_finite() || !z1.is_finite() || !radius.is_finite() {
        return Err(invalid("a stock cylinder needs finite numbers"));
    }
    if !(z1 > z0) || !(radius > 0.0) {
        return Err(invalid("a stock cylinder needs a positive height and radius"));
    }
    let segments = segments.clamp(3, MAX_STOCK_SEGMENTS);
    let ring = |z: f64, k: usize| {
        let a = std::f64::consts::TAU * (k as f64) / (segments as f64);
        Vec3::new(centre[0] + radius * a.cos(), centre[1] + radius * a.sin(), z)
    };
    let radial = |k: usize| {
        let a = std::f64::consts::TAU * ((k as f64) + 0.5) / (segments as f64);
        Vec3::new(a.cos(), a.sin(), 0.0)
    };
    let mut mesh = Mesh::default();
    for k in 0..segments {
        let k2 = (k + 1) % segments;
        push_quad(&mut mesh, [ring(z0, k), ring(z0, k2), ring(z1, k2), ring(z1, k)], radial(k));
        // Caps as triangles from the centre.
        let (c0, c1) = (Vec3::new(centre[0], centre[1], z0), Vec3::new(centre[0], centre[1], z1));
        push_quad(&mut mesh, [c0, ring(z0, k), ring(z0, k2), c0], Vec3::new(0.0, 0.0, -1.0));
        push_quad(&mut mesh, [c1, ring(z1, k), ring(z1, k2), c1], Vec3::Z);
    }
    Ok(mesh)
}

/// The mesh of an operation's stock, or `None` for a body stock (whose mesh is the
/// body's own).
pub fn stock_mesh(stock: &Stock) -> Result<Option<Mesh>> {
    let Some((min, max)) = stock.extent() else { return Ok(None) };
    let mesh = match &stock.kind {
        StockKind::Box { .. } => box_mesh(min, max)?,
        StockKind::Cylinder { radius, .. } => cylinder_mesh([(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0], min[2], max[2], *radius, DEFAULT_STOCK_SEGMENTS)?,
        StockKind::Body { .. } => return Ok(None),
    };
    Ok(Some(mesh))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Allowance;

    #[test]
    fn box_mesh_is_closed_and_right() {
        let mesh = box_mesh([-5.0, -10.0, 0.0], [5.0, 10.0, 30.0]).unwrap();
        assert_eq!(mesh.triangles.len(), 12);
        let props = mesh.mass_properties().expect("a closed box measures");
        assert!((props.volume - 10.0 * 20.0 * 30.0).abs() < 1e-6, "volume {}", props.volume);
    }

    #[test]
    fn cylinder_mesh_is_closed_and_right() {
        let mesh = cylinder_mesh([0.0, 0.0], 0.0, 10.0, 5.0, 64).unwrap();
        let props = mesh.mass_properties().expect("a closed cylinder measures");
        let exact = std::f64::consts::PI * 25.0 * 10.0;
        assert!((props.volume - exact).abs() / exact < 0.001, "volume {} vs {}", props.volume, exact);
    }

    #[test]
    fn hostile_meshes_are_refused() {
        assert!(box_mesh([0.0, 0.0, 0.0], [1.0, 1.0, f64::NAN]).is_err());
        assert!(box_mesh([0.0, 0.0, 1.0], [1.0, 1.0, 0.0]).is_err());
        assert!(cylinder_mesh([0.0, 0.0], 0.0, 1.0, -1.0, 16).is_err());
        assert!(cylinder_mesh([0.0, 0.0], 1.0, 0.0, 1.0, 16).is_err());
    }

    #[test]
    fn stock_mesh_follows_the_kind() {
        let boxy = Stock { kind: StockKind::Box { x: 10.0, y: 10.0, z: 10.0 }, origin: [0.0; 3], allowance: Allowance::default() };
        assert!(stock_mesh(&boxy).unwrap().is_some());
        let round = Stock { kind: StockKind::Cylinder { radius: 5.0, height: 10.0 }, origin: [0.0; 3], allowance: Allowance::default() };
        assert!(stock_mesh(&round).unwrap().is_some());
        let body = Stock { kind: StockKind::Body { body: "Blank".into() }, origin: [0.0; 3], allowance: Allowance::default() };
        assert!(stock_mesh(&body).unwrap().is_none());
    }
}
