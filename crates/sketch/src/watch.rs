//! Progress for long solves run off the window thread (a pattern's copies, #7824). The
//! caller installs a [`Watch`] around the work with [`watching`]; the solve reports how far
//! it is. Work that nobody watches runs as before.

use std::cell::RefCell;
use std::sync::Arc;

/// Someone waiting for a long solve: told how far it is.
pub trait Watch: Send + Sync {
    /// `done` of `total` units of the current stage (`stage`, such as "pattern copies").
    fn progress(&self, stage: &'static str, done: usize, total: usize);
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<dyn Watch>>> = const { RefCell::new(None) };
}

/// Run `f` with `watch` seeing the solves it does on this thread.
pub fn watching<R>(watch: Arc<dyn Watch>, f: impl FnOnce() -> R) -> R {
    let previous = CURRENT.with(|c| c.replace(Some(watch)));
    struct Restore(Option<Option<Arc<dyn Watch>>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(previous) = self.0.take() {
                CURRENT.with(|c| *c.borrow_mut() = previous);
            }
        }
    }
    let _restore = Restore(Some(previous));
    f()
}

/// Report progress when this thread's work is watched; free otherwise.
pub(crate) fn progress(stage: &'static str, done: usize, total: usize) {
    CURRENT.with(|c| {
        if let Some(w) = c.borrow().as_ref() {
            w.progress(stage, done, total);
        }
    });
}
