// DAP (Debug Adapter Protocol) command
// Starts a DAP server for IDE debugging integration (VS Code, etc.)
//
// The DAP server wraps the VBC interpreter with debug hooks, enabling
// breakpoints, step-through, variable inspection, and stack traces.

use crate::error::{CliError, Result};
use crate::ui;

/// DAP transport mode.
#[derive(Debug, Clone, Copy)]
pub enum Transport {
    /// Standard stdio transport (default for VS Code).
    Stdio,
    /// TCP socket transport.
    Socket(u16),
}

/// Execute the DAP server command.
pub fn execute(transport: Transport) -> Result<()> {
    match transport {
        Transport::Stdio => {
            ui::info("Starting Verum DAP server on stdio");
            verum_dap::run_stdio().map_err(|e| CliError::Custom(format!("DAP server error: {}", e)))
        }
        Transport::Socket(port) => {
            ui::info(&format!("Starting Verum DAP server on port {}", port));
            verum_dap::run_tcp(port)
                .map_err(|e| CliError::Custom(format!("DAP server error: {}", e)))
        }
    }
}
