//! Interactive REPL for Verum (deprecated)
//!
//! This module is deprecated. Use `commands::repl` instead.
//!
//! The REPL is being migrated to VBC-first architecture.
//! See `crates/verum_cli/src/commands/repl.rs` for the active implementation.

use crate::error::Result;
use verum_common::Text;

/// Start the REPL (deprecated - redirects to commands::repl)
pub fn start(_prelude: Option<Text>) -> Result<()> {
    // Redirect to commands::repl
    crate::commands::repl::execute(None)
}
