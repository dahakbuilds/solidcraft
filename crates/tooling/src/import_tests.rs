use super::*;

const GTC_SAMPLE: &str = "\
( a GTC/p21 file as a post-processor writes it
UNIT/MM
TOOL,
TOOL_NO/1, , , , , , , , ,
TOOL_TYPE/BALL,
NAME/6 mm ball nose,
SHANK,10.000,10.000,60.000,40.000,0,0,0,0,0,0,
CORNER,3.000,3.000,0.000,0.000,0.000,
CUTDEPTH,15.000,
HOLDER,ER32 BT40,
TOOL,
TOOL_TYPE/END,
NAME/10 mm flat,
CUT_DIAMETER/10.0,
LENGTH/70.0,
SHANK,10.000,10.000,70.000,50.000,
TOOL_TYPE/DRILL,
NAME/8 mm drill 118 deg,
CUT_DIAMETER/8.0,
POINT_ANGLE/118,
";

#[test]
fn gtc_reads_the_common_records() {
    let report = parse_gtc(GTC_SAMPLE, false);
    assert_eq!(report.tools.len(), 3, "{:?}", report.skipped);
    assert!(report.skipped.is_empty());
    let ball = &report.tools[0];
    assert_eq!(ball.family, Family::EndMillBall);
    assert_eq!(ball.label, "6 mm ball nose");
    assert_eq!(ball.params.get("cutting_diameter"), Some(&6.0), "a ball's diameter is twice its corner");
    assert_eq!(ball.params.get("flute_length"), Some(&15.0));
    assert_eq!(ball.holder_type.as_deref(), Some("er32_bt40"));
    assert_eq!(ball.stickout, Some(40.0));
    assert_eq!(ball.vendor_properties.get("gtc.tool_type").map(String::as_str), Some("BALL"));
    let flat = &report.tools[1];
    assert_eq!(flat.family, Family::EndMillFlat);
    assert_eq!(flat.params.get("cutting_diameter"), Some(&10.0));
    assert_eq!(flat.params.get("overall_length"), Some(&70.0));
    assert_eq!(flat.params.get("shank_diameter"), Some(&10.0));
    assert_eq!(flat.params.get("taper_angle"), Some(&0.0));
    // the third block has no `TOOL,` marker: the first record opens it implicitly
    let drill = &report.tools[2];
    assert_eq!(drill.family, Family::Drill);
    assert_eq!(drill.params.get("point_angle"), Some(&118.0));
}

#[test]
fn gtc_converts_inch_files() {
    let report = parse_gtc("UNIT/INCH\nTOOL,\nTOOL_TYPE/END\nCUT_DIAMETER/0.5\nLENGTH/3\n", false);
    assert_eq!(report.tools.len(), 1, "{:?}", report.skipped);
    assert!((report.tools[0].params["cutting_diameter"] - 12.7).abs() < 1e-9);
    assert!((report.tools[0].params["overall_length"] - 76.2).abs() < 1e-9);
    assert_eq!(report.tools[0].params["taper_angle"], 0.0, "angles never scale with the unit");
}

#[test]
fn gtc_taper_types_carry_their_default_angle() {
    let report = parse_gtc("TOOL,\nTOOL_TYPE/TAPER_BALL\nCORNER/5\nTAPER/7\n", false);
    assert_eq!(report.tools.len(), 1, "{:?}", report.skipped);
    assert_eq!(report.tools[0].family, Family::EndMillBall);
    assert_eq!(report.tools[0].params["taper_angle"], 7.0);
    assert_eq!(report.tools[0].params["cutting_diameter"], 10.0);
}

#[test]
fn gtc_end_with_a_corner_is_a_bull_nose() {
    let report = parse_gtc("TOOL,\nTOOL_TYPE/END\nCUT_DIAMETER/12\nCORNER/1.5\n", false);
    assert_eq!(report.tools.len(), 1, "{:?}", report.skipped);
    assert_eq!(report.tools[0].family, Family::EndMillBull);
    assert_eq!(report.tools[0].params["corner_radius"], 1.5);
}

#[test]
fn gtc_infers_a_family_without_a_tool_type() {
    let report = parse_gtc("TOOL,\nNAME/plain rod\nCUT_DIAMETER/8\n", false);
    assert_eq!(report.tools.len(), 1, "{:?}", report.skipped);
    assert_eq!(report.tools[0].family, Family::EndMillFlat);
    assert_eq!(report.tools[0].label, "plain rod");
}

#[test]
fn gtc_skips_what_it_cannot_use_and_says_why() {
    let report = parse_gtc("TOOL,\nTOOL_TYPE/FACE\nNAME/face mill\nTOOL,\nNAME/silent block\n", false);
    assert!(report.tools.is_empty());
    assert_eq!(report.skipped.len(), 2);
    assert!(report.skipped[0].why.contains("face mill") || report.skipped[0].what == "face mill", "{:?}", report.skipped);
    assert!(report.skipped[1].why.contains("neither"), "{:?}", report.skipped);
}

#[test]
fn gtc_holder_names_must_be_specific() {
    let report = parse_gtc("TOOL,\nTOOL_TYPE/END\nCUT_DIAMETER/10\nHOLDER,ER32\n", false);
    assert_eq!(report.tools.len(), 1, "{:?}", report.skipped);
    assert_eq!(report.tools[0].holder_type, None, "a bare ER32 could sit on several spindles");
    assert!(report.tools[0].notes.as_deref().is_some_and(|n| n.contains("ER32")));
    let report = parse_gtc("TOOL,\nTOOL_TYPE/END\nCUT_DIAMETER/10\nHOLDER,ER32 collet chuck BT40\nSHANK,10,10,60,55\n", false);
    assert_eq!(report.tools[0].holder_type.as_deref(), Some("er32_bt40"));
    assert_eq!(report.tools[0].stickout, Some(55.0));
}

#[test]
fn gtc_hostile_input_never_panics_and_never_passes() {
    let mut nasty = String::from("UNIT/MM\n");
    for i in 0..2000 {
        nasty.push_str(&format!("TOOL,{i}\nTOOL_TYPE/END\nCUT_DIAMETER/1e{}/\n", i % 40));
    }
    nasty.push_str(&format!("NAME/{}\n", "x".repeat(MAX_LINE * 2)));
    nasty.push_str("CORNER/1e300\n");
    let report = parse_gtc(&nasty, false);
    assert!(report.tools.len() + report.skipped.len() <= MAX_TOOLS + 1);
    for tool in &report.tools {
        assert!(tool.params.values().all(|v| v.is_finite() && *v <= 600.0));
        assert!(tool.label.len() <= 128);
    }
}

#[test]
fn gtc_stops_at_the_tool_cap() {
    let mut many = String::new();
    for i in 0..(MAX_TOOLS + 50) {
        many.push_str(&format!("TOOL,{i}\nTOOL_TYPE/END\nCUT_DIAMETER/10\n"));
    }
    let report = parse_gtc(&many, false);
    assert_eq!(report.tools.len(), MAX_TOOLS);
    assert!(report.skipped.iter().any(|s| s.why.contains("more than")));
}

#[test]
fn json_reads_tools_in_both_shapes() {
    let report = parse_json(
        r#"[{"family": "end_mill_bull", "label": "bull 10", "params": {"cutting_diameter": 10, "corner_radius": 1}},
            {"family": "end_mill_flat", "label": "flat inch", "unit": "inch", "params": {"cutting_diameter": 0.5},
             "holder_type": "er32_bt40", "stickout": 2.5, "vendor": "Acme"}]"#,
    )
    .unwrap();
    assert_eq!(report.tools.len(), 2, "{:?}", report.skipped);
    assert_eq!(report.tools[0].params["flute_length"], 20.0, "missing parameters take the family default");
    assert!((report.tools[1].params["cutting_diameter"] - 12.7).abs() < 1e-9);
    assert_eq!(report.tools[1].holder_type.as_deref(), Some("er32_bt40"));
    assert_eq!(report.tools[1].stickout, Some(63.5), "the stickout arrives in the file's unit too");
    assert_eq!(report.tools[1].vendor.as_deref(), Some("Acme"));
    let wrapped = parse_json(r#"{"tools": [{"family": "drill", "label": "d", "params": {}}]}"#).unwrap();
    assert_eq!(wrapped.tools.len(), 1);
}

#[test]
fn json_reports_bad_tools_as_skips_and_bad_files_as_errors() {
    let report = parse_json(r#"[{"family": "nope", "label": "x"}, {"family": "drill", "label": "ok", "params": {}}]"#).unwrap();
    assert_eq!(report.tools.len(), 1);
    assert_eq!(report.skipped.len(), 1);
    assert!(report.skipped[0].why.contains("family"));
    assert!(parse_json("not json at all").is_err());
    assert!(parse_json(r#"{"unexpected": 1}"#).is_err());
    assert!(
        parse_json(r#"[{"family": "drill", "label": "x", "params": {"cutting_diameter": 1e400}}]"#).is_err(),
        "a number out of range fails the file"
    );
    let report = parse_json(r#"[{"family": "drill", "label": "x", "params": {"cutting_diameter": 1e39}}]"#).unwrap();
    assert!(report.tools.is_empty() && report.skipped.len() == 1, "an absurd number is a skip, not a crash");
    let hostile: Vec<String> = (0..(MAX_TOOLS + 1)).map(|i| format!(r#"{{"family": "drill", "label": "t{i}", "params": {{}}}}"#)).collect();
    assert!(parse_json(&format!("[{}]", hostile.join(","))).is_err());
}

#[test]
fn json_holder_and_stickout_are_validated() {
    let report = parse_json(r#"[{"family": "drill", "label": "x", "params": {}, "holder_type": "nope"}]"#).unwrap();
    assert!(report.tools.is_empty());
    assert!(report.skipped[0].why.contains("holder"));
    let report = parse_json(r#"[{"family": "drill", "label": "x", "params": {}, "holder_type": "er32_bt40", "stickout": 1}]"#).unwrap();
    assert!(report.tools.is_empty(), "a stickout inside the holder nose is refused");
}
