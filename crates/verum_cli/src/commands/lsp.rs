// LSP (Language Server Protocol) command
// Provides IDE integration with real-time type checking, CBGR cost hints,
// refinement validation, and counterexample generation via SMT solver
//
// This module is a thin CLI wrapper that delegates to the verum_lsp crate for all LSP functionality.

use crate::error::{CliError, Result};
use crate::ui;

use std::future::Future;
use std::pin::Pin;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tower_lsp::jsonrpc::Result as JrResult;
use tower_lsp::{LspService, Server};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use verum_lsp::Backend;
use verum_lsp::refinement_validation::{
    InferRefinementParams, PromoteToCheckedParams, ValidateRefinementParams,
};

/// `Pin<Box<dyn Future + Send>>`-returning wrapper around a `Backend::handle_*`
/// async method.
///
/// tower-lsp 0.20's `.custom_method` is bound by
/// `for<'a> Method<&'a S, P, R>`, which in turn requires a single concrete
/// `Fut: Future + Send` type. Passing an `async fn` method directly fails
/// the HRTB because `async fn` returns `impl Future + 'a` whose concrete
/// type varies with `&self`'s lifetime.
///
/// Boxing the future erases it into a named type per lifetime
/// (`Pin<Box<dyn Future + Send + 'a>>`) that satisfies both `Fn` and `Send`,
/// so `.custom_method` accepts it. The `Send` bound is satisfied because
/// `smt_worker` keeps every Z3 value off the handler's path — the
/// validator only holds an `SmtWorkerHandle` (`Send + Sync`).
macro_rules! boxed_handler {
    ($method:ident, $params:ty) => {{
        // Nameable free function so the returned `Pin<Box<dyn Future + 'a>>`
        // can explicitly relate its lifetime to `&'a Backend`. A bare closure
        // doesn't let us bind the two lifetimes, and Rust then complains with
        // "lifetime may not live long enough".
        fn handler<'a>(
            backend: &'a Backend,
            params: $params,
        ) -> Pin<Box<dyn Future<Output = JrResult<serde_json::Value>> + Send + 'a>> {
            Box::pin(backend.$method(params))
        }
        handler
    }};
}

/// Build an `LspService` with every custom `verum/*` JSON-RPC method wired
/// into the router.
///
/// tower-lsp's `LspService::new` only routes the standard LSP methods; any
/// custom name is silently dropped unless registered with `.custom_method`.
/// This helper is the single place that knows about Verum-specific requests
/// so the three transport entry points (stdio / TCP / named-pipe) stay in
/// sync and no method leaks as `MethodNotFound` at runtime.
fn build_verum_service() -> (LspService<Backend>, tower_lsp::ClientSocket) {
    LspService::build(Backend::new)
        .custom_method(
            "verum/validateRefinement",
            boxed_handler!(handle_validate_refinement, ValidateRefinementParams),
        )
        .custom_method(
            "verum/promoteToChecked",
            boxed_handler!(handle_promote_to_checked, PromoteToCheckedParams),
        )
        .custom_method(
            "verum/inferRefinement",
            boxed_handler!(handle_infer_refinement, InferRefinementParams),
        )
        .custom_method(
            "verum/getProfile",
            boxed_handler!(handle_get_profile, serde_json::Value),
        )
        .custom_method(
            "verum/getEscapeAnalysis",
            boxed_handler!(handle_get_escape_analysis, serde_json::Value),
        )
        .finish()
}

/// LSP transport mode
#[derive(Debug, Clone, Copy)]
pub enum Transport {
    Stdio,
    Socket(u16),
    Pipe,
}

/// Execute LSP server command
///
/// This function starts the Verum Language Server using the specified transport.
/// It delegates all LSP protocol handling to the `verum_lsp` crate.
pub fn execute(transport: Transport) -> Result<()> {
    ui::info("Starting Verum Language Server");

    match transport {
        Transport::Stdio => {
            ui::debug("Using stdio transport");
            run_stdio_server()
        }
        Transport::Socket(port) => {
            ui::debug(&format!("Using TCP socket on port {}", port));
            run_socket_server(port)
        }
        Transport::Pipe => {
            ui::debug("Using named pipe transport");
            run_pipe_server()
        }
    }
}

/// Run LSP server with stdio transport using the verum_lsp crate
fn run_stdio_server() -> Result<()> {
    // Initialize logging to a file for debugging (don't use stdout/stderr as they're used by LSP)
    init_logging();

    // Create and run the async runtime
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Custom(format!("Failed to create async runtime: {}", e)))?;

    runtime.block_on(async {
        run_lsp_server().await;
    });

    ui::info("Verum Language Server stopped");
    Ok(())
}

/// Initialize logging for the LSP server
fn init_logging() {
    // Log to a file since stdout/stderr are used by LSP protocol
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/verum-lsp.log")
        .ok();

    if let Some(file) = log_file {
        let _ = tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(std::sync::Arc::new(file))
                    .with_ansi(false),
            )
            .try_init();
    }
}

/// Run the LSP server asynchronously
async fn run_lsp_server() {
    tracing::info!("Starting Verum Language Server via CLI");

    let (service, socket) = build_verum_service();

    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .serve(service)
        .await;

    tracing::info!("Verum Language Server stopped");
}

/// Run LSP server with TCP socket transport
fn run_socket_server(port: u16) -> Result<()> {
    // Initialize logging to a file for debugging
    init_logging();

    // Create and run the async runtime
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Custom(format!("Failed to create async runtime: {}", e)))?;

    runtime.block_on(async {
        run_lsp_server_socket(port).await;
    });

    ui::info("Verum Language Server stopped");
    Ok(())
}

/// Run the LSP server asynchronously with TCP socket transport
async fn run_lsp_server_socket(port: u16) {
    tracing::info!("Starting Verum Language Server on TCP port {}", port);

    // Bind to localhost
    let listener = match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => {
            tracing::info!("LSP server listening on 127.0.0.1:{}", port);
            ui::info(&format!("LSP server listening on 127.0.0.1:{}", port));
            listener
        }
        Err(e) => {
            tracing::error!("Failed to bind to port {}: {}", port, e);
            eprintln!("Error: Failed to bind to port {}: {}", port, e);
            return;
        }
    };

    loop {
        // Accept incoming connections
        let (stream, addr) = match listener.accept().await {
            Ok((stream, addr)) => {
                tracing::info!("Accepted connection from {}", addr);
                (stream, addr)
            }
            Err(e) => {
                tracing::error!("Failed to accept connection: {}", e);
                continue;
            }
        };

        // Handle each connection in a separate task
        tokio::spawn(async move {
            tracing::info!("Handling LSP client from {}", addr);

            if let Err(e) = handle_lsp_connection(stream).await {
                tracing::error!("Error handling connection from {}: {}", addr, e);
            }

            tracing::info!("Client {} disconnected", addr);
        });
    }
}

/// Handle a single LSP connection over a stream
async fn handle_lsp_connection<S>(stream: S) -> std::io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read, write) = tokio::io::split(stream);

    let (service, socket) = build_verum_service();

    Server::new(read, write, socket).serve(service).await;

    Ok(())
}

/// Run LSP server with named pipe transport
#[cfg(unix)]
fn run_pipe_server() -> Result<()> {
    // Initialize logging to a file for debugging
    init_logging();

    // Create and run the async runtime
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Custom(format!("Failed to create async runtime: {}", e)))?;

    runtime.block_on(async {
        run_lsp_server_pipe().await;
    });

    ui::info("Verum Language Server stopped");
    Ok(())
}

/// Run the LSP server asynchronously with Unix domain socket (named pipe) transport
#[cfg(unix)]
async fn run_lsp_server_pipe() {
    use std::fs;
    use tokio::net::UnixListener;

    let socket_path = "/tmp/verum-lsp.sock";

    tracing::info!(
        "Starting Verum Language Server on Unix socket {}",
        socket_path
    );

    // Remove old socket file if it exists
    let _ = fs::remove_file(socket_path);

    // Bind to Unix domain socket
    let listener = match UnixListener::bind(socket_path) {
        Ok(listener) => {
            tracing::info!("LSP server listening on {}", socket_path);
            ui::info(&format!("LSP server listening on {}", socket_path));
            listener
        }
        Err(e) => {
            tracing::error!("Failed to bind to {}: {}", socket_path, e);
            eprintln!("Error: Failed to bind to {}: {}", socket_path, e);
            return;
        }
    };

    // Ensure socket is cleaned up on exit
    let socket_path_cleanup = socket_path.to_string();
    let cleanup = move || {
        let _ = fs::remove_file(&socket_path_cleanup);
    };

    // Register cleanup handler
    let cleanup_clone = cleanup.clone();
    if let Err(e) = ctrlc::set_handler(move || {
        cleanup_clone();
        std::process::exit(0);
    }) {
        tracing::warn!("Failed to set Ctrl-C handler: {}", e);
    }

    loop {
        // Accept incoming connections
        let (stream, addr) = match listener.accept().await {
            Ok((stream, addr)) => {
                tracing::info!("Accepted connection from {:?}", addr);
                (stream, addr)
            }
            Err(e) => {
                tracing::error!("Failed to accept connection: {}", e);
                continue;
            }
        };

        // Handle each connection in a separate task
        tokio::spawn(async move {
            tracing::info!("Handling LSP client from {:?}", addr);

            if let Err(e) = handle_lsp_pipe_connection(stream).await {
                tracing::error!("Error handling connection from {:?}: {}", addr, e);
            }

            tracing::info!("Client {:?} disconnected", addr);
        });
    }
}

/// Handle a single LSP connection over a Unix domain socket
#[cfg(unix)]
async fn handle_lsp_pipe_connection(stream: tokio::net::UnixStream) -> std::io::Result<()> {
    let (read, write) = tokio::io::split(stream);

    let (service, socket) = build_verum_service();

    Server::new(read, write, socket).serve(service).await;

    Ok(())
}

/// Run LSP server with named pipe transport (Windows)
#[cfg(windows)]
fn run_pipe_server() -> Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    // Initialize logging to a file for debugging
    init_logging();

    // Create and run the async runtime
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Custom(format!("Failed to create async runtime: {}", e)))?;

    runtime.block_on(async {
        run_lsp_server_pipe_windows().await;
    });

    ui::info("Verum Language Server stopped");
    Ok(())
}

/// Run the LSP server asynchronously with Windows named pipe transport
#[cfg(windows)]
async fn run_lsp_server_pipe_windows() {
    use tokio::net::windows::named_pipe::ServerOptions;

    let pipe_name = r"\\.\pipe\verum-lsp";

    tracing::info!("Starting Verum Language Server on named pipe {}", pipe_name);
    ui::info(&format!("LSP server listening on {}", pipe_name));

    loop {
        // Create named pipe server
        let server = match ServerOptions::new()
            .first_pipe_instance(true)
            .create(pipe_name)
        {
            Ok(server) => {
                tracing::info!("Created named pipe instance");
                server
            }
            Err(e) => {
                tracing::error!("Failed to create named pipe: {}", e);
                eprintln!("Error: Failed to create named pipe: {}", e);
                return;
            }
        };

        // Wait for client connection
        if let Err(e) = server.connect().await {
            tracing::error!("Failed to connect to client: {}", e);
            continue;
        }

        tracing::info!("Client connected to named pipe");

        // Handle the connection in a separate task
        tokio::spawn(async move {
            if let Err(e) = handle_lsp_pipe_connection_windows(server).await {
                tracing::error!("Error handling named pipe connection: {}", e);
            }

            tracing::info!("Named pipe client disconnected");
        });
    }
}

/// Handle a single LSP connection over a Windows named pipe
#[cfg(windows)]
async fn handle_lsp_pipe_connection_windows(
    stream: tokio::net::windows::named_pipe::NamedPipeServer,
) -> std::io::Result<()> {
    let (read, write) = tokio::io::split(stream);

    let (service, socket) = build_verum_service();

    Server::new(read, write, socket).serve(service).await;

    Ok(())
}
