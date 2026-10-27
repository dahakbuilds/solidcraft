//! A pattern's copies solve group by group instead of joining one oversized dense system
//! (#7824, #5280): the seed's group solves as it always did, each copy solves against it.

use std::sync::{Arc, Mutex};

use solidcraft_geom::Vec2;

use crate::*;

/// A free seed point rectangular-patterned `count` times along x: `count - 1` copies of
/// two unknowns each, so 3001 copies pass `MAX_COMPONENT_VARS` (3000 × 2 + the seed's 2).
fn point_row(count: usize) -> Sketch {
    let mut sk = Sketch::new();
    sk.add_point(Vec2::new(1.0, 2.0), Some("p")).unwrap();
    let kind = SketchPatternKind::Rectangular {
        dir: Vec2::new(1.0, 0.0),
        dir2: None,
        direction: None,
        direction2: None,
        count,
        count2: 1,
        spacing: 10.0,
        spacing2: 0.0,
        alignment: PatternAlignment::Right,
        alignment2: PatternAlignment::Right,
        reversed: false,
        reversed2: false,
    };
    sk.patterns.insert(
        0,
        SketchPattern {
            id: "pat".into(),
            entities: vec!["p".into()],
            kind: kind.clone(),
            instances: Vec::new(),
            expressions: Default::default(),
            count_expression: None,
        },
    );
    sk.update_pattern(0, kind).unwrap();
    sk
}

/// The copy point ids of a point-row pattern, in slot order.
fn copies(sk: &Sketch) -> Vec<usize> {
    sk.patterns[0].instances.iter().map(|i| sk.point_index(&i.points[0].1).unwrap()).collect()
}

fn assert_copies_on_the_row(sk: &Sketch, seed_x: f64) {
    for (i, &q) in copies(sk).iter().enumerate() {
        let at = sk.point(q).unwrap();
        assert!((at.x - (seed_x + 10.0 * (i + 1) as f64)).abs() < 1e-6, "copy {i} at {at:?}");
        assert!((at.y - 2.0).abs() < 1e-6, "copy {i} at {at:?}");
    }
}

#[test]
fn a_pattern_past_the_component_wall_solves_copy_by_copy() {
    let mut sk = point_row(3001);
    let rep = solve(&mut sk);
    assert!(rep.oversized.is_none(), "the star split must clear the wall: {:?}", rep.oversized);
    assert!(rep.ok(), "{rep:?}");
    // The seed alone keeps its freedom: the copies are determined given it.
    assert_eq!(rep.dof, 2, "{rep:?}");
    assert!(!rep.point_determined[sk.point_index("p").unwrap()]);
    assert_copies_on_the_row(&sk, 1.0);

    // Move the seed: every copy re-solves against it in its own small system.
    let p = sk.point_index("p").unwrap();
    sk.points[p].pos = Vec2::new(7.0, 2.0);
    let rep = solve(&mut sk);
    assert!(rep.ok() && rep.oversized.is_none(), "{rep:?}");
    assert_copies_on_the_row(&sk, 7.0);
}

#[test]
fn copies_tied_to_each_other_still_report_oversized() {
    let mut sk = point_row(3001);
    // A distance between each pair of neighbours merges all the copies into one group:
    // the star is gone, and the merged remainder is what the oversized report names.
    let qs = copies(&sk);
    for w in qs.windows(2) {
        sk.add_constraint(ConstraintKind::Distance { p: w[0], q: w[1], value: 10.0 }, None).unwrap();
    }
    let rep = solve(&mut sk);
    assert_eq!(rep.oversized, Some(qs.len() * 2), "the merged copies stay one group");
    assert!(!rep.ok());
}

#[test]
fn a_batched_pattern_reports_progress() {
    #[derive(Default)]
    struct Seen(Mutex<Vec<(usize, usize)>>);
    impl crate::watch::Watch for Seen {
        fn progress(&self, _stage: &'static str, done: usize, total: usize) {
            self.0.lock().unwrap().push((done, total));
        }
    }
    let seen = Arc::new(Seen::default());
    let mut sk = point_row(3001);
    crate::watch::watching(seen.clone(), || {
        assert!(solve(&mut sk).ok());
    });
    let seen = seen.0.lock().unwrap();
    assert!(!seen.is_empty(), "the batched copies report");
    assert!(seen.iter().all(|&(_, t)| t == seen[0].1), "one total throughout");
    assert_eq!(seen.last().unwrap().0, seen.last().unwrap().1, "the last report is complete");
    assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0 || w[1].0 < w[0].0 / 2), "each solve runs the bar up");
}

#[test]
fn a_pattern_under_the_wall_is_untouched_by_the_split() {
    // Small patterns never reach the split: same solve as before it existed.
    let mut sk = point_row(50);
    let rep = solve(&mut sk);
    assert!(rep.ok() && rep.oversized.is_none() && rep.dof == 2, "{rep:?}");
    assert_copies_on_the_row(&sk, 1.0);
}
