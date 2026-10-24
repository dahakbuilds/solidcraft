//! The starter pack: a small set of common milling cutters to seed a fresh default
//! library, so the Tool Library has something to show on first open. These are
//! generic shop tools described from public dimensional conventions — no vendor
//! catalogue data — and every entry carries the pack's own provenance.

use std::collections::BTreeMap;

use crate::{
    ItemKind, Provenance, Result, ToolEntry,
    family::Family,
    import::fill_params,
    invalid,
};

fn provenance() -> Provenance {
    Provenance { license: "CC0-1.0".into(), author: Some("SolidCraft starter pack".into()), source_url: None }
}

/// (family, label, parameters) for each starter tool. Lengths are millimetres and
/// angles degrees; anything left out takes the family default.
const PACK: &[(Family, &str, &[(&str, f64)])] = &[
    (
        Family::EndMillFlat,
        "10 mm flat end mill",
        &[("cutting_diameter", 10.0), ("flute_length", 20.0), ("overall_length", 70.0), ("shank_diameter", 10.0)],
    ),
    (
        Family::EndMillBall,
        "6 mm ball nose",
        &[("cutting_diameter", 6.0), ("flute_length", 15.0), ("overall_length", 50.0), ("shank_diameter", 6.0)],
    ),
    (
        Family::EndMillBull,
        "12 mm bull nose R1.5",
        &[("cutting_diameter", 12.0), ("corner_radius", 1.5), ("flute_length", 25.0), ("overall_length", 75.0), ("shank_diameter", 12.0)],
    ),
    (
        Family::Drill,
        "8 mm drill 118 deg",
        &[("cutting_diameter", 8.0), ("flute_length", 30.0), ("overall_length", 60.0), ("shank_diameter", 8.0), ("point_angle", 118.0)],
    ),
    (Family::ChamferMill, "10 mm chamfer 90 deg", &[("cutting_diameter", 10.0), ("included_angle", 90.0)]),
    (
        Family::Tap,
        "M8x1.25 tap",
        &[("major_diameter", 8.0), ("pitch", 1.25), ("thread_length", 20.0), ("overall_length", 63.0), ("shank_diameter", 6.3)],
    ),
];

/// Build the starter pack for a library: every entry validated and id-assigned.
pub fn starter_pack(library_id: &str) -> Result<Vec<ToolEntry>> {
    let mut out = Vec::new();
    for (family, label, params) in PACK {
        let mut given = BTreeMap::new();
        for (key, value) in params.iter().copied() {
            given.insert(key.to_string(), value);
        }
        let params = fill_params(*family, given, false).map_err(invalid)?;
        let mut entry = ToolEntry {
            id: String::new(),
            revision: String::new(),
            kind: ItemKind::Cutting,
            family: family.as_str().to_string(),
            label: label.to_string(),
            vendor: None,
            product_id: None,
            product_url: None,
            params,
            holder_type: None,
            stickout: None,
            machine_connection: None,
            work_connection: None,
            notes: None,
            provenance: provenance(),
            vendor_properties: BTreeMap::new(),
        };
        entry.assign_id(library_id)?;
        out.push(entry);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pack_validates_and_is_deterministic() {
        let first = starter_pack("user.tools").unwrap();
        assert_eq!(first.len(), PACK.len());
        assert_eq!(first, starter_pack("user.tools").unwrap(), "the pack is content-stable, so re-seeding dedupes");
        let mut library = crate::ToolLibrary::new("user.tools", "My tools").unwrap();
        library.entries = first.clone();
        library.validate().unwrap();
        for entry in &first {
            assert!(entry.id.starts_with("user.tools/cutting-"), "{}", entry.id);
            assert_eq!(entry.provenance.author.as_deref(), Some("SolidCraft starter pack"));
        }
    }

    #[test]
    fn the_pack_ids_follow_the_library() {
        let other = starter_pack("shop.tools").unwrap();
        assert!(other.iter().all(|e| e.id.starts_with("shop.tools/")));
        assert_ne!(starter_pack("a").unwrap()[0].id, starter_pack("b").unwrap()[0].id);
    }
}
