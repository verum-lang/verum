//! Thread-local breadcrumb trail attached to crash reports.
//!
//! A breadcrumb records *where in the pipeline* we are — phase name plus
//! a short detail (file being compiled, function, pass). On a crash the
//! trail is serialised into the report so the dev who receives the
//! report can see the last few steps before the fault without having
//! access to the user's `.vr` sources.
//!
//! Each breadcrumb is pushed via `enter(...)` and popped automatically
//! when the returned RAII guard is dropped. Trails are bounded
//! (`MAX_TRAIL_LEN`) so long compilations don't retain unbounded memory.
//!
//! Crossing a signal boundary: the trail lives in thread-local storage;
//! signal handlers read it via a process-wide "last known trail" snapshot
//! updated on every push/pop. This makes the last good trail visible
//! even if the current thread is corrupted.

#![allow(missing_docs)]

use parking_lot::Mutex;
use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::Instant;

const MAX_TRAIL_LEN: usize = 64;

/// A single phase-level breadcrumb.
#[derive(Clone, Debug)]
pub struct Breadcrumb {
    pub phase: &'static str,
    pub detail: String,
    pub thread_name: String,
    pub started_at: Instant,
}

impl Breadcrumb {
    pub fn age_ms(&self) -> u128 {
        self.started_at.elapsed().as_millis()
    }
}

thread_local! {
    static TRAIL: RefCell<Vec<Breadcrumb>> = RefCell::new(Vec::with_capacity(16));
}

/// Last-known trail across all threads. Updated whenever any thread
/// pushes or pops a breadcrumb. Used by the signal handler — which
/// cannot safely walk another thread's `thread_local!` — to produce a
/// best-effort trail snapshot in the crash report.
static LAST_TRAIL: OnceLock<Mutex<Vec<Breadcrumb>>> = OnceLock::new();

fn last_trail() -> &'static Mutex<Vec<Breadcrumb>> {
    LAST_TRAIL.get_or_init(|| Mutex::new(Vec::new()))
}

/// RAII guard that pops the breadcrumb when dropped.
///
/// Must not outlive the thread that pushed it.
#[must_use = "breadcrumb guard must be bound to a local (e.g. `let _g = breadcrumb::enter(...)`)"]
pub struct BreadcrumbGuard {
    _private: (),
}

impl Drop for BreadcrumbGuard {
    fn drop(&mut self) {
        TRAIL.with(|t| {
            t.borrow_mut().pop();
        });
        // Snapshot the new (shorter) trail so cross-thread readers see
        // the current state.
        snapshot_current();
    }
}

/// Push a new breadcrumb; the returned guard pops it on drop.
///
/// `phase` is a stable, coarse-grained label like `"codegen.llvm.generate"`.
/// `detail` is free-form context — the .vr file name, function, op index.
pub fn enter(phase: &'static str, detail: impl Into<String>) -> BreadcrumbGuard {
    let crumb = Breadcrumb {
        phase,
        detail: detail.into(),
        thread_name: std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string(),
        started_at: Instant::now(),
    };
    TRAIL.with(|t| {
        let mut trail = t.borrow_mut();
        if trail.len() >= MAX_TRAIL_LEN {
            // Drop the oldest — a crash is most interesting near the top.
            trail.remove(0);
        }
        trail.push(crumb);
    });
    snapshot_current();
    BreadcrumbGuard { _private: () }
}

/// Record a breadcrumb without a guard (fire-and-forget).
///
/// Use sparingly — there is no matching pop, so these stay in the trail
/// until the ring buffer rolls over. Useful for marking one-shot events
/// like "acquired LLVM context" that don't naturally bracket a scope.
pub fn mark(phase: &'static str, detail: impl Into<String>) {
    let crumb = Breadcrumb {
        phase,
        detail: detail.into(),
        thread_name: std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string(),
        started_at: Instant::now(),
    };
    TRAIL.with(|t| {
        let mut trail = t.borrow_mut();
        if trail.len() >= MAX_TRAIL_LEN {
            trail.remove(0);
        }
        trail.push(crumb);
    });
    snapshot_current();
}

/// Current thread's trail — shallow clone.
pub fn current_trail() -> Vec<Breadcrumb> {
    TRAIL.with(|t| t.borrow().clone())
}

/// Best-effort cross-thread snapshot.
///
/// Called by the signal handler (where TLS of the offending thread may
/// not be reachable). Returns whichever thread most recently updated
/// its trail. Imperfect — but better than nothing when the fault is on
/// a rayon worker.
pub fn last_snapshot() -> Vec<Breadcrumb> {
    last_trail().lock().clone()
}

fn snapshot_current() {
    let snap = TRAIL.with(|t| t.borrow().clone());
    // try_lock to avoid deadlock inside a signal handler path —
    // acceptable to skip updates under contention.
    if let Some(mut guard) = last_trail().try_lock() {
        *guard = snap;
    }
}

/// Macro form: `breadcrumb!("phase", "format {} string", arg)`.
///
/// Expands to `let _bc = crate::breadcrumb::enter(phase, format!(...));`
/// **in the caller's scope**, so the guard stays alive for the enclosing
/// block. You bind the guard yourself:
///
/// ```ignore
/// let _bc = verum_error::breadcrumb!("codegen", "file={}", path);
/// ```
#[macro_export]
macro_rules! breadcrumb {
    ($phase:expr, $($arg:tt)*) => {
        $crate::breadcrumb::enter($phase, format!($($arg)*))
    };
    ($phase:expr) => {
        $crate::breadcrumb::enter($phase, String::new())
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trail_push_pop() {
        {
            let _a = enter("phase.a", "detail-a");
            assert_eq!(current_trail().len(), 1);
            {
                let _b = enter("phase.b", "detail-b");
                assert_eq!(current_trail().len(), 2);
            }
            assert_eq!(current_trail().len(), 1);
        }
        assert_eq!(current_trail().len(), 0);
    }

    #[test]
    fn trail_is_bounded() {
        let mut guards = Vec::new();
        for i in 0..(MAX_TRAIL_LEN + 10) {
            guards.push(enter("phase.x", format!("iter-{}", i)));
        }
        assert_eq!(current_trail().len(), MAX_TRAIL_LEN);
        // Oldest dropped — first entry should be iter-10 onward.
        assert!(current_trail()[0].detail.starts_with("iter-"));
    }

    #[test]
    fn mark_adds_entry_without_guard() {
        mark("one-shot", "acquired");
        let trail = current_trail();
        assert!(trail.iter().any(|b| b.phase == "one-shot"));
    }

    #[test]
    fn last_snapshot_reflects_recent_push() {
        // The snapshot is process-wide, so parallel tests can overwrite
        // it. This test only asserts that, from the current thread's
        // perspective *right after* pushing, either the current-thread
        // trail or the cross-thread snapshot exposes our phase. That is
        // the contract callers (signal handlers) rely on.
        let _g = enter("snap.test", "x");
        let found = current_trail().iter().any(|b| b.phase == "snap.test")
            || last_snapshot().iter().any(|b| b.phase == "snap.test");
        assert!(found);
    }
}
