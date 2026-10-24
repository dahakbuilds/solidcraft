use super::*;
use crate::Session;
use serde_json::json;

fn flat_params() -> Value {
    json!({"cutting_diameter": 10.0, "flute_length": 20.0, "overall_length": 70.0, "shank_diameter": 10.0})
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
