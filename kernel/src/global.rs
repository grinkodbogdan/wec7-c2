//! Global kernel state cell.

use core::cell::UnsafeCell;

/// A global mutable cell for kernel state.
///
/// Sound only under the M1 concurrency model: a single CPU, interrupts
/// masked whenever kernel code runs, and callers never holding two
/// references obtained from the same cell at once.
pub struct Global<T>(UnsafeCell<T>);

// SAFETY: see the type documentation — there is no concurrent access.
unsafe impl<T> Sync for Global<T> {}

impl<T> Global<T> {
    pub const fn new(v: T) -> Self {
        Global(UnsafeCell::new(v))
    }

    /// # Safety
    /// The caller must not create overlapping references to the contents.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get(&self) -> &mut T {
        &mut *self.0.get()
    }
}
