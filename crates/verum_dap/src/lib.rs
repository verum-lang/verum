//! # Verum DAP — Debug Adapter Protocol Server
//!
//! Implements the [Debug Adapter Protocol](https://microsoft.github.io/debug-adapter-protocol/)
//! for the Verum programming language. The DAP server wraps the VBC interpreter with debug
//! hooks, enabling IDE debugging features:
//!
//! - **Breakpoints**: Set and resolve breakpoints via VBC source maps
//! - **Step-through**: Step over, step in, step out using VBC call stack tracking
//! - **Variable inspection**: Read VBC registers with debug variable names
//! - **Stack traces**: Walk the VBC call stack with source location mapping
//!
//! # Transport
//!
//! The server supports two transport modes:
//!
//! - **stdio** (default): Reads from stdin, writes to stdout. Standard for VS Code.
//! - **TCP**: Listens on a port for a single client connection.
//!
//! # Usage
//!
//! ```ignore
//! // Run on stdio (for VS Code launch.json integration)
//! verum_dap::run_stdio().unwrap();
//!
//! // Run on TCP port 4711 (for testing)
//! verum_dap::run_tcp(4711).unwrap();
//! ```

pub mod types;
pub mod session;
pub mod variables;
pub mod adapter;
pub mod server;

// Re-export the main entry points.
pub use server::{run_stdio, run_tcp};
