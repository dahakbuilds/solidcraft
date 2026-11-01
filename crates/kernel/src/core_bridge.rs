//! The solidcore bridge: conversions between solidcore's arena B-rep and truck types.
//!
//! The read side ([`core_to_solid`]) is here: a completed `solidcore::Brep` (a primitive
//! or, later, a boolean result) becomes a truck `Solid` so it can enter [`Body`] while
//! operations are routed to solidcore one by one. The write side ([`solid_to_core`])
//! hands truck bodies to solidcore operations and is under construction.
//!
//! Conversions are geometric, not parametric: a truck face carries its boundary wires
//! in 3D and derives its own trims, so a converted face only needs its surface to
//! *contain* the wires, not to share solidcore's UV parameterisation. Curves that
//! cannot be expressed exactly in truck (B-splines for now) fail with
//! [`KernelError::Failed`], and the routed operation falls back to truck.

use mt::InnerSpace;
use solidcore::curve::curve::Curve as ScCurve;
use solidcore::curve::curve::CurveKind as ScCurveKind;
use solidcore::geom::extent2d::Extent2d;
use solidcore::math::{Point2 as ScPoint2, Point3 as ScPoint3, Vec3 as ScVec3};
use solidcore::surface::surface::Surface as ScSurface;
use solidcore::surface::surface::SurfaceKind;
use solidcore::topology::{Brep, Edge as ScEdge, VertexId};
use solidcore::{Kernel, KernelOptions};
use std::collections::HashMap;
use truck_modeling as mt;

use crate::{Body, KernelError, Result, guard};

/// A fresh solidcore kernel: all tolerances and caches live in this value, so each
/// routed operation can hold its own.
pub(crate) fn core_kernel() -> Kernel {
    Kernel::new(&KernelOptions::default())
}

/// A completed solidcore B-rep as a truck solid (see the module docs).
///
/// # Errors
/// [`KernelError`] if a face, surface or curve is not yet convertible, or the result
/// is not a closed solid.
pub(crate) fn core_to_solid(brep: &Brep, k: &Kernel) -> Result<mt::Solid> {
    guard("solidcore to truck", || {
        // truck compares vertices by instance id, so each solidcore vertex becomes one
        // truck vertex cloned into every edge that uses it.
        let mut vertices: HashMap<VertexId, mt::Vertex> = HashMap::new();
        // ... and likewise truck groups shell edges by instance id, so each solidcore
        // edge becomes one truck edge shared by its faces.
        let mut edge_cache: HashMap<solidcore::topology::EdgeId, Vec<(mt::Edge, (VertexId, VertexId))>> = HashMap::new();
        // Ids minted for the midpoints of split closed edges (below the arena's range).
        let mut synthetic: u32 = u32::MAX;
        let mut faces = if let Some(faces) = sphere_patch_shell(brep)? {
            faces
        } else if let Some(faces) = torus_patch_shell(brep)? {
            faces
        } else if let Some(faces) = cylinder_half_shells(brep, k, &mut vertices, &mut edge_cache, &mut synthetic)? {
            faces
        } else {
            let mut faces: Vec<mt::Face> = Vec::new();
            for (fid, face) in brep.faces().iter() {
                let surface = face.surface.as_ref().ok_or_else(|| invalid(format!("face {fid:?} has no surface")))?;
                let mt_surface = convert_surface(surface, face.uv_domain, k)?;
                let mut wires = Vec::new();
                for lid in &face.loops {
                    let loop_ = brep.loop_(*lid).map_err(|e| invalid(format!("loop: {e}")))?;
                    // One wire per loop, seam edge and all: truck's own primitives carry a
                    // seam edge on a closed surface's face (the edge the face uses twice,
                    // once each way), and its mesher expects it — a face trimmed only by
                    // full circles comes out degenerate. So the loop converts whole.
                    let mut run: Vec<mt::Edge> = Vec::new();
                    let mut run_front: Option<VertexId> = None;
                    let mut run_back: Option<VertexId> = None;
                    for le in &loop_.edges {
                        let edge = brep.edge(le.edge).map_err(|e| invalid(format!("edge: {e}")))?;
                        let mut pieces = convert_edge(brep, le.edge, edge, k, &mut vertices, &mut edge_cache, &mut synthetic)?;
                        if !le.forward {
                            pieces.reverse();
                        }
                        for (piece, (front, back)) in pieces {
                            let (front, back) = if le.forward { (front, back) } else { (back, front) };
                            let piece = if le.forward { piece } else { piece.inverse() };
                            if run_back.is_some() && run_back != Some(front) {
                                return Err(invalid(format!("face {fid:?} loop {lid:?} is not a closed run")));
                            }
                            if run_front.is_none() {
                                run_front = Some(front);
                            }
                            run_back = Some(back);
                            run.push(piece);
                        }
                    }
                    match (run_front, run_back) {
                        (Some(f), Some(b)) if f == b => wires.push(mt::Wire::from(run)),
                        (None, None) => {}
                        _ => return Err(invalid(format!("face {fid:?} loop {lid:?} does not close"))),
                    }
                }
                // new_unchecked: a split closed boundary edge shares its seam vertex with
                // its neighbour beyond truck's simplicity check; the shell and solid checks
                // below still validate the result.
                faces.push(mt::Face::new_unchecked(wires, mt_surface));
            }
            faces
        };
        if faces.is_empty() {
            return Err(invalid("B-rep has no faces"));
        }
        // Which way is out is a geometric question, not a flag convention: solidcore's
        // loop directions follow the material side, and neither the edge flags between
        // faces nor each face's own winding in UV reliably ties that to the surface
        // normal of the truck surface we built (the two kernels' parameter conventions
        // differ face by face). So mesh the shell with every face on its surface-normal
        // side and ask each face directly: a ray from its mesh centroid along its area
        // vector that crosses the shell's other triangles an odd number of times has
        // dived into the solid, and the face must flip.
        let shell: mt::Shell = faces.clone().into();
        if let Some((plan, _)) = mt::Solid::try_new(vec![shell]).ok().and_then(|s| crate::tess::plan(&s, 0.05, None, None).ok()) {
            let mesh = plan.evaluate();
            let flipped = orient_faces(&mut faces, &mesh);
            // Safety net: the probes are local and can all agree on a grazing ray,
            // but the whole shell cannot enclose negative volume. Re-sum the mesh
            // with each face's decided orientation; if the solid comes out inside
            // out, every face flips together.
            let mut volume = 0.0;
            for (t, &fi) in mesh.triangles.iter().zip(&mesh.tri_face) {
                if let (Some(a), Some(b), Some(c)) =
                    (mesh.positions.get(t[0] as usize), mesh.positions.get(t[1] as usize), mesh.positions.get(t[2] as usize))
                {
                    let sign = if flipped.get(fi as usize).copied().unwrap_or(false) { -1.0 } else { 1.0 };
                    volume += sign * (*a).dot((*b).cross(*c)) / 6.0;
                }
            }
            if volume < 0.0 {
                for face in &mut faces {
                    flip_face(face);
                }
            }
        }
        let shell: mt::Shell = faces.into();
        mt::Solid::try_new(vec![shell]).map_err(|e| invalid(format!("solid: {e}")))
    })
}

/// The converted sphere as truck's own six-patch layout, when the B-rep is a whole sphere.
///
/// truck's mesher meshes a single full-wrap sphere face to nothing (the same lesson the
/// cylinder half-shells taught), and truck's record and patch machinery are built around
/// the six-patch sphere. The dimensions come from solidcore; the layout is truck's, so a
/// converted sphere enters the same code paths a truck-native one does. Returns
/// `Ok(None)` for anything that is not exactly a whole sphere.
fn sphere_patch_shell(brep: &Brep) -> Result<Option<Vec<mt::Face>>> {
    if brep.faces().len() != 1 {
        return Ok(None);
    }
    let (_, face) = brep.faces().iter().next().ok_or_else(|| invalid("empty arena"))?;
    if face.loops.len() > 1 {
        return Ok(None);
    }
    let Some(surface) = face.surface.as_ref() else {
        return Ok(None);
    };
    let SurfaceKind::Sphere(sphere) = surface.kind() else {
        return Ok(None);
    };
    let c = sphere.base().position().origin();
    let center = solidcraft_geom::Vec3::new(c.x, c.y, c.z);
    Ok(Some(crate::build::sphere_patch_faces(
        center,
        sphere.radius(),
        [solidcraft_geom::Vec3::X, solidcraft_geom::Vec3::Y, solidcraft_geom::Vec3::Z],
    )?))
}

/// The converted torus as truck's own patch layout, when the B-rep is a whole torus
/// about the Z axis. Like [`sphere_patch_shell`]: dimensions from solidcore, the layout
/// (and its seam structure) from truck's revolve builder, so patch-driven code paths
/// treat a converted torus like a truck-native one.
fn torus_patch_shell(brep: &Brep) -> Result<Option<Vec<mt::Face>>> {
    if brep.faces().len() != 1 {
        return Ok(None);
    }
    let (_, face) = brep.faces().iter().next().ok_or_else(|| invalid("empty arena"))?;
    if face.loops.len() > 1 {
        return Ok(None);
    }
    let Some(surface) = face.surface.as_ref() else {
        return Ok(None);
    };
    let SurfaceKind::Torus(torus) = surface.kind() else {
        return Ok(None);
    };
    let position = torus.base().position();
    let (o, axis) = (position.origin(), position.z_axis());
    if axis.x.abs() > 1e-9 || axis.y.abs() > 1e-9 || (axis.z - 1.0).abs() > 1e-9 {
        return Ok(None);
    }
    let center = solidcraft_geom::Vec3::new(o.x, o.y, o.z);
    let plane = solidcraft_geom::Plane { origin: center, ..solidcraft_geom::Plane::XZ };
    let region = solidcraft_geom::Region2 {
        outer: solidcraft_geom::Loop2::circle(solidcraft_geom::Vec2::new(torus.major_radius(), 0.0), torus.minor_radius()),
        holes: vec![],
    };
    let mut bodies = crate::build::revolve(&plane, &[region], solidcraft_geom::Vec2::ZERO, solidcraft_geom::Vec2::Y, std::f64::consts::TAU)?;
    let body = bodies.pop().ok_or_else(|| invalid("torus"))?;
    Ok(Some(body.solid.face_iter().cloned().collect()))
}

/// The solidcore cylinder is one full-wrap surface of revolution closed by two caps:
/// truck's mesher drops full-wrap faces and its SSI hangs on them, so a cylinder
/// shell is rebuilt the way truck builds its own — two planar caps and two half-shell
/// sides, each side a ruled NURBS strip between the matching halves of the cap
/// circles. Every edge stays one shared instance, the circles split at the seam and at
/// the half-way point, so the shell closes under truck's pairing. Returns `Ok(None)`
/// for anything that is not exactly this shell, leaving it to the generic conversion.
fn cylinder_half_shells(
    brep: &Brep,
    k: &Kernel,
    vertices: &mut HashMap<VertexId, mt::Vertex>,
    edge_cache: &mut HashMap<solidcore::topology::EdgeId, Vec<(mt::Edge, (VertexId, VertexId))>>,
    synthetic: &mut u32,
) -> Result<Option<Vec<mt::Face>>> {
    if brep.faces().len() != 3 {
        return Ok(None);
    }
    let mut side: Option<&solidcore::topology::Face> = None;
    let mut caps: Vec<&solidcore::topology::Face> = Vec::new();
    for (_, face) in brep.faces().iter() {
        match face.surface.as_ref().map(|s| s.kind()) {
            Some(SurfaceKind::Cone(cone)) if cone.is_cylinder() && side.is_none() => side = Some(face),
            Some(SurfaceKind::Plane(_)) => caps.push(face),
            _ => return Ok(None),
        }
    }
    let Some(side) = side else {
        return Ok(None);
    };
    if caps.len() != 2 {
        return Ok(None);
    }
    // Each cap is one loop of one closed circular edge; the side is one loop of the
    // two circles and the seam line, the line used twice once each way.
    let mut cap_edges: Vec<solidcore::topology::EdgeId> = Vec::new();
    for cap in &caps {
        let [lid] = cap.loops.as_slice() else {
            return Ok(None);
        };
        let lp = brep.loop_(*lid).map_err(|e| invalid(format!("loop: {e}")))?;
        let [le] = lp.edges.as_slice() else {
            return Ok(None);
        };
        let edge = brep.edge(le.edge).map_err(|e| invalid(format!("edge: {e}")))?;
        if edge.vertices[0] != edge.vertices[1] || !matches!(edge.curve.as_ref().map(|c| c.kind()), Some(ScCurveKind::Ellipse(el)) if el.is_circle())
        {
            return Ok(None);
        }
        cap_edges.push(le.edge);
    }
    let [side_lid] = side.loops.as_slice() else {
        return Ok(None);
    };
    let side_lp = brep.loop_(*side_lid).map_err(|e| invalid(format!("loop: {e}")))?;
    let seams: Vec<_> = side_lp.edges.iter().map(|le| le.edge).filter(|id| !cap_edges.contains(id)).collect();
    let [seam_id, seam_again] = seams.as_slice() else {
        return Ok(None);
    };
    if seam_again != seam_id {
        return Ok(None);
    }
    let seam = brep.edge(*seam_id).map_err(|e| invalid(format!("edge: {e}")))?;
    if seam.vertices[0] == seam.vertices[1] {
        return Ok(None);
    }
    // The cap whose circle starts at the seam's front vertex is the "bottom" the wires
    // below are wound around.
    let bottom_first = brep.edge(cap_edges[0]).map_err(|e| invalid(format!("edge: {e}")))?.vertices[0] == seam.vertices[0];
    let (bot_id, top_id) = if bottom_first { (cap_edges[0], cap_edges[1]) } else { (cap_edges[1], cap_edges[0]) };
    let seam_pieces = convert_edge(brep, *seam_id, seam, k, vertices, edge_cache, synthetic)?;
    let bot_pieces = convert_edge(brep, bot_id, brep.edge(bot_id).map_err(|e| invalid(format!("edge: {e}")))?, k, vertices, edge_cache, synthetic)?;
    let top_pieces = convert_edge(brep, top_id, brep.edge(top_id).map_err(|e| invalid(format!("edge: {e}")))?, k, vertices, edge_cache, synthetic)?;
    let ([line0], [p1, _], [q1, _]) = (seam_pieces.as_slice(), bot_pieces.as_slice(), top_pieces.as_slice()) else {
        return Ok(None);
    };
    let (line0, _) = line0.clone();
    let (p1, _) = p1.clone();
    let p2 = bot_pieces[1].0.clone();
    let (q1, _) = q1.clone();
    let q2 = top_pieces[1].0.clone();
    // The strip's height: the seam runs base circle to top circle along the axis.
    let h = line0.back().point() - line0.front().point();
    // The ruling the two halves share, at the circles' half-way vertices.
    let ruling = mt::Edge::try_new(p1.back(), q1.back(), mt::Curve::Line(mt::Line(p1.back().point(), q1.back().point())))
        .map_err(|e| invalid(format!("ruling: {e}")))?;
    // The sides are ruled NURBS strips fitted through the converted arcs, not the
    // analytic cylinder solidcore names: truck's boolean special-cases analytic faces,
    // but on the fuzz's tangent-cylinder seeds (seed 169) the analytic pair sends it
    // into a numerical retry loop that runs for many minutes, while the fitted strip
    // refuses the result cleanly in under a second. Fitted-and-refused beats analytic-
    // and-hung; revisit when the boolean's retry ladder learns to give up.
    let strip = |low: &mt::Edge| -> Result<mt::Surface> {
        let mt::Curve::NurbsCurve(arc) = low.oriented_curve() else {
            return Err(unsupported("cylinder side curve"));
        };
        let net: Vec<Vec<mt::Vector4>> =
            arc.control_points().iter().map(|c| vec![*c, mt::Vector4::new(c.x + h.x * c.w, c.y + h.y * c.w, c.z + h.z * c.w, c.w)]).collect();
        let surface = mt::BSplineSurface::try_new((arc.knot_vec().clone(), mt::KnotVec::from(vec![0.0, 0.0, 1.0, 1.0])), net)
            .map_err(|e| invalid(format!("cylinder side: {e}")))?;
        Ok(mt::Surface::NurbsSurface(mt::NurbsSurface::new(surface)))
    };
    let cap_surface = |cap: &solidcore::topology::Face| match cap.surface.as_ref() {
        Some(surface) => convert_surface(surface, cap.uv_domain, k),
        None => Err(invalid("cap without a surface")),
    };
    let (bottom, top) = if bottom_first { (caps[0], caps[1]) } else { (caps[1], caps[0]) };
    let faces = vec![
        mt::Face::new_unchecked(vec![mt::Wire::from(vec![p1.inverse(), p2.inverse()])], cap_surface(bottom)?),
        mt::Face::new_unchecked(vec![mt::Wire::from(vec![p1, ruling.clone(), q1.inverse(), line0.inverse()])], strip(&bot_pieces[0].0)?),
        mt::Face::new_unchecked(vec![mt::Wire::from(vec![p2, line0, q2.inverse(), ruling.inverse()])], strip(&bot_pieces[1].0)?),
        mt::Face::new_unchecked(vec![mt::Wire::from(vec![q1, q2])], cap_surface(top)?),
    ];
    Ok(Some(faces))
}

/// Set each face's orientation by asking the meshed shell which way is out.
/// With every flag true, each face's mesh triangles wind by its surface normal; a
/// ray from a face's area-weighted centroid along its area vector that crosses the
/// other faces' triangles an odd number of times has dived into the solid, so that
/// face's material side is the other way. A flipped face inverts its flag *and*
/// reverses its wires: truck pairs shell edges by the flag-aware direction, so the
/// two together keep every shared edge running opposite ways in its two faces while
/// the face's normal turns outward. This reads nothing from solidcore's loop
/// directions or truck's edge flags — pure geometry, the only convention both
/// kernels agree on.
fn orient_faces(faces: &mut [mt::Face], mesh: &solidcraft_geom::Mesh) -> Vec<bool> {
    use solidcraft_geom::Vec3;
    let n = faces.len();
    let mut flipped = vec![false; n];
    // Per face, its largest triangle: its normal is the local winding direction.
    // (The area-weighted sum of all a face's normals cancels for a full wrap
    // around an axis, so the aggregate is no probe to steer by.)
    let mut probe = vec![None; n];
    for (ti, (t, &fi)) in mesh.triangles.iter().zip(&mesh.tri_face).enumerate() {
        let (Some(a), Some(b), Some(c)) = (mesh.positions.get(t[0] as usize), mesh.positions.get(t[1] as usize), mesh.positions.get(t[2] as usize))
        else {
            continue;
        };
        let (a, b, c) = (*a, *b, *c);
        let nv = (b - a).cross(c - a);
        let w = nv.len();
        let i = fi as usize;
        if let Some(slot) = probe.get_mut(i)
            && slot.map_or(0.0, |(_, _, _, w0)| w0) < w
        {
            *slot = Some((ti, (a + b + c) * (1.0 / 3.0), nv, w));
        }
    }
    for (i, face) in faces.iter_mut().enumerate() {
        let Some((probe_ti, centroid, nv, w)) = probe[i] else { continue };
        let Some(dir) = nv.normalized() else { continue };
        // Start beyond the face's local surface patch: a triangle's centroid sits a
        // sagitta inside the curved surface it chords, and a ray from there crosses the
        // mesh's local fan on its way out, which reads as dipping into the solid. A push
        // of the probe triangle's own scale clears that fan (and the mesh, which lies
        // within the chord tolerance of the surface) while staying inside any solid the
        // face bounds on a true inward winding.
        let origin = centroid + dir * (w.sqrt().max(1e-6));
        // Three near-parallel rays: a single one grazing an edge or vertex would
        // miscount the crossings; the majority of three will not all graze.
        let side = dir.cross(if dir.x.abs() < 0.9 { Vec3::X } else { Vec3::Y }).normalized().unwrap_or(Vec3::Z);
        let dirs = [dir, (dir + side * 0.1).normalized().unwrap_or(dir), (dir - side * 0.1).normalized().unwrap_or(dir)];
        let mut odd = 0;
        for d in dirs {
            let mut hits = 0;
            for (ti, t) in mesh.triangles.iter().enumerate() {
                // Only the probe triangle itself is skipped: a full-wrap face's ray
                // must still cross that same face's far side.
                if ti == probe_ti {
                    continue;
                }
                let (Some(a), Some(b), Some(c)) =
                    (mesh.positions.get(t[0] as usize), mesh.positions.get(t[1] as usize), mesh.positions.get(t[2] as usize))
                else {
                    continue;
                };
                if ray_crosses(origin, d, *a, *b, *c) {
                    hits += 1;
                }
            }
            if hits % 2 == 1 {
                odd += 1;
            }
        }
        if odd >= 2 {
            flip_face(face);
            flipped[i] = true;
        }
    }
    flipped
}

/// Reverse a face: its wires run the other way and its material flag follows.
fn flip_face(face: &mut mt::Face) {
    let wires: Vec<_> = face
        .boundaries()
        .into_iter()
        .map(|mut w| {
            w.invert();
            w
        })
        .collect();
    *face = mt::Face::new_unchecked(wires, face.surface());
    face.invert();
}

/// Whether the ray `o` + `t`·`d` (t > 0) crosses the triangle `a`-`b`-`c`
/// (Möller-Trumbore; a ray parallel to the triangle's plane never crosses).
fn ray_crosses(
    o: solidcraft_geom::Vec3,
    d: solidcraft_geom::Vec3,
    a: solidcraft_geom::Vec3,
    b: solidcraft_geom::Vec3,
    c: solidcraft_geom::Vec3,
) -> bool {
    let (e1, e2) = (b - a, c - a);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return false;
    }
    let f = 1.0 / det;
    let s = o - a;
    let u = f * s.dot(p);
    if !(0.0..=1.0).contains(&u) {
        return false;
    }
    let q = s.cross(e1);
    let v = f * d.dot(q);
    if !(0.0..=1.0).contains(&(u + v)) {
        return false;
    }
    f * e2.dot(q) > 1e-9
}

/// A solidcore primitive B-rep as a [`Body`]: run `make` on a fresh kernel and
/// convert, so a routed primitive is one call. Any error falls out for the caller's
/// fallback to truck.
pub(crate) fn core_primitive(make: impl FnOnce(&mut Kernel) -> std::result::Result<Brep, solidcore::SolidError>) -> Result<Body> {
    let mut k = core_kernel();
    let brep = make(&mut k).map_err(curve_err)?;
    core_to_body(&brep, &k)
}

/// A SolidCraft point as a solidcore point.
pub(crate) fn sc_point(v: solidcraft_geom::Vec3) -> ScPoint3 {
    ScPoint3 { x: v.x, y: v.y, z: v.z }
}

/// A completed solidcore B-rep as a [`Body`] (see [`core_to_solid`]).
pub(crate) fn core_to_body(brep: &Brep, k: &Kernel) -> Result<Body> {
    core_to_solid(brep, k).and_then(Body::new)
}

/// A truck solid as a solidcore B-rep (the write side of the bridge).
///
/// Under construction: solidcore's arena B-rep needs its use records (face, loop,
/// edge and vertex uses, UV trims and regions) wired, not just the geometry records,
/// so this reports failure until the construction path lands.
// The first solidcore-side operation that needs a truck operand (plan M3, booleans).
#[allow(dead_code, reason = "the routing that consumes this lands in the next milestone")]
pub(crate) fn solid_to_core(_solid: &mt::Solid, _k: &mut Kernel) -> Result<Brep> {
    Err(KernelError::Failed("solidcore bridge: truck to solidcore is not implemented yet".into()))
}

/// A solidcore edge as a truck edge, its curve over the edge's used interval, its
/// vertices taken from the shared cache (see [`core_to_solid`]).
fn convert_edge(
    brep: &Brep,
    id: solidcore::topology::EdgeId,
    edge: &ScEdge,
    k: &Kernel,
    vertices: &mut HashMap<VertexId, mt::Vertex>,
    cache: &mut HashMap<solidcore::topology::EdgeId, Vec<(mt::Edge, (VertexId, VertexId))>>,
    synthetic: &mut u32,
) -> Result<Vec<(mt::Edge, (VertexId, VertexId))>> {
    if let Some(existing) = cache.get(&id) {
        return Ok(existing.clone());
    }
    let curve = edge.curve.as_ref().ok_or_else(|| invalid("edge without a curve"))?;
    let [start, end] = edge.vertices;
    let front = shared_vertex(brep, vertices, start)?;
    let back = shared_vertex(brep, vertices, end)?;
    let (t0, t1) = (edge.interval.min(), edge.interval.max());
    let pieces = if front.id() == back.id() {
        // A closed edge (a full circle or seam): both ends are the one vertex. truck
        // wires must stay simple, so split it at the midpoint into two arcs, the way
        // truck's own primitives split their seam circles.
        let tm = (t0 + t1) * 0.5;
        let mid_point = curve.evaluate_point(k, tm).map_err(curve_err)?;
        let mid = mt::Vertex::new(p3(mid_point));
        *synthetic -= 1;
        let mid_id = VertexId::from_index(*synthetic);
        let first = mt::Edge::try_new(&front, &mid, convert_curve(curve, t0, tm, k)?).map_err(|e| invalid(format!("edge: {e}")))?;
        let second = mt::Edge::try_new(&mid, &back, convert_curve(curve, tm, t1, k)?).map_err(|e| invalid(format!("edge: {e}")))?;
        vec![(first, (start, mid_id)), (second, (mid_id, end))]
    } else {
        let only = mt::Edge::try_new(&front, &back, convert_curve(curve, t0, t1, k)?).map_err(|e| invalid(format!("edge: {e}")))?;
        vec![(only, (start, end))]
    };
    cache.insert(id, pieces.clone());
    Ok(pieces)
}

/// The truck vertex for a solidcore vertex id, made once and cloned onward so every
/// edge that uses it shares one truck instance.
fn shared_vertex(brep: &Brep, cache: &mut HashMap<VertexId, mt::Vertex>, id: VertexId) -> Result<mt::Vertex> {
    if let Some(vertex) = cache.get(&id) {
        return Ok(vertex.clone());
    }
    let point = brep.vertex(id).map_err(|e| invalid(format!("edge vertex: {e}")))?.point;
    let vertex = mt::Vertex::new(p3(point));
    cache.insert(id, vertex.clone());
    Ok(vertex)
}

/// A solidcore curve over `[t0, t1]` as a truck curve.
fn convert_curve(curve: &ScCurve, t0: f64, t1: f64, k: &Kernel) -> Result<mt::Curve> {
    match curve.kind() {
        ScCurveKind::Line(_) => {
            let a = curve.evaluate_point(k, t0).map_err(curve_err)?;
            let b = curve.evaluate_point(k, t1).map_err(curve_err)?;
            let (a, b) = (p3(a), p3(b));
            if (b - a).magnitude() < 1e-12 {
                return Err(invalid("degenerate line edge"));
            }
            Ok(mt::Curve::Line(mt::Line(a, b)))
        }
        ScCurveKind::Ellipse(ellipse) => {
            let canon = ellipse.canonical();
            ellipse_arc_to_nurbs(&canon, curve, t0, t1, k)
        }
        kind => Err(unsupported(format!("edge curve {}", kind_name(kind)))),
    }
}

/// The curve's own parameter range for the kinds the bridge can convert, used where a
/// curve is a surface generator rather than an edge (no edge interval to borrow).
/// This is the curve's natural interval — the space `evaluate_point` reads — not the
/// canonical STEP angular domain, which for an ellipse is in degrees and would land
/// every probe outside the curve.
fn curve_domain(curve: &ScCurve) -> Option<(f64, f64)> {
    match curve.kind() {
        ScCurveKind::Line(_) | ScCurveKind::Ellipse(_) => {
            let d = curve.natural_interval();
            Some((d.min(), d.max()))
        }
        _ => None,
    }
}

/// An ellipse (circles included) arc or full turn as an exact rational quadratic
/// NURBS. The angles come from evaluated points rather than the canonical domain, so
/// the NURB-versus-STEP parameter direction (`inside_out`) needs no special case.
fn ellipse_arc_to_nurbs(canon: &solidcore::curve::ellipse::p2::EllipseCanonical, curve: &ScCurve, t0: f64, t1: f64, k: &Kernel) -> Result<mt::Curve> {
    let (rx, ry) = (canon.radius_x, canon.radius_y);
    if !(rx.is_finite() && ry.is_finite()) || rx <= 0.0 || ry <= 0.0 {
        return Err(unsupported("ellipse radii"));
    }
    let origin = p3(canon.position.origin());
    let (x, y) = (v3(canon.position.x_axis()), v3(canon.position.y_axis()));
    let angle_at = |t: f64| -> Result<f64> {
        let p = curve.evaluate_point(k, t).map_err(curve_err)?;
        let d = p3(p) - origin;
        Ok((y.dot(d) / ry).atan2(x.dot(d) / rx))
    };
    let (a0, am) = (angle_at(t0)?, angle_at((t0 + t1) * 0.5)?);
    // The arc runs twice as far as its midpoint: true for the symmetric parameterisations
    // the port produces, and it recovers the direction without knowing `inside_out`.
    let sweep = (2.0 * wrap_pi(am - a0)).clamp(-std::f64::consts::TAU, std::f64::consts::TAU);
    if sweep.abs() < 1e-9 {
        return Err(invalid("degenerate arc"));
    }
    let segments = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let dth = sweep / segments as f64;
    let weight = (dth * 0.5).cos();
    let point_at = |th: f64| origin + x * (rx * th.cos()) + y * (ry * th.sin());
    let deriv_at = |th: f64| x * (-rx * th.sin()) + y * (ry * th.cos());
    let mut points = Vec::with_capacity(2 * segments + 1);
    let mut weights = Vec::with_capacity(2 * segments + 1);
    // Degree 2, clamped: the knot vector is control points + 3 long, so the end knots
    // repeat three times and each interior knot twice.
    let mut knots = vec![0.0, 0.0, 0.0];
    for i in 0..=segments {
        let th = a0 + dth * i as f64;
        points.push(point_at(th));
        weights.push(1.0);
        if i < segments {
            points.push(point_at(th) + deriv_at(th) * (dth * 0.5).tan());
            weights.push(weight);
            if i > 0 {
                knots.push(i as f64 / segments as f64);
                knots.push(i as f64 / segments as f64);
            }
        }
    }
    knots.extend([1.0, 1.0, 1.0]);
    let bsp = mt::BSplineCurve::try_new(mt::KnotVec::from(knots), points).map_err(|e| invalid(format!("arc: {e}")))?;
    let nurbs = mt::NurbsCurve::try_from_bspline_and_weights(bsp, weights).map_err(|e| invalid(format!("arc: {e}")))?;
    Ok(mt::Curve::NurbsCurve(nurbs))
}

/// A solidcore surface as a truck surface, over the face's UV domain.
fn convert_surface(surface: &ScSurface, uv: Extent2d, k: &Kernel) -> Result<mt::Surface> {
    Ok(match surface.kind() {
        SurfaceKind::Plane(plane) => {
            let a = plane.position();
            let (u, v) = (v3(a.x_axis()), v3(a.y_axis()));
            if u.cross(v).magnitude() < 1e-12 {
                return Err(invalid("degenerate plane axes"));
            }
            // Truck planes carry a unit-square range, so scale the axes to the face's own
            // UV rectangle: the face's boundary then lands inside the surface's range,
            // exactly as a truck-native box face does.
            let (umin, umax) = (uv.u_interval().min(), uv.u_interval().max());
            let (vmin, vmax) = (uv.v_interval().min(), uv.v_interval().max());
            let o = p3(a.origin()) + u * umin + v * vmin;
            mt::Surface::Plane(mt::Plane::new(o, o + u * (umax - umin), o + v * (vmax - vmin)))
        }
        SurfaceKind::Cone(cone) => {
            // Cone and cylinder share the kind; the canonical placement puts the origin
            // on the axis at the base circle. The meridian is the line through two
            // surface points at one angle and the linear domain's two ends.
            let canon = cone.canonical();
            let (u0, u1) = (uv.u_interval().min(), uv.u_interval().max());
            let (v0, v1) = (uv.v_interval().min(), uv.v_interval().max());
            let (linear, angle) = if canon.swap_uv { ((u0, u1), (v0, v1)) } else { ((v0, v1), (u0, u1)) };
            let at = |ang: f64, lin: f64| {
                let (u, v) = if canon.swap_uv { (lin, ang) } else { (ang, lin) };
                surface.evaluate_point(k, ScPoint2::new(u, v)).map_err(curve_err).map(p3)
            };
            let (a, b) = (at(angle.0, linear.0)?, at(angle.0, linear.1)?);
            if (b - a).magnitude() < 1e-12 {
                return Err(invalid("degenerate cone meridian"));
            }
            revolved(mt::Curve::Line(mt::Line(a, b)), canon.origin.origin(), canon.origin.z_axis())
        }
        SurfaceKind::Sphere(sphere) => surface_of_revolution(sphere.base(), k)?,
        SurfaceKind::Torus(torus) => surface_of_revolution(torus.base(), k)?,
        SurfaceKind::SurfOfRevolution(revolution) => surface_of_revolution(revolution, k)?,
        kind => return Err(unsupported(format!("surface {}", surface_kind_name(kind)))),
    })
}

/// A solidcore surface of revolution (sphere, torus and plain revolutions all hold
/// one) as a truck `RevolutedCurve` over the generator's own domain.
fn surface_of_revolution(revolution: &solidcore::surface::surf_of_revolution::p1::SurfOfRevolution, k: &Kernel) -> Result<mt::Surface> {
    let generator = revolution.gen_curve().ok_or_else(|| invalid("surface of revolution without a generator"))?;
    let (t0, t1) = curve_domain(generator).ok_or_else(|| unsupported(format!("generator curve {}", kind_name(generator.kind()))))?;
    let curve = convert_curve(generator, t0, t1, k)?;
    let position = revolution.position();
    Ok(revolved(curve, position.origin(), position.z_axis()))
}

fn revolved(curve: mt::Curve, origin: solidcore::math::Point3, axis: solidcore::math::Vec3) -> mt::Surface {
    mt::Surface::RevolutedCurve(mt::Processor::new(mt::RevolutedCurve::by_revolution(curve, p3(origin), v3(axis))))
}

fn kind_name(kind: &ScCurveKind) -> &'static str {
    match kind {
        ScCurveKind::BSpline(_) => "b-spline",
        ScCurveKind::Hermite(_) => "Hermite",
        ScCurveKind::Hyperbola(_) => "hyperbola",
        ScCurveKind::Projected(_) => "projected",
        ScCurveKind::Iso(_) => "iso",
        ScCurveKind::Offset(_) => "offset",
        ScCurveKind::Conic(_) => "conic",
        ScCurveKind::CrvInVolume(_) => "curve in volume",
        _ => "other",
    }
}

fn surface_kind_name(kind: &SurfaceKind) -> &'static str {
    match kind {
        SurfaceKind::BSpline(_) => "b-spline",
        SurfaceKind::CurveBounded(_) => "curve bounded",
        SurfaceKind::Step(_) => "STEP",
        SurfaceKind::OffsetSurface(_) => "offset",
        SurfaceKind::SrfInVolume(_) => "surface in volume",
        SurfaceKind::SurfOfExtrusion(_) => "extrusion",
        _ => "other",
    }
}

/// Wrap an angle difference into (-pi, pi].
fn wrap_pi(a: f64) -> f64 {
    let tau = 2.0 * std::f64::consts::PI;
    let mut x = (a + std::f64::consts::PI) % tau;
    if x < 0.0 {
        x += tau;
    }
    x - std::f64::consts::PI
}

fn p3(p: ScPoint3) -> mt::Point3 {
    mt::Point3::new(p.x, p.y, p.z)
}

fn v3(v: ScVec3) -> mt::Vector3 {
    mt::Vector3::new(v.x, v.y, v.z)
}

fn invalid(detail: impl std::fmt::Display) -> KernelError {
    KernelError::Invalid(format!("solidcore bridge: {detail}"))
}

fn unsupported(detail: impl std::fmt::Display) -> KernelError {
    KernelError::Failed(format!("not supported yet: solidcore bridge: {detail}"))
}

fn curve_err(e: solidcore::SolidError) -> KernelError {
    KernelError::Failed(format!("solidcore bridge: evaluate: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure;
    use solidcore::math::Point3;

    fn kernel() -> Kernel {
        core_kernel()
    }

    fn point(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    /// Every primitive the bridge supports: counts match, and the measured volume and
    /// area agree with the analytic values.
    #[test]
    fn solidcore_primitives_convert_to_measurable_solids() {
        let mut k = kernel();
        let cases: Vec<(Brep, f64, f64)> = vec![
            {
                // 2 x 3 x 4 box: volume 24, area 52.
                let brep = solidcore::primitive::api_primitives::create_box(&mut k, point(0.0, 0.0, 0.0), 2.0, 3.0, 4.0).unwrap();
                (brep, 24.0, 52.0)
            },
            {
                // r 1.5, h 3 cylinder: volume pi r^2 h, area 2 pi r (r + h).
                let brep = solidcore::primitive::api_primitives::create_cylinder(&mut k, point(1.0, 2.0, 3.0), 1.5, 3.0).unwrap();
                (brep, std::f64::consts::PI * 1.5_f64.powi(2) * 3.0, 2.0 * std::f64::consts::PI * 1.5 * (1.5 + 3.0))
            },
            {
                // r 2 sphere: volume 4/3 pi r^3, area 4 pi r^2.
                let brep = solidcore::primitive::api_primitives::create_sphere(&mut k, point(0.0, 0.0, 0.0), 2.0).unwrap();
                (brep, 4.0 / 3.0 * std::f64::consts::PI * 8.0, 4.0 * std::f64::consts::PI * 4.0)
            },
            {
                // R 3, r 1 torus: volume 2 pi^2 R r^2, area 4 pi^2 R r.
                let brep = solidcore::primitive::api_primitives::create_torus(&mut k, point(0.0, 0.0, 0.0), 3.0, 1.0).unwrap();
                (brep, 2.0 * std::f64::consts::PI.powi(2) * 3.0, 4.0 * std::f64::consts::PI.powi(2) * 3.0)
            },
        ];
        for (brep, want_volume, want_area) in cases {
            let body = core_to_body(&brep, &k).unwrap();
            let m = measure(&body).unwrap();
            assert!((m.volume - want_volume).abs() <= want_volume.abs() * 1e-3, "volume {} vs {want_volume}", m.volume);
            assert!((m.area - want_area).abs() <= want_area.abs() * 1e-2, "area {} vs {want_area}", m.area);
        }
    }

    #[test]
    fn a_box_sits_on_its_origin_corner() {
        // The box_solid routing assumes solidcore's origin is the low corner.
        let mut k = kernel();
        let brep = solidcore::primitive::api_primitives::create_box(&mut k, point(1.0, 2.0, 3.0), 2.0, 3.0, 4.0).unwrap();
        let solid = core_to_solid(&brep, &k).unwrap();
        let (mut lo, mut hi) = (solidcraft_geom::Vec3::new(f64::MAX, f64::MAX, f64::MAX), solidcraft_geom::Vec3::new(f64::MIN, f64::MIN, f64::MIN));
        for v in solid.vertex_iter() {
            let p = crate::body::from_p3(v.point());
            lo = lo.min(p);
            hi = hi.max(p);
        }
        assert!((lo - solidcraft_geom::Vec3::new(1.0, 2.0, 3.0)).len() < 1e-9, "{lo:?} {hi:?}");
        assert!((hi - solidcraft_geom::Vec3::new(3.0, 5.0, 7.0)).len() < 1e-9, "{lo:?} {hi:?}");
    }

    #[test]
    fn a_box_converts_with_six_planar_faces() {
        let mut k = kernel();
        let brep = solidcore::primitive::api_primitives::create_box(&mut k, point(0.0, 0.0, 0.0), 1.0, 1.0, 1.0).unwrap();
        let solid = core_to_solid(&brep, &k).unwrap();
        let shells = solid.boundaries();
        assert_eq!(shells.len(), 1);
        assert_eq!(shells[0].face_iter().count(), 6);
        assert!(shells[0].face_iter().all(|f| matches!(f.surface(), mt::Surface::Plane(_))));
        // The unit box measures 1 in volume and 6 in area, and the faces point out:
        // a flipped face would turn the measured volume negative or the mesh inside out.
        let body = Body::new(solid).unwrap();
        let m = measure(&body).unwrap();
        assert!((m.volume - 1.0).abs() < 1e-6);
        assert!((m.area - 6.0).abs() < 1e-6);
    }

    #[test]
    fn a_cone_converts() {
        let mut k = kernel();
        // r 1 base, r 0.5 top, height 3: volume pi h / 3 (R^2 + R r + r^2) = 1.75 pi.
        let brep = solidcore::primitive::api_primitives::create_cone(&mut k, point(0.0, 0.0, 0.0), 1.0, 0.5, 3.0).unwrap();
        let body = core_to_body(&brep, &k).unwrap();
        let m = measure(&body).unwrap();
        let want = 1.75 * std::f64::consts::PI;
        assert!((m.volume - want).abs() <= want * 1e-3, "volume {} vs {want}", m.volume);
    }
}
