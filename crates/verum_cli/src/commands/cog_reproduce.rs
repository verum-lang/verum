//! `verum cog reproduce` — Phase 15 of the precompiled-stdlib
//! archive epic.
//!
//! Verifies that a registry-distributed `.vbca` artefact is the
//! byte-identical output of compiling the cog's source tarball with
//! the active compiler. Catches:
//!
//!   * Registry tampering — a malicious registry serving a
//!     hand-modified `.vbca` that no longer corresponds to the
//!     advertised source.
//!   * Reproducibility regressions — codegen changes that break
//!     deterministic ID assignment / archive layout.
//!
//! Three modes:
//!
//!   1. Fully local: `--source-dir DIR --reference-vbca FILE`.
//!      Precompile the local source tree, byte-compare to a local
//!      reference archive.  Useful for CI / pre-release verification.
//!   2. Tarball-mode: `--source-tar PATH --reference-vbca FILE`.
//!      Extract the tarball, precompile, byte-compare locally.
//!   3. Remote: `<name>@<version> [--registry URL]`.  Fetch the
//!      source tarball + reference `.vbca` from the registry,
//!      precompile locally, byte-compare.  Detects registry
//!      tampering end-to-end.
//!
//! Reuses the Phase 12 `precompile_cog` orchestrator + the Phase 14
//! `vbca_fetcher` + the existing `RegistryClient::download` source
//! fetcher.  Adds only the byte-compare primitive + the CLI surface.

use std::path::{Path, PathBuf};

use verum_compiler::precompile::{PrecompileCogConfig, precompile_cog};

use crate::error::CliError;
use crate::ui;

/// Outcome of a reproducibility check.  Distinguishes match / size-
/// mismatch / byte-mismatch / fetch-failure / compile-failure so the
/// caller can pick an exit code and the user can act on the report.
#[derive(Debug)]
pub enum ReproduceReport {
    /// Byte-identical match.  `size` is the archive size in bytes.
    Match { size: u64 },
    /// Local archive and reference differ in size.  Always
    /// indicates non-reproducibility / tampering.
    SizeMismatch { local: u64, reference: u64 },
    /// Sizes match but bytes diverge.  `first_diff_offset` is the
    /// zero-based offset of the first differing byte.
    ByteMismatch {
        size: u64,
        first_diff_offset: u64,
        local_byte: u8,
        reference_byte: u8,
    },
}

impl ReproduceReport {
    /// True when the archives are byte-identical.
    pub fn is_match(&self) -> bool {
        matches!(self, ReproduceReport::Match { .. })
    }
}

/// Byte-compare two `.vbca` files on disk.  Streams in 64 KiB
/// chunks so very large archives don't pull the whole file into RAM
/// twice.  Returns a typed `ReproduceReport`.
pub fn byte_compare_archives(
    local: &Path,
    reference: &Path,
) -> Result<ReproduceReport, CliError> {
    use std::io::Read;

    let local_size = std::fs::metadata(local)
        .map_err(|e| CliError::Custom(format!("stat {}: {e}", local.display())))?
        .len();
    let ref_size = std::fs::metadata(reference)
        .map_err(|e| CliError::Custom(format!("stat {}: {e}", reference.display())))?
        .len();

    if local_size != ref_size {
        return Ok(ReproduceReport::SizeMismatch {
            local: local_size,
            reference: ref_size,
        });
    }

    let mut local_file = std::fs::File::open(local)
        .map_err(|e| CliError::Custom(format!("open {}: {e}", local.display())))?;
    let mut ref_file = std::fs::File::open(reference)
        .map_err(|e| CliError::Custom(format!("open {}: {e}", reference.display())))?;

    let mut local_buf = vec![0u8; 64 * 1024];
    let mut ref_buf = vec![0u8; 64 * 1024];
    let mut offset: u64 = 0;

    loop {
        let n_local = local_file
            .read(&mut local_buf)
            .map_err(|e| CliError::Custom(format!("read {}: {e}", local.display())))?;
        let n_ref = ref_file
            .read(&mut ref_buf)
            .map_err(|e| CliError::Custom(format!("read {}: {e}", reference.display())))?;
        if n_local != n_ref {
            // Should not happen — we already checked sizes — but
            // surface as a byte-mismatch at the boundary for safety.
            return Ok(ReproduceReport::ByteMismatch {
                size: local_size,
                first_diff_offset: offset + n_local.min(n_ref) as u64,
                local_byte: 0,
                reference_byte: 0,
            });
        }
        if n_local == 0 {
            return Ok(ReproduceReport::Match { size: local_size });
        }
        for i in 0..n_local {
            if local_buf[i] != ref_buf[i] {
                return Ok(ReproduceReport::ByteMismatch {
                    size: local_size,
                    first_diff_offset: offset + i as u64,
                    local_byte: local_buf[i],
                    reference_byte: ref_buf[i],
                });
            }
        }
        offset += n_local as u64;
    }
}

/// Extract a `.tar.gz` source archive into `dest_dir`.  Reuses the
/// same `tar` + `flate2` stack as `cache_manager::extract`.
fn extract_source_tar(tar_path: &Path, dest_dir: &Path) -> Result<(), CliError> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    std::fs::create_dir_all(dest_dir).map_err(|e| {
        CliError::Custom(format!(
            "create dest dir {}: {e}",
            dest_dir.display()
        ))
    })?;

    let file = std::fs::File::open(tar_path)
        .map_err(|e| CliError::Custom(format!("open {}: {e}", tar_path.display())))?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    archive.unpack(dest_dir).map_err(|e| {
        CliError::Custom(format!(
            "unpack {}: {e}",
            tar_path.display()
        ))
    })?;
    Ok(())
}

/// Resolve the actual cog root inside an extracted tarball.  Source
/// archives commonly wrap the contents in a single top-level
/// directory (`<name>-<version>/`); when that's the case the
/// `Verum.toml` lives one level down.  Returns the directory that
/// contains `Verum.toml`.
fn locate_cog_root(extract_dir: &Path) -> Result<PathBuf, CliError> {
    if extract_dir.join("Verum.toml").is_file() {
        return Ok(extract_dir.to_path_buf());
    }
    // Probe one level down — typical tarball layout.
    let entries = std::fs::read_dir(extract_dir).map_err(|e| {
        CliError::Custom(format!(
            "read extracted dir {}: {e}",
            extract_dir.display()
        ))
    })?;
    let mut candidates: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && path.join("Verum.toml").is_file() {
            candidates.push(path);
        }
    }
    match candidates.len() {
        0 => Err(CliError::Custom(format!(
            "no Verum.toml found in extracted source at {} \
             (probed top level + one level down)",
            extract_dir.display()
        ))),
        1 => Ok(candidates.remove(0)),
        _ => Err(CliError::Custom(format!(
            "multiple cog roots found in extracted source at {} — \
             tarball layout is ambiguous",
            extract_dir.display()
        ))),
    }
}

/// Parse `<name>@<version>` into `(name, version)`.  Returns `None`
/// for any input lacking the `@` separator.
fn parse_spec(spec: &str) -> Option<(String, String)> {
    let (name, version) = spec.split_once('@')?;
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some((name.to_string(), version.to_string()))
}

/// Entry point dispatched from `main.rs`.  Resolves the three modes,
/// drives the precompile + byte-compare, and surfaces a typed
/// `ReproduceReport`.
pub fn run(
    spec: Option<String>,
    source_dir: Option<PathBuf>,
    source_tar: Option<PathBuf>,
    reference_vbca: Option<PathBuf>,
    registry_base: Option<String>,
    keep_workdir: bool,
    verbose: bool,
) -> Result<(), CliError> {
    // Working directory for precompile + downloads.  Caller can opt
    // into preservation via --keep-workdir for post-mortem inspection.
    let workdir = std::env::temp_dir().join(format!(
        "verum-reproduce-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&workdir).map_err(|e| {
        CliError::Custom(format!(
            "create workdir {}: {e}",
            workdir.display()
        ))
    })?;

    let parsed_spec = spec.as_deref().and_then(parse_spec);

    // ---------------------------------------------------------------
    // Step 1: resolve the source tree we'll precompile.
    // ---------------------------------------------------------------
    let source_root: PathBuf = if let Some(dir) = source_dir.as_ref() {
        if !dir.join("Verum.toml").is_file() {
            return Err(CliError::Custom(format!(
                "--source-dir {} has no Verum.toml",
                dir.display()
            )));
        }
        dir.canonicalize().map_err(|e| {
            CliError::Custom(format!(
                "canonicalize {}: {e}",
                dir.display()
            ))
        })?
    } else if let Some(tar_path) = source_tar.as_ref() {
        ui::step(&format!(
            "Extracting source tarball {}",
            tar_path.display()
        ));
        let extract_dir = workdir.join("source");
        extract_source_tar(tar_path, &extract_dir)?;
        locate_cog_root(&extract_dir)?
    } else if let Some((name, version)) = parsed_spec.as_ref() {
        // Remote mode: download source tarball from registry.
        let registry = registry_base.as_deref().unwrap_or(crate::registry::DEFAULT_REGISTRY);
        let tar_dest = workdir.join(format!("{name}-{version}.tar.gz"));
        ui::step(&format!(
            "Fetching source tarball for {name}@{version} from {registry}"
        ));
        let client = crate::registry::RegistryClient::new(registry)?;
        client.download(name, version, &tar_dest).map_err(|e| {
            CliError::Custom(format!(
                "registry source download for {name}@{version}: {e}"
            ))
        })?;
        let extract_dir = workdir.join("source");
        extract_source_tar(&tar_dest, &extract_dir)?;
        locate_cog_root(&extract_dir)?
    } else {
        return Err(CliError::Custom(
            "no source provided — pass `<name>@<version>`, `--source-dir DIR`, or `--source-tar PATH`"
                .into(),
        ));
    };

    // ---------------------------------------------------------------
    // Step 2: precompile the source tree to a local `.vbca`.
    // ---------------------------------------------------------------
    let local_out = workdir.join("local.vbca");
    let mut cfg = PrecompileCogConfig::for_cog(&source_root).map_err(|e| {
        CliError::Custom(format!(
            "resolve cog at {}: {e}",
            source_root.display()
        ))
    })?;
    cfg.output_path = local_out.clone();
    cfg.verbose = verbose;

    ui::step(&format!(
        "Precompiling {} {} for reproducibility check",
        cfg.cog_name, cfg.cog_version
    ));
    let result = precompile_cog(&cfg).map_err(|e| {
        CliError::Custom(format!("local precompile failed: {e:?}"))
    })?;
    ui::detail("Modules", &format!("{}", result.modules_compiled));
    ui::detail("Functions", &format!("{}", result.functions_compiled));
    ui::detail("Local size", &format!("{} bytes", result.output_size));

    // ---------------------------------------------------------------
    // Step 3: resolve the reference `.vbca` (local or remote).
    // ---------------------------------------------------------------
    let reference_path: PathBuf = if let Some(p) = reference_vbca.as_ref() {
        if !p.is_file() {
            return Err(CliError::Custom(format!(
                "--reference-vbca {} not found or not a file",
                p.display()
            )));
        }
        p.clone()
    } else if let Some((name, version)) = parsed_spec.as_ref() {
        let registry = registry_base.as_deref().unwrap_or(crate::registry::DEFAULT_REGISTRY);
        let compiler_version = env!("CARGO_PKG_VERSION");
        let cache_root = workdir.join("vbca-cache");
        ui::step(&format!(
            "Fetching reference VBCA for {name}@{version} (compiler {compiler_version}) from {registry}"
        ));
        match crate::registry::vbca_fetcher::fetch_vbca(
            &cache_root,
            registry,
            name,
            version,
            compiler_version,
        ) {
            crate::registry::vbca_fetcher::VbcaFetchOutcome::CacheHit { path }
            | crate::registry::vbca_fetcher::VbcaFetchOutcome::Downloaded { path } => path,
            crate::registry::vbca_fetcher::VbcaFetchOutcome::NotAvailable => {
                return Err(CliError::Custom(format!(
                    "registry has no VBCA for {name}@{version} \
                     (compiler {compiler_version}) — \
                     reproducibility check needs an existing reference archive"
                )));
            }
            crate::registry::vbca_fetcher::VbcaFetchOutcome::Failed { reason } => {
                return Err(CliError::Custom(format!(
                    "registry VBCA fetch failed: {reason}"
                )));
            }
        }
    } else {
        return Err(CliError::Custom(
            "no reference VBCA provided — pass `<name>@<version>` or `--reference-vbca PATH`"
                .into(),
        ));
    };

    ui::detail("Reference", &reference_path.display().to_string());

    // ---------------------------------------------------------------
    // Step 4: byte-compare and report.
    // ---------------------------------------------------------------
    let report = byte_compare_archives(&local_out, &reference_path)?;
    match &report {
        ReproduceReport::Match { size } => {
            ui::step(&format!(
                "Reproducibility check PASSED — {size} bytes byte-identical"
            ));
        }
        ReproduceReport::SizeMismatch { local, reference } => {
            ui::step("Reproducibility check FAILED — size mismatch");
            ui::detail("Local size", &format!("{local} bytes"));
            ui::detail("Reference size", &format!("{reference} bytes"));
        }
        ReproduceReport::ByteMismatch {
            size,
            first_diff_offset,
            local_byte,
            reference_byte,
        } => {
            ui::step("Reproducibility check FAILED — byte mismatch");
            ui::detail("Archive size", &format!("{size} bytes"));
            ui::detail(
                "First diff offset",
                &format!("{first_diff_offset} (0x{first_diff_offset:x})"),
            );
            ui::detail(
                "Local byte",
                &format!("0x{local_byte:02x}"),
            );
            ui::detail(
                "Reference byte",
                &format!("0x{reference_byte:02x}"),
            );
        }
    }

    if !keep_workdir {
        let _ = std::fs::remove_dir_all(&workdir);
    } else {
        ui::detail("Workdir kept", &workdir.display().to_string());
    }

    if !report.is_match() {
        return Err(CliError::Custom(
            "reproducibility check failed — see report above".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_bytes(path: &Path, bytes: &[u8]) {
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(bytes).unwrap();
    }

    #[test]
    fn parse_spec_accepts_canonical_form() {
        assert_eq!(
            parse_spec("foo@1.2.3"),
            Some(("foo".to_string(), "1.2.3".to_string()))
        );
        assert_eq!(
            parse_spec("verum_std@0.1.0-rc1"),
            Some(("verum_std".to_string(), "0.1.0-rc1".to_string()))
        );
    }

    #[test]
    fn parse_spec_rejects_malformed() {
        assert_eq!(parse_spec("noversion"), None);
        assert_eq!(parse_spec("@1.0.0"), None);
        assert_eq!(parse_spec("name@"), None);
        assert_eq!(parse_spec(""), None);
    }

    #[test]
    fn byte_compare_match_for_identical() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.vbca");
        let b = dir.path().join("b.vbca");
        let payload: Vec<u8> = (0..1024u32).map(|n| (n & 0xff) as u8).collect();
        write_bytes(&a, &payload);
        write_bytes(&b, &payload);
        let report = byte_compare_archives(&a, &b).unwrap();
        match report {
            ReproduceReport::Match { size } => assert_eq!(size, 1024),
            other => panic!("expected Match, got {other:?}"),
        }
    }

    #[test]
    fn byte_compare_size_mismatch_for_differing_lengths() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.vbca");
        let b = dir.path().join("b.vbca");
        write_bytes(&a, &[0u8; 100]);
        write_bytes(&b, &[0u8; 200]);
        let report = byte_compare_archives(&a, &b).unwrap();
        match report {
            ReproduceReport::SizeMismatch { local, reference } => {
                assert_eq!(local, 100);
                assert_eq!(reference, 200);
            }
            other => panic!("expected SizeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn byte_compare_byte_mismatch_reports_offset() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.vbca");
        let b = dir.path().join("b.vbca");
        let mut payload_a: Vec<u8> = (0..2048u32).map(|n| (n & 0xff) as u8).collect();
        let mut payload_b = payload_a.clone();
        // Flip a single byte at offset 1337.
        payload_a[1337] = 0xAA;
        payload_b[1337] = 0xBB;
        write_bytes(&a, &payload_a);
        write_bytes(&b, &payload_b);
        let report = byte_compare_archives(&a, &b).unwrap();
        match report {
            ReproduceReport::ByteMismatch {
                size,
                first_diff_offset,
                local_byte,
                reference_byte,
            } => {
                assert_eq!(size, 2048);
                assert_eq!(first_diff_offset, 1337);
                assert_eq!(local_byte, 0xAA);
                assert_eq!(reference_byte, 0xBB);
            }
            other => panic!("expected ByteMismatch, got {other:?}"),
        }
    }

    #[test]
    fn byte_compare_streams_large_archive() {
        // Confirm the 64 KiB chunk loop reaches the second chunk
        // when the diff lands past the first read.
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.vbca");
        let b = dir.path().join("b.vbca");
        let mut payload_a = vec![0u8; 200_000];
        let mut payload_b = payload_a.clone();
        // Diff at offset 100k — well past the 64k chunk boundary.
        payload_a[100_000] = 1;
        payload_b[100_000] = 2;
        write_bytes(&a, &payload_a);
        write_bytes(&b, &payload_b);
        let report = byte_compare_archives(&a, &b).unwrap();
        match report {
            ReproduceReport::ByteMismatch {
                first_diff_offset, ..
            } => {
                assert_eq!(first_diff_offset, 100_000);
            }
            other => panic!("expected ByteMismatch, got {other:?}"),
        }
    }
}
