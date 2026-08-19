//! Allocates past the ceiling on purpose, so the guard can be watched
//! working. Run with a small ceiling:
//!
//! ```text
//! VERUM_MEMORY_CEILING_MB=64 cargo run -p verum_common --example memory_ceiling_probe
//! ```
#[global_allocator]
static ALLOC: verum_common::memory_budget::BudgetedAllocator =
    verum_common::memory_budget::BudgetedAllocator::new();

fn main() {
    let mut held: Vec<Vec<u8>> = Vec::new();
    for i in 0..16 {
        held.push(vec![7u8; 16 * 1024 * 1024]);
        eprintln!("probe: {} blocks, live {} MB", i + 1, verum_common::memory_budget::live_bytes() / (1024 * 1024));
    }
    println!("probe survived with {} blocks — the ceiling did NOT fire", held.len());
}
