//! Demo of Error Context Protocol features
//!
//! Run with:
//! ```
//! cargo run --example error_context_demo
//! VERUM_BACKTRACE=1 cargo run --example error_context_demo
//! ```

use std::io;
use verum_diagnostics::context;
use verum_diagnostics::context_protocol::*;

// Simulated operations that can fail
fn connect_database() -> Result<(), io::Error> {
    Err(io::Error::new(
        io::ErrorKind::ConnectionRefused,
        "Connection refused",
    ))
}

fn load_user_data(user_id: &str) -> Result<String, ErrorWithContext<io::Error>> {
    connect_database().with_context(|| {
        format!(
            "Failed to connect to database\n\
             Host: db-primary.example.com:5432\n\
             User: {}\n\
             Pool: 0 active / 10 max",
            user_id
        )
    })?;
    Ok(String::from("user data"))
}

fn process_user(user_id: &str) -> Result<(), ErrorWithContext<io::Error>> {
    let _data = load_user_data(user_id)?;

    Ok(())
}

fn handle_request(user_id: &str) -> Result<(), ErrorWithContext<io::Error>> {
    process_user(user_id)
        .map_err(|e| e.with_metadata("user_id", ContextValue::Text(user_id.into())))?;

    Ok(())
}

fn main() {
    println!("=== Error Context Protocol Demo ===\n");

    // Trigger an error
    match handle_request("usr_12345") {
        Ok(_) => println!("Success!"),
        Err(e) => {
            println!("--- FULL FORMAT ---");
            println!("{}\n", e.display_full());

            println!("--- USER FORMAT ---");
            println!("{}\n", e.display_user());

            println!("--- DEVELOPER FORMAT ---");
            println!("{}\n", e.display_developer());

            println!("--- LOG FORMAT ---");
            println!("{}\n", e.display_log());

            println!("--- ERROR DETAILS ---");
            println!("Context message: {}", e.context.message);
            println!("Location: {}", e.context.location);
            println!("Context chain depth: {}", e.context.context_chain.len());
            println!("Metadata entries: {}", e.context.metadata.len());

            if let Some(bt) = e.backtrace() {
                println!("\nBacktrace available with {} frames", bt.frames().len());
                println!("(Set VERUM_BACKTRACE=1 to see backtrace)");
            } else {
                println!("\nNo backtrace (set VERUM_BACKTRACE=1 to enable)");
            }
        }
    }

    println!("\n=== Performance Demonstration ===\n");

    // Success path - should have zero overhead
    let start = std::time::Instant::now();
    for i in 0..1_000_000 {
        let result: Result<i32, io::Error> = Ok(i);
        let _ = result.with_context(|| format!("Expensive formatting: {}", i * 2));
    }
    let elapsed = start.elapsed();
    println!("Success path: 1M iterations in {:?}", elapsed);
    println!(
        "Per operation: ~{}ns (should be near-zero)",
        elapsed.as_nanos() / 1_000_000
    );

    println!("\n=== Macro Demo ===\n");

    fn demo_macro() -> Result<(), ErrorWithContext<io::Error>> {
        let result: Result<(), io::Error> =
            Err(io::Error::new(io::ErrorKind::NotFound, "not found"));
        context!(result, "Using context! macro")?;
        Ok(())
    }

    match demo_macro() {
        Ok(_) => {}
        Err(e) => println!("Macro error: {}", e.display_user()),
    }

    println!("\n=== Integration Examples ===\n");

    // Show different error types
    let io_err: Result<(), io::Error> = Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "permission denied",
    ));
    match io_err.context("File operation failed") {
        Ok(_) => {}
        Err(e) => println!("IO Error with context: {}", e.display_user()),
    }

    println!("\n=== Demo Complete ===");
}
