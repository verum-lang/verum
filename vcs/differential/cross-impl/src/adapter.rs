//! Adapters for Different Implementation Types
//!
//! This module provides adapter implementations for communicating with
//! different Verum language implementations:
//!
//! - ProcessAdapter: Spawns implementation as a subprocess
//! - SocketAdapter: Communicates via TCP/Unix socket
//! - EmbeddedAdapter: Directly links to implementation library

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::protocol::{
    Protocol, ProtocolError, ProtocolErrorKind,
    Request, Response, Event,
    ExecuteRequest, ExecuteResponse, ExecuteOptions,
    CapabilityRequest, CapabilityResponse,
    ErrorResponse, ErrorCode,
};
use crate::Implementation;

/// Adapter configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterConfig {
    /// Timeout for operations in milliseconds
    pub timeout_ms: u64,
    /// Buffer size for I/O
    pub buffer_size: usize,
    /// Whether to use JSON-RPC protocol
    pub use_json_rpc: bool,
    /// Environment variables
    pub env_vars: HashMap<String, String>,
}

impl Default for AdapterConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 30_000,
            buffer_size: 64 * 1024,
            use_json_rpc: false,
            env_vars: HashMap::new(),
        }
    }
}

/// Adapter type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdapterKind {
    /// Process-based adapter (subprocess)
    Process,
    /// Socket-based adapter (TCP/Unix)
    Socket,
    /// Embedded adapter (library)
    Embedded,
}

/// Adapter trait
pub trait Adapter: Send + Sync {
    /// Get the kind of adapter
    fn kind(&self) -> AdapterKind;

    /// Execute a program
    fn execute(&mut self, request: &ExecuteRequest) -> Result<ExecuteResponse>;

    /// Query capabilities
    fn capabilities(&mut self) -> Result<CapabilityResponse>;

    /// Check if adapter is available
    fn is_available(&self) -> bool;

    /// Get implementation name
    fn implementation_name(&self) -> &str;

    /// Close the adapter
    fn close(&mut self) -> Result<()>;
}

// =============================================================================
// Process Adapter
// =============================================================================

/// Adapter that spawns implementation as a subprocess
pub struct ProcessAdapter {
    /// Implementation info
    implementation: Implementation,
    /// Adapter configuration
    config: AdapterConfig,
    /// Whether the implementation is available
    available: bool,
}

impl ProcessAdapter {
    /// Create a new process adapter
    pub fn new(implementation: &Implementation) -> Result<Self> {
        let mut adapter = Self {
            implementation: implementation.clone(),
            config: AdapterConfig::default(),
            available: false,
        };

        // Check if binary exists
        adapter.available = implementation.binary_path.exists()
            || which::which(&implementation.binary_path).is_ok();

        Ok(adapter)
    }

    /// Set configuration
    pub fn with_config(mut self, config: AdapterConfig) -> Self {
        self.config = config;
        self
    }

    /// Execute via subprocess
    fn execute_subprocess(&self, source: &str, options: &ExecuteOptions) -> Result<ExecuteResponse> {
        let start = Instant::now();
        let timeout = Duration::from_millis(self.config.timeout_ms);

        // Create temp file for source
        let temp_dir = tempfile::tempdir()?;
        let temp_file = temp_dir.path().join("test.vr");
        std::fs::write(&temp_file, source)?;

        // Build command
        let mut cmd = Command::new(&self.implementation.binary_path);
        cmd.arg(&temp_file);
        cmd.args(&self.implementation.extra_args);
        cmd.envs(&self.implementation.env_vars);
        cmd.envs(&self.config.env_vars);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Add optimization flags
        if options.optimization_level > 0 {
            cmd.arg(format!("-O{}", options.optimization_level));
        }

        if options.debug_mode {
            cmd.arg("--debug");
        }

        // Spawn and wait
        let mut child = cmd.spawn()
            .with_context(|| format!("Failed to spawn {}", self.implementation.name))?;

        let result = self.wait_with_timeout(&mut child, timeout);
        let duration = start.elapsed();

        // Capture output
        let stdout = child.stdout.take()
            .map(|h| self.read_all(h))
            .unwrap_or_default();
        let stderr = child.stderr.take()
            .map(|h| self.read_all(h))
            .unwrap_or_default();

        match result {
            WaitResult::Exited(code) => {
                Ok(ExecuteResponse {
                    success: code == 0,
                    exit_code: Some(code),
                    stdout,
                    stderr,
                    duration_ms: duration.as_millis() as u64,
                    timed_out: false,
                    crashed: false,
                    signal: None,
                    peak_memory: None,
                    metadata: HashMap::new(),
                })
            }
            WaitResult::TimedOut => {
                let _ = child.kill();
                let _ = child.wait();

                Ok(ExecuteResponse {
                    success: false,
                    exit_code: None,
                    stdout,
                    stderr,
                    duration_ms: duration.as_millis() as u64,
                    timed_out: true,
                    crashed: false,
                    signal: None,
                    peak_memory: None,
                    metadata: HashMap::new(),
                })
            }
            WaitResult::Signaled(signal) => {
                Ok(ExecuteResponse {
                    success: false,
                    exit_code: None,
                    stdout,
                    stderr,
                    duration_ms: duration.as_millis() as u64,
                    timed_out: false,
                    crashed: true,
                    signal: Some(signal),
                    peak_memory: None,
                    metadata: HashMap::new(),
                })
            }
        }
    }

    /// Wait for process with timeout
    fn wait_with_timeout(&self, child: &mut Child, timeout: Duration) -> WaitResult {
        let start = Instant::now();

        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if let Some(code) = status.code() {
                        return WaitResult::Exited(code);
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::ExitStatusExt;
                        if let Some(signal) = status.signal() {
                            return WaitResult::Signaled(signal);
                        }
                    }
                    return WaitResult::Exited(-1);
                }
                Ok(None) => {
                    if start.elapsed() > timeout {
                        return WaitResult::TimedOut;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return WaitResult::Signaled(0),
            }
        }
    }

    /// Read all output from a handle
    fn read_all<R: std::io::Read>(&self, reader: R) -> String {
        let reader = BufReader::new(reader);
        reader.lines()
            .filter_map(|l| l.ok())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Result of waiting for a process
enum WaitResult {
    Exited(i32),
    TimedOut,
    Signaled(i32),
}

impl Adapter for ProcessAdapter {
    fn kind(&self) -> AdapterKind {
        AdapterKind::Process
    }

    fn execute(&mut self, request: &ExecuteRequest) -> Result<ExecuteResponse> {
        if !self.available {
            return Err(anyhow::anyhow!(
                "Implementation {} is not available",
                self.implementation.name
            ));
        }

        self.execute_subprocess(&request.source, &request.options)
    }

    fn capabilities(&mut self) -> Result<CapabilityResponse> {
        // Query version to get capabilities
        let output = Command::new(&self.implementation.binary_path)
            .arg("--capabilities")
            .output()
            .or_else(|_| {
                Command::new(&self.implementation.binary_path)
                    .arg("--version")
                    .output()
            })?;

        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();

        Ok(CapabilityResponse {
            implementation: self.implementation.name.clone(),
            version,
            capabilities: vec![],
            language_version: "1.0".to_string(),
            features: self.implementation.features.clone(),
            limitations: vec![],
        })
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn implementation_name(&self) -> &str {
        &self.implementation.name
    }

    fn close(&mut self) -> Result<()> {
        // Nothing to close for process adapter
        Ok(())
    }
}

// =============================================================================
// Socket Adapter
// =============================================================================

/// Adapter that communicates via socket
pub struct SocketAdapter {
    /// Implementation name
    name: String,
    /// Socket address
    address: String,
    /// Connection (if established)
    connection: Option<std::net::TcpStream>,
    /// Configuration
    config: AdapterConfig,
    /// Message counter for IDs
    message_id: u64,
}

impl SocketAdapter {
    /// Create a new socket adapter
    pub fn new(name: impl Into<String>, address: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            address: address.into(),
            connection: None,
            config: AdapterConfig::default(),
            message_id: 0,
        }
    }

    /// Connect to the implementation
    pub fn connect(&mut self) -> Result<()> {
        let timeout = Duration::from_millis(self.config.timeout_ms);
        let stream = std::net::TcpStream::connect_timeout(
            &self.address.parse()?,
            timeout,
        )?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        self.connection = Some(stream);
        Ok(())
    }

    /// Send a message and receive response
    fn send_receive(&mut self, request: &Request) -> Result<Response> {
        let conn = self.connection.as_mut()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;

        self.message_id += 1;

        // Serialize request
        let msg = crate::protocol::Message::request(self.message_id, request.clone());
        let json = serde_json::to_string(&msg)?;

        // Send
        writeln!(conn, "{}", json)?;
        conn.flush()?;

        // Receive
        let mut reader = BufReader::new(conn);
        let mut response_line = String::new();
        reader.read_line(&mut response_line)?;

        let response_msg: crate::protocol::Message = serde_json::from_str(&response_line)?;

        match response_msg.kind {
            crate::protocol::MessageKind::Response(resp) => Ok(resp),
            _ => Err(anyhow::anyhow!("Unexpected message kind")),
        }
    }
}

impl Adapter for SocketAdapter {
    fn kind(&self) -> AdapterKind {
        AdapterKind::Socket
    }

    fn execute(&mut self, request: &ExecuteRequest) -> Result<ExecuteResponse> {
        if self.connection.is_none() {
            self.connect()?;
        }

        let req = Request::Execute(request.clone());
        let response = self.send_receive(&req)?;

        match response {
            Response::Execute(exec_resp) => Ok(exec_resp),
            Response::Error(err) => Err(anyhow::anyhow!("{}: {}", err.code, err.message)),
            _ => Err(anyhow::anyhow!("Unexpected response type")),
        }
    }

    fn capabilities(&mut self) -> Result<CapabilityResponse> {
        if self.connection.is_none() {
            self.connect()?;
        }

        let req = Request::Capability(CapabilityRequest {
            capabilities: vec![],
        });
        let response = self.send_receive(&req)?;

        match response {
            Response::Capability(cap_resp) => Ok(cap_resp),
            Response::Error(err) => Err(anyhow::anyhow!("{}: {}", err.code, err.message)),
            _ => Err(anyhow::anyhow!("Unexpected response type")),
        }
    }

    fn is_available(&self) -> bool {
        self.connection.is_some()
            || std::net::TcpStream::connect_timeout(
                &self.address.parse().unwrap_or_else(|_| "127.0.0.1:0".parse().unwrap()),
                Duration::from_millis(100),
            ).is_ok()
    }

    fn implementation_name(&self) -> &str {
        &self.name
    }

    fn close(&mut self) -> Result<()> {
        if let Some(mut conn) = self.connection.take() {
            // Send shutdown message
            let msg = crate::protocol::Message::request(self.message_id + 1, Request::Shutdown);
            let json = serde_json::to_string(&msg)?;
            let _ = writeln!(conn, "{}", json);
        }
        Ok(())
    }
}

// =============================================================================
// Embedded Adapter
// =============================================================================

/// Adapter that embeds the implementation directly
pub struct EmbeddedAdapter {
    /// Implementation name
    name: String,
    /// Configuration
    config: AdapterConfig,
    /// Whether initialized
    initialized: bool,
}

impl EmbeddedAdapter {
    /// Create a new embedded adapter
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            config: AdapterConfig::default(),
            initialized: false,
        }
    }

    /// Initialize the embedded implementation
    pub fn initialize(&mut self) -> Result<()> {
        // In a real implementation, this would initialize the embedded runtime
        self.initialized = true;
        Ok(())
    }
}

impl Adapter for EmbeddedAdapter {
    fn kind(&self) -> AdapterKind {
        AdapterKind::Embedded
    }

    fn execute(&mut self, request: &ExecuteRequest) -> Result<ExecuteResponse> {
        if !self.initialized {
            self.initialize()?;
        }

        // In a real implementation, this would directly call the interpreter
        // For now, return an error indicating this is not implemented
        Err(anyhow::anyhow!(
            "Embedded adapter execution not yet implemented"
        ))
    }

    fn capabilities(&mut self) -> Result<CapabilityResponse> {
        Ok(CapabilityResponse {
            implementation: self.name.clone(),
            version: "embedded".to_string(),
            capabilities: vec![],
            language_version: "1.0".to_string(),
            features: vec![],
            limitations: vec!["Embedded mode".to_string()],
        })
    }

    fn is_available(&self) -> bool {
        true // Embedded is always available if compiled in
    }

    fn implementation_name(&self) -> &str {
        &self.name
    }

    fn close(&mut self) -> Result<()> {
        self.initialized = false;
        Ok(())
    }
}

// =============================================================================
// Adapter Factory
// =============================================================================

/// Create an adapter for an implementation
pub fn create_adapter(impl_: &Implementation) -> Result<Box<dyn Adapter>> {
    // Default to process adapter
    let adapter = ProcessAdapter::new(impl_)?;
    Ok(Box::new(adapter))
}

/// Create adapters for all standard implementations
pub fn create_standard_adapters() -> HashMap<String, Box<dyn Adapter>> {
    let mut adapters: HashMap<String, Box<dyn Adapter>> = HashMap::new();

    let implementations = crate::standard_implementations();

    for impl_ in implementations {
        if let Ok(adapter) = create_adapter(&impl_) {
            if adapter.is_available() {
                adapters.insert(impl_.name.clone(), adapter);
            }
        }
    }

    adapters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapter_config_default() {
        let config = AdapterConfig::default();
        assert_eq!(config.timeout_ms, 30_000);
        assert_eq!(config.buffer_size, 64 * 1024);
    }

    #[test]
    fn test_process_adapter_creation() {
        let impl_ = Implementation::new("test", "/nonexistent/binary");
        let adapter = ProcessAdapter::new(&impl_).unwrap();

        assert_eq!(adapter.kind(), AdapterKind::Process);
        assert!(!adapter.is_available());
        assert_eq!(adapter.implementation_name(), "test");
    }

    #[test]
    fn test_socket_adapter_creation() {
        let adapter = SocketAdapter::new("test", "127.0.0.1:9999");

        assert_eq!(adapter.kind(), AdapterKind::Socket);
        assert_eq!(adapter.implementation_name(), "test");
    }

    #[test]
    fn test_embedded_adapter() {
        let mut adapter = EmbeddedAdapter::new("embedded");

        assert_eq!(adapter.kind(), AdapterKind::Embedded);
        assert!(adapter.is_available());

        adapter.initialize().unwrap();
        assert!(adapter.initialized);

        adapter.close().unwrap();
        assert!(!adapter.initialized);
    }
}
