use super::*;
use crate::{Family, profile};
use std::collections::BTreeMap;

fn params(items: &[(&str, f64)]) -> BTreeMap<String, f64> {
    items.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn flat_mill() -> CutterProfile {
    profile::of(
        Family::EndMillFlat,
        &params(&[("cutting_diameter", 10.0), ("flute_length", 20.0), ("overall_length", 70.0), ("shank_diameter", 10.0)]),
    )
    .unwrap()
}

fn ball_mill() -> CutterProfile {
    profile::of(
        Family::EndMillBall,
        &params(&[("cutting_diameter", 10.0), ("flute_length", 15.0), ("overall_length", 60.0), ("shank_diameter", 10.0)]),
    )
    .unwrap()
}

fn maps(top: f64, part_top: f64) -> (ZMap, ZMap) {
    let stock = ZMap::flat((0.0, 0.0), (40.0, 40.0), top, 0.5).unwrap();
    let part = ZMap::flat((0.0, 0.0), (40.0, 40.0), part_top, 0.5).unwrap();
    (stock, part)
}

fn pt(x: f64, y: f64, z: f64, cut: bool) -> ClPoint {
    ClPoint::new(x, y, z, cut).unwrap()
}

#[test]
fn profile_height_queries_are_analytic() {
    let flat = flat_mill();
    assert_eq!(flat.cut_height_at(0.0), Some(0.0));
    assert_eq!(flat.cut_height_at(5.0), Some(0.0), "the flat rim cuts at the tip plane");
    assert_eq!(flat.cut_height_at(5.01), None, "nothing cuts beyond the radius");
    assert_eq!(flat.envelope_height_at(5.0), Some(0.0));
    let ball = ball_mill();
    // the ball surface at radius r sits R - sqrt(R² - r²) above the tip
    let r = 3.0;
    let expect = 5.0 - (25.0 - r * r).sqrt();
    assert!((ball.cut_height_at(r).unwrap() - expect).abs() < 1e-9);
    assert!((ball.cut_height_at(0.0).unwrap() - 0.0).abs() < 1e-9);
}

#[test]
fn a_flat_mill_pockets_down_to_the_part_top() {
    let (mut stock, part) = maps(10.0, 5.0);
    let path = [pt(5.0, 20.0, 12.0, false), pt(5.0, 20.0, 5.0, true), pt(35.0, 20.0, 5.0, true)];
    let report = simulate(&flat_mill(), &path, &mut stock, &part, &SimOptions::default()).unwrap();
    assert_eq!(report.gouge_count, 0, "cutting exactly to the part top is not a gouge");
    assert_eq!(report.collisions, 0);
    // swept area: 30 × 10 rectangle plus a radius-5 circle, five millimetres deep
    let analytic = (300.0 + std::f64::consts::PI * 25.0) * 5.0;
    assert!((report.removed_mm3 - analytic).abs() / analytic < 0.05, "removed {} vs analytic {analytic}", report.removed_mm3);
    let (lo, hi) = stock.extent();
    assert!(lo > 4.99 && lo < 5.01, "the pocket floor sits on the part top, got {lo}");
    assert!((hi - 10.0).abs() < 1e-9, "untouched stock stays at the top");
    assert!(report.remaining_cells > 0, "the shoulder above the part top is still standing");
    assert!((report.remaining_worst - 5.0).abs() < 1e-9);
}

#[test]
fn cutting_below_the_part_is_a_gouge() {
    let (mut stock, part) = maps(10.0, 5.0);
    let path = [pt(5.0, 20.0, 12.0, false), pt(5.0, 20.0, 4.0, true), pt(35.0, 20.0, 4.0, true)];
    let report = simulate(&flat_mill(), &path, &mut stock, &part, &SimOptions::default()).unwrap();
    assert!(report.gouge_count > 0);
    assert!((report.gouge_worst - 1.0).abs() < 1e-9, "one millimetre below the part top");
    assert!(!report.gouges.is_empty());
    for g in &report.gouges {
        assert!((g[2] - 1.0).abs() < 1e-9);
    }
}

#[test]
fn a_ball_nose_tangent_to_the_part_cuts_a_clean_groove() {
    let (mut stock, part) = maps(10.0, 5.0);
    let path = [pt(5.0, 20.0, 12.0, false), pt(5.0, 20.0, 5.0, true), pt(35.0, 20.0, 5.0, true)];
    let report = simulate(&ball_mill(), &path, &mut stock, &part, &SimOptions::default()).unwrap();
    assert_eq!(report.gouge_count, 0, "tangent contact never dips below the part");
    assert!(report.removed_mm3 > 0.0, "the groove removes real volume");
}

#[test]
fn a_rapid_that_dips_into_the_stock_is_a_collision() {
    let (mut stock, part) = maps(10.0, 5.0);
    let low = [pt(5.0, 20.0, 12.0, false), pt(35.0, 20.0, 8.0, false)];
    let report = simulate(&flat_mill(), &low, &mut stock, &part, &SimOptions::default()).unwrap();
    assert!(report.collisions > 0, "the tool sweeps through the stock top");
    assert_eq!(report.removed_mm3, 0.0, "a rapid never cuts");
    let (hi, _) = stock.extent();
    assert!((hi - 10.0).abs() < 1e-9);
    let high = [pt(5.0, 20.0, 12.0, false), pt(35.0, 20.0, 12.0, false)];
    let report = simulate(&flat_mill(), &high, &mut stock, &part, &SimOptions::default()).unwrap();
    assert_eq!(report.collisions, 0);
}

#[test]
fn grids_must_match_and_bounds_are_enforced() {
    let stock = ZMap::flat((0.0, 0.0), (40.0, 40.0), 10.0, 0.5).unwrap();
    let other = ZMap::flat((0.0, 0.0), (40.0, 40.0), 10.0, 1.0).unwrap();
    let path = [pt(5.0, 20.0, 12.0, false), pt(35.0, 20.0, 5.0, true)];
    let mut stock_copy = stock.clone();
    assert!(simulate(&flat_mill(), &path, &mut stock_copy, &other, &SimOptions::default()).is_err());
    // a grid finer than the cell floor over a wide footprint is refused
    assert!(ZMap::flat((0.0, 0.0), (40.0, 40.0), 10.0, 0.05).is_err());
    // hostile bounds
    assert!(ZMap::flat((0.0, 0.0), (0.0, 10.0), 10.0, 0.5).is_err());
    assert!(ZMap::flat((0.0, 0.0), (f64::NAN, 10.0), 10.0, 0.5).is_err());
    assert!(ClPoint::new(f64::NAN, 0.0, 0.0, true).is_none());
}

#[test]
fn a_path_too_fine_for_the_budget_is_refused_up_front() {
    let (mut stock, part) = maps(10.0, 5.0);
    let mut path = vec![pt(0.0, 20.0, 5.0, true)];
    for i in 1..=100 {
        path.push(pt(i as f64 * 100.0, 20.0, 5.0, true));
    }
    let opts = SimOptions { gouge_tol: 0.05, step: 0.05 };
    assert!(simulate(&flat_mill(), &path, &mut stock, &part, &opts).is_err());
}

#[test]
fn an_empty_or_single_point_path_reports_nothing() {
    let (mut stock, part) = maps(10.0, 5.0);
    let report = simulate(&flat_mill(), &[], &mut stock, &part, &SimOptions::default()).unwrap();
    assert_eq!(report.samples, 0);
    assert_eq!(report.remaining_cells, stock.shape().0 * stock.shape().1);
    let one = [pt(1.0, 1.0, 12.0, false)];
    let report = simulate(&flat_mill(), &one, &mut stock, &part, &SimOptions::default()).unwrap();
    assert_eq!(report.samples, 0);
}
