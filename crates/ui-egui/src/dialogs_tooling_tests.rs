use serde_json::json;

use crate::scenario::Harness;

fn temp_library(name: &str) -> String {
    std::env::temp_dir().join(format!("solidcraft-tools-ui-{name}-{}.json", std::process::id())).to_string_lossy().into_owned()
}

/// Open the window on a fresh library file (the default path would write to the user folder).
fn open_on(h: &mut Harness, path: &str) {
    let mut d = super::ToolingPanel { families: super::families(&mut h.app), path: path.to_string(), ..Default::default() };
    assert!(!d.families.is_empty(), "the family schema loads");
    d.reload(&mut h.app);
    assert!(d.error.is_none(), "the library opens: {:?}", d.error);
    h.app.tooling = Some(d);
    h.frames(2);
}

#[test]
fn the_toolbar_command_opens_the_window() {
    let mut h = Harness::new();
    h.step(&json!({"start": "tool.library.open"})).unwrap();
    assert!(h.app.tooling.is_some(), "ui.start opens the Tool Library window");
    h.frames(2);
    assert!(h.widget("Add").is_some(), "the empty library still offers the form");
}

#[test]
fn the_window_adds_edits_and_deletes_a_tool() {
    let mut h = Harness::new();
    let path = temp_library("roundtrip");
    open_on(&mut h, &path);
    {
        let d = h.app.tooling.as_mut().unwrap();
        d.start_add();
        d.form.label = "10 mm flat".into();
    }
    h.frames(2);
    h.step(&json!({"widget": "Add tool"})).unwrap();
    h.frames(2);
    assert!(h.widget("10 mm flat").is_some(), "the new tool is in the list");
    {
        let d = h.app.tooling.as_ref().unwrap();
        assert!(
            d.selected.is_some(),
            "the new tool stays picked; entries: {:?}",
            d.entries().iter().map(|e| e["id"].as_str().unwrap_or("?").to_string()).collect::<Vec<_>>()
        );
        assert!(d.preview.is_some(), "the preview was drawn");
        assert!(d.entries().iter().any(|e| e["label"] == "10 mm flat"));
    }
    {
        let d = h.app.tooling.as_mut().unwrap();
        d.form.label = "10 mm flat short".into();
    }
    h.frames(2);
    h.step(&json!({"widget": "Apply"})).unwrap();
    h.frames(2);
    assert!(h.widget("10 mm flat short").is_some());
    {
        // absence is checked on the panel state: the accesskit diff can keep a stale node
        // for a label that just changed, which is a harness detail, not a product bug.
        let d = h.app.tooling.as_ref().unwrap();
        let labels: Vec<String> = d.entries().iter().map(|e| e["label"].as_str().unwrap_or_default().to_string()).collect();
        assert_eq!(labels, vec!["10 mm flat short".to_string()], "the old label is gone");
    }
    let del_pos = h.widget("Delete tool");
    let apply_pos = h.widget("Apply");
    let add_pos = h.widget("Add");
    h.step(&json!({"widget": "Delete tool"})).unwrap();
    h.frames(2);
    {
        let d = h.app.tooling.as_ref().unwrap();
        assert!(
            d.entries().is_empty(),
            "the tool is gone; error {:?}, selected {:?}, ids {:?}, del {:?}, apply {:?}, add {:?}",
            d.error,
            d.selected,
            d.entries().iter().map(|e| e["id"].as_str().unwrap_or("?").to_string()).collect::<Vec<_>>(),
            del_pos,
            apply_pos,
            add_pos
        );
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn hostile_parameters_are_refused_in_the_form() {
    let mut h = Harness::new();
    let path = temp_library("hostile");
    open_on(&mut h, &path);
    {
        let d = h.app.tooling.as_mut().unwrap();
        d.start_add();
        d.form.label = "bad".into();
        d.form.params[0] = "not a number".into();
    }
    h.frames(2);
    h.step(&json!({"widget": "Add tool"})).unwrap();
    h.frames(2);
    let d = h.app.tooling.as_ref().unwrap();
    assert!(d.error.is_some(), "the form reports the bad number");
    assert!(d.entries().is_empty(), "nothing got in");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_search_box_filters_the_list() {
    let mut h = Harness::new();
    let path = temp_library("search");
    open_on(&mut h, &path);
    for label in ["6 mm ball", "8 mm drill"] {
        h.app
            .session
            .execute(
                "tool.library.add",
                &json!({"path": path, "family": "end_mill_ball", "label": label,
                        "params": {"cutting_diameter": 6.0, "flute_length": 15.0, "overall_length": 50.0, "shank_diameter": 6.0}}),
            )
            .unwrap();
    }
    open_on(&mut h, &path);
    h.app.tooling.as_mut().unwrap().search = "drill".into();
    h.frames(2);
    assert!(h.widget("8 mm drill").is_some());
    let shown: Vec<String> = h.app.tooling.as_ref().unwrap().filtered().iter().map(|e| e["label"].as_str().unwrap_or_default().to_string()).collect();
    assert_eq!(shown, vec!["8 mm drill".to_string()], "the other tool is filtered out");
    let _ = std::fs::remove_file(&path);
}
