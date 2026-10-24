use super::*;

#[test]
fn a_square_pocket_decodes_to_a_cut_path() {
    let text = "O1001\nG21 G90 G17 G40 G49 G80\nT1 M6\nS3000 M3\nG0 X15 Y20 Z5\nG1 Z-1 F200\nG1 X45 Y20\nG1 X45 Y40\nG1 X15 Y40\nG1 X15 Y20\nG0 Z10\nM30\n";
    let path = read(text).unwrap();
    assert_eq!(path.tools, vec!["T1"]);
    assert_eq!(path.points.len(), 7, "{:?}", path.points);
    assert!(!path.points[0].cut, "the rapid to position does not cut");
    assert!(path.points[1].cut, "the plunge feeds");
    assert_eq!(path.points[1].z, -1.0);
    assert_eq!(path.points[6].z, 10.0);
    assert!(!path.points[6].cut);
    assert!(path.warnings.is_empty(), "{:?}", path.warnings);
}

#[test]
fn motion_mode_is_modal() {
    let path = read("G1 X1\nX2\n").unwrap();
    assert_eq!(path.points.len(), 2);
    assert_eq!(path.points[1].x, 2.0);
    assert!(path.points[1].cut, "still feeding");
}

#[test]
fn a_stop_shares_its_block_with_the_last_move() {
    let path = read("G91 G1 X5 M30\nG1 X99\n").unwrap();
    assert_eq!(path.points.len(), 1, "the move on the M30 line still counts");
    assert_eq!(path.points[0].x, 5.0);
}

#[test]
fn incremental_and_inch_units_scale_moves() {
    let path = read("G20 G91\nG0 X1 Y1 Z1\nG1 Z-.05\nG1 X1\n").unwrap();
    assert!((path.points[0].x - 25.4).abs() < 1e-9);
    assert!((path.points[1].z - (25.4 - 1.27)).abs() < 1e-9);
    assert!((path.points[2].x - 50.8).abs() < 1e-9);
}

#[test]
fn comments_and_blank_lines_are_skipped() {
    let path = read("(hello)\n; bye\n\nG1 X1 Y1 ; feed\n(H) G0 X99\n").unwrap();
    assert_eq!(path.points.len(), 1);
    assert_eq!(path.points[0].x, 1.0);
}

#[test]
fn tool_changes_are_recorded() {
    let path = read("T2\nM6\nT3\n").unwrap();
    assert_eq!(path.tools, vec!["T2", "T3"], "a trailing T is still a tool the file used");
}

#[test]
fn arcs_canned_cycles_and_homing_are_refused() {
    assert!(read("G2 X1 Y1 I1 J1").unwrap_err().to_string().contains("not supported yet: arc moves"));
    assert!(read("G81 X1 Y1 Z-1").unwrap_err().to_string().contains("canned cycles"));
    assert!(read("G28 X0 Y0").unwrap_err().to_string().contains("return-to-home"));
}

#[test]
fn unknown_words_warn_instead_of_failing() {
    let path = read("G1 X1 Q5\n").unwrap();
    assert_eq!(path.points.len(), 1);
    assert!(path.warnings.iter().any(|w| w.contains("`Q`")), "{:?}", path.warnings);
}

#[test]
fn hostile_input_is_refused() {
    assert!(read("G1 X\n").is_err(), "a word without a number");
    assert!(read("\u{3c0}\n").is_err(), "not a word letter");
    let many = "G1 X1\n".repeat(MAX_LINES + 1);
    assert!(read(&many).is_err(), "the line cap");
    let big = format!("(x{}) G1 X1", "y".repeat(MAX_BYTES));
    assert!(read(&big).is_err(), "the size cap");
}
