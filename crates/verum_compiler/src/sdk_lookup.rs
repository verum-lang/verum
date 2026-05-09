//! On-disk Verum SDK lookup, blake3-versioned to the embedded
//! precompiled stdlib archive.
//!
//! # Architecture
//!
//! Production `verum` binaries embed `runtime.vbca` +
//! `runtime.core_metadata` for fast `verum run` / `verum build`
//! cold-start, but never embed stdlib `.vr` source.  Tools that
//! genuinely need source — LSP "go to definition", DAP debugger
//! step-through, the `verum audit` corpus walker — read the source
//! from a separate **SDK** directory installed alongside the binary.
//!
//! The SDK is content-addressed by the blake3 of the precompiled
//! archive's source-input hash, so the SDK on disk is *guaranteed*
//! to match the precompiled archive baked into the binary.  Any
//! drift surfaces as a clear "SDK version mismatch" error rather
//! than silently showing the user pre-update source for code that
//! the new compiler typechecked.
//!
//! # Layout
//!
//! ```text
//! ~/.verum/
//! └── sdk-<blake3-hex-prefix-16>/
//!     ├── core/                 # canonical stdlib source
//!     │   ├── mod.vr
//!     │   ├── base/
//!     │   │   └── …
//!     │   └── …
//!     └── manifest.toml         # full blake3 + verum semver + build-id
//! ```
//!
//! # Lookup order
//!
//! 1. `VERUM_SDK_PATH` env var (explicit override; no blake3 check)
//! 2. `$HOME/.verum/sdk-<prefix>/core/` where `<prefix>` is the
//!    first 16 hex chars of the embedded archive's content_hash
//! 3. (dev mode) `<workspace_root>/core/` when the binary lives
//!    under `<workspace_root>/target/`
//!
//! # API
//!
//! ```ignore
//! use verum_compiler::sdk_lookup::SdkLookup;
//!
//! match SdkLookup::find() {
//!     Some(sdk) => {
//!         let path = sdk.core_path();         // /home/user/.verum/sdk-…/core
//!         let src = sdk.read_module_source("core.text")?;
//!     }
//!     None => {
//!         eprintln!("Verum SDK not installed.  Run `verum sdk install`.");
//!     }
//! }
//! ```

use std::path::{Path, PathBuf};

/// Length of the blake3-prefix that names the SDK directory.  16
/// hex chars = 64 bits of collision space — sufficient for SDK
/// identity while keeping the path short.  Drift contract: the
/// installer + `find_default_sdk_root` must use the same prefix
/// length.
pub const SDK_BLAKE3_PREFIX_LEN: usize = 16;

/// A resolved SDK root.  Carries both the directory path and the
/// blake3 hash it claims to be content-addressed at, so verify-
/// against-archive checks have a single source of truth.
#[derive(Debug, Clone)]
pub struct SdkRoot {
    /// Absolute path to `<sdk-root>/core/`.
    core_path: PathBuf,
    /// blake3 hex prefix this SDK was installed under.  `None` for
    /// `VERUM_SDK_PATH` overrides (the user explicitly opts out of
    /// the version check).
    blake3_prefix: Option<String>,
}

impl SdkRoot {
    /// Path to the SDK's `core/` directory.
    pub fn core_path(&self) -> &Path {
        &self.core_path
    }

    /// blake3 hex prefix this SDK was installed under, when known.
    /// `None` for `VERUM_SDK_PATH` overrides (the user opts out of
    /// version checking explicitly).
    pub fn blake3_prefix(&self) -> Option<&str> {
        self.blake3_prefix.as_deref()
    }

    /// Verify the SDK matches an expected blake3 hex digest.
    ///
    /// Returns `Ok(())` when the SDK's prefix matches the first
    /// `SDK_BLAKE3_PREFIX_LEN` chars of `expected`, or when the SDK
    /// was loaded via `VERUM_SDK_PATH` (no prefix to check).
    pub fn verify_blake3(&self, expected: &str) -> Result<(), SdkError> {
        let prefix = match &self.blake3_prefix {
            Some(p) => p,
            None => return Ok(()), // env-var override: trust the user
        };
        let expected_prefix = &expected[..expected.len().min(SDK_BLAKE3_PREFIX_LEN)];
        if prefix == expected_prefix {
            Ok(())
        } else {
            Err(SdkError::VersionMismatch {
                sdk_prefix: prefix.clone(),
                expected_prefix: expected_prefix.to_string(),
            })
        }
    }

    /// Enumerate every `.vr` file in the SDK's `core/` tree.  Each
    /// entry is `(dotted_module_path, absolute_file_path)`.  The
    /// dotted path follows the same convention as
    /// `stdlib_index::file_path_to_module_path`:
    /// `core/text/sso.vr` → `"core.text.sso"`,
    /// `core/text/mod.vr` → `"core.text"`.
    ///
    /// Used by dev tools (classifier, audit) that need to walk the
    /// full stdlib source tree.  Production code paths
    /// (typecheck / codegen) consume CoreMetadata + .vbca instead and
    /// MUST NOT call this.
    pub fn iter_modules(&self) -> Result<Vec<(String, std::path::PathBuf)>, SdkError> {
        let mut out = Vec::new();
        Self::walk_dir(&self.core_path, &self.core_path, &mut out)?;
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    fn walk_dir(
        root: &Path,
        dir: &Path,
        out: &mut Vec<(String, std::path::PathBuf)>,
    ) -> Result<(), SdkError> {
        let entries = std::fs::read_dir(dir).map_err(|e| SdkError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| SdkError::Io {
                path: dir.to_path_buf(),
                source: e,
            })?;
            let path = entry.path();
            if path.is_dir() {
                Self::walk_dir(root, &path, out)?;
            } else if path.extension().is_some_and(|e| e == "vr") {
                let rel = path.strip_prefix(root).map_err(|_| SdkError::Io {
                    path: path.clone(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "child path not under SDK root",
                    ),
                })?;
                let dotted = Self::rel_to_dotted(rel);
                out.push((dotted, path));
            }
        }
        Ok(())
    }

    fn rel_to_dotted(rel: &Path) -> String {
        let normalised = rel.to_string_lossy().replace('\\', "/");
        let mut parts: Vec<&str> = vec!["core"];
        for component in normalised.split('/') {
            if component.is_empty() {
                continue;
            }
            let trimmed = component.strip_suffix(".vr").unwrap_or(component);
            parts.push(trimmed);
        }
        let joined = parts.join(".");
        joined
            .strip_suffix(".mod")
            .map(str::to_string)
            .unwrap_or(joined)
    }

    /// Read the source of a module by dotted path
    /// (e.g. `"core.text.sso"` → `<core>/text/sso.vr`).
    ///
    /// Honours the same dotted-path → file-path convention as
    /// `stdlib_index::file_path_to_module_path` (in reverse):
    /// `core.X.Y` maps to `<core>/X/Y.vr` first, then
    /// `<core>/X/Y/mod.vr` as the namespace fallback.
    pub fn read_module_source(&self, dotted_path: &str) -> Result<String, SdkError> {
        let stripped = dotted_path
            .strip_prefix("core.")
            .or_else(|| {
                if dotted_path == "core" {
                    Some("")
                } else {
                    None
                }
            })
            .ok_or_else(|| SdkError::ModuleNotInCore {
                path: dotted_path.to_string(),
            })?;

        // Try `<core>/<stripped>.vr` first (single-file module).
        let primary = if stripped.is_empty() {
            self.core_path.join("mod.vr")
        } else {
            self.core_path.join(format!(
                "{}.vr",
                stripped.replace('.', std::path::MAIN_SEPARATOR_STR)
            ))
        };
        if primary.is_file() {
            return std::fs::read_to_string(&primary).map_err(|e| SdkError::Io {
                path: primary,
                source: e,
            });
        }

        // Fallback: `<core>/<stripped>/mod.vr` (namespace module).
        let secondary = if stripped.is_empty() {
            self.core_path.join("mod.vr")
        } else {
            self.core_path
                .join(stripped.replace('.', std::path::MAIN_SEPARATOR_STR))
                .join("mod.vr")
        };
        if secondary.is_file() {
            return std::fs::read_to_string(&secondary).map_err(|e| SdkError::Io {
                path: secondary,
                source: e,
            });
        }

        Err(SdkError::ModuleNotFound {
            path: dotted_path.to_string(),
            tried: vec![primary, secondary],
        })
    }
}

/// Resolve the SDK root for the running `verum` binary.
///
/// Returns `None` when no SDK is installed AND no `VERUM_SDK_PATH`
/// override is set.  Callers that genuinely need source (LSP /
/// debugger / audit) should surface `SdkError::NotInstalled` to
/// the user with a clear remediation pointer:
/// `verum sdk install --version <x.y.z>`.
pub struct SdkLookup;

impl SdkLookup {
    /// Resolve the SDK root.  Honours `VERUM_SDK_PATH` first, then
    /// `~/.verum/sdk-<blake3-prefix>/core/`.
    ///
    /// `archive_blake3_prefix` is the first
    /// [`SDK_BLAKE3_PREFIX_LEN`] hex chars of the embedded
    /// archive's content_hash, used to find the matching on-disk
    /// SDK.  Pass an empty string when the caller doesn't care
    /// about version matching (e.g. early-bootstrap probing).
    pub fn find(archive_blake3_prefix: &str) -> Option<SdkRoot> {
        // 1. Explicit override — trusted, no version check.
        if let Ok(path) = std::env::var("VERUM_SDK_PATH") {
            let p = PathBuf::from(&path);
            let core = p.join("core");
            if core.is_dir() && core.join("mod.vr").is_file() {
                return Some(SdkRoot {
                    core_path: core,
                    blake3_prefix: None,
                });
            }
            // VERUM_SDK_PATH set but invalid — fall through to default
            // search rather than failing silently.  Diagnostic at the
            // call site.
        }

        // 2. Default search: `$HOME/.verum/sdk-<prefix>/core/`.
        if !archive_blake3_prefix.is_empty()
            && let Some(home) = home_dir()
        {
            let candidate = home
                .join(".verum")
                .join(format!("sdk-{}", archive_blake3_prefix))
                .join("core");
            if candidate.is_dir() && candidate.join("mod.vr").is_file() {
                return Some(SdkRoot {
                    core_path: candidate,
                    blake3_prefix: Some(archive_blake3_prefix.to_string()),
                });
            }
        }

        None
    }

    /// Convenience wrapper: derive the blake3 prefix from a full
    /// hex digest then call [`Self::find`].
    pub fn find_for_blake3(full_hex: &str) -> Option<SdkRoot> {
        let prefix = &full_hex[..full_hex.len().min(SDK_BLAKE3_PREFIX_LEN)];
        Self::find(prefix)
    }
}

/// Cross-platform `$HOME` lookup without pulling in the `dirs` /
/// `home` crates as new dependencies.
fn home_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return Some(PathBuf::from(home));
        }
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        if !profile.is_empty() {
            return Some(PathBuf::from(profile));
        }
    }
    None
}

/// SDK resolution / verification errors.
#[derive(Debug)]
pub enum SdkError {
    /// No SDK found via env var or default lookup.  User remediation:
    /// `verum sdk install --version <x.y.z>` (matching the running
    /// `verum` binary's stdlib version).
    NotInstalled,

    /// SDK directory exists but its blake3 prefix differs from the
    /// embedded archive's hash.  User remediation: re-install the
    /// SDK that matches the running binary, or downgrade the
    /// binary.
    VersionMismatch {
        /// Prefix the SDK was installed under (e.g. `"7a2063f35ee99847"`).
        sdk_prefix: String,
        /// Expected prefix (the embedded archive's blake3 prefix).
        expected_prefix: String,
    },

    /// Caller asked for a module not under the `core.` namespace.
    /// SDK only ships the stdlib; user-cog source lives elsewhere.
    ModuleNotInCore { path: String },

    /// SDK is installed but doesn't contain the requested module.
    ModuleNotFound { path: String, tried: Vec<PathBuf> },

    /// Filesystem read error — propagated for the caller's diagnostic.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl std::fmt::Display for SdkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SdkError::NotInstalled => write!(
                f,
                "Verum SDK not installed.  Run `verum sdk install` to fetch \
                 the stdlib source matching this binary."
            ),
            SdkError::VersionMismatch {
                sdk_prefix,
                expected_prefix,
            } => write!(
                f,
                "Verum SDK version mismatch: installed SDK is at blake3 prefix \
                 `{}` but this binary embeds archive prefix `{}`.  Run \
                 `verum sdk install` to refresh.",
                sdk_prefix, expected_prefix
            ),
            SdkError::ModuleNotInCore { path } => write!(
                f,
                "module `{}` is not under the `core.` namespace; SDK ships \
                 only the stdlib",
                path
            ),
            SdkError::ModuleNotFound { path, tried } => {
                write!(
                    f,
                    "module `{}` not found in SDK.  Tried: {}",
                    path,
                    tried
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                )
            }
            SdkError::Io { path, source } => write!(f, "I/O error reading {}: {}", path.display(), source),
        }
    }
}

impl std::error::Error for SdkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SdkError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn verum_sdk_path_override_resolves_when_layout_is_valid() {
        let tmp = TempDir::new().unwrap();
        let sdk_root = tmp.path();
        let core = sdk_root.join("core");
        std::fs::create_dir_all(&core).unwrap();
        std::fs::write(core.join("mod.vr"), "// empty").unwrap();

        // SAFETY: serial test; tests sharing this env-var would
        // interleave, but cargo's default `--test-threads` doesn't
        // serialise across binaries.  This test owns the env for
        // its lifetime via the guard.
        let _guard = EnvGuard::set("VERUM_SDK_PATH", sdk_root.to_str().unwrap());

        let resolved = SdkLookup::find("deadbeefdeadbeef")
            .expect("override should resolve");
        assert_eq!(resolved.core_path(), core.as_path());
        assert_eq!(resolved.blake3_prefix(), None);
    }

    #[test]
    fn missing_sdk_returns_none() {
        let _guard = EnvGuard::unset_pair("VERUM_SDK_PATH", "HOME");
        let _home_guard = EnvGuard::set("HOME", "/nonexistent-home-9c4b");
        // Use a prefix that can't possibly match a real install.
        let resolved = SdkLookup::find("ffffffff_no_sdk");
        assert!(resolved.is_none(), "expected no SDK to be found");
    }

    #[test]
    fn version_mismatch_surfaces_clear_error() {
        let sdk = SdkRoot {
            core_path: PathBuf::from("/dummy"),
            blake3_prefix: Some("7a2063f35ee99847".to_string()),
        };
        match sdk.verify_blake3("deadbeefdeadbeef0000000000000000") {
            Err(SdkError::VersionMismatch {
                sdk_prefix,
                expected_prefix,
            }) => {
                assert_eq!(sdk_prefix, "7a2063f35ee99847");
                assert_eq!(expected_prefix, "deadbeefdeadbeef");
            }
            other => panic!("expected VersionMismatch, got {:?}", other),
        }
    }

    #[test]
    fn override_sdk_skips_blake3_check() {
        let sdk = SdkRoot {
            core_path: PathBuf::from("/dummy"),
            blake3_prefix: None, // VERUM_SDK_PATH path
        };
        sdk.verify_blake3("anything").expect("override skips check");
    }

    /// RAII guard for env vars within tests.  Restores prior value
    /// on drop, even if the test panics.
    struct EnvGuard {
        keys: Vec<(String, Option<String>)>,
    }
    impl EnvGuard {
        fn set(key: &str, value: &str) -> Self {
            let prior = std::env::var(key).ok();
            // SAFETY: cross-thread set is unsound on some platforms,
            // but cargo runs tests in a single thread per binary by
            // default for filesystem-test isolation.  These tests
            // don't spawn additional threads.
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                keys: vec![(key.to_string(), prior)],
            }
        }
        fn unset_pair(a: &str, b: &str) -> Self {
            let prior_a = std::env::var(a).ok();
            let prior_b = std::env::var(b).ok();
            unsafe {
                std::env::remove_var(a);
                std::env::remove_var(b);
            }
            Self {
                keys: vec![
                    (a.to_string(), prior_a),
                    (b.to_string(), prior_b),
                ],
            }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in self.keys.drain(..) {
                unsafe {
                    match v {
                        Some(prior) => std::env::set_var(&k, prior),
                        None => std::env::remove_var(&k),
                    }
                }
            }
        }
    }
}
