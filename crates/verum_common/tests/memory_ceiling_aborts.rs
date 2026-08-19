//! The seatbelt has to be shown to fasten.
//!
//! `BudgetedAllocator` ends the process when live bytes cross the
//! ceiling, so the behaviour cannot be asserted in-process: the
//! assertion would die with it. The test therefore re-runs ITSELF as a
//! child with a deliberately tiny ceiling (64 MB), has the child
//! allocate past it, and checks that the child stopped the way it
//! promises to — with the ceiling's own exit status and the ceiling
//! message on stderr.
//!
//! The megabyte knob exists for this: an earlier version used the
//! smallest GB ceiling and had to allocate a gigabyte to see the
//! seatbelt fasten, which on a loaded machine hung instead of
//! finishing. A guard's test must be cheap, or it stops being run.
//!
//! Without this, the ceiling would be a comment. The defect it guards
//! against is precisely a guard that does not guard: `ulimit -v` on
//! macOS reports success, leaves the limit unlimited, and lets the
//! machine fill up — which is why this allocator exists at all.

use std::process::Command;

/// Set in the child to select the allocating branch.
const CHILD_MARKER: &str = "VERUM_MEMORY_CEILING_TEST_CHILD";

#[global_allocator]
static ALLOC: verum_common::memory_budget::BudgetedAllocator =
    verum_common::memory_budget::BudgetedAllocator::new();

#[test]
fn crossing_the_ceiling_aborts_with_a_legible_message() {
    if std::env::var(CHILD_MARKER).is_ok() {
        // Child: allocate past a 1 GB ceiling in chunks the allocator
        // sees one at a time, and keep them live so the count climbs.
        let mut held: Vec<Vec<u8>> = Vec::new();
        for _ in 0..8 {
            held.push(vec![7u8; 16 * 1024 * 1024]);
        }
        // Unreachable when the ceiling works; touch the data so the
        // optimiser cannot elide the allocations if it ever does not.
        println!("child survived with {} blocks", held.len());
        return;
    }

    let exe = std::env::current_exe().expect("test binary path");
    let output = Command::new(exe)
        .arg("crossing_the_ceiling_aborts_with_a_legible_message")
        .arg("--exact")
        .arg("--nocapture")
        .env(CHILD_MARKER, "1")
        .env("VERUM_MEMORY_CEILING_MB", "64")
        .output()
        .expect("spawn child");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(verum_common::memory_budget::EXIT_CODE_MEMORY_CEILING),
        "child should stop with the ceiling's own status; stdout was:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        stderr.contains("MEMORY CEILING REACHED"),
        "child died without the ceiling message; stderr was:\n{}",
        stderr
    );
    assert!(
        stderr.contains("VERUM_MEMORY_CEILING_GB"),
        "the message must name the knob that changes it; stderr was:\n{}",
        stderr
    );
}
