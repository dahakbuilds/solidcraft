use super::*;

fn entry(family: &str, pairs: &[(&str, f64)]) -> ToolEntry {
    ToolEntry {
        id: String::new(),
        revision: String::new(),
        kind: ItemKind::Cutting,
        family: family.into(),
        label: "Test 10 mm".into(),
        vendor: Some("Testmaker".into()),
        product_id: Some("TM-10".into()),
        product_url: None,
        params: pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        machine_connection: None,
        work_connection: Some(Connection { standard: "hsk-a".into(), size: "63".into() }),
        notes: None,
        provenance: Provenance::default(),
        vendor_properties: BTreeMap::new(),
    }
}

fn flat_entry() -> ToolEntry {
    entry("end_mill_flat", &[("cutting_diameter", 10.0), ("flute_length", 20.0), ("overall_length", 70.0), ("shank_diameter", 10.0)])
}

#[test]
fn round_trip_survives_encode_and_decode() {
    let mut library = ToolLibrary::new("user.test", "Test tools").unwrap();
    let mut e = flat_entry();
    e.assign_id(&library.id).unwrap();
    assert!(e.id.starts_with("user.test/cutting-"));
    library.entries.push(e.clone());
    let bytes = library.encode().unwrap();
    let back = ToolLibrary::decode(&bytes).unwrap();
    assert_eq!(back, library);
    assert_eq!(back.entries[0].revision, e.revision);
    assert_eq!(e.revision.len(), 64);
}

#[test]
fn revision_tracks_content_not_identity() {
    let mut a = flat_entry();
    a.assign_id("user.test").unwrap();
    let revision = a.revision.clone();
    let mut b = a.clone();
    b.id = "user.test/cutting-00000000".into();
    assert_eq!(b.content_revision().unwrap(), revision, "identity fields are not content");
    let mut c = a.clone();
    c.params.insert("flute_length".into(), 25.0);
    assert_ne!(c.content_revision().unwrap(), revision);
}

#[test]
fn tampered_revision_and_foreign_identity_are_refused() {
    let mut library = ToolLibrary::new("user.test", "Test tools").unwrap();
    let mut e = flat_entry();
    e.assign_id(&library.id).unwrap();
    library.entries.push(e);
    // tamper: keep the old revision but change a parameter
    library.entries[0].params.insert("overall_length".into(), 99.0);
    assert!(library.validate().unwrap_err().to_string().contains("revision"));
    // foreign library id
    let mut library = ToolLibrary::new("user.test", "Test tools").unwrap();
    let mut e = flat_entry();
    e.assign_id("someone.else").unwrap();
    library.entries.push(e);
    assert!(library.validate().is_err());
}

#[test]
fn unknown_fields_and_format_are_refused() {
    let mut library = ToolLibrary::new("user.test", "Test tools").unwrap();
    let mut e = flat_entry();
    e.assign_id(&library.id).unwrap();
    library.entries.push(e);
    let mut value: serde_json::Value = serde_json::to_value(&library).unwrap();
    value["entries"][0]["smuggled"] = serde_json::json!(1);
    assert!(ToolLibrary::decode(&serde_json::to_vec(&value).unwrap()).is_err());
    let mut value: serde_json::Value = serde_json::to_value(&library).unwrap();
    value["format"] = serde_json::json!("solidcraft-tooling/2");
    assert!(ToolLibrary::decode(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn unsupported_kinds_and_families_say_not_supported_yet() {
    let mut e = flat_entry();
    e.kind = ItemKind::Holder;
    assert!(e.content_revision().unwrap_err().to_string().starts_with("not supported yet"));
    let mut e = flat_entry();
    e.family = "turn_groover".into();
    e.assign_id("user.test").unwrap_err();
}

#[test]
fn hostile_entry_fields_are_refused() {
    let mut e = flat_entry();
    e.label = "line\nbreak".into();
    assert!(e.assign_id("user.test").is_err());
    let mut e = flat_entry();
    e.label = " ".repeat(200);
    assert!(e.assign_id("user.test").is_err());
    let mut e = flat_entry();
    e.params.insert("evil".into(), 1.0);
    assert!(e.assign_id("user.test").is_err());
    let mut e = flat_entry();
    e.params.insert("cutting_diameter".into(), f64::NAN);
    assert!(e.assign_id("user.test").is_err());
    let mut e = flat_entry();
    e.machine_connection = Some(Connection { standard: String::new(), size: "63".into() });
    assert!(e.assign_id("user.test").is_err());
}

#[test]
fn library_identity_rules() {
    assert!(ToolLibrary::new("User.Test", "Upper").is_err());
    assert!(ToolLibrary::new("user.test", "").is_err());
    assert!(ToolLibrary::new("user.test", " Padded ").is_err());
}
