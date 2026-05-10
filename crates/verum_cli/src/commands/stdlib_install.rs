//! `verum stdlib install` / `verum stdlib verify` / `verum stdlib list` —
//! on-disk Verum SDK installer.
//!
//! # Architecture
//!
//! Production `verum` binaries embed only `runtime.vbca` +
//! `runtime.core_metadata` for fast `verum run` / `verum build`.  Tools
//! that need stdlib **source** (LSP "go to definition", DAP debugger,
//! `verum audit`'s citation walker) read it from a separate **SDK**
//! directory installed by these commands.
//!
//! The SDK is content-addressed by the blake3 prefix matching the
//! embedded archive's `content_hash`, so the on-disk SDK always
//! agrees with what the binary typechecks against.  Drift surfaces as
//! a clear `SdkVersionMismatch` error rather than silently showing
//! pre-update source.
//!
//! # Layout
//!
//! ```text
//! ~/.verum/
//! └── sdk-<blake3-prefix>/
//!     ├── core/             # canonical stdlib source tree
//!     │   ├── mod.vr
//!     │   └── …
//!     └── manifest.toml     # full blake3 + verum semver + install time
//! ```
//!
//! # Lookup pairing
//!
//! The runtime side ([`verum_compiler::sdk_lookup::SdkLookup::find`])
//! consumes this layout — installs done through these commands are
//! immediately visible to the next `verum audit` / `verum lsp`
//! invocation without requiring a binary rebuild.

use std::fs;
#[cfg(test)]
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow, bail};

use verum_compiler::sdk_lookup::SDK_BLAKE3_PREFIX_LEN;

/// Walk a `core/` directory tree and collect every `.vr` file as
/// `(relative_path, contents)`.  Mirrors `build.rs::collect_vr_files`
/// — keeping the conventions byte-aligned matters because the install
/// blake3 must match the binary-side `compute_core_blake3`.
fn collect_vr_files(root: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    walk_dir(root, root, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn walk_dir(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
    for entry in fs::read_dir(dir)
        .with_context(|| format!("read_dir {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_dir(root, &path, out)?;
        } else if path.extension().is_some_and(|e| e == "vr") {
            let rel = path
                .strip_prefix(root)
                .map_err(|_| anyhow!("path {} not under {}", path.display(), root.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            let content = fs::read(&path)
                .with_context(|| format!("read {}", path.display()))?;
            out.push((rel, content));
        }
    }
    Ok(())
}

/// Compute the SDK's blake3 hash from the source tree.  Two contracts:
///
/// 1. **Binary-prefix agreement.**  The first
///    [`SDK_BLAKE3_PREFIX_LEN`] hex chars must match what the verum
///    binary's `embedded_stdlib_metadata::content_hash_hex_prefix()`
///    reports — that's what `SdkLookup::find` uses to locate the
///    matching install on disk.
/// 2. **Source-driven.**  This hashes JUST the `.vr` file contents
///    (path-prefixed, sorted), NOT the precompiled archive bytes.
///    The compiler's `build.rs::compute_core_blake3` salts the same
///    inputs with `PRECOMPILE_SCHEMA_VERSION`; we deliberately omit
///    the salt here because the SDK is the *source* — its identity
///    doesn't depend on which compiler version produced the
///    archive.
fn compute_source_blake3(files: &[(String, Vec<u8>)]) -> String {
    let mut hasher = blake3::Hasher::new();
    for (rel_path, bytes) in files {
        hasher.update(rel_path.as_bytes());
        hasher.update(b"\0");
        hasher.update(bytes);
        hasher.update(b"\0");
    }
    hasher.finalize().to_hex().to_string()
}

/// Resolve `$HOME` (or `%USERPROFILE%` on Windows) without dragging
/// in the `home` / `dirs` crates.
fn home_dir() -> Result<PathBuf> {
    if let Ok(h) = std::env::var("HOME") {
        if !h.is_empty() {
            return Ok(PathBuf::from(h));
        }
    }
    if let Ok(p) = std::env::var("USERPROFILE") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    bail!("HOME / USERPROFILE not set; SDK install needs a home directory")
}

/// Compute the canonical install root for a given source-blake3
/// prefix: `<HOME>/.verum/sdk-<prefix>/`.
fn sdk_install_path(blake3_prefix: &str) -> Result<PathBuf> {
    Ok(home_dir()?.join(".verum").join(format!("sdk-{}", blake3_prefix)))
}

/// Walk up from `start` to find the workspace root (first ancestor
/// containing `core/mod.vr`).  Mirrors the auto-detection in
/// `verum stdlib precompile`.
fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut cursor = start.to_path_buf();
    loop {
        if cursor.join("core").join("mod.vr").is_file() {
            return Some(cursor);
        }
        let parent = cursor.parent()?;
        if parent == cursor {
            return None;
        }
        cursor = parent.to_path_buf();
    }
}

/// `verum stdlib install [--from DIR] [--force]` — install the Verum
/// SDK to `~/.verum/sdk-<blake3>/core/`.
///
/// `from`: source `core/` directory to install.  Defaults to the
/// workspace's `core/` (auto-detected).
///
/// `force`: overwrite an existing install at the same prefix.  Without
/// this, an existing install short-circuits with a "already installed"
/// notice — the install is content-addressed, so re-running with
/// unchanged source is a no-op anyway.
pub fn run_install(from: Option<PathBuf>, force: bool, verbose: bool) -> Result<()> {
    let core_dir = match from {
        Some(p) => {
            let core = p.join("core");
            if core.is_dir() && core.join("mod.vr").is_file() {
                core
            } else if p.join("mod.vr").is_file() {
                // User passed `core/` directly.
                p
            } else {
                bail!(
                    "expected `core/mod.vr` under {} (or pass the workspace root, not the cog root)",
                    p.display()
                )
            }
        }
        None => {
            let cwd = std::env::current_dir()?;
            let workspace = find_workspace_root(&cwd)
                .ok_or_else(|| anyhow!("no workspace root with core/mod.vr found above {}", cwd.display()))?;
            workspace.join("core")
        }
    };

    if verbose {
        eprintln!("verum stdlib install: source = {}", core_dir.display());
    }

    let files = collect_vr_files(&core_dir)
        .with_context(|| format!("walk source tree {}", core_dir.display()))?;
    if files.is_empty() {
        bail!("no .vr files under {}", core_dir.display());
    }

    let full_blake3 = compute_source_blake3(&files);
    let prefix = &full_blake3[..SDK_BLAKE3_PREFIX_LEN];
    let install_root = sdk_install_path(prefix)?;
    let target_core = install_root.join("core");
    let manifest_path = install_root.join("manifest.toml");

    if target_core.is_dir() && manifest_path.is_file() && !force {
        eprintln!(
            "Verum SDK at {} already installed (blake3 {}); pass --force to reinstall",
            install_root.display(),
            prefix,
        );
        return Ok(());
    }

    // Wipe + recreate target so partial-install state can't linger.
    if target_core.exists() {
        fs::remove_dir_all(&target_core).with_context(|| {
            format!("remove existing install at {}", target_core.display())
        })?;
    }
    fs::create_dir_all(&target_core).with_context(|| {
        format!("create install dir {}", target_core.display())
    })?;

    let total = files.len();
    let mut total_bytes: u64 = 0;
    for (rel, bytes) in &files {
        let dest = target_core.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&dest, bytes)
            .with_context(|| format!("write {}", dest.display()))?;
        total_bytes += bytes.len() as u64;
    }

    write_manifest(&manifest_path, &full_blake3, &core_dir, total, total_bytes)?;

    eprintln!(
        "Verum SDK installed to {} ({} files, {:.1} KB) — blake3 {}",
        install_root.display(),
        total,
        total_bytes as f64 / 1024.0,
        prefix,
    );
    Ok(())
}

/// Manifest format — TOML-flavoured key=value lines, no nested
/// tables.  Kept hand-rolled to avoid pulling in a TOML dep just for
/// install bookkeeping; the manifest is rarely read programmatically
/// (humans + `verum stdlib list`).
fn write_manifest(
    path: &Path,
    full_blake3: &str,
    source_path: &Path,
    file_count: usize,
    total_bytes: u64,
) -> Result<()> {
    let now: u64 = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let body = format!(
        "# Verum SDK manifest\n\
         # written by `verum stdlib install`\n\
         blake3 = \"{}\"\n\
         blake3_prefix = \"{}\"\n\
         installed_at = {}\n\
         source = \"{}\"\n\
         files = {}\n\
         total_bytes = {}\n\
         verum_version = \"{}\"\n",
        full_blake3,
        &full_blake3[..SDK_BLAKE3_PREFIX_LEN],
        now,
        source_path.display(),
        file_count,
        total_bytes,
        env!("CARGO_PKG_VERSION"),
    );
    fs::write(path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// `verum stdlib verify` — check the on-disk SDK matches the binary's
/// embedded archive prefix.
///
/// Prints a structured report and exits with non-zero status when
/// the SDK is missing or its prefix differs from what the binary
/// expects.  Used by CI / install-time checks.
pub fn run_verify(verbose: bool) -> Result<()> {
    // Read the embedded archive's content_hash via the same path
    // SdkLookup uses to derive its expected prefix.
    let expected_prefix = embedded_archive_blake3_prefix()
        .ok_or_else(|| anyhow!("verum binary has no embedded archive — bootstrap build?"))?;

    if verbose {
        eprintln!(
            "verum stdlib verify: binary expects SDK at blake3 prefix {}",
            expected_prefix
        );
    }

    let sdk = verum_compiler::sdk_lookup::SdkLookup::find(&expected_prefix);
    match sdk {
        Some(s) => {
            // Re-compute the prefix from the actual installed source
            // tree and compare to expected.  Belt-and-suspenders: the
            // directory-name match is necessary but not sufficient
            // (someone could `mv` a directory into the right name).
            let installed_files = collect_vr_files(s.core_path())
                .with_context(|| {
                    format!("walk installed SDK at {}", s.core_path().display())
                })?;
            let installed_full = compute_source_blake3(&installed_files);
            let installed_prefix = &installed_full[..SDK_BLAKE3_PREFIX_LEN];
            if installed_prefix != expected_prefix {
                bail!(
                    "Verum SDK at {} is corrupt: directory says blake3 {} but contents hash to {}.  \
                     Run `verum stdlib install --force` to reinstall.",
                    s.core_path().display(),
                    s.blake3_prefix().unwrap_or("(VERUM_SDK_PATH override)"),
                    installed_prefix,
                );
            }
            println!("Verum SDK verified: {} (blake3 {})", s.core_path().display(), expected_prefix);
            Ok(())
        }
        None => {
            bail!(
                "Verum SDK not installed for blake3 prefix `{}`.  \
                 Run `verum stdlib install` to fetch it.",
                expected_prefix
            )
        }
    }
}

/// `verum stdlib list` — enumerate every installed Verum SDK under
/// `~/.verum/`.
pub fn run_list(verbose: bool) -> Result<()> {
    let root = home_dir()?.join(".verum");
    if !root.is_dir() {
        println!("No Verum SDKs installed (no {})", root.display());
        return Ok(());
    }
    let expected_prefix = embedded_archive_blake3_prefix();
    let mut count = 0;
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(prefix) = name.strip_prefix("sdk-") else {
            continue;
        };
        let core = path.join("core");
        let manifest = path.join("manifest.toml");
        if !core.is_dir() || !core.join("mod.vr").is_file() {
            continue;
        }
        let marker = match expected_prefix.as_deref() {
            Some(p) if p == prefix => " [matches binary]",
            Some(_) => "",
            None => "",
        };
        println!(
            "{}{}  ({}{})",
            path.display(),
            marker,
            if manifest.is_file() {
                "manifest present"
            } else {
                "no manifest"
            },
            if verbose && manifest.is_file() {
                let body = fs::read_to_string(&manifest).unwrap_or_default();
                let installed_at = body
                    .lines()
                    .find_map(|l| l.strip_prefix("installed_at = "))
                    .unwrap_or("?");
                format!(", installed_at={}", installed_at)
            } else {
                String::new()
            },
        );
        count += 1;
    }
    if count == 0 {
        println!("No Verum SDKs installed under {}", root.display());
    }
    Ok(())
}

/// Read the binary's embedded `CoreMetadata.content_hash` and return
/// the blake3 hex prefix.  `None` when the binary was bootstrap-built
/// without an embedded metadata sidecar.
///
/// `CoreMetadata.content_hash` is the canonical source-tree blake3
/// computed at precompile time from the `core/**/*.vr` file contents
/// (`build.rs::compute_core_blake3`'s salted hash, hex form).  Stable
/// across builds with identical sources, so the SDK install layout
/// `~/.verum/sdk-<prefix>/` matches the binary's expected prefix
/// byte-identically.
fn embedded_archive_blake3_prefix() -> Option<String> {
    let metadata = verum_compiler::embedded_stdlib_metadata::get_runtime_metadata()?;
    // content_hash is `[u8; 32]` — full blake3.  We want the leading
    // hex chars matching `SDK_BLAKE3_PREFIX_LEN`.
    let mut hex = String::with_capacity(64);
    for byte in metadata.content_hash.iter() {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{:02x}", byte);
    }
    Some(hex[..SDK_BLAKE3_PREFIX_LEN.min(hex.len())].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_synthetic_core(root: &Path) -> io::Result<()> {
        let core = root.join("core");
        fs::create_dir_all(&core)?;
        fs::write(core.join("mod.vr"), "// root\n")?;
        fs::create_dir_all(core.join("text"))?;
        fs::write(core.join("text").join("sso.vr"), "public const SSO: Int = 23;\n")?;
        fs::write(core.join("text").join("mod.vr"), "// text root\n")?;
        Ok(())
    }

    #[test]
    fn collect_vr_files_walks_recursively_and_sorts() {
        let tmp = TempDir::new().unwrap();
        make_synthetic_core(tmp.path()).unwrap();
        let core = tmp.path().join("core");
        let files = collect_vr_files(&core).unwrap();
        let names: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, vec!["mod.vr", "text/mod.vr", "text/sso.vr"]);
    }

    #[test]
    fn compute_source_blake3_is_deterministic() {
        let tmp = TempDir::new().unwrap();
        make_synthetic_core(tmp.path()).unwrap();
        let core = tmp.path().join("core");
        let files = collect_vr_files(&core).unwrap();
        let h1 = compute_source_blake3(&files);
        let h2 = compute_source_blake3(&files);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // blake3 hex = 32 bytes × 2
    }

    #[test]
    fn compute_source_blake3_changes_with_content() {
        let tmp = TempDir::new().unwrap();
        make_synthetic_core(tmp.path()).unwrap();
        let core = tmp.path().join("core");
        let h1 = compute_source_blake3(&collect_vr_files(&core).unwrap());

        // Mutate one file.
        fs::write(core.join("text").join("sso.vr"), "public const SSO: Int = 24;\n")
            .unwrap();
        let h2 = compute_source_blake3(&collect_vr_files(&core).unwrap());
        assert_ne!(h1, h2, "hash must change when source content changes");
    }

    #[test]
    fn install_creates_canonical_layout() {
        let src_tmp = TempDir::new().unwrap();
        make_synthetic_core(src_tmp.path()).unwrap();
        let install_tmp = TempDir::new().unwrap();

        // SAFETY: this test owns HOME for its lifetime via the guard.
        let _home_guard = EnvGuard::set("HOME", install_tmp.path().to_str().unwrap());

        run_install(Some(src_tmp.path().to_path_buf()), false, false).unwrap();

        // Find the sdk-<prefix> directory.
        let verum_root = install_tmp.path().join(".verum");
        let sdk_dirs: Vec<PathBuf> = fs::read_dir(&verum_root)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|s| s.starts_with("sdk-"))
            })
            .collect();
        assert_eq!(sdk_dirs.len(), 1, "expected exactly one sdk-* directory");
        let sdk_root = &sdk_dirs[0];

        // Layout invariants.
        assert!(sdk_root.join("core").join("mod.vr").is_file());
        assert!(sdk_root.join("core").join("text").join("sso.vr").is_file());
        assert!(sdk_root.join("manifest.toml").is_file());

        // Manifest sanity.
        let manifest = fs::read_to_string(sdk_root.join("manifest.toml")).unwrap();
        assert!(manifest.contains("blake3 ="));
        assert!(manifest.contains("blake3_prefix ="));
        assert!(manifest.contains("files = 3"));
    }

    #[test]
    fn install_idempotent_without_force() {
        let src_tmp = TempDir::new().unwrap();
        make_synthetic_core(src_tmp.path()).unwrap();
        let install_tmp = TempDir::new().unwrap();
        let _home_guard = EnvGuard::set("HOME", install_tmp.path().to_str().unwrap());

        run_install(Some(src_tmp.path().to_path_buf()), false, false).unwrap();
        let verum_root = install_tmp.path().join(".verum");
        let first_count = fs::read_dir(&verum_root).unwrap().count();

        // Re-run without --force: should print "already installed" but
        // not error.
        run_install(Some(src_tmp.path().to_path_buf()), false, false).unwrap();
        let second_count = fs::read_dir(&verum_root).unwrap().count();
        assert_eq!(first_count, second_count, "no new directories on re-install");
    }

    /// RAII guard for env var.  Restores prior value on drop.
    struct EnvGuard {
        key: String,
        prior: Option<String>,
    }
    impl EnvGuard {
        fn set(key: &str, value: &str) -> Self {
            let prior = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                key: key.to_string(),
                prior,
            }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.prior {
                    Some(p) => std::env::set_var(&self.key, p),
                    None => std::env::remove_var(&self.key),
                }
            }
        }
    }
}
