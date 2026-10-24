use crate::model::*;
use crate::toolpath::{Segment, Toolpath};
use serde_json::json;

#[test]
fn empty_setup_saves_nothing() {
    let setup = CamSetup::default();
    assert!(setup.is_empty());
    let text = serde_json::to_string(&setup).unwrap();
    assert_eq!(text, r#"{"operations":[],"fixtures":[],"origin":{"position":[0.0,0.0,0.0],"x_axis":[1.0,0.0,0.0],"z_axis":[0.0,0.0,1.0]}}"#);
}

#[test]
fn setup_round_trips_through_json() {
    let setup = CamSetup {
        operations: vec![Operation {
            id: 3,
            name: "Pocket 6mm".into(),
            tool: Some(ToolRef { number: 4, entry: "abc".into(), name: "6mm end mill".into() }),
            stock: Some(Stock::box_stock(100.0, 80.0, 20.0, Allowance { plus_z: 1.0, ..Default::default() })),
            toolpath: Some(Toolpath {
                segments: vec![Segment::Line { from: [0.0, 0.0, 5.0], to: [10.0, 0.0, 5.0], cut: true }],
            }),
            comment: None,
        }],
        fixtures: vec![Fixture { id: 7, name: "Vise".into(), path: "parts/vise.step".into(), origin: [0.0, -40.0, 0.0] }],
        origin: WorkOrigin::default(),
        tool_library: Some("tools/shop.json".into()),
    };
    setup.check().unwrap();
    let text = serde_json::to_string(&setup).unwrap();
    let back: CamSetup = serde_json::from_str(&text).unwrap();
    assert_eq!(setup, back);
}

#[test]
fn box_extent_adds_allowances_per_face() {
    let stock = Stock {
        kind: StockKind::Box { x: 10.0, y: 10.0, z: 10.0 },
        origin: [5.0, 5.0, 0.0],
        allowance: Allowance { plus_x: 1.0, minus_x: 2.0, plus_y: 0.0, minus_y: 0.0, plus_z: 3.0, minus_z: 0.5 },
    };
    let (min, max) = stock.extent().unwrap();
    assert_eq!(min, [-2.0, 0.0, -0.5]);
    assert_eq!(max, [11.0, 10.0, 13.0]);
}

#[test]
fn cylinder_extent_is_its_bounding_box() {
    let stock = Stock { kind: StockKind::Cylinder { radius: 5.0, height: 10.0 }, origin: [0.0, 0.0, 1.0], allowance: Allowance::default() };
    let (min, max) = stock.extent().unwrap();
    assert_eq!(min, [-5.0, -5.0, 1.0]);
    assert_eq!(max, [5.0, 5.0, 11.0]);
}

#[test]
fn body_stock_has_no_extent() {
    let stock = Stock { kind: StockKind::Body { body: "Blank".into() }, origin: [0.0; 3], allowance: Allowance::default() };
    assert!(stock.extent().is_none());
}

#[test]
fn hostile_values_are_refused() {
    // NaN sizes, negative allowances, oversized coordinates, non-unit axes.
    assert!(StockKind::Box { x: f64::NAN, y: 1.0, z: 1.0 }.check().is_err());
    assert!(Allowance { minus_x: -1.0, ..Allowance::default() }.check().is_err());
    assert!(Stock { kind: StockKind::Box { x: 1e12, y: 1.0, z: 1.0 }, origin: [0.0; 3], allowance: Allowance::default() }.check().is_err());
    assert!(WorkOrigin { position: [0.0; 3], x_axis: [2.0, 0.0, 0.0], z_axis: [0.0, 0.0, 1.0] }.check().is_err());
    assert!(WorkOrigin { position: [0.0; 3], x_axis: [0.0, 0.0, 1.0], z_axis: [0.0, 0.0, 1.0] }.check().is_err());
    let dup = CamSetup {
        operations: vec![Operation { id: 1, name: "a".into(), ..Default::default() }, Operation { id: 1, name: "b".into(), ..Default::default() }],
        ..Default::default()
    };
    assert!(dup.check().is_err());
}

#[test]
fn unknown_fields_are_refused() {
    assert!(serde_json::from_value::<CamSetup>(json!({"operations": [], "fixtures": [], "origin": {"position": [0, 0, 0], "x_axis": [1, 0, 0], "z_axis": [0, 0, 1]}, "smuggled": 1})).is_err());
}
