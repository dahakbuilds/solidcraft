use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

struct File(String);
impl File {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(
            solidcraft_io::vfs::temp_dir()
                .join(format!("solidcraft-tools-{}-{}.json", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
                .to_string_lossy()
                .into_owned(),
        )
    }
}
impl Drop for File {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn session() -> Session {
    let mut s = Session::default();
    s.execute("solid.box", &json!({"length": 10, "width": 10, "height": 10})).unwrap();
    s.mark_saved();
    s
}
fn flat_params() -> Value {
    json!({"cutting_diameter": 10.0, "flute_length": 20.0, "overall_length": 70.0, "shank_diameter": 10.0})
}
fn create(s: &mut Session, file: &File) {
    s.execute("tool.library.create", &json!({"path": file.0})).unwrap();
}
fn add(s: &mut Session, file: &File, extra: Value) -> Value {
    let mut p = json!({"path": file.0, "family": "end_mill_flat", "label": "10 mm flat", "params": flat_params()});
    if let (Some(map), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
        for (k, v) in extra {
            map.insert(k.clone(), v.clone());
        }
    }
    s.execute("tool.library.add", &p).unwrap()
}

#[test]
fn families_lists_every_milling_family_with_its_schema() {
    let mut s = session();
    let out = s.execute("tool.families", &json!({})).unwrap();
    let list = out["families"].as_array().unwrap();
    assert_eq!(list.len(), 12);
    assert_eq!(list[0]["id"], "end_mill_flat");
    for f in list {
        assert!(!f["params"].as_array().unwrap().is_empty(), "{}", f["id"]);
    }
    assert!(list.iter().any(|f| f["id"] == "end_mill_bull"));
}

#[test]
fn library_round_trip_never_touches_the_design() {
    let file = File::new();
    let mut s = session();
    let doc = s.doc.clone();
    let history = (s.undo.len(), s.redo.len(), s.revision);
    create(&mut s, &file);
    let added = add(&mut s, &file, json!({"vendor": "Testmaker"}));
    let entry = added["entry"].as_str().unwrap().to_string();
    assert!(entry.starts_with("user.tools/cutting-"));
    assert_eq!(added["revision"].as_str().unwrap().len(), 64);
    let seen = s.execute("tool.library.inspect", &json!({"path": file.0})).unwrap();
    assert_eq!(seen["library"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(seen["library"]["entries"][0]["vendor"], "Testmaker");
    // editing changes the content revision, so the id moves and `previous` names the old one
    let edited = s
        .execute("tool.library.edit", &json!({"path": file.0, "entry": entry, "params": {"cutting_diameter": 12.0, "flute_length": 20.0, "overall_length": 70.0, "shank_diameter": 10.0}}))
        .unwrap();
    assert_ne!(edited["entry"], entry);
    assert_eq!(edited["previous"], entry);
    let removed = s.execute("tool.library.remove", &json!({"path": file.0, "entry": edited["entry"]})).unwrap();
    assert_eq!(removed["entries"], 0);
    assert!(s.execute("tool.library.remove", &json!({"path": file.0, "entry": entry})).is_err());
    // the design and history are untouched throughout
    assert!(Arc::ptr_eq(&doc, &s.doc));
    assert_eq!((s.undo.len(), s.redo.len(), s.revision), history);
    assert!(!s.is_dirty());
}

#[test]
fn add_refuses_unknown_families_hostile_params_and_unbuilt_kinds() {
    let file = File::new();
    let mut s = session();
    create(&mut s, &file);
    for (params, why) in [
        (json!({"cutting_diameter": f64::NAN, "flute_length": 20.0, "overall_length": 70.0, "shank_diameter": 10.0}), "NaN"),
        (json!({"cutting_diameter": 10.0, "flute_length": 20.0, "overall_length": 70.0, "shank_diameter": 10.0, "magic": 1.0}), "unknown"),
        (json!({"cutting_diameter": 10.0, "flute_length": 80.0, "overall_length": 70.0, "shank_diameter": 10.0}), "cross"),
    ] {
        let error = s.execute("tool.library.add", &json!({"path": file.0, "family": "end_mill_flat", "label": "x", "params": params})).unwrap_err();
        assert!(
            error.to_string().contains("params") || error.to_string().contains("required") || error.to_string().contains("invalid"),
            "{why}: {error}"
        );
    }
    // omitted parameters take their schema defaults, so a partial set is accepted
    s.execute(
        "tool.library.add",
        &json!({"path": file.0, "family": "end_mill_flat", "label": "partial", "params": {"cutting_diameter": 10.0, "flute_length": 20.0}}),
    )
    .unwrap();
    assert!(
        s.execute("tool.library.add", &json!({"path": file.0, "family": "turn_groover", "label": "x", "params": {}}))
            .unwrap_err()
            .to_string()
            .contains("turn_groover")
    );
    assert!(
        s.execute("tool.library.add", &json!({"path": file.0, "family": "end_mill_flat", "label": "x", "params": flat_params(), "kind": "holder"}))
            .unwrap_err()
            .to_string()
            .contains("not supported yet")
    );
    assert!(
        s.execute("tool.library.add", &json!({"path": file.0, "family": "end_mill_flat", "label": "x", "params": flat_params(), "smuggled": 1}))
            .is_err()
    );
    // only the partial add got in
    assert_eq!(s.execute("tool.library.inspect", &json!({"path": file.0})).unwrap()["library"]["entries"].as_array().unwrap().len(), 1);
}

#[test]
fn profile_command_matches_the_kernel_math() {
    let mut s = session();
    let out = s
        .execute("tool.profile", &json!({"family": "end_mill_ball", "params": {"cutting_diameter": 10.0, "flute_length": 20.0, "overall_length": 50.0, "shank_diameter": 10.0}}))
        .unwrap();
    assert_eq!(out["max_radius"], 5.0);
    assert_eq!(out["cutting_length"], 20.0);
    assert_eq!(out["segments"][0]["type"], "arc");
    assert_eq!(out["segments"][0]["radius"], 5.0);
    // omitted parameters fill from the schema defaults (a 10 mm flat: flute 20, overall 70)
    let partial = s.execute("tool.profile", &json!({"family": "end_mill_flat", "params": {"cutting_diameter": 10.0}})).unwrap();
    assert_eq!(partial["max_radius"], 5.0);
    assert_eq!(partial["cutting_length"], 20.0);
    assert_eq!(partial["overall_length"], 70.0);
}

#[test]
fn inch_parameters_are_stored_in_millimetres() {
    let file = File::new();
    let mut s = session();
    create(&mut s, &file);
    add(
        &mut s,
        &file,
        json!({"unit": "inch", "params": {"cutting_diameter": 0.5, "flute_length": 1.0, "overall_length": 3.0, "shank_diameter": 0.5}}),
    );
    let seen = s.execute("tool.library.inspect", &json!({"path": file.0})).unwrap();
    let params = &seen["library"]["entries"][0]["params"];
    assert!((params["cutting_diameter"].as_f64().unwrap() - 12.7).abs() < 1e-9);
    assert!((params["overall_length"].as_f64().unwrap() - 76.2).abs() < 1e-9);
}

#[test]
fn create_respects_no_clobber_and_missing_files_are_errors() {
    let file = File::new();
    let mut s = session();
    create(&mut s, &file);
    assert!(s.execute("tool.library.create", &json!({"path": file.0})).is_err());
    s.execute("tool.library.create", &json!({"path": file.0, "overwrite": true})).unwrap();
    let missing = File::new();
    assert!(s.execute("tool.library.inspect", &json!({"path": missing.0})).is_err());
    assert!(s.execute("tool.library.add", &json!({"path": missing.0, "family": "end_mill_flat", "label": "x", "params": flat_params()})).is_err());
    assert!(s.execute("tool.library.create", &json!({"path": file.0, "name": " Padded "})).is_err());
}

#[test]
fn identical_entries_are_refused_and_connections_record() {
    let file = File::new();
    let mut s = session();
    create(&mut s, &file);
    add(&mut s, &file, json!({"machine_connection": {"standard": "hsk-a", "size": "63"}, "work_connection": {"standard": "cyl", "size": "10"}}));
    assert!(s.execute("tool.library.add", &json!({"path": file.0, "family": "end_mill_flat", "label": "10 mm flat", "params": flat_params(), "machine_connection": {"standard": "hsk-a", "size": "63"}, "work_connection": {"standard": "cyl", "size": "10"}})).is_err());
    let seen = s.execute("tool.library.inspect", &json!({"path": file.0})).unwrap();
    assert_eq!(seen["library"]["entries"][0]["machine_connection"]["standard"], "hsk-a");
    assert_eq!(seen["library"]["entries"][0]["work_connection"]["size"], "10");
}

#[test]
fn open_creates_a_missing_library_and_reopens_it() {
    let file = File::new();
    let mut s = session();
    let opened = s.execute("tool.library.open", &json!({"path": file.0})).unwrap();
    assert_eq!(opened["path"], file.0);
    assert_eq!(opened["library"]["entries"].as_array().unwrap().len(), 0);
    add(&mut s, &file, json!({"vendor": "Testmaker"}));
    let reopened = s.execute("tool.library.open", &json!({"path": file.0})).unwrap();
    assert_eq!(reopened["library"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(reopened["library"]["entries"][0]["vendor"], "Testmaker");
    // the design is untouched
    assert!(!s.is_dirty());
    assert!(s.execute("tool.library.open", &json!({"path": "not-a-library.txt"})).is_err());
}

#[test]
fn holders_lists_the_catalogue() {
    let mut s = session();
    let out = s.execute("tool.holders", &json!({})).unwrap();
    let list = out["holders"].as_array().unwrap();
    assert!(list.len() >= 12);
    assert!(list.iter().any(|h| h["id"] == "er32_bt40"));
    for h in list {
        assert!(h["bore_min"].as_f64().unwrap() < h["bore_max"].as_f64().unwrap(), "{}", h["id"]);
        assert!(h["nose_length"].as_f64().unwrap() > 0.0);
    }
}

#[test]
fn assembly_composes_tool_and_holder() {
    let mut s = session();
    // er32_bt40: nose 30 mm below the girth, taper 55 mm above; stickout 60 from the tip.
    let out = s
        .execute(
            "tool.assembly",
            &json!({"family": "end_mill_flat", "params": flat_params(), "holder_type": "er32_bt40", "stickout": 60.0, "steps": 12}),
        )
        .unwrap();
    assert_eq!(out["reach"].as_f64().unwrap(), 30.0);
    assert_eq!(out["overall_length"].as_f64().unwrap(), 115.0);
    let mesh = &out["mesh"];
    let tris = mesh["triangles"].as_array().unwrap();
    assert!(!tris.is_empty());
    assert_eq!(tris.len(), mesh["roles"].as_array().unwrap().len());
    let roles = mesh["roles"].as_array().unwrap();
    assert!(roles.iter().any(|r| r == "cutting"));
    assert!(roles.iter().any(|r| r == "holder"));
    // a stickout inside the holder nose is refused
    assert!(
        s.execute("tool.assembly", &json!({"family": "end_mill_flat", "params": flat_params(), "holder_type": "er32_bt40", "stickout": 10.0})).is_err()
    );
    // an unknown holder is refused
    assert!(
        s.execute("tool.assembly", &json!({"family": "end_mill_flat", "params": flat_params(), "holder_type": "nope", "stickout": 60.0})).is_err()
    );
}

#[test]
fn entries_carry_holder_and_stickout() {
    let mut s = session();
    let file = File::new();
    create(&mut s, &file);
    let out = add(&mut s, &file, json!({"holder_type": "er32_bt40", "stickout": 60.0}));
    let id = out["entry"].as_str().unwrap().to_string();
    let seen = s.execute("tool.library.inspect", &json!({"path": file.0})).unwrap();
    assert_eq!(seen["library"]["entries"][0]["holder_type"], "er32_bt40");
    assert_eq!(seen["library"]["entries"][0]["stickout"], 60.0);
    // editing to clear the holder clears the stickout with it
    s.execute("tool.library.edit", &json!({"path": file.0, "entry": id, "holder_type": ""})).unwrap();
    let seen = s.execute("tool.library.inspect", &json!({"path": file.0})).unwrap();
    let entry = &seen["library"]["entries"][0];
    assert!(entry.get("holder_type").is_none());
    assert!(entry.get("stickout").is_none());
    // a stickout without a holder is refused
    assert!(s.execute("tool.library.add", &json!({"path": file.0, "family": "end_mill_flat", "label": "x", "params": flat_params(), "stickout": 60.0})).is_err());
    // a stickout inside the holder nose is refused
    assert!(s.execute("tool.library.add", &json!({"path": file.0, "family": "end_mill_flat", "label": "x", "params": flat_params(), "holder_type": "er32_bt40", "stickout": 5.0})).is_err());
}
