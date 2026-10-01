//! Epoch-Based Memory Reclamation (EBMR) infrastructure.
//!
//! Wraps `crossbeam-epoch` to provide safe, wait-free memory reclamation
//! during asynchronous block migrations and directory doubling.

pub use crossbeam_epoch::{pin, unprotected, Atomic, Guard, Owned, Shared};

/// Defers deallocation of a retired object until all epoch readers have completed.
///
/// # Arguments
/// * `value` - The boxed or owned object to reclaim.
/// * `guard` - Active epoch guard tracking current reader references.
#[inline]
pub fn defer_drop<T: Send + 'static>(value: T, guard: &Guard) {
    guard.defer(move || drop(value));
}

/// Executes a closure within an active epoch-pinned critical section.
///
/// Pinning informs concurrent threads that this reader may be accessing shared
/// memory, preventing deferred deallocations from releasing underlying pointers.
#[inline]
pub fn with_epoch<F, R>(f: F) -> R
where
    F: FnOnce(&Guard) -> R,
{
    let guard = &pin();
    f(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct DroppableResource {
        flag: Arc<AtomicBool>,
    }

    impl Drop for DroppableResource {
        fn drop(&mut self) {
            self.flag.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn test_epoch_deferred_reclamation() {
        let flag = Arc::new(AtomicBool::new(false));
        let resource = DroppableResource {
            flag: Arc::clone(&flag),
        };

        with_epoch(|guard| {
            defer_drop(resource, guard);
            // Resource must not drop immediately while guard is pinned
            assert!(!flag.load(Ordering::SeqCst));
        });

        // Trigger garbage collection pass
        pin().flush();
    }
}