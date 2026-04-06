#![allow(clippy::disallowed_types)] // Build scripts can use String/Vec directly

//! Build script for verum_cli.
//!
//! Sets up environment variables for the CLI.

use std::env;

fn main() {
    // Export TARGET for runtime detection
    let target = env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=VERUM_HOST_TARGET={}", target);
    println!("cargo:rustc-env=TARGET={}", target);

    println!("cargo:rerun-if-changed=build.rs");
}
