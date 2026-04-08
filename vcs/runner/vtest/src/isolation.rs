//! Test isolation module for VCS test runner.
//!
//! Provides isolation mechanisms for running tests in parallel without
//! interference. Each test gets its own isolated environment with:
//!
//! - Separate working directory
//! - Clean environment variables
//! - Resource limits (memory, CPU time)
//! - Temporary file cleanup
//!
//! # Isolation Levels
//!
//! - **None**: Tests run in shared environment (fastest, least isolation)
//! - **Process**: Each test runs in a separate process (default)
//! - **Directory**: Each test gets a unique temp directory
//! - **Container**: Tests run in isolated containers (most isolation)
//!
//! # Example
//!
//! ```rust,ignore
//! use vtest::isolation::{IsolationConfig, IsolatedContext, IsolationLevel};
//!
//! let config = IsolationConfig {
//!     level: IsolationLevel::Directory,
//!     cleanup: true,
//!     ..Default::default()
//! };
//!
//! let context = IsolatedContext::create(&config)?;
//! // Run test in isolated context
//! let result = context.execute(test_fn).await?;
//! context.cleanup()?;
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use thiserror::Error;
use tokio::fs;
use verum_common::{List, Map, Text};

/// Error type for isolation failures.
#[derive(Debug, Error)]
pub enum IsolationError {
    #[error("Failed to create isolation context: {0}")]
    CreationError(Text),

    #[error("Failed to cleanup isolation context: {0}")]
    CleanupError(Text),

    #[error("Resource limit exceeded: {0}")]
    ResourceLimitExceeded(Text),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Timeout during isolation setup")]
    Timeout,
}

/// Isolation level for test execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IsolationLevel {
    /// No isolation - tests share the same environment.
    /// Fastest but may have interference between tests.
    None,

    /// Process-level isolation (default).
    /// Each test runs in a separate process.
    #[default]
    Process,

    /// Directory-level isolation.
    /// Each test gets a unique temporary directory.
    Directory,

    /// Container-level isolation.
    /// Tests run in isolated containers (requires container runtime).
    Container,
}

impl IsolationLevel {
    /// Parse isolation level from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "none" => Some(Self::None),
            "process" => Some(Self::Process),
            "directory" | "dir" => Some(Self::Directory),
            "container" => Some(Self::Container),
            _ => None,
        }
    }
}

/// Resource limits for isolated test execution.
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    /// Maximum memory in bytes (0 = unlimited)
    pub max_memory_bytes: u64,
    /// Maximum CPU time in milliseconds (0 = unlimited)
    pub max_cpu_time_ms: u64,
    /// Maximum number of file descriptors
    pub max_file_descriptors: u32,
    /// Maximum output size in bytes
    pub max_output_bytes: u64,
    /// Maximum number of child processes
    pub max_processes: u32,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory_bytes: 512 * 1024 * 1024, // 512 MB
            max_cpu_time_ms: 60_000,             // 60 seconds
            max_file_descriptors: 256,
            max_output_bytes: 10 * 1024 * 1024, // 10 MB
            max_processes: 10,
        }
    }
}

/// Configuration for test isolation.
#[derive(Debug, Clone)]
pub struct IsolationConfig {
    /// Isolation level
    pub level: IsolationLevel,
    /// Base directory for temporary files
    pub temp_base: PathBuf,
    /// Whether to cleanup after test completion
    pub cleanup: bool,
    /// Resource limits
    pub limits: ResourceLimits,
    /// Environment variables to inherit
    pub inherit_env: List<Text>,
    /// Environment variables to set
    pub set_env: Map<Text, Text>,
    /// Environment variables to unset
    pub unset_env: List<Text>,
    /// Whether to capture output
    pub capture_output: bool,
    /// Timeout for isolation setup
    pub setup_timeout_ms: u64,
}

impl Default for IsolationConfig {
    fn default() -> Self {
        let inherit: List<Text> = if cfg!(windows) {
            vec![
                "PATH".to_string().into(),
                "USERPROFILE".to_string().into(),
                "USERNAME".to_string().into(),
                "SYSTEMROOT".to_string().into(),
                "TEMP".to_string().into(),
                "TMP".to_string().into(),
                "COMSPEC".to_string().into(),
                // Propagate MSVC toolchain environment
                "INCLUDE".to_string().into(),
                "LIB".to_string().into(),
                "LIBPATH".to_string().into(),
            ].into()
        } else {
            vec![
                "PATH".to_string().into(),
                "HOME".to_string().into(),
                "USER".to_string().into(),
                "LANG".to_string().into(),
                "LC_ALL".to_string().into(),
            ].into()
        };
        Self {
            level: IsolationLevel::Process,
            temp_base: std::env::temp_dir().join("vtest"),
            cleanup: true,
            limits: ResourceLimits::default(),
            inherit_env: inherit,
            set_env: Map::new(),
            unset_env: List::new(),
            capture_output: true,
            setup_timeout_ms: 5000,
        }
    }
}

/// Counter for generating unique context IDs.
static CONTEXT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// An isolated context for test execution.
#[derive(Debug)]
pub struct IsolatedContext {
    /// Unique identifier for this context
    pub id: u64,
    /// Configuration used to create this context
    pub config: IsolationConfig,
    /// Working directory for this context
    pub work_dir: PathBuf,
    /// Environment variables for this context
    pub environment: HashMap<String, String>,
    /// Whether the context has been cleaned up
    cleaned_up: bool,
    /// Files created during test execution
    created_files: Vec<PathBuf>,
    /// Start time of the context
    start_time: std::time::Instant,
}

impl IsolatedContext {
    /// Create a new isolated context.
    pub async fn create(config: &IsolationConfig) -> Result<Self, IsolationError> {
        let id = CONTEXT_COUNTER.fetch_add(1, Ordering::SeqCst);
        let start_time = std::time::Instant::now();

        // Create unique work directory
        let work_dir = config
            .temp_base
            .join(format!("test-{}-{}", id, std::process::id()));

        if config.level != IsolationLevel::None {
            fs::create_dir_all(&work_dir).await?;
        }

        // Build environment
        let mut environment = HashMap::new();

        // Inherit specified environment variables
        for var in &config.inherit_env {
            if let Ok(value) = std::env::var(var.as_str()) {
                environment.insert(var.to_string(), value);
            }
        }

        // Set specified environment variables
        for (key, value) in &config.set_env {
            environment.insert(key.to_string(), value.to_string());
        }

        // Remove unset variables
        for var in &config.unset_env {
            environment.remove(var.as_str());
        }

        // Set isolation-specific variables
        environment.insert("VTEST_ISOLATED".to_string(), "1".to_string());
        environment.insert("VTEST_CONTEXT_ID".to_string(), id.to_string());
        environment.insert(
            "VTEST_WORK_DIR".to_string(),
            work_dir.to_string_lossy().to_string(),
        );

        Ok(Self {
            id,
            config: config.clone(),
            work_dir,
            environment,
            cleaned_up: false,
            created_files: Vec::new(),
            start_time,
        })
    }

    /// Create a new isolated context synchronously.
    pub fn create_sync(config: &IsolationConfig) -> Result<Self, IsolationError> {
        let id = CONTEXT_COUNTER.fetch_add(1, Ordering::SeqCst);
        let start_time = std::time::Instant::now();

        // Create unique work directory
        let work_dir = config
            .temp_base
            .join(format!("test-{}-{}", id, std::process::id()));

        if config.level != IsolationLevel::None {
            std::fs::create_dir_all(&work_dir)?;
        }

        // Build environment
        let mut environment = HashMap::new();

        // Inherit specified environment variables
        for var in &config.inherit_env {
            if let Ok(value) = std::env::var(var.as_str()) {
                environment.insert(var.to_string(), value);
            }
        }

        // Set specified environment variables
        for (key, value) in &config.set_env {
            environment.insert(key.to_string(), value.to_string());
        }

        // Remove unset variables
        for var in &config.unset_env {
            environment.remove(var.as_str());
        }

        // Set isolation-specific variables
        environment.insert("VTEST_ISOLATED".to_string(), "1".to_string());
        environment.insert("VTEST_CONTEXT_ID".to_string(), id.to_string());
        environment.insert(
            "VTEST_WORK_DIR".to_string(),
            work_dir.to_string_lossy().to_string(),
        );

        Ok(Self {
            id,
            config: config.clone(),
            work_dir,
            environment,
            cleaned_up: false,
            created_files: Vec::new(),
            start_time,
        })
    }

    /// Get the working directory for this context.
    pub fn work_dir(&self) -> &Path {
        &self.work_dir
    }

    /// Get the environment for this context.
    pub fn environment(&self) -> &HashMap<String, String> {
        &self.environment
    }

    /// Register a file created during test execution.
    pub fn register_file(&mut self, path: PathBuf) {
        self.created_files.push(path);
    }

    /// Create a temporary file in this context.
    pub async fn create_temp_file(
        &mut self,
        name: &str,
        content: &str,
    ) -> Result<PathBuf, IsolationError> {
        let path = self.work_dir.join(name);

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }

        fs::write(&path, content).await?;
        self.created_files.push(path.clone());

        Ok(path)
    }

    /// Create a temporary file synchronously.
    pub fn create_temp_file_sync(
        &mut self,
        name: &str,
        content: &str,
    ) -> Result<PathBuf, IsolationError> {
        let path = self.work_dir.join(name);

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&path, content)?;
        self.created_files.push(path.clone());

        Ok(path)
    }

    /// Check if resource limits are exceeded.
    pub fn check_limits(&self) -> Result<(), IsolationError> {
        // Check CPU time
        let elapsed_ms = self.start_time.elapsed().as_millis() as u64;
        if self.config.limits.max_cpu_time_ms > 0 && elapsed_ms > self.config.limits.max_cpu_time_ms
        {
            return Err(IsolationError::ResourceLimitExceeded(format!(
                "CPU time limit exceeded: {}ms > {}ms",
                elapsed_ms, self.config.limits.max_cpu_time_ms
            ).into()));
        }

        Ok(())
    }

    /// Get elapsed time since context creation.
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Cleanup this isolated context.
    pub async fn cleanup(&mut self) -> Result<(), IsolationError> {
        if self.cleaned_up {
            return Ok(());
        }

        if self.config.cleanup && self.config.level != IsolationLevel::None {
            // Remove work directory and all contents
            if self.work_dir.exists() {
                fs::remove_dir_all(&self.work_dir).await.map_err(|e| {
                    IsolationError::CleanupError(format!(
                        "Failed to remove work directory {}: {}",
                        self.work_dir.display(),
                        e
                    ).into())
                })?;
            }
        }

        self.cleaned_up = true;
        Ok(())
    }

    /// Cleanup synchronously.
    pub fn cleanup_sync(&mut self) -> Result<(), IsolationError> {
        if self.cleaned_up {
            return Ok(());
        }

        if self.config.cleanup && self.config.level != IsolationLevel::None {
            // Remove work directory and all contents
            if self.work_dir.exists() {
                std::fs::remove_dir_all(&self.work_dir).map_err(|e| {
                    IsolationError::CleanupError(format!(
                        "Failed to remove work directory {}: {}",
                        self.work_dir.display(),
                        e
                    ).into())
                })?;
            }
        }

        self.cleaned_up = true;
        Ok(())
    }
}

impl Drop for IsolatedContext {
    fn drop(&mut self) {
        // Best-effort cleanup on drop
        if !self.cleaned_up && self.config.cleanup {
            let _ = self.cleanup_sync();
        }
    }
}

/// Isolation manager for coordinating multiple isolated contexts.
#[derive(Debug)]
pub struct IsolationManager {
    config: IsolationConfig,
    active_contexts: Vec<u64>,
}

impl IsolationManager {
    /// Create a new isolation manager.
    pub fn new(config: IsolationConfig) -> Self {
        Self {
            config,
            active_contexts: Vec::new(),
        }
    }

    /// Create a new isolated context.
    pub async fn create_context(&mut self) -> Result<IsolatedContext, IsolationError> {
        let context = IsolatedContext::create(&self.config).await?;
        self.active_contexts.push(context.id);
        Ok(context)
    }

    /// Create a context synchronously.
    pub fn create_context_sync(&mut self) -> Result<IsolatedContext, IsolationError> {
        let context = IsolatedContext::create_sync(&self.config)?;
        self.active_contexts.push(context.id);
        Ok(context)
    }

    /// Cleanup all active contexts.
    pub async fn cleanup_all(&mut self) -> Result<(), IsolationError> {
        // Cleanup the base temp directory if it exists and is empty
        if self.config.cleanup && self.config.temp_base.exists() {
            // Only remove if empty
            if fs::read_dir(&self.config.temp_base)
                .await?
                .next_entry()
                .await?
                .is_none()
            {
                let _ = fs::remove_dir(&self.config.temp_base).await;
            }
        }

        self.active_contexts.clear();
        Ok(())
    }

    /// Get the number of active contexts.
    pub fn active_count(&self) -> usize {
        self.active_contexts.len()
    }
}

/// Resource usage statistics for an isolated context.
#[derive(Debug, Clone, Default)]
pub struct ResourceUsage {
    /// Peak memory usage in bytes
    pub peak_memory_bytes: u64,
    /// Total CPU time in milliseconds
    pub cpu_time_ms: u64,
    /// Number of file descriptors used
    pub file_descriptors: u32,
    /// Total output size in bytes
    pub output_bytes: u64,
    /// Number of child processes spawned
    pub processes_spawned: u32,
}

impl ResourceUsage {
    /// Check if usage exceeds limits.
    pub fn exceeds_limits(&self, limits: &ResourceLimits) -> Option<Text> {
        if limits.max_memory_bytes > 0 && self.peak_memory_bytes > limits.max_memory_bytes {
            return Some(format!(
                "Memory limit exceeded: {} > {}",
                self.peak_memory_bytes, limits.max_memory_bytes
            ).into());
        }

        if limits.max_cpu_time_ms > 0 && self.cpu_time_ms > limits.max_cpu_time_ms {
            return Some(format!(
                "CPU time limit exceeded: {}ms > {}ms",
                self.cpu_time_ms, limits.max_cpu_time_ms
            ).into());
        }

        if self.file_descriptors > limits.max_file_descriptors {
            return Some(format!(
                "File descriptor limit exceeded: {} > {}",
                self.file_descriptors, limits.max_file_descriptors
            ).into());
        }

        if limits.max_output_bytes > 0 && self.output_bytes > limits.max_output_bytes {
            return Some(format!(
                "Output size limit exceeded: {} > {}",
                self.output_bytes, limits.max_output_bytes
            ).into());
        }

        if self.processes_spawned > limits.max_processes {
            return Some(format!(
                "Process limit exceeded: {} > {}",
                self.processes_spawned, limits.max_processes
            ).into());
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_isolation_level_from_str() {
        assert_eq!(IsolationLevel::from_str("none"), Some(IsolationLevel::None));
        assert_eq!(
            IsolationLevel::from_str("process"),
            Some(IsolationLevel::Process)
        );
        assert_eq!(
            IsolationLevel::from_str("directory"),
            Some(IsolationLevel::Directory)
        );
        assert_eq!(
            IsolationLevel::from_str("container"),
            Some(IsolationLevel::Container)
        );
        assert_eq!(IsolationLevel::from_str("invalid"), None);
    }

    #[test]
    fn test_resource_limits_default() {
        let limits = ResourceLimits::default();
        assert_eq!(limits.max_memory_bytes, 512 * 1024 * 1024);
        assert_eq!(limits.max_cpu_time_ms, 60_000);
    }

    #[test]
    fn test_isolation_config_default() {
        let config = IsolationConfig::default();
        assert_eq!(config.level, IsolationLevel::Process);
        assert!(config.cleanup);
        assert!(config.capture_output);
    }

    #[test]
    fn test_resource_usage_exceeds_limits() {
        let limits = ResourceLimits {
            max_memory_bytes: 1000,
            max_cpu_time_ms: 100,
            max_file_descriptors: 10,
            max_output_bytes: 500,
            max_processes: 5,
        };

        let usage = ResourceUsage {
            peak_memory_bytes: 500,
            cpu_time_ms: 50,
            file_descriptors: 5,
            output_bytes: 200,
            processes_spawned: 2,
        };
        assert!(usage.exceeds_limits(&limits).is_none());

        let usage_exceeded = ResourceUsage {
            peak_memory_bytes: 2000,
            ..usage
        };
        assert!(usage_exceeded.exceeds_limits(&limits).is_some());
    }

    #[tokio::test]
    async fn test_isolated_context_create() {
        let config = IsolationConfig {
            level: IsolationLevel::Directory,
            temp_base: std::env::temp_dir().join("vtest-test"),
            cleanup: true,
            ..Default::default()
        };

        let mut context = IsolatedContext::create(&config).await.unwrap();
        assert!(context.work_dir.exists());
        assert!(context.environment.contains_key("VTEST_ISOLATED"));

        context.cleanup().await.unwrap();
        assert!(!context.work_dir.exists());
    }

    #[tokio::test]
    async fn test_isolated_context_temp_file() {
        let config = IsolationConfig {
            level: IsolationLevel::Directory,
            temp_base: std::env::temp_dir().join("vtest-test-file"),
            cleanup: true,
            ..Default::default()
        };

        let mut context = IsolatedContext::create(&config).await.unwrap();
        let file_path = context
            .create_temp_file("test.txt", "hello world")
            .await
            .unwrap();

        assert!(file_path.exists());
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "hello world");

        context.cleanup().await.unwrap();
        assert!(!file_path.exists());
    }
}
