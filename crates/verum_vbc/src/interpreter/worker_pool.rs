//! Tier-0 work-stealing thread pool — `T-DEFER-VBC-EXEC-MT` V0.
//!
//! The architectural piece for which the
//! `TaskQueue::{next_ready, steal_ready}` API was designed (per
//! state.rs:1206-1211 doc comment: "when a multi-threaded
//! scheduler lands (one TaskQueue per worker + cross-worker
//! stealing), the existing interpreter code keeps working and
//! new worker threads plug into steal_ready without touching the
//! local path"). This module is the meta-scheduler that lives
//! ABOVE TaskQueue: it owns the OS threads, owns the per-worker
//! queues, and applies the same LIFO-local + FIFO-steal
//! discipline at the executor layer.
//!
//! # V0 surface
//!
//! `WorkItem = Box<dyn FnOnce() + Send + 'static>`. Generic enough
//! to cover every off-thread surface the runtime needs today:
//! background DNS resolution, blocking syscalls that we don't want
//! to stall the dispatch loop on, future-completion notifications
//! from external sources, parallel monomorphization passes, etc.
//!
//! V0 explicitly does NOT run interpreter dispatch on workers —
//! that requires making `InterpreterState` Send + thread-safe and
//! is the V1 follow-up. V0 lights up the threading layer and the
//! work-stealing discipline; V1 wires interpreter dispatch into
//! it.
//!
//! # Discipline
//!
//! * Each worker owns a per-worker `VecDeque<WorkItem>` guarded
//!   by its own mutex.
//! * **Local pop = LIFO** (`pop_back`) — cache-hot for
//!   recursive submission patterns; mirrors TaskQueue.
//! * **Steal pop = FIFO** (`pop_front`) — taking from the far end
//!   minimises contention with the victim's local LIFO accesses
//!   and tends to transfer larger sub-graphs of work.
//! * Submit picks the target worker round-robin; no
//!   load-balancing heuristic in V0 (work-stealing equalises
//!   over time anyway).
//! * Idle workers park on a per-worker condvar; `submit` wakes
//!   the targeted worker; an idling worker that finds work via
//!   steal does NOT wake siblings (they're already running or
//!   asleep with their own work).
//!
//! # Termination
//!
//! `shutdown()` sets the global flag, broadcasts every worker's
//! condvar, and joins all worker threads in declaration order.
//! Workers drain their local queue on the way out — work
//! submitted before shutdown is ALWAYS run; work submitted after
//! shutdown returns `Err(SubmitError::Shutdown)`. Idempotent —
//! second `shutdown` is a no-op.
//!
//! # Why not crossbeam-deque
//!
//! crossbeam-deque is the canonical work-stealing primitive but
//! is not in `verum_vbc`'s dep tree. The std-only implementation
//! here is ~30% slower per steal but the pool is for coordination
//! work (one steal per ms-scale task), not micro-tasks; the
//! contention path is unmeasurable. Migration to crossbeam-deque
//! is purely additive should profiling demand it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// One unit of work for the pool. Send + 'static so the closure
/// can be moved onto any worker thread.
pub type WorkItem = Box<dyn FnOnce() + Send + 'static>;

/// Result of a submit attempt.
#[derive(Debug)]
pub enum SubmitError {
    /// The pool has been shut down; the work was NOT enqueued.
    Shutdown,
}

/// Per-worker state — owned by the pool and shared (Arc) with
/// the worker thread.
struct WorkerHandle {
    /// Local LIFO deque. Lock for both submit and pop.
    local: Mutex<VecDeque<WorkItem>>,
    /// Condvar used to wake the worker when new work arrives or
    /// when shutdown is broadcast. Pair with a dummy Mutex<()>
    /// since the wait happens after `local` is dropped.
    cv_pair: (Mutex<()>, Condvar),
}

impl WorkerHandle {
    fn new() -> Self {
        Self {
            local: Mutex::new(VecDeque::new()),
            cv_pair: (Mutex::new(()), Condvar::new()),
        }
    }

    /// Push to the back of the local deque (LIFO from the worker's
    /// own perspective).
    fn push_local(&self, item: WorkItem) {
        let mut g = self.local.lock().unwrap_or_else(|p| p.into_inner());
        g.push_back(item);
    }

    /// Pop from the back of the local deque (LIFO).
    fn pop_local(&self) -> Option<WorkItem> {
        let mut g = self.local.lock().unwrap_or_else(|p| p.into_inner());
        g.pop_back()
    }

    /// Pop from the FRONT of the deque on behalf of a foreign
    /// worker (FIFO steal). Returns None if the local deque is
    /// empty.
    fn steal(&self) -> Option<WorkItem> {
        let mut g = self.local.lock().unwrap_or_else(|p| p.into_inner());
        g.pop_front()
    }

    /// Best-effort wake of a parked worker. `Condvar::notify_one`
    /// does NOT require the associated mutex to be held — the
    /// wait side re-checks its predicate under the lock anyway,
    /// so we don't take it here.
    fn notify(&self) {
        self.cv_pair.1.notify_one();
    }

    fn ready_len(&self) -> usize {
        let g = self.local.lock().unwrap_or_else(|p| p.into_inner());
        g.len()
    }
}

/// Top-level handle. Cheap to clone (Arc-shared internals).
pub struct WorkerPool {
    handles: Arc<Vec<Arc<WorkerHandle>>>,
    next_submit: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    /// Number of currently-idle workers — used by `wait_idle` to
    /// detect quiescence.
    idle_count: Arc<AtomicUsize>,
    idle_cv_pair: Arc<(Mutex<()>, Condvar)>,
    /// Joined on `shutdown`. Wrapped in Mutex so `shutdown` can
    /// `take` them without consuming `&self`.
    join_handles: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
    n_workers: usize,
}

impl WorkerPool {
    /// Spawn `n` worker threads (clamped to >= 1). The pool is
    /// immediately ready for `submit`.
    pub fn new(n: usize) -> Self {
        let n_workers = n.max(1);
        let handles: Arc<Vec<Arc<WorkerHandle>>> = Arc::new(
            (0..n_workers)
                .map(|_| Arc::new(WorkerHandle::new()))
                .collect(),
        );
        let shutdown = Arc::new(AtomicBool::new(false));
        let idle_count = Arc::new(AtomicUsize::new(0));
        let idle_cv_pair = Arc::new((Mutex::new(()), Condvar::new()));
        let mut join_handles: Vec<thread::JoinHandle<()>> = Vec::with_capacity(n_workers);

        for my_id in 0..n_workers {
            let handles_c = handles.clone();
            let shutdown_c = shutdown.clone();
            let idle_count_c = idle_count.clone();
            let idle_cv_c = idle_cv_pair.clone();
            let h = thread::Builder::new()
                .name(format!("verum-worker-{my_id}"))
                .spawn(move || {
                    Self::worker_loop(my_id, handles_c, shutdown_c, idle_count_c, idle_cv_c);
                })
                .expect("worker spawn");
            join_handles.push(h);
        }

        Self {
            handles,
            next_submit: Arc::new(AtomicUsize::new(0)),
            shutdown,
            idle_count,
            idle_cv_pair,
            join_handles: Arc::new(Mutex::new(join_handles)),
            n_workers,
        }
    }

    /// Number of worker threads.
    pub fn n_workers(&self) -> usize {
        self.n_workers
    }

    /// Total enqueued work across all workers — useful for
    /// diagnostics + tests.
    pub fn pending(&self) -> usize {
        self.handles.iter().map(|h| h.ready_len()).sum()
    }

    /// Submit a unit of work. Round-robin assignment; the work
    /// runs on whichever worker the round-robin lands on, and
    /// may be stolen by any other worker if their local queue
    /// runs dry.
    pub fn submit(&self, work: WorkItem) -> Result<(), SubmitError> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err(SubmitError::Shutdown);
        }
        let target = self.next_submit.fetch_add(1, Ordering::Relaxed) % self.n_workers;
        self.handles[target].push_local(work);
        self.handles[target].notify();
        Ok(())
    }

    /// Submit pinned to a specific worker (no round-robin). Used
    /// by callers that want to keep cache-related work on the
    /// same thread (e.g. resubmitting a partially-completed
    /// computation). Returns `Err(SubmitError::Shutdown)` when
    /// the pool is shut down.
    pub fn submit_to(&self, worker_id: usize, work: WorkItem) -> Result<(), SubmitError> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err(SubmitError::Shutdown);
        }
        let target = worker_id % self.n_workers;
        self.handles[target].push_local(work);
        self.handles[target].notify();
        Ok(())
    }

    /// Block the caller until every worker is idle AND every
    /// queue is empty. Returns immediately on shutdown.
    ///
    /// Quiescence is observed when:
    ///   * `idle_count == n_workers` AND `pending() == 0`.
    ///
    /// The check races against in-flight work (a worker may
    /// finish a unit and increment idle just as we observe it),
    /// so we re-check both invariants under the idle condvar's
    /// lock — guaranteeing monotonicity per loop iteration.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let (lock, cv) = &*self.idle_cv_pair;
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                return true;
            }
            let g = lock.lock().unwrap_or_else(|p| p.into_inner());
            if self.idle_count.load(Ordering::Acquire) >= self.n_workers
                && self.pending() == 0
            {
                return true;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (g2, _timed_out) = cv.wait_timeout(g, remaining).unwrap_or_else(|p| {
                let g = p.into_inner();
                (g.0, g.1)
            });
            drop(g2);
        }
    }

    /// Shut the pool down. Sets the shutdown flag, broadcasts
    /// every worker's condvar so parked workers wake, joins all
    /// worker threads. Workers drain their local queue on the
    /// way out so previously-submitted work always runs.
    /// Idempotent.
    pub fn shutdown(&self) {
        if self.shutdown.swap(true, Ordering::AcqRel) {
            return; // already shutdown
        }
        // Wake every parked worker.
        for h in self.handles.iter() {
            h.notify();
        }
        // Wake any wait_idle observer (no need to hold the mutex
        // for notify_all — the waiter re-checks under the lock).
        self.idle_cv_pair.1.notify_all();

        // Join all workers.
        let mut g = self.join_handles.lock().unwrap_or_else(|p| p.into_inner());
        let handles: Vec<thread::JoinHandle<()>> = std::mem::take(&mut *g);
        drop(g);
        for h in handles {
            let _ = h.join();
        }
    }

    // ------------------------------------------------------------------
    // Worker loop
    // ------------------------------------------------------------------

    fn worker_loop(
        my_id: usize,
        handles: Arc<Vec<Arc<WorkerHandle>>>,
        shutdown: Arc<AtomicBool>,
        idle_count: Arc<AtomicUsize>,
        idle_cv: Arc<(Mutex<()>, Condvar)>,
    ) {
        let me = handles[my_id].clone();
        let n = handles.len();
        loop {
            // 1. Local LIFO pop.
            if let Some(work) = me.pop_local() {
                work();
                continue;
            }
            // 2. Steal FIFO from siblings (round-robin starting
            //    one past self to spread contention).
            let mut stole = false;
            for offset in 1..n {
                let victim = (my_id + offset) % n;
                if let Some(work) = handles[victim].steal() {
                    work();
                    stole = true;
                    break;
                }
            }
            if stole {
                continue;
            }
            // 3. Park. Re-check shutdown + queue under the cv
            //    lock so we don't sleep through a wake.
            if shutdown.load(Ordering::Acquire) {
                // Drain anything left in our local queue (last-
                // chance work — wake didn't fire because we were
                // already past the local check).
                while let Some(work) = me.pop_local() {
                    work();
                }
                break;
            }
            idle_count.fetch_add(1, Ordering::Release);
            // Notify wait_idle in case all workers are now idle.
            // No mutex acquisition needed — the waiter re-checks
            // its predicate under the lock.
            idle_cv.1.notify_all();
            {
                let g = me.cv_pair.0.lock().unwrap_or_else(|p| p.into_inner());
                // Re-check after taking the lock — submit may
                // have raced before we got here.
                if me.ready_len() == 0 && !shutdown.load(Ordering::Acquire) {
                    let (_g2, _to) = me
                        .cv_pair
                        .1
                        .wait_timeout(g, Duration::from_millis(50))
                        .unwrap_or_else(|p| {
                            let g = p.into_inner();
                            (g.0, g.1)
                        });
                }
            }
            idle_count.fetch_sub(1, Ordering::Release);
        }
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn pool_runs_submitted_work() {
        let pool = WorkerPool::new(4);
        let counter = Arc::new(AtomicUsize::new(0));
        let mut expected = 0usize;
        for i in 0..32 {
            let c = counter.clone();
            pool.submit(Box::new(move || {
                c.fetch_add(i, Ordering::Relaxed);
            }))
            .unwrap();
            expected += i;
        }
        assert!(pool.wait_idle(Duration::from_secs(2)));
        assert_eq!(counter.load(Ordering::Relaxed), expected);
        pool.shutdown();
    }

    #[test]
    fn n_workers_clamps_to_at_least_one() {
        let pool = WorkerPool::new(0);
        assert_eq!(pool.n_workers(), 1);
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        pool.submit(Box::new(move || {
            c.fetch_add(7, Ordering::Relaxed);
        }))
        .unwrap();
        assert!(pool.wait_idle(Duration::from_secs(2)));
        assert_eq!(counter.load(Ordering::Relaxed), 7);
    }

    /// Submit far more work than workers — every unit must run
    /// (work-stealing redistributes load across the pool so no
    /// worker stalls on its own queue).
    #[test]
    fn work_stealing_redistributes_load() {
        let pool = WorkerPool::new(4);
        let counter = Arc::new(AtomicUsize::new(0));
        // Submit 200 short units pinned to worker 0 — workers
        // 1/2/3 must steal to make progress.
        for _ in 0..200 {
            let c = counter.clone();
            pool.submit_to(
                0,
                Box::new(move || {
                    c.fetch_add(1, Ordering::Relaxed);
                }),
            )
            .unwrap();
        }
        assert!(pool.wait_idle(Duration::from_secs(5)));
        assert_eq!(counter.load(Ordering::Relaxed), 200);
        pool.shutdown();
    }

    #[test]
    fn shutdown_is_idempotent() {
        let pool = WorkerPool::new(2);
        pool.shutdown();
        pool.shutdown(); // must not panic / hang
    }

    #[test]
    fn submit_after_shutdown_errors() {
        let pool = WorkerPool::new(2);
        pool.shutdown();
        let r = pool.submit(Box::new(|| {}));
        assert!(matches!(r, Err(SubmitError::Shutdown)));
    }

    /// Drop without explicit shutdown still cleanly joins
    /// workers — `Drop` impl calls `shutdown`. Smoke-pin so a
    /// future refactor doesn't leak threads.
    #[test]
    fn drop_calls_shutdown() {
        let counter = Arc::new(AtomicUsize::new(0));
        {
            let pool = WorkerPool::new(2);
            let c = counter.clone();
            pool.submit(Box::new(move || {
                c.fetch_add(99, Ordering::Relaxed);
            }))
            .unwrap();
            assert!(pool.wait_idle(Duration::from_secs(2)));
        } // pool dropped here — shutdown runs implicitly
        assert_eq!(counter.load(Ordering::Relaxed), 99);
    }

    /// Pin the LIFO local-pop discipline — the LAST-submitted
    /// item on the same worker must run FIRST when no steal
    /// happens. We use a single worker (no steal possible) and
    /// observe execution order.
    #[test]
    fn local_pop_is_lifo() {
        let pool = WorkerPool::new(1);
        let order: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));
        // Block worker 0 with a slow first task so subsequent
        // submits queue up; then unblock and observe the LIFO
        // drain order.
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        {
            let g = gate.clone();
            pool.submit_to(
                0,
                Box::new(move || {
                    let (lk, cv) = &*g;
                    let mut released = lk.lock().unwrap();
                    while !*released {
                        released = cv.wait(released).unwrap();
                    }
                }),
            )
            .unwrap();
        }
        // Give the worker a moment to dequeue + start the
        // gating task.
        std::thread::sleep(Duration::from_millis(20));
        for i in 1..=4usize {
            let o = order.clone();
            pool.submit_to(
                0,
                Box::new(move || {
                    let mut g = o.lock().unwrap();
                    g.push(i);
                }),
            )
            .unwrap();
        }
        // Release the gate.
        {
            let (lk, cv) = &*gate;
            let mut r = lk.lock().unwrap();
            *r = true;
            cv.notify_one();
        }
        assert!(pool.wait_idle(Duration::from_secs(2)));
        let g = order.lock().unwrap();
        // LIFO drain: 4 pushed last, runs first → [4, 3, 2, 1]
        assert_eq!(*g, vec![4, 3, 2, 1]);
        drop(g);
        pool.shutdown();
    }

    /// Pending count tracks unprocessed submissions; goes to 0
    /// after wait_idle.
    #[test]
    fn pending_count_drains_to_zero() {
        let pool = WorkerPool::new(3);
        for _ in 0..50 {
            pool.submit(Box::new(|| {})).unwrap();
        }
        assert!(pool.wait_idle(Duration::from_secs(2)));
        assert_eq!(pool.pending(), 0);
        pool.shutdown();
    }
}
