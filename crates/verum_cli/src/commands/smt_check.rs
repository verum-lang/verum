// `verum verify --check-smt-formula <FILE>` handler.
//
// Reads an SMT-LIB 2 file from disk, dispatches it to the
// configured solver (currently Z3), and prints the verdict on
// stdout. Thin wrapper over `verum_smt::smtlib_check`: the
// library layer handles the solver call, this module handles
// file I/O + CLI-specific error formatting.

use std::path::Path;

use crate::error::{CliError, Result};
use verum_smt::smtlib_check::{check_smtlib_string, CheckError};

/// Run the `--check-smt-formula FILE` dispatch.
pub fn run(path: &Path, solver: &str, timeout_s: u64) -> Result<()> {
    let contents = std::fs::read_to_string(path).map_err(|e| {
        CliError::Custom(format!("reading {}: {}", path.display(), e).into())
    })?;

    match check_smtlib_string(&contents, solver, timeout_s) {
        Ok(verdict) => {
            println!("{}", verdict.as_str());
            Ok(())
        }
        Err(CheckError::NoCheckSat) => Err(CliError::InvalidArgument(
            format!(
                "SMT-LIB file {} contains no `(check-sat)` directive — \
                 nothing to check",
                path.display()
            )
            .into(),
        )),
        Err(CheckError::UnsupportedSolver(msg)) => {
            Err(CliError::InvalidArgument(
                format!("--check-smt-formula: unsupported solver: {}", msg)
                    .into(),
            ))
        }
    }
}
