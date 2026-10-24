use crate::toolpath::{Segment, Toolpath, DEFAULT_TOLERANCE};
use serde_json::json;
use solidcraft_geom::Vec3;

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

#[test]
fn lines_chain_points_and_keep_cut_flags() {
    // cuts[i + 1]: the move into point i + 1.
    let path = Toolpath::from_points(&[p(0.0, 0.0, 0.0), p(10.0, 0.0, 0.0), p(10.0, 5.0, 0.0)], &[false, true, false]).unwrap();
    assert_eq!(path.len(), 2);
    assert!(path.segments[0].cut(), "the move into point 1 feeds");
    assert!(!path.segments[1].cut(), "the move into point 2 is a rapid");
    let lines = path.tessellate(DEFAULT_TOLERANCE).unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].points.len(), 2);
    assert!(!lines[1].cut);
}

#[test]
fn quarter_arc_tessellates_on_its_radius() {
    let arc = Segment::Arc { from: [1.0, 0.0, 0.0], to: [0.0, 1.0, 0.0], center: [0.0; 3], normal: [0.0, 0.0, 1.0], clockwise: false, cut: true };
    let path = Toolpath { segments: vec![arc] };
    path.check().unwrap();
    let lines = path.tessellate(0.01).unwrap();
    assert_eq!(lines.len(), 1);
    let pts = &lines[0].points;
    assert!(pts.len() >= 4, "a quarter arc at 0.01 tolerance needs several chords, got {}", pts.len());
    for pt in pts {
        assert!((pt.len() - 1.0).abs() < 1e-9, "point {pt:?} off the radius");
    }
    assert_eq!(pts.first().unwrap().x, 1.0);
    assert!(pts.last().unwrap().y > 0.99);
    // The midpoint of a counter-clockwise quarter arc sits at 45°.
    let mid = pts[pts.len() / 2];
    assert!(mid.x > 0.0 && mid.y > 0.0);
}

#[test]
fn clockwise_sweeps_the_other_way() {
    let arc = Segment::Arc { from: [1.0, 0.0, 0.0], to: [0.0, 1.0, 0.0], center: [0.0; 3], normal: [0.0, 0.0, 1.0], clockwise: true, cut: true };
    let lines = Toolpath { segments: vec![arc] }.tessellate(0.01).unwrap();
    let pts = &lines[0].points;
    // Clockwise from +X to +Y is the long way (270°): the midpoint sits at -135°.
    let mid = pts[pts.len() / 2];
    assert!(mid.x < 0.0 && mid.y < 0.0, "midpoint {mid:?} is not on the long sweep");
    assert!(pts.len() > 10, "a 270° sweep needs many chords");
}

#[test]
fn coincident_ends_make_a_full_circle() {
    let arc = Segment::Arc { from: [2.0, 0.0, 0.0], to: [2.0, 0.0, 0.0], center: [0.0; 3], normal: [0.0, 0.0, 1.0], clockwise: false, cut: true };
    let lines = Toolpath { segments: vec![arc] }.tessellate(0.05).unwrap();
    let pts = &lines[0].points;
    let gap = pts.first().unwrap().dist(*pts.last().unwrap());
    assert!(gap < 1e-9, "the circle must close, gap {gap}");
}

#[test]
fn hostile_arcs_are_refused() {
    let bad = |from: [f64; 3], to: [f64; 3], center: [f64; 3], normal: [f64; 3]| {
        Toolpath { segments: vec![Segment::Arc { from, to, center, normal, clockwise: false, cut: true }] }.check()
    };
    assert!(bad([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0; 3], [0.0, 0.0, 0.0]).is_err(), "zero normal");
    assert!(bad([1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0; 3], [0.0, 0.0, 1.0]).is_err(), "end off the radius");
    assert!(bad([1.0, 0.0, 1.0], [0.0, 1.0, 0.0], [0.0; 3], [0.0, 0.0, 1.0]).is_err(), "start off the plane");
    assert!(bad([f64::NAN, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0; 3], [0.0, 0.0, 1.0]).is_err(), "NaN");
    assert!(Toolpath { segments: vec![Segment::Line { from: [1.0, 0.0, 0.0], to: [1.0, 0.0, 0.0], cut: true }] }.check().is_err(), "empty line");
}

#[test]
fn segments_round_trip_through_json() {
    let path = Toolpath {
        segments: vec![
            Segment::Line { from: [0.0, 0.0, 1.0], to: [5.0, 0.0, 1.0], cut: false },
            Segment::Arc { from: [5.0, 0.0, 1.0], to: [5.0, 3.0, 1.0], center: [5.0, 1.5, 1.0], normal: [0.0, 0.0, 1.0], clockwise: false, cut: true },
        ],
    };
    let value = serde_json::to_value(&path).unwrap();
    assert_eq!(value["segments"][0]["type"], "line");
    assert_eq!(value["segments"][1]["type"], "arc");
    let back: Toolpath = serde_json::from_value(value).unwrap();
    assert_eq!(path, back);
    // `cut` may be left out: a move feeds unless said otherwise.
    let bare: Segment = serde_json::from_value(json!({"type": "line", "from": [0.0, 0.0, 0.0], "to": [1.0, 0.0, 0.0]})).unwrap();
    assert!(bare.cut());
}

#[test]
fn bounds_cover_the_path() {
    let path = Toolpath {
        segments: vec![
            Segment::Line { from: [-2.0, 0.0, 1.0], to: [0.0, 0.0, 1.0], cut: true },
            Segment::Arc { from: [0.0, 0.0, 1.0], to: [0.0, 2.0, 1.0], center: [0.0, 1.0, 1.0], normal: [0.0, 0.0, 1.0], clockwise: false, cut: true },
        ],
    };
    // The CCW arc bulges to +X through (1, 1, 1).
    let (min, max) = path.bounds().unwrap();
    assert!((min[0] + 2.0).abs() < 1e-6, "min x {min:?}");
    assert!(min[1].abs() < 1e-6 && (min[2] - 1.0).abs() < 1e-6, "min {min:?}");
    assert!((max[0] - 1.0).abs() < 0.01, "the arc bulges to x=1: {max:?}");
    assert!((max[1] - 2.0).abs() < 1e-6, "max y {max:?}");
    assert!(Toolpath::default().bounds().is_none());
}
