//! Which geometry kernel answers the routed operations (Preferences ▸ Design ▸
//! Geometry kernel).
//!
//! The truck path is the stable default; the solidcore path is experimental and every
//! routed operation falls back to truck when solidcore reports an error. The choice is
//! a process-wide atomic so any operation can read it per call without threading it
//! through the whole workspace.

use std::sync::atomic::{AtomicU8, Ordering};

/// The B-rep kernels a build can route operations to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KernelChoice {
    /// The truck-based kernel: the stable default, and the fallback for every solidcore
    /// failure.
    #[default]
    Truck,
    /// The solidcore kernel (the SMLib port): experimental, routed operations only.
    Solidcore,
}

impl KernelChoice {
    /// Every choice, in preference order.
    pub const ALL: [Self; 2] = [Self::Truck, Self::Solidcore];

    /// The id as preferences and the control channel spell it.
    pub fn id(self) -> &'static str {
        match self {
            Self::Truck => "truck",
            Self::Solidcore => "solidcore",
        }
    }

    /// Back from a preference or environment id; anything else is `None`.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|choice| choice.id() == id)
    }
}

static SELECTED: AtomicU8 = AtomicU8::new(0);

/// The choice the `SOLIDCRAFT_KERNEL` environment variable asks for, read once. It
/// overrides Preferences for the whole process so the test matrix can run any suite in
/// solidcore mode; an unrecognised value reads as no override.
fn environment_override() -> Option<KernelChoice> {
    use std::sync::OnceLock;
    static OVERRIDE: OnceLock<Option<KernelChoice>> = OnceLock::new();
    *OVERRIDE.get_or_init(|| std::env::var("SOLIDCRAFT_KERNEL").ok().as_deref().and_then(KernelChoice::from_id))
}

/// The kernel new operations route to (see [`select`]).
pub fn selected() -> KernelChoice {
    environment_override().unwrap_or_else(|| match SELECTED.load(Ordering::Relaxed) {
        1 => KernelChoice::Solidcore,
        _ => KernelChoice::Truck,
    })
}

/// Whether solidcore answers routed operations right now (see [`selected`]).
pub fn solidcore_selected() -> bool {
    selected() == KernelChoice::Solidcore
}

/// Sets the kernel routed operations use, from Preferences. The environment override
/// wins when present, so a test run cannot be switched back at runtime.
pub fn select(choice: KernelChoice) {
    SELECTED.store(choice as u8, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for choice in KernelChoice::ALL {
            assert_eq!(KernelChoice::from_id(choice.id()), Some(choice));
        }
        assert_eq!(KernelChoice::from_id("occt"), None);
        assert_eq!(KernelChoice::default(), KernelChoice::Truck);
    }

    #[test]
    fn selection_round_trips_unless_the_environment_overrides() {
        if std::env::var_os("SOLIDCRAFT_KERNEL").is_some() {
            return; // the override wins by design; the matrix covers it.
        }
        let was = selected();
        select(KernelChoice::Solidcore);
        assert_eq!(selected(), KernelChoice::Solidcore);
        assert!(solidcore_selected());
        select(KernelChoice::Truck);
        assert_eq!(selected(), KernelChoice::Truck);
        select(was);
    }
}
