// Error types for the Verum CLI
// Provides comprehensive error handling with user-friendly messages

use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, CliError>;

#[derive(Debug)]
pub enum CliError {
    Io(std::io::Error),
    ConfigParse(toml::de::Error),
    ConfigSerialize(toml::ser::Error),
    ProjectNotFound(PathBuf),
    FileNotFound(String),
    SemverParse(semver::Error),
    PatternError(String),
    GlobError(String),
    NotifyError(notify::Error),
    WalkdirError(walkdir::Error),
    JsonError(serde_json::Error),
    InvalidProjectName(String),
    ProjectExists(PathBuf),
    CompilationFailed(String),
    ParseError {
        file: PathBuf,
        line: usize,
        col: usize,
        message: String,
    },
    TypeError(String),
    CbgrError(String),
    LinkError(String),
    Codegen(String),
    TestsFailed {
        passed: usize,
        failed: usize,
    },
    BenchmarkFailed(String),
    DependencyNotFound(String),
    VersionConflict {
        package: String,
        required: String,
        found: String,
    },
    Network(String),
    Registry(String),
    TemplateNotFound(String),
    InvalidTarget(String),
    CacheCorrupted,
    PermissionDenied(PathBuf),
    CommandNotFound(String),
    InvalidArgument(String),
    VerificationFailed(String),
    RuntimeError(String),
    ProfilingFailed(String),
    ReplError(String),
    FeatureNotImplemented {
        feature: verum_common::Text,
        planned_version: verum_common::Text,
        workaround: verum_common::Text,
    },
    GitError(String),
    DirtyWorkingDirectory(String),
    Custom(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Io(e) => write!(f, "{}", e),
            CliError::ConfigParse(e) => write!(f, "failed to parse Verum.toml: {}", e),
            CliError::ConfigSerialize(e) => write!(f, "failed to serialize config: {}", e),
            CliError::ProjectNotFound(path) => {
                write!(f, "project file not found: {}", path.display())
            }
            CliError::FileNotFound(name) => write!(f, "file not found: {}", name),
            CliError::SemverParse(e) => write!(f, "invalid version: {}", e),
            CliError::PatternError(msg) => write!(f, "pattern error: {}", msg),
            CliError::GlobError(msg) => write!(f, "glob error: {}", msg),
            CliError::NotifyError(e) => write!(f, "file watch error: {}", e),
            CliError::WalkdirError(e) => write!(f, "directory traversal error: {}", e),
            CliError::JsonError(e) => write!(f, "JSON error: {}", e),
            CliError::InvalidProjectName(name) => {
                write!(f, "invalid project name: {}", name)
            }
            CliError::ProjectExists(path) => {
                write!(f, "project already exists at {}", path.display())
            }
            CliError::CompilationFailed(msg) => write!(f, "{}", msg),
            CliError::ParseError {
                file,
                line,
                col,
                message,
            } => write!(f, "{}:{}:{}: {}", file.display(), line, col, message),
            CliError::TypeError(msg) => write!(f, "{}", msg),
            CliError::CbgrError(msg) => write!(f, "CBGR safety check failed: {}", msg),
            CliError::LinkError(msg) => write!(f, "linking failed: {}", msg),
            CliError::Codegen(msg) => write!(f, "code generation failed: {}", msg),
            CliError::TestsFailed { passed, failed } => {
                let total = passed + failed;
                write!(
                    f,
                    "{} of {} tests failed ({} passed)",
                    failed, total, passed
                )
            }
            CliError::BenchmarkFailed(msg) => write!(f, "{}", msg),
            CliError::DependencyNotFound(name) => {
                write!(f, "dependency '{}' not found", name)
            }
            CliError::VersionConflict {
                package,
                required,
                found,
            } => write!(
                f,
                "'{}' requires version {}, but {} is resolved",
                package, required, found
            ),
            CliError::Network(msg) => write!(f, "{}", msg),
            CliError::Registry(msg) => write!(f, "registry: {}", msg),
            CliError::TemplateNotFound(name) => {
                write!(f, "template '{}' not found", name)
            }
            CliError::InvalidTarget(target) => {
                write!(f, "unsupported target: {}", target)
            }
            CliError::CacheCorrupted => write!(f, "build cache is corrupted"),
            CliError::PermissionDenied(path) => {
                write!(f, "permission denied: {}", path.display())
            }
            CliError::CommandNotFound(cmd) => {
                write!(f, "unknown command: {}", cmd)
            }
            CliError::InvalidArgument(msg) => write!(f, "invalid argument: {}", msg),
            CliError::VerificationFailed(msg) => write!(f, "{}", msg),
            CliError::RuntimeError(msg) => write!(f, "{}", msg),
            CliError::ProfilingFailed(msg) => write!(f, "profiling failed: {}", msg),
            CliError::ReplError(msg) => write!(f, "REPL: {}", msg),
            CliError::FeatureNotImplemented {
                feature,
                planned_version,
                workaround,
            } => write!(
                f,
                "'{}' is not yet implemented (planned for {})\n  workaround: {}",
                feature.as_str(),
                planned_version.as_str(),
                workaround.as_str()
            ),
            CliError::GitError(msg) => write!(f, "git: {}", msg),
            CliError::DirtyWorkingDirectory(msg) => {
                write!(f, "working directory has uncommitted changes: {}", msg)
            }
            CliError::Custom(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CliError::Io(e) => Some(e),
            CliError::ConfigParse(e) => Some(e),
            CliError::ConfigSerialize(e) => Some(e),
            CliError::SemverParse(e) => Some(e),
            CliError::NotifyError(e) => Some(e),
            CliError::WalkdirError(e) => Some(e),
            CliError::JsonError(e) => Some(e),
            _ => None,
        }
    }
}

// From impls for automatic conversion (previously provided by thiserror)
impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Io(e)
    }
}

impl From<toml::de::Error> for CliError {
    fn from(e: toml::de::Error) -> Self {
        CliError::ConfigParse(e)
    }
}

impl From<toml::ser::Error> for CliError {
    fn from(e: toml::ser::Error) -> Self {
        CliError::ConfigSerialize(e)
    }
}

impl From<semver::Error> for CliError {
    fn from(e: semver::Error) -> Self {
        CliError::SemverParse(e)
    }
}

impl From<notify::Error> for CliError {
    fn from(e: notify::Error) -> Self {
        CliError::NotifyError(e)
    }
}

impl From<walkdir::Error> for CliError {
    fn from(e: walkdir::Error) -> Self {
        CliError::WalkdirError(e)
    }
}

impl From<serde_json::Error> for CliError {
    fn from(e: serde_json::Error) -> Self {
        CliError::JsonError(e)
    }
}

impl CliError {
    pub fn custom(msg: impl Into<String>) -> Self {
        CliError::Custom(msg.into())
    }

    pub fn verification_failed(msg: impl Into<String>) -> Self {
        CliError::VerificationFailed(msg.into())
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            // Compilation errors (101)
            CliError::CompilationFailed(_)
            | CliError::ParseError { .. }
            | CliError::TypeError(_)
            | CliError::CbgrError(_)
            | CliError::Codegen(_) => 101,

            // Test failures (102)
            CliError::TestsFailed { .. } => 102,

            // Benchmark failures (103)
            CliError::BenchmarkFailed(_) => 103,

            // Link errors (104)
            CliError::LinkError(_) => 104,

            // Verification failures (105)
            CliError::VerificationFailed(_) => 105,

            // Unimplemented features (106)
            CliError::FeatureNotImplemented { .. } => 106,

            // Permission denied (standard Unix code)
            CliError::PermissionDenied(_) => 13,

            // Everything else
            _ => 1,
        }
    }

    /// Returns an optional hint message to help the user resolve the error.
    pub fn hint(&self) -> Option<String> {
        match self {
            CliError::FileNotFound(_) | CliError::ProjectNotFound(_) => Some(
                "Did you mean to run from a project directory? Try `verum init --type binary`"
                    .to_string(),
            ),
            CliError::CompilationFailed(msg) if msg.contains("unresolved") => {
                Some("Try `verum check` first to see all type errors".to_string())
            }
            CliError::VersionConflict { .. } => {
                Some("Run `verum deps update` to resolve conflicts".to_string())
            }
            CliError::CacheCorrupted => {
                Some("Run `verum clean` to clear the build cache and retry".to_string())
            }
            CliError::DependencyNotFound(name) => Some(format!(
                "Check that '{}' is listed in your Verum.toml [dependencies]",
                name
            )),
            CliError::CommandNotFound(cmd) => Some(format!(
                "Run `verum help` to see available commands. Did you mean a different subcommand than '{}'?",
                cmd
            )),
            CliError::InvalidTarget(target) => Some(format!(
                "Run `verum target list` to see supported targets. '{}' is not recognized.",
                target
            )),
            CliError::PermissionDenied(_) => {
                Some("Check file permissions or try running with elevated privileges".to_string())
            }
            _ => None,
        }
    }

    /// Returns the error category label for display purposes.
    pub fn category(&self) -> &'static str {
        match self {
            CliError::CompilationFailed(_)
            | CliError::ParseError { .. }
            | CliError::TypeError(_)
            | CliError::CbgrError(_)
            | CliError::Codegen(_)
            | CliError::LinkError(_) => "compilation",

            CliError::TestsFailed { .. } => "test",
            CliError::BenchmarkFailed(_) => "benchmark",

            CliError::VerificationFailed(_) => "verification",

            CliError::Io(_)
            | CliError::NotifyError(_)
            | CliError::WalkdirError(_) => "io",

            CliError::ConfigParse(_) | CliError::ConfigSerialize(_) => "config",

            CliError::Network(_) | CliError::Registry(_) => "network",

            CliError::DependencyNotFound(_) | CliError::VersionConflict { .. } => "dependency",

            CliError::PermissionDenied(_) => "permission",

            CliError::FeatureNotImplemented { .. } => "unsupported",

            CliError::RuntimeError(_) => "runtime",

            _ => "error",
        }
    }

    /// Check if this error is recoverable
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            CliError::Io(_)
                | CliError::Network(_)
                | CliError::NotifyError(_)
                | CliError::DependencyNotFound(_)
        )
    }

    /// Check if this error is fatal
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            CliError::CacheCorrupted | CliError::PermissionDenied(_)
        )
    }
}

// Convert CliError to the unified VerumError type for cross-crate error handling
impl From<CliError> for verum_error::unified::VerumError {
    fn from(err: CliError) -> Self {
        use verum_error::unified::VerumError;

        match err {
            CliError::Io(io_err) => VerumError::IoError {
                message: io_err.to_string().into(),
            },
            CliError::ConfigParse(e) => VerumError::Other {
                message: format!("config parse error: {}", e).into(),
            },
            CliError::ConfigSerialize(e) => VerumError::Other {
                message: format!("config serialize error: {}", e).into(),
            },
            CliError::ProjectNotFound(path) => VerumError::Other {
                message: format!("project not found: {}", path.display()).into(),
            },
            CliError::FileNotFound(name) => VerumError::Other {
                message: format!("file not found: {}", name).into(),
            },
            CliError::SemverParse(e) => VerumError::Other {
                message: format!("semver parse error: {}", e).into(),
            },
            CliError::PatternError(msg) => VerumError::Other {
                message: format!("pattern error: {}", msg).into(),
            },
            CliError::GlobError(msg) => VerumError::Other {
                message: format!("glob error: {}", msg).into(),
            },
            CliError::NotifyError(e) => VerumError::IoError {
                message: format!("file watch error: {}", e).into(),
            },
            CliError::WalkdirError(e) => VerumError::IoError {
                message: format!("directory traversal error: {}", e).into(),
            },
            CliError::JsonError(e) => VerumError::Other {
                message: format!("JSON error: {}", e).into(),
            },
            CliError::InvalidProjectName(name) => VerumError::Other {
                message: format!("invalid project name: {}", name).into(),
            },
            CliError::ProjectExists(path) => VerumError::Other {
                message: format!("project already exists: {}", path.display()).into(),
            },
            CliError::CompilationFailed(msg) => VerumError::Other {
                message: format!("compilation failed: {}", msg).into(),
            },
            CliError::ParseError {
                file,
                line,
                col,
                message,
            } => VerumError::ParseErrors(vec![format!(
                "{}:{}:{}: {}",
                file.display(),
                line,
                col,
                message
            ).into()].into()),
            CliError::TypeError(msg) => VerumError::TypeMismatch {
                expected: "unknown".into(),
                actual: msg.into(),
            },
            CliError::CbgrError(msg) => VerumError::Other {
                message: format!("CBGR error: {}", msg).into(),
            },
            CliError::LinkError(msg) => VerumError::Other {
                message: format!("link error: {}", msg).into(),
            },
            CliError::Codegen(msg) => VerumError::Other {
                message: format!("codegen error: {}", msg).into(),
            },
            CliError::TestsFailed { passed, failed } => VerumError::Other {
                message: format!("tests failed: {} passed, {} failed", passed, failed).into(),
            },
            CliError::BenchmarkFailed(msg) => VerumError::Other {
                message: format!("benchmark failed: {}", msg).into(),
            },
            CliError::DependencyNotFound(name) => VerumError::Other {
                message: format!("dependency not found: {}", name).into(),
            },
            CliError::VersionConflict {
                package,
                required,
                found,
            } => VerumError::Other {
                message: format!(
                    "version conflict: {} requires {}, but {} was found",
                    package, required, found
                ).into(),
            },
            CliError::Network(msg) => VerumError::NetworkError { message: msg.into() },
            CliError::Registry(msg) => VerumError::NetworkError {
                message: format!("registry error: {}", msg).into(),
            },
            CliError::TemplateNotFound(name) => VerumError::Other {
                message: format!("template not found: {}", name).into(),
            },
            CliError::InvalidTarget(target) => VerumError::Other {
                message: format!("invalid target: {}", target).into(),
            },
            CliError::CacheCorrupted => VerumError::Other {
                message: "build cache corrupted".into(),
            },
            CliError::PermissionDenied(path) => VerumError::IoError {
                message: format!("permission denied: {}", path.display()).into(),
            },
            CliError::CommandNotFound(cmd) => VerumError::Other {
                message: format!("command not found: {}", cmd).into(),
            },
            CliError::InvalidArgument(msg) => VerumError::Other {
                message: format!("invalid argument: {}", msg).into(),
            },
            CliError::VerificationFailed(msg) => VerumError::VerificationFailed {
                reason: msg.into(),
                counterexample: None,
            },
            CliError::RuntimeError(msg) => VerumError::ExecutionError { message: msg.into() },
            CliError::ProfilingFailed(msg) => VerumError::Other {
                message: format!("profiling failed: {}", msg).into(),
            },
            CliError::ReplError(msg) => VerumError::Other {
                message: format!("REPL error: {}", msg).into(),
            },
            CliError::FeatureNotImplemented {
                feature,
                planned_version,
                workaround,
            } => VerumError::NotImplemented {
                feature: format!(
                    "{} (planned: {}). Workaround: {}",
                    feature.as_str(),
                    planned_version.as_str(),
                    workaround.as_str()
                ).into(),
            },
            CliError::GitError(msg) => VerumError::Other {
                message: format!("git error: {}", msg).into(),
            },
            CliError::DirtyWorkingDirectory(msg) => VerumError::Other {
                message: format!("dirty working directory: {}", msg).into(),
            },
            CliError::Custom(msg) => VerumError::Other { message: msg.into() },
        }
    }
}
