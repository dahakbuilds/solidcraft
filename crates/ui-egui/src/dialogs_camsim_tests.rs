use serde_json::json;

use crate::scenario::Harness;

fn flat_entry() -> serde_json::Value {
    json!({
        "family": "end_mill_flat",
        "label": "10 mm flat end mill",
        "params": {"cutting_diameter": 10.0, "flute_length": 20.0, "overall_length": 70.0, "shank_diameter": 10.0},
    })
}

fn open_with_tool(h: &mut Harness) {
    let mut d = super::CamSimPanel::default();
    d.entries = vec![flat_entry()];
    d.tool = 1;
    h.app.camsim = Some(d);
    h.frames(2);
}

#[test]
fn the_toolbar_command_opens_the_window() {
    let mut h = Harness::new();
    h.step(&json!({"start": "cam.simulate"})).unwrap();
    assert!(h.app.camsim.is_some(), "ui.start opens the CAM Simulation window");
    h.frames(2);
    assert!(h.widget("Simulate").is_some(), "the window offers the Simulate button");
}

#[test]
fn a_clean_pocket_reports_no_gouges() {
    let mut h = Harness::new();
    open_with_tool(&mut h);
    h.step(&json!({"widget": "Simulate"})).unwrap();
    h.frames(2);
    let d = h.app.camsim.as_ref().unwrap();
    assert!(d.error.is_none(), "{:?}", d.error);
    let r = d.report.as_ref().expect("a report after the run");
    assert_eq!(r["gouges"], 0, "{r}");
    assert_eq!(r["collisions"], 0);
    assert!(r["removed_mm3"].as_f64().unwrap() > 0.0, "the pocket removed volume");
    assert!(d.mesh.is_some(), "the machined surface is ready to draw");
}

#[test]
fn a_floor_below_the_part_shows_gouges() {
    let mut h = Harness::new();
    open_with_tool(&mut h);
    h.app.camsim.as_mut().unwrap().floor_z = "4".into(); // the part top is 5
    h.step(&json!({"widget": "Simulate"})).unwrap();
    h.frames(2);
    let d = h.app.camsim.as_ref().unwrap();
    let r = d.report.as_ref().expect("a report after the run");
    assert!(r["gouges"].as_u64().unwrap() > 0, "{r}");
    assert!((r["gouge_worst_mm"].as_f64().unwrap() - 1.0).abs() < 1e-6);
}

#[test]
fn a_missing_tool_is_refused_not_run() {
    let mut h = Harness::new();
    h.app.camsim = Some(super::CamSimPanel::default());
    h.frames(2);
    h.step(&json!({"widget": "Simulate"})).unwrap();
    h.frames(2);
    let d = h.app.camsim.as_ref().unwrap();
    assert!(d.report.is_none());
    assert!(d.error.is_some(), "pick a tool first");
}
