//! Backend Switching Examples - Complete Usage Guide
//!
//! This example demonstrates all backend switching capabilities:
//! - Manual backend selection
//! - Automatic backend selection
//! - Fallback strategies
//! - Portfolio solving
//! - Cross-validation
//! - Configuration management
//!
//! Run with:
//! ```bash
//! cargo run --example backend_switching --features cvc5
//! ```
//!
//! NOTE: Requires the `cvc5` feature to be enabled

#[cfg(not(feature = "cvc5"))]
fn main() {
    println!("This example requires the `cvc5` feature.");
    println!("Run with: cargo run --example backend_switching --features cvc5");
}

#[cfg(feature = "cvc5")]
use verum_smt::{
    BackendChoice, BackendFallbackConfig, BackendPortfolioConfig, PortfolioMode,
    SmtBackendSwitcher, SmtConfig, BackendValidationConfig,
};

#[cfg(feature = "cvc5")]
fn main() {
    println!("=== Backend Switching Examples ===\n");

    // Example 1: Manual Z3 Selection
    example_1_manual_z3();

    // Example 2: Manual CVC5 Selection
    example_2_manual_cvc5();

    // Example 3: Automatic Selection
    example_3_auto_selection();

    // Example 4: Fallback Strategy
    example_4_fallback();

    // Example 5: Portfolio Solving
    example_5_portfolio();

    // Example 6: Cross-Validation
    example_6_cross_validation();

    // Example 7: Configuration Files
    example_7_config_files();

    // Example 8: Environment Variables
    example_8_env_variables();

    // Example 9: Configuration Presets
    example_9_presets();

    println!("\n=== All Examples Completed ===");
}

/// Example 1: Manual Z3 Selection
#[cfg(feature = "cvc5")]
fn example_1_manual_z3() {
    println!("--- Example 1: Manual Z3 Selection ---");

    let config = SmtConfig {
        backend: BackendChoice::Z3,
        timeout_ms: 10000, // 10 seconds
        verbose: true,
        ..Default::default()
    };

    let mut switcher = SmtBackendSwitcher::new(config.to_switcher_config());

    println!("✓ Created switcher with Z3 backend");
    println!("  Backend: {:?}", switcher.current_backend());
    println!("  Configuration: Z3 with 10s timeout\n");

    // Solve example problem (empty assertions = trivially SAT).
    // Real code would construct a List<Expr> with actual assertions.
    let assertions = verum_common::List::<verum_ast::Expr>::new();
    let result = switcher.solve(&assertions);

    println!("  Result: {:?}\n", result);
}

/// Example 2: Manual CVC5 Selection
#[cfg(feature = "cvc5")]
fn example_2_manual_cvc5() {
    println!("--- Example 2: Manual CVC5 Selection ---");

    let config = SmtConfig {
        backend: BackendChoice::Cvc5,
        timeout_ms: 10000,
        verbose: true,
        ..Default::default()
    };

    let switcher = SmtBackendSwitcher::new(config.to_switcher_config());

    println!("✓ Created switcher with CVC5 backend");
    println!("  Backend: {:?}", switcher.current_backend());
    println!("  Configuration: CVC5 with 10s timeout\n");
}

/// Example 3: Automatic Selection
#[cfg(feature = "cvc5")]
fn example_3_auto_selection() {
    println!("--- Example 3: Automatic Selection ---");

    let config = SmtConfig {
        backend: BackendChoice::Auto,
        timeout_ms: 10000,
        verbose: true,
        ..Default::default()
    };

    let _switcher = SmtBackendSwitcher::new(config.to_switcher_config());

    println!("✓ Created switcher with Auto backend");
    println!("  The solver will automatically choose Z3 or CVC5");
    println!("  based on problem characteristics\n");
}

/// Example 4: Fallback Strategy
#[cfg(feature = "cvc5")]
fn example_4_fallback() {
    println!("--- Example 4: Fallback Strategy ---");

    let config = SmtConfig {
        backend: BackendChoice::Z3,
        timeout_ms: 10000,
        verbose: true,
        fallback: BackendFallbackConfig {
            enabled: true,
            on_timeout: true,
            on_unknown: true,
            on_error: true,
            max_attempts: 3,
        },
        ..Default::default()
    };

    let _switcher = SmtBackendSwitcher::new(config.to_switcher_config());

    println!("✓ Created switcher with Z3 + fallback to CVC5");
    println!("  Fallback triggers:");
    println!("    - On timeout: {}", config.fallback.on_timeout);
    println!("    - On unknown: {}", config.fallback.on_unknown);
    println!("    - On error: {}", config.fallback.on_error);
    println!("    - Max attempts: {}", config.fallback.max_attempts);
    println!();

    // Simulate timeout scenario
    println!("  Simulating timeout scenario...");
    println!("  Z3 would timeout → fallback to CVC5");
    println!("  CVC5 would solve successfully ✓\n");
}

/// Example 5: Portfolio Solving
#[cfg(feature = "cvc5")]
fn example_5_portfolio() {
    println!("--- Example 5: Portfolio Solving ---");

    let config = SmtConfig {
        backend: BackendChoice::Portfolio,
        timeout_ms: 30000,
        verbose: true,
        portfolio: BackendPortfolioConfig {
            enabled: true,
            mode: PortfolioMode::FirstResult,
            max_threads: 2,
            timeout_per_solver: 30000,
            kill_on_first: true,
        },
        ..Default::default()
    };

    let _switcher = SmtBackendSwitcher::new(config.to_switcher_config());

    println!("✓ Created portfolio solver");
    println!("  Mode: FirstResult (return first SAT/UNSAT)");
    println!("  Threads: {}", config.portfolio.max_threads);
    println!(
        "  Timeout per solver: {}ms",
        config.portfolio.timeout_per_solver
    );
    println!();

    println!("  Expected behavior:");
    println!("    1. Launch Z3 in thread 1");
    println!("    2. Launch CVC5 in thread 2");
    println!("    3. Return first result");
    println!("    4. Kill other thread");
    println!();

    // Try Consensus mode
    println!("  Alternative: Consensus mode");
    let _consensus_config = SmtConfig {
        backend: BackendChoice::Portfolio,
        portfolio: BackendPortfolioConfig {
            enabled: true,
            mode: PortfolioMode::Consensus,
            ..config.portfolio
        },
        ..config
    };

    println!("    - Wait for both Z3 and CVC5");
    println!("    - Verify they agree");
    println!("    - Fail if they disagree\n");
}

/// Example 6: Cross-Validation
#[cfg(feature = "cvc5")]
fn example_6_cross_validation() {
    println!("--- Example 6: Cross-Validation ---");

    let config = SmtConfig {
        backend: BackendChoice::Z3,
        timeout_ms: 30000,
        verbose: true,
        validation: BackendValidationConfig {
            enabled: true,
            cross_validate: true,
            fail_on_mismatch: true,
            log_mismatches: true,
        },
        ..Default::default()
    };

    println!("✓ Created solver with cross-validation");
    println!("  Primary: Z3");
    println!("  Validator: CVC5");
    println!("  Fail on mismatch: {}", config.validation.fail_on_mismatch);
    println!("  Log mismatches: {}", config.validation.log_mismatches);
    println!();

    println!("  Workflow:");
    println!("    1. Solve with Z3");
    println!("    2. Independently solve with CVC5");
    println!("    3. Compare results");
    println!("    4. Return Z3 result if match");
    println!("    5. Error if mismatch\n");
}

/// Example 7: Configuration Files
#[cfg(feature = "cvc5")]
fn example_7_config_files() {
    println!("--- Example 7: Configuration Files ---");

    // Create default configuration
    let config = SmtConfig::default();

    // Save to TOML
    match config.to_toml_file("/tmp/verum_smt.toml") {
        Ok(_) => {
            println!("✓ Saved configuration to /tmp/verum_smt.toml");
            println!("  Backend: {:?}", config.backend);
            println!("  Timeout: {}ms", config.timeout_ms);
        }
        Err(e) => {
            println!("✗ Failed to save TOML: {}", e);
        }
    }

    // Save to JSON
    match config.to_json_file("/tmp/verum_smt.json") {
        Ok(_) => {
            println!("✓ Saved configuration to /tmp/verum_smt.json");
        }
        Err(e) => {
            println!("✗ Failed to save JSON: {}", e);
        }
    }

    // Load from TOML
    match SmtConfig::from_toml_file("/tmp/verum_smt.toml") {
        Ok(loaded) => {
            println!("✓ Loaded configuration from TOML");
            println!("  Backend: {:?}", loaded.backend);
        }
        Err(e) => {
            println!("✗ Failed to load TOML: {}", e);
        }
    }

    println!();
}

/// Example 8: Environment Variables
#[cfg(feature = "cvc5")]
fn example_8_env_variables() {
    println!("--- Example 8: Environment Variables ---");

    // Set example environment variables.
    // SAFETY (edition 2024): `set_var` is unsafe because modifying the
    // process environment is not thread-safe. In this single-threaded
    // example it's fine.
    unsafe {
        std::env::set_var("VERUM_SMT_BACKEND", "portfolio");
        std::env::set_var("VERUM_SMT_TIMEOUT", "60000");
        std::env::set_var("VERUM_SMT_VERBOSE", "true");
        std::env::set_var("VERUM_SMT_FALLBACK", "true");
        std::env::set_var("VERUM_SMT_PORTFOLIO_MODE", "consensus");
    }

    // Load configuration from environment
    let config = SmtConfig::from_env();

    println!("✓ Loaded configuration from environment variables");
    println!("  VERUM_SMT_BACKEND={:?}", config.backend);
    println!("  VERUM_SMT_TIMEOUT={}ms", config.timeout_ms);
    println!("  VERUM_SMT_VERBOSE={}", config.verbose);
    println!("  VERUM_SMT_FALLBACK={}", config.fallback.enabled);
    println!("  VERUM_SMT_PORTFOLIO_MODE={:?}", config.portfolio.mode);
    println!();

    println!("  Available environment variables:");
    println!("    - VERUM_SMT_BACKEND: z3, cvc5, auto, portfolio");
    println!("    - VERUM_SMT_TIMEOUT: milliseconds");
    println!("    - VERUM_SMT_VERBOSE: true/false");
    println!("    - VERUM_SMT_FALLBACK: true/false");
    println!("    - VERUM_SMT_FALLBACK_ON_TIMEOUT: true/false");
    println!("    - VERUM_SMT_FALLBACK_ON_UNKNOWN: true/false");
    println!("    - VERUM_SMT_FALLBACK_ON_ERROR: true/false");
    println!("    - VERUM_SMT_PORTFOLIO_MODE: first, consensus, vote");
    println!("    - VERUM_SMT_PORTFOLIO_THREADS: number");
    println!("    - VERUM_SMT_CROSS_VALIDATE: true/false");
    println!();
}

/// Example 9: Configuration Presets
#[cfg(feature = "cvc5")]
fn example_9_presets() {
    println!("--- Example 9: Configuration Presets ---");

    // Development preset
    let dev = SmtConfig::development();
    println!("✓ Development Preset");
    println!("  Backend: {:?}", dev.backend);
    println!("  Timeout: {}ms", dev.timeout_ms);
    println!("  Fallback: {}", dev.fallback.enabled);
    println!("  Portfolio: {}", dev.portfolio.enabled);
    println!("  Proofs: {}", dev.z3.enable_proofs);
    println!();

    // Production preset
    let prod = SmtConfig::production();
    println!("✓ Production Preset");
    println!("  Backend: {:?}", prod.backend);
    println!("  Timeout: {}ms", prod.timeout_ms);
    println!("  Fallback: {}", prod.fallback.enabled);
    println!("  Validation: {}", prod.validation.enabled);
    println!("  Cross-validate: {}", prod.validation.cross_validate);
    println!();

    // Performance preset
    let perf = SmtConfig::performance();
    println!("✓ Performance Preset");
    println!("  Backend: {:?}", perf.backend);
    println!("  Portfolio: {}", perf.portfolio.enabled);
    println!("  Mode: {:?}", perf.portfolio.mode);
    println!("  Threads: {}", perf.portfolio.max_threads);
    println!();

    // Debugging preset
    let debug = SmtConfig::debugging();
    println!("✓ Debugging Preset");
    println!("  Backend: {:?}", debug.backend);
    println!("  Timeout: {}ms", debug.timeout_ms);
    println!("  Verbose: {}", debug.verbose);
    println!("  Validation: {}", debug.validation.enabled);
    println!("  CVC5 verbosity: {}", debug.cvc5.verbosity);
    println!();
}
