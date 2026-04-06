//! Panic Unwinding Examples
//!
//! Demonstrates the complete panic handling and unwinding system.

use verum_runtime::{
    Runtime, RuntimeConfig, PanicMode,
    panic::{PanicHandler, panic_stats},
    panic_hooks::{set_panic_hook, PanicHookBuilder, FileHook},
    unwinding::{with_unwind_guard, UnwindGuard},
};
use verum_std::core::Text;

/// Example 1: Basic panic handling with different modes
fn example_panic_modes() {
    println!("=== Example 1: Panic Modes ===\n");

    // Abort mode (production default)
    println!("1. Abort Mode:");
    let abort_handler = PanicHandler::new(PanicMode::Abort);
    println!("   - Terminates immediately on panic");
    println!("   - No unwinding, minimal overhead");
    println!("   - Best for production\n");

    // Unwind mode (debugging)
    println!("2. Unwind Mode:");
    let unwind_handler = PanicHandler::new(PanicMode::Unwind);
    println!("   - Unwinds stack with cleanup");
    println!("   - CBGR cleanup performed");
    println!("   - Stack traces available");
    println!("   - Best for debugging\n");

    // Catch mode (testing)
    println!("3. Catch Mode:");
    let catch_handler = PanicHandler::new(PanicMode::CatchAndLog);
    println!("   - Captures panic for inspection");
    println!("   - Allows tests to continue");
    println!("   - Best for testing\n");
}

/// Example 2: Custom panic hooks
fn example_panic_hooks() {
    println!("=== Example 2: Custom Panic Hooks ===\n");

    // Simple hook
    println!("1. Simple Hook:");
    set_panic_hook(|info| {
        eprintln!("CUSTOM PANIC: {}", info.message);
    });
    println!("   - Registered custom panic hook\n");

    // Builder pattern
    println!("2. Builder Pattern:");
    PanicHookBuilder::new()
        .with_stderr_output()
        .with_location_logging()
        .with_backtrace_logging()
        .with_statistics()
        .build();
    println!("   - Built composable panic hook\n");

    // File logging hook
    println!("3. File Logging:");
    if let Ok(file_hook) = FileHook::new("/tmp/verum_panics.log") {
        println!("   - Logging panics to {}", file_hook.path().display());
    }
    println!();
}

/// Example 3: Panic statistics
fn example_panic_stats() {
    println!("=== Example 3: Panic Statistics ===\n");

    let stats = panic_stats();

    println!("Total panics: {}", stats.total_panics());

    if let verum_std::core::Maybe::Some(rate) = stats.panic_rate() {
        println!("Panic rate: {:.2} panics/second", rate);
    }

    if let verum_std::core::Maybe::Some(mtbp) = stats.mean_time_between_panics() {
        println!("Mean time between panics: {:.2} ms", mtbp);
    }

    println!("\nTop panic locations:");
    for (location, count) in stats.top_panic_locations(5).iter() {
        println!("  {} - {} panics", location, count);
    }

    println!("\nPanics by thread:");
    for (thread, count) in stats.panics_by_thread().iter() {
        println!("  {} - {} panics", thread, count);
    }
    println!();
}

/// Example 4: Unwind guards for RAII cleanup
fn example_unwind_guards() {
    println!("=== Example 4: Unwind Guards ===\n");

    println!("1. Basic Guard:");
    let result = with_unwind_guard(
        || {
            println!("   - Executing protected code");
            42
        },
        || {
            println!("   - Cleanup on unwind");
        }
    );
    println!("   - Result: {}\n", result);

    println!("2. Manual Guard:");
    {
        let guard = UnwindGuard::new(|| {
            println!("   - Guard cleanup");
        });

        println!("   - Doing work...");

        guard.defuse(); // Success - no cleanup
    }
    println!();
}

/// Example 5: CBGR cleanup during unwinding
fn example_cbgr_cleanup() {
    println!("=== Example 5: CBGR Cleanup ===\n");

    use verum_cbgr::panic_cleanup::{with_cbgr_guard, panic_cleanup_stats};

    println!("Executing code with CBGR guard:");
    let result = with_cbgr_guard(|| {
        // This would allocate CBGR-managed memory
        // and ensure cleanup on panic
        println!("  - Allocating CBGR memory");
        println!("  - Performing operations");
        println!("  - Cleanup guaranteed on panic");
        42
    });

    println!("  - Result: {}", result);

    let stats = panic_cleanup_stats();
    println!("\nCBGR Cleanup Statistics:");
    println!("  - Total cleanups: {}", stats.total_cleanups());
    println!("  - Allocations cleaned: {}", stats.total_allocations_cleaned());
    println!();
}

/// Example 6: Production panic configuration
fn example_production_config() {
    println!("=== Example 6: Production Configuration ===\n");

    let mut config = RuntimeConfig::production();
    config.panic_mode = PanicMode::Abort;

    println!("Production Configuration:");
    println!("  - Panic mode: Abort");
    println!("  - No unwinding overhead");
    println!("  - Fast failure");
    println!("  - Minimal memory usage");
    println!("\nRecommended for production deployments\n");
}

/// Example 7: Development panic configuration
fn example_development_config() {
    println!("=== Example 7: Development Configuration ===\n");

    let mut config = RuntimeConfig::development();
    config.panic_mode = PanicMode::Unwind;

    println!("Development Configuration:");
    println!("  - Panic mode: Unwind");
    println!("  - Full stack traces");
    println!("  - CBGR cleanup");
    println!("  - Detailed debugging info");
    println!("\nRecommended for development and debugging\n");
}

/// Example 8: Integration with runtime
fn example_runtime_integration() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Example 8: Runtime Integration ===\n");

    let mut config = RuntimeConfig::default();
    config.panic_mode = PanicMode::Unwind;

    let runtime = Runtime::new(config)?;

    // Install custom panic hook
    runtime.panic_handler().install_hook();

    println!("Runtime configured with:");
    println!("  - Panic mode: {:?}", runtime.panic_handler().mode());
    println!("  - Custom hooks installed");
    println!("  - Statistics tracking enabled");

    // Get panic statistics
    let stats = runtime.panic_handler().panic_history();
    println!("  - Panic history: {} entries", stats.len());
    println!();

    Ok(())
}

/// Example 9: Thread-specific panic handling
fn example_thread_specific() {
    use verum_runtime::panic_hooks::thread_specific_hook;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    println!("=== Example 9: Thread-Specific Hooks ===\n");

    let worker_panic_count = Arc::new(AtomicU64::new(0));
    let counter_clone = worker_panic_count.clone();

    let hook = thread_specific_hook(
        Text::from("worker"),
        move |info| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
            eprintln!("Worker thread panic: {}", info.message);
        }
    );

    println!("  - Registered thread-specific hook for 'worker' thread");
    println!("  - Hook only fires for panics in that thread");
    println!();
}

/// Example 10: Monitoring and alerting
fn example_monitoring() {
    println!("=== Example 10: Monitoring & Alerting ===\n");

    let stats = panic_stats();

    // Check if panic rate is concerning
    if let verum_std::core::Maybe::Some(rate) = stats.panic_rate() {
        println!("Current panic rate: {:.2}/sec", rate);

        if rate > 1.0 {
            println!("⚠️  WARNING: High panic rate detected!");
            println!("   Consider investigating:");

            let top_locations = stats.top_panic_locations(3);
            for (location, count) in top_locations.iter() {
                println!("   - {} ({} panics)", location, count);
            }
        } else {
            println!("✓ Panic rate is normal");
        }
    }

    println!();
}

fn main() {
    println!("\n╔═══════════════════════════════════════╗");
    println!("║  Verum Panic Unwinding Examples      ║");
    println!("╚═══════════════════════════════════════╝\n");

    example_panic_modes();
    example_panic_hooks();
    example_panic_stats();
    example_unwind_guards();
    example_cbgr_cleanup();
    example_production_config();
    example_development_config();

    if let Err(e) = example_runtime_integration() {
        eprintln!("Runtime integration error: {}", e);
    }

    example_thread_specific();
    example_monitoring();

    println!("═══════════════════════════════════════\n");
    println!("All examples completed successfully!");
    println!("\nKey Takeaways:");
    println!("  1. Use Abort mode in production for fast failure");
    println!("  2. Use Unwind mode in development for debugging");
    println!("  3. Install custom hooks for logging/alerting");
    println!("  4. Monitor panic statistics for health checks");
    println!("  5. Use guards to ensure cleanup on panic");
    println!("  6. CBGR cleanup is automatic during unwinding");
    println!();
}
