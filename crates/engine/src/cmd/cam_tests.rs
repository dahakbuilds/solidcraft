use super::*;
use crate::Session;
use serde_json::json;

fn flat_params() -> Value {
    json!({"cutting_diameter": 10.0, "flute_length": 20.0, "overall_length": 70.0, "shank_diameter": 10.0})
}

/// A scratch G-code file with the given text.
fn gcode_file(text: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = solidcraft_io::vfs::temp_dir()
        .join(format!("solidcraft-cam-{}-{}.nc", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)))
        .to_string_lossy()
        .into_owned();
    solidcraft_io::vfs::write(&path, text.as_bytes()).unwrap();
    path
}

#[test]
fn cam_simulate_reports_a_gouge_below_the_part() {
    let mut s = Session::default();
    let out = s
        .execute(
            "cam.simulate",
            &json!({
                "family": "end_mill_flat",
                "params": flat_params(),
                "path": [[5, 20, 12], [5, 20, 4], [35, 20, 4]],
                "stock": {"min": [0, 0, 0], "max": [40, 40, 10]},
                "part_top": 5,
                "cell": 0.5,
            }),
        )
        .unwrap();
    assert!(out["gouges"].as_u64().unwrap() > 0, "{out}");
    assert!((out["gouge_worst_mm"].as_f64().unwrap() - 1.0).abs() < 1e-6);
    assert!(out["removed_mm3"].as_f64().unwrap() > 0.0);
    assert_eq!(out["collisions"].as_u64().unwrap(), 0);
    assert!(!out["gouge_points"].as_array().unwrap().is_empty());
}

#[test]
fn cam_simulate_passes_a_clean_pocket() {
    let mut s = Session::default();
    let out = s
        .execute(
            "cam.simulate",
            &json!({
                "family": "end_mill_flat",
                "params": flat_params(),
                "path": [[5, 20, 12], [5, 20, 5], [35, 20, 5]],
                "stock": {"min": [0, 0, 0], "max": [40, 40, 10]},
                "part_top": 5,
                "cell": 0.5,
            }),
        )
        .unwrap();
    assert_eq!(out["gouges"], 0, "{out}");
    assert_eq!(out["collisions"], 0);
    assert!(out["removed_mm3"].as_f64().unwrap() > 0.0);
}

#[test]
fn cam_simulate_flags_a_rapid_crash() {
    let mut s = Session::default();
    let out = s
        .execute(
            "cam.simulate",
            &json!({
                "family": "end_mill_flat",
                "params": flat_params(),
                "path": [{ "x": 5, "y": 20, "z": 12 }, { "x": 35, "y": 20, "z": 8, "cut": false }],
                "stock": {"min": [0, 0, 0], "max": [40, 40, 10]},
                "cell": 0.5,
            }),
        )
        .unwrap();
    assert!(out["collisions"].as_u64().unwrap() > 0, "{out}");
    assert_eq!(out["removed_mm3"].as_f64().unwrap(), 0.0);
}

#[test]
fn cam_simulate_refuses_hostile_requests() {
    let mut s = Session::default();
    let stock = json!({"min": [0, 0, 0], "max": [10, 10, 5]});
    // empty path
    assert!(s.execute("cam.simulate", &json!({"family": "end_mill_flat", "params": flat_params(), "path": [], "stock": stock, "cell": 1})).is_err());
    // non-finite coordinate
    assert!(
        s.execute(
            "cam.simulate",
            &json!({"family": "end_mill_flat", "params": flat_params(), "path": [[0, 0, 1], [null, 1, 1]], "stock": stock, "cell": 1})
        )
        .is_err()
    );
    // a grid finer than the cell floor over the footprint
    assert!(
        s.execute("cam.simulate", &json!({"family": "end_mill_flat", "params": flat_params(), "path": [[0, 0, 1], [5, 5, 1]], "stock": {"min": [0, 0, 0], "max": [40, 40, 10]}, "cell": 0.05})).is_err()
    );
    // unknown family and bad params
    assert!(s.execute("cam.simulate", &json!({"family": "nope", "params": {}, "path": [[0, 0, 1], [1, 1, 1]], "stock": stock, "cell": 1})).is_err());
    assert!(
        s.execute(
            "cam.simulate",
            &json!({"family": "end_mill_flat", "params": {"cutting_diameter": -3}, "path": [[0, 0, 1], [1, 1, 1]], "stock": stock, "cell": 1})
        )
        .is_err()
    );
    // inverted stock
    assert!(s.execute("cam.simulate", &json!({"family": "end_mill_flat", "params": flat_params(), "path": [[0, 0, 1], [1, 1, 1]], "stock": {"min": [0, 0, 5], "max": [10, 10, 4]}, "cell": 1})).is_err());
}

#[test]
fn cam_simulate_replays_a_gcode_file() {
    let mut s = Session::default();
    let file = gcode_file("O1001\nG21 G90\nG0 X5 Y20 Z12\nG1 Z4 F200\nG1 X35 Y20\nG0 Z12\nM30\n");
    let out = s
        .execute(
            "cam.simulate",
            &json!({
                "family": "end_mill_flat",
                "params": flat_params(),
                "file": file,
                "stock": {"min": [0, 0, 0], "max": [40, 40, 10]},
                "part_top": 5,
                "cell": 0.5,
            }),
        )
        .unwrap();
    assert!(out["gouges"].as_u64().unwrap() > 0, "{out}");
    assert!((out["gouge_worst_mm"].as_f64().unwrap() - 1.0).abs() < 1e-6);
    assert!(out["removed_mm3"].as_f64().unwrap() > 0.0);
    assert!(out["warnings"].as_array().unwrap().is_empty(), "{out}");
}

#[test]
fn cam_simulate_gcode_keeps_reader_warnings() {
    let mut s = Session::default();
    let file = gcode_file("G1 X1 Y1 Q5\n");
    let out = s
        .execute(
            "cam.simulate",
            &json!({
                "family": "end_mill_flat",
                "params": flat_params(),
                "file": file,
                "stock": {"min": [0, 0, 0], "max": [40, 40, 10]},
                "cell": 0.5,
            }),
        )
        .unwrap();
    assert!(out["warnings"].as_array().unwrap().iter().any(|w| w.as_str().is_some_and(|w| w.contains("`Q`"))), "{out}");
}

#[test]
fn cam_simulate_gcode_refuses_bad_requests() {
    let mut s = Session::default();
    let stock = json!({"min": [0, 0, 0], "max": [10, 10, 5]});
    let file = gcode_file("G1 X1 Y1\n");
    let base = json!({"family": "end_mill_flat", "params": flat_params(), "stock": stock, "cell": 1});
    // both sources at once
    let mut both = base.clone();
    both["path"] = json!([[0, 0, 1]]);
    both["file"] = json!(file);
    assert!(s.execute("cam.simulate", &both).is_err());
    // neither source
    assert!(s.execute("cam.simulate", &base).is_err());
    // unknown format
    let mut cls = base.clone();
    cls["file"] = json!(file);
    cls["format"] = json!("cls");
    assert!(s.execute("cam.simulate", &cls).is_err());
    // a file that is not there
    let mut missing = base.clone();
    missing["file"] = json!("solidcraft-no-such-toolpath.nc");
    assert!(s.execute("cam.simulate", &missing).is_err());
    // arcs are refused with a clear message
    let arcs = gcode_file("G2 X1 Y1 I1 J1\n");
    let mut arc = base.clone();
    arc["file"] = json!(arcs);
    let err = s.execute("cam.simulate", &arc).unwrap_err().to_string();
    assert!(err.contains("not supported yet: arc moves"), "{err}");
}
