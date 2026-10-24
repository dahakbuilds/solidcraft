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

// ---- the CAM setup: operations, stock, toolpaths, fixtures, origin ----

#[test]
fn operations_round_trip_and_undo() {
    let mut s = Session::default();
    let out = s.execute("cam.operation.add", &json!({"name": "Pocket 6mm", "comment": "first cut"})).unwrap();
    let id = out["id"].as_u64().unwrap();
    assert_eq!(id, 1);
    let setup = s.execute("cam.setup", &json!({})).unwrap();
    assert_eq!(setup["operations"].as_array().unwrap().len(), 1);
    assert_eq!(setup["operations"][0]["name"], "Pocket 6mm");
    // A second operation gets a fresh id; renaming sticks.
    let second = s.execute("cam.operation.add", &json!({})).unwrap();
    let id2 = second["id"].as_u64().unwrap();
    assert_ne!(id, id2);
    assert_eq!(second["name"], "Operation 2");
    s.execute("cam.operation.rename", &json!({"id": id2, "name": "Drill tops"})).unwrap();
    let setup = s.execute("cam.setup", &json!({})).unwrap();
    assert_eq!(setup["operations"][1]["name"], "Drill tops");
    // Undo removes the rename, then the second operation.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    let setup = s.execute("cam.setup", &json!({})).unwrap();
    assert_eq!(setup["operations"].as_array().unwrap().len(), 1);
    // Removing the last operation leaves an empty setup.
    s.execute("cam.operation.remove", &json!({"id": id})).unwrap();
    let setup = s.execute("cam.setup", &json!({})).unwrap();
    assert!(setup["operations"].as_array().unwrap().is_empty());
}

#[test]
fn stock_kinds_and_allowances() {
    let mut s = Session::default();
    let id = s.execute("cam.operation.add", &json!({"name": "op"})).unwrap()["id"].as_u64().unwrap();
    let out = s
        .execute(
            "cam.stock.set",
            &json!({"id": id, "kind": "box", "x": 100.0, "y": 80.0, "z": 20.0, "origin": [10.0, 0.0, 0.0], "allowance": {"plus_z": 1.0}}),
        )
        .unwrap();
    assert_eq!(out["stock"]["kind"], "box");
    assert_eq!(out["stock"]["allowance"]["plus_z"], 1.0);
    // A cylinder and a body stock are both accepted; a bad size is not.
    s.execute("cam.stock.set", &json!({"id": id, "kind": "cylinder", "radius": 25.0, "height": 40.0})).unwrap();
    s.execute("cam.stock.set", &json!({"id": id, "kind": "body", "body": "Blank"})).unwrap();
    assert!(s.execute("cam.stock.set", &json!({"id": id, "kind": "box", "x": -5.0, "y": 1.0, "z": 1.0})).is_err());
    assert!(s.execute("cam.stock.set", &json!({"id": id, "kind": "box", "x": f64::NAN, "y": 1.0, "z": 1.0})).is_err());
    assert!(s.execute("cam.stock.set", &json!({"id": 999, "kind": "box", "x": 1.0, "y": 1.0, "z": 1.0})).is_err());
}

#[test]
fn toolpath_segments_are_checked() {
    let mut s = Session::default();
    let id = s.execute("cam.operation.add", &json!({"name": "op"})).unwrap()["id"].as_u64().unwrap();
    let out = s
        .execute(
            "cam.toolpath.set",
            &json!({"id": id, "segments": [
                {"type": "line", "from": [0.0, 0.0, 5.0], "to": [10.0, 0.0, 5.0], "cut": false},
                {"type": "arc", "from": [10.0, 0.0, 5.0], "to": [10.0, 4.0, 5.0], "center": [10.0, 2.0, 5.0], "normal": [0.0, 0.0, 1.0]},
            ]}),
        )
        .unwrap();
    assert_eq!(out["segments"], 2);
    let setup = s.execute("cam.setup", &json!({})).unwrap();
    assert_eq!(setup["operations"][0]["toolpath"]["segments"][1]["type"], "arc");
    // An arc whose end is off the radius is refused; so is an empty line.
    assert!(
        s.execute(
            "cam.toolpath.set",
            &json!({"id": id, "segments": [{"type": "arc", "from": [1.0, 0.0, 0.0], "to": [2.0, 0.0, 0.0], "center": [0.0, 0.0, 0.0], "normal": [0.0, 0.0, 1.0]}]}),
        )
        .is_err()
    );
    assert!(s.execute("cam.toolpath.set", &json!({"id": id, "segments": [{"type": "line", "from": [0.0, 0.0, 0.0], "to": [0.0, 0.0, 0.0]}]})).is_err());
    s.execute("cam.toolpath.remove", &json!({"id": id})).unwrap();
    assert!(s.execute("cam.toolpath.remove", &json!({"id": id})).is_err(), "removing twice fails");
}

#[test]
fn gcode_load_sets_up_the_operation() {
    let mut s = Session::default();
    let file = gcode_file("O1001\nG21 G90\nT2 M6\nG0 X0 Y0 Z5\nG1 Z-2 F150\nG1 X40 Y0\nG1 X40 Y20\nG0 Z5\nM30\n");
    let out = s.execute("cam.toolpath.load", &json!({"file": file, "allowance": 2.0})).unwrap();
    let id = out["id"].as_u64().unwrap();
    assert_eq!(out["tools"].as_array().unwrap().len(), 1);
    assert_eq!(out["warnings"].as_array().unwrap().len(), 0);
    let setup = s.execute("cam.setup", &json!({})).unwrap();
    let op = &setup["operations"][0];
    assert_eq!(op["id"].as_u64().unwrap(), id);
    assert_eq!(op["tool"]["number"], 2, "the T2 M6 change is detected");
    assert_eq!(op["tool"]["name"], "T2");
    // Stock: 40 x 20 footprint from the moves, 2 mm of margin on the sides.
    let stock = &op["stock"];
    assert_eq!(stock["kind"], "box");
    assert!((stock["x"].as_f64().unwrap() - 40.0).abs() < 1e-9);
    assert!((stock["allowance"]["plus_x"].as_f64().unwrap() - 2.0).abs() < 1e-9);
    // Loading onto the same operation replaces the path and keeps the name.
    let again = s.execute("cam.toolpath.load", &json!({"file": file, "id": id})).unwrap();
    assert_eq!(again["id"], json!(id));
    let setup = s.execute("cam.setup", &json!({})).unwrap();
    assert_eq!(setup["operations"].as_array().unwrap().len(), 1);
}

#[test]
fn gcode_load_refuses_bad_input() {    let mut s = Session::default();
    // a file that is not there
    assert!(s.execute("cam.toolpath.load", &json!({"file": "solidcraft-no-such-toolpath.nc"})).is_err());
    let empty = gcode_file("; nothing but comments\n");
    assert!(s.execute("cam.toolpath.load", &json!({"file": empty})).is_err(), "no moves");
    let arcs = gcode_file("G2 X1 Y1 I1 J1\n");
    assert!(s.execute("cam.toolpath.load", &json!({"file": arcs})).is_err(), "arcs are refused for now");
    assert!(s.execute("cam.toolpath.load", &json!({"file": empty, "format": "cls"})).is_err(), "unknown format");
    assert!(s.execute("cam.toolpath.load", &json!({"file": empty, "allowance": -1})).is_err());
    assert!(s.execute("cam.toolpath.load", &json!({"file": empty, "id": 999})).is_err(), "unknown operation");
}

#[test]
fn cam_machine_turns_gcode_into_a_body() {
    let mut s = Session::default();
    let file = gcode_file("O1\nG21 G90\nT1 M6\nG0 X2 Y10 Z3\nG1 Z1 F100\nG1 X28 Y10\nG0 Z3\nM30\n");
    let out = s
        .execute(
            "cam.machine",
            &json!({
                "family": "end_mill_flat",
                "params": flat_params(),
                "file": file,
                "stock": {"min": [0.0, 0.0, 0.0], "max": [30.0, 20.0, 2.0]},
                "part_top": 1.0,
                "cell": 1.0,
                "body": "Plate",
            }),
        )
        .unwrap();
    let body = out["body"].as_str().unwrap_or_default().to_string();
    assert!(!body.is_empty(), "a body came back: {out}");
    assert!(out["removed_mm3"].as_f64().unwrap_or(0.0) > 0.0, "material was removed: {out}");
    assert_eq!(out["gouges"], 0, "the cut stops at the part top");
    let names: Vec<String> = s.model.state().bodies.iter().map(|b| b.name.clone()).collect();
    assert!(names.contains(&body), "the body is in the design: {names:?} vs {body}");
    // A ragged Z-map rim may keep it a mesh body; either way the part is in the design.
    assert!(s.model.state().bodies.iter().any(|b| b.name == body));
}

#[test]
fn cam_machine_refuses_bad_requests() {
    let mut s = Session::default();
    // No tool family.
    assert!(s.execute("cam.machine", &json!({"path": [[0, 0, 0]], "stock": {"min": [0, 0, 0], "max": [1, 1, 1]}})).is_err());
    // Stock with no height.
    assert!(
        s.execute(
            "cam.machine",
            &json!({"family": "end_mill_flat", "params": flat_params(), "path": [[0, 0, 0], [1, 0, 0]], "stock": {"min": [0.0, 0.0, 0.0], "max": [10.0, 10.0, 0.0]}})
        )
        .is_err()
    );
    // Unknown extra keys are refused (they reach the flattened simulation request).
    assert!(
        s.execute(
            "cam.machine",
            &json!({"family": "end_mill_flat", "params": flat_params(), "path": [[0, 0, 0], [1, 0, 0]], "stock": {"min": [0.0, 0.0, 0.0], "max": [10.0, 10.0, 2.0]}, "smuggled": 1})
        )
        .is_err()
    );
}

#[test]
fn cam_post_save_list_and_run() {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = solidcraft_io::vfs::temp_dir()
        .join(format!("solidcraft-post-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).unwrap();
    let folder = dir.to_string_lossy().into_owned();
    let mut s = Session::default();
    let out = s
        .execute("cam.post.save", &json!({"folder": folder, "post": {"id": "my_mill", "name": "My Mill", "header": ["(mine)"], "footer": ["M30"]}}))
        .unwrap();
    assert_eq!(out["id"], "my_mill");
    let list = s.execute("cam.post.list", &json!({"folder": folder})).unwrap();
    let posts = list["posts"].as_array().unwrap();
    assert!(posts.iter().any(|p| p["id"] == "my_mill" && p["builtin"] == false), "{list}");
    assert!(posts.iter().any(|p| p["id"] == "iso_mill" && p["builtin"] == true), "{list}");
    let run = s
        .execute(
            "cam.post.run",
            &json!({
                "id": "my_mill", "folder": folder, "feed": 300,
                "segments": [
                    {"type": "line", "from": [0.0, 0.0, 5.0], "to": [0.0, 0.0, 0.0], "cut": false},
                    {"type": "line", "from": [0.0, 0.0, 0.0], "to": [10.0, 0.0, 0.0], "cut": true},
                ],
            }),
        )
        .unwrap();
    let gcode = run["gcode"].as_str().unwrap();
    assert!(gcode.starts_with("(mine)\nG21\nG90\nM3 S8000\n"), "{gcode}");
    assert!(gcode.contains("G1 X10 Y0 Z0 F300"), "{gcode}");
    assert!(gcode.ends_with("M30\n"), "{gcode}");
    // A program can also be written to a file, and run from an operation's toolpath.
    let program = dir.join("program.nc");
    let run = s
        .execute("cam.post.run", &json!({"segments": [{"type": "line", "from": [0.0, 0.0, 0.0], "to": [1.0, 0.0, 0.0]}], "out": program.to_string_lossy()}))
        .unwrap();
    assert_eq!(run["out"].as_str().unwrap(), program.to_string_lossy());
    assert!(std::fs::read_to_string(&program).unwrap().contains("G1 X1"));
    // Hostile requests: unknown post, both sources, neither, a bad id, extra keys.
    assert!(s.execute("cam.post.run", &json!({"id": "nope", "segments": []})).is_err());
    assert!(s.execute("cam.post.run", &json!({"operation": 1, "segments": []})).is_err());
    assert!(s.execute("cam.post.run", &json!({})).is_err());
    assert!(s.execute("cam.post.save", &json!({"folder": folder, "post": {"id": "bad id!", "name": "x"}})).is_err());
    assert!(s.execute("cam.post.save", &json!({"folder": folder, "post": {"id": "ok", "name": "x", "smuggled": 1}})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fixtures_and_origin() {
    let mut s = Session::default();
    let file = gcode_file("G1 X1 Y1\n");
    let out = s.execute("cam.fixture.add", &json!({"path": file, "name": "Vise", "origin": [0.0, -40.0, 0.0]})).unwrap();
    let fid = out["id"].as_u64().unwrap();
    s.execute("cam.fixture.place", &json!({"id": fid, "origin": [5.0, -40.0, 0.0]})).unwrap();
    let setup = s.execute("cam.setup", &json!({})).unwrap();
    assert_eq!(setup["fixtures"][0]["origin"][0], 5.0);
    s.execute("cam.fixture.remove", &json!({"id": fid})).unwrap();
    assert!(s.execute("cam.fixture.remove", &json!({"id": fid})).is_err());

    s.execute("cam.origin.set", &json!({"position": [10.0, 20.0, 0.0], "z_axis": [0.0, 1.0, 0.0], "x_axis": [1.0, 0.0, 0.0]})).unwrap();
    let setup = s.execute("cam.setup", &json!({})).unwrap();
    assert_eq!(setup["origin"]["position"][0], 10.0);
    assert_eq!(setup["origin"]["z_axis"][1], 1.0);
    // Non-unit and non-perpendicular axes are refused.
    assert!(s.execute("cam.origin.set", &json!({"x_axis": [2.0, 0.0, 0.0]})).is_err());
    assert!(s.execute("cam.origin.set", &json!({"x_axis": [0.0, 1.0, 0.0]})).is_err(), "parallel to the current z_axis");
    assert!(s.execute("cam.origin.set", &json!({})).is_err(), "give something");
}

#[test]
fn setup_survives_a_design_round_trip() {
    let mut s = Session::default();
    s.execute("cam.operation.add", &json!({"name": "Pocket"})).unwrap();
    s.execute("cam.stock.set", &json!({"id": 1, "kind": "box", "x": 50.0, "y": 50.0, "z": 10.0})).unwrap();
    let doc = s.doc.clone();
    let text = serde_json::to_string(&*doc).unwrap();
    assert!(text.contains("\"cam\""), "the setup is saved: {text}");
    let back: solidcraft_doc::Document = serde_json::from_str(&text).unwrap();
    assert_eq!(back.cam.operations.len(), 1);
    assert_eq!(back.cam.operations[0].name, "Pocket");
    // A design without CAM saves no `cam` member at all.
    let clean = serde_json::to_string(&solidcraft_doc::Document::new("empty")).unwrap();
    assert!(!clean.contains("\"cam\":"), "an empty setup is not saved");
}
