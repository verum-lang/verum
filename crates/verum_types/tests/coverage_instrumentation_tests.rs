#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! Coverage instrumentation drift guard (#60).
//!
//! `vcs/runner/vtest/src/executor.rs` wires the `--coverage` flag end-to-end:
//!
//!   * `RunnerConfig.coverage: bool` (lib.rs) — set from `opts.coverage` in main.rs.
//!   * `ExecutorConfig.coverage: bool` — propagated from RunnerConfig.
//!   * `ExecutorConfig.coverage_output: Option<PathBuf>` — path for lcov output.
//!   * In `run_process()`: when `coverage == true`, sets `VERUM_COVERAGE=1` in the
//!     subprocess environment so the runtime emits per-line hit counters.
//!   * Also sets `VERUM_COVERAGE_OUTPUT=<path>` when `coverage_output` is set.
//!
//! This drift guard pins:
//!   1. `ExecutorConfig` has `coverage: bool` field.
//!   2. `ExecutorConfig` has `coverage_output: Option<PathBuf>` field.
//!   3. `ExecutorConfig::default()` sets `coverage: false`.
//!   4. `run_process` (or a coverage injection point) references `VERUM_COVERAGE`.
//!   5. `VERUM_COVERAGE_OUTPUT` is also referenced.
//!   6. `RunnerConfig` has `coverage: bool` in lib.rs.
//!   7. main.rs propagates `opts.coverage` to `executor_config.coverage`.

const EXECUTOR_RS: &str = include_str!("../../../vcs/runner/vtest/src/executor.rs");
const LIB_RS:      &str = include_str!("../../../vcs/runner/vtest/src/lib.rs");
const MAIN_RS:     &str = include_str!("../../../vcs/runner/vtest/src/main.rs");

// ── 1. ExecutorConfig has coverage: bool ─────────────────────────────────────

#[test]
fn executor_config_has_coverage_bool_field() {
    assert!(
        EXECUTOR_RS.contains("pub coverage: bool"),
        "ExecutorConfig must have 'pub coverage: bool' field"
    );
}

// ── 2. ExecutorConfig has coverage_output: Option<PathBuf> ───────────────────

#[test]
fn executor_config_has_coverage_output_field() {
    assert!(
        EXECUTOR_RS.contains("coverage_output: Option<PathBuf>"),
        "ExecutorConfig must have 'coverage_output: Option<PathBuf>' field"
    );
}

// ── 3. Default value is false ─────────────────────────────────────────────────

#[test]
fn executor_config_default_coverage_is_false() {
    assert!(
        EXECUTOR_RS.contains("coverage: false"),
        "ExecutorConfig::default() must set 'coverage: false'"
    );
}

// ── 4. run_process injects VERUM_COVERAGE env var ─────────────────────────────

#[test]
fn run_process_injects_verum_coverage_env_var() {
    assert!(
        EXECUTOR_RS.contains("VERUM_COVERAGE"),
        "executor.rs must reference 'VERUM_COVERAGE' for coverage env var injection"
    );
}

#[test]
fn run_process_injects_coverage_on_true() {
    // The injection must be conditional: `if self.config.coverage { ... }`
    assert!(
        EXECUTOR_RS.contains("self.config.coverage"),
        "executor.rs must gate coverage env injection on 'self.config.coverage'"
    );
}

// ── 5. VERUM_COVERAGE_OUTPUT env var is referenced ────────────────────────────

#[test]
fn run_process_sets_verum_coverage_output() {
    assert!(
        EXECUTOR_RS.contains("VERUM_COVERAGE_OUTPUT"),
        "executor.rs must set 'VERUM_COVERAGE_OUTPUT' env var for coverage output path"
    );
}

// ── 6. RunnerConfig has coverage: bool ───────────────────────────────────────

#[test]
fn runner_config_has_coverage_bool() {
    assert!(
        LIB_RS.contains("pub coverage: bool"),
        "RunnerConfig in lib.rs must have 'pub coverage: bool' field"
    );
}

#[test]
fn runner_config_coverage_defaults_to_false() {
    assert!(
        LIB_RS.contains("coverage: false"),
        "RunnerConfig::default() in lib.rs must set 'coverage: false'"
    );
}

// ── 7. main.rs propagates coverage flag to executor_config ───────────────────

#[test]
fn main_propagates_coverage_to_executor_config() {
    assert!(
        MAIN_RS.contains("executor_config.coverage"),
        "main.rs must propagate coverage flag to 'config.executor_config.coverage'"
    );
}

#[test]
fn main_sets_coverage_from_opts() {
    assert!(
        MAIN_RS.contains("opts.coverage"),
        "main.rs must read 'opts.coverage' from CLI options"
    );
}
