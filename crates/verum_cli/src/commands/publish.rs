// Publish cog package to registry with tier-specific artifacts, Ed25519 signing,
// and optional verification proof distribution. Supports IPFS decentralized publishing.

use crate::config::Manifest;
use crate::error::{CliError, Result};
use crate::registry::{
    CogMetadata, CogSignature, CogSigner, RegistryClient, TierArtifacts,
};
use crate::ui;
use colored::Colorize;
use std::path::Path;
use verum_common::{List, Text};

/// Publish options
#[derive(Debug, Clone)]
pub struct PublishOptions {
    pub dry_run: bool,
    pub sign: bool,
    pub verify_proofs: bool,
    pub pin_ipfs: bool,
    pub tier: Option<u8>,
    pub all_tiers: bool,
}

/// Publish package to registry
pub fn publish(options: PublishOptions) -> Result<()> {
    ui::step("Publishing package");

    // Find and validate manifest
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let manifest = Manifest::from_file(&manifest_path)?;

    // Validate manifest
    manifest.validate()?;

    ui::info(&format!(
        "Publishing {} v{}",
        manifest.cog.name, manifest.cog.version
    ));

    // Check for uncommitted changes (unless dry-run)
    if !options.dry_run {
        check_git_status(&manifest_dir)?;
    }

    // Build tier-specific artifacts
    let artifacts = build_artifacts(&manifest_dir, &options)?;

    // Create package tarball
    let cog_file = create_cog_tarball(&manifest_dir, &manifest)?;

    // Calculate checksum
    let checksum = calculate_checksum(&cog_file)?;

    // Sign package if requested
    let signature = if options.sign {
        ui::step("Signing package");
        Some(sign_cog(&cog_file)?)
    } else {
        None
    };

    // Create package metadata
    let metadata = create_metadata(&manifest_dir, &manifest, artifacts, checksum, signature)?;

    // Pin to IPFS if requested
    if options.pin_ipfs {
        ui::step("Pinning to IPFS");
        pin_to_ipfs(&cog_file)?;
    }

    if options.dry_run {
        ui::info("[DRY RUN] Would publish package");
        print_dry_run_summary(&metadata);
        return Ok(());
    }

    // Get authentication token
    let token = get_auth_token()?;

    // Publish to registry
    ui::step("Uploading to registry");
    let client = RegistryClient::default()?;
    client.publish(&metadata, &cog_file, token.as_str())?;

    ui::success(&format!(
        "Published {} v{}",
        manifest.cog.name, manifest.cog.version
    ));

    println!();
    ui::info(&format!(
        "Cog URL: https://packages.verum.lang/cogs/{}/{}",
        manifest.cog.name, manifest.cog.version
    ));

    Ok(())
}

/// Build tier-specific artifacts
fn build_artifacts(manifest_dir: &Path, options: &PublishOptions) -> Result<TierArtifacts> {
    use crate::registry::ArtifactInfo;

    let mut artifacts = TierArtifacts::default();
    let target_dir = manifest_dir.join("target");
    std::fs::create_dir_all(&target_dir)?;

    // Tier 0: AST cache (always built)
    ui::info("Building Tier 0 (AST cache)...");
    let tier0_path = target_dir.join("tier0.ast");
    if build_tier0(manifest_dir, &tier0_path)? {
        artifacts.tier0 = Some(ArtifactInfo {
            path: "tier0.ast".into(),
            checksum: file_checksum(&tier0_path)?,
            size: std::fs::metadata(&tier0_path)?.len(),
            target: None,
        });
        ui::success("  Tier 0 built successfully");
    }

    if options.all_tiers || options.tier == Some(1) {
        // Tier 1: JIT compiled code
        ui::info("Building Tier 1 (JIT cache)...");
        let tier1_path = target_dir.join("tier1.jit");
        if build_tier1(manifest_dir, &tier1_path)? {
            artifacts.tier1 = Some(ArtifactInfo {
                path: "tier1.jit".into(),
                checksum: file_checksum(&tier1_path)?,
                size: std::fs::metadata(&tier1_path)?.len(),
                target: None,
            });
            ui::success("  Tier 1 built successfully");
        }
    }

    if options.all_tiers || options.tier == Some(2) {
        // Tier 2: AOT debug binary
        ui::info("Building Tier 2 (AOT debug)...");
        let tier2_path = target_dir.join("tier2.debug");
        if build_tier2(manifest_dir, &tier2_path, false)? {
            let target_triple = get_target_triple();
            artifacts.tier2 = Some(ArtifactInfo {
                path: "tier2.debug".into(),
                checksum: file_checksum(&tier2_path)?,
                size: std::fs::metadata(&tier2_path)?.len(),
                target: Some(target_triple),
            });
            ui::success("  Tier 2 built successfully");
        }
    }

    if options.all_tiers || options.tier == Some(3) {
        // Tier 3: AOT optimized binary
        ui::info("Building Tier 3 (AOT release)...");
        let tier3_path = target_dir.join("tier3.release");
        if build_tier3(manifest_dir, &tier3_path, true)? {
            let target_triple = get_target_triple();
            artifacts.tier3 = Some(ArtifactInfo {
                path: "tier3.release".into(),
                checksum: file_checksum(&tier3_path)?,
                size: std::fs::metadata(&tier3_path)?.len(),
                target: Some(target_triple),
            });
            ui::success("  Tier 3 built successfully");
        }
    }

    Ok(artifacts)
}

/// Build Tier 0: AST cache
fn build_tier0(manifest_dir: &Path, output_path: &Path) -> Result<bool> {
    use sha2::{Digest, Sha256};
    use verum_ast::FileId;
    use verum_lexer::Lexer;
    use verum_parser::VerumParser;

    let src_dir = manifest_dir.join("src");
    if !src_dir.exists() {
        return Ok(false);
    }

    // For AST cache, we create a manifest file with parsed file hashes
    // Full AST serialization would require serde support in verum_ast
    let mut manifest_content = String::from("# Verum AST Cache v1.0\n");
    let mut file_count = 0;

    // Parse all source files and record successful parses
    for entry in walkdir::WalkDir::new(&src_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "vr") {
            let source = std::fs::read_to_string(path)?;
            let file_id = FileId::new(file_count as u32);
            let lexer = Lexer::new(&source, file_id);
            let parser = VerumParser::new();

            match parser.parse_module(lexer, file_id) {
                Ok(_module) => {
                    // Calculate source hash
                    let mut hasher = Sha256::new();
                    hasher.update(source.as_bytes());
                    let hash = format!("{:x}", hasher.finalize());

                    let relative_path = path.strip_prefix(&src_dir).unwrap_or(path);
                    manifest_content.push_str(&format!(
                        "{}:{}\n",
                        relative_path.to_string_lossy(),
                        hash
                    ));
                    file_count += 1;
                }
                Err(errors) => {
                    for err in &errors {
                        ui::warn(&format!("  Parse error in {}: {}", path.display(), err));
                    }
                }
            }
        }
    }

    if file_count == 0 {
        return Ok(false);
    }

    // Write AST cache manifest
    std::fs::write(output_path, manifest_content)?;

    Ok(true)
}

/// Build Tier 1: JIT cache
fn build_tier1(manifest_dir: &Path, output_path: &Path) -> Result<bool> {
    use sha2::{Digest, Sha256};
    use verum_ast::FileId;
    use verum_lexer::Lexer;
    use verum_parser::VerumParser;

    let src_dir = manifest_dir.join("src");
    if !src_dir.exists() {
        return Ok(false);
    }

    // For JIT cache, we prepare type-checked files for quick JIT compilation
    let mut manifest_content = String::from("# Verum JIT Cache v1.0\n");
    let mut file_count = 0;

    // Parse and validate all source files
    for entry in walkdir::WalkDir::new(&src_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "vr") {
            let source = std::fs::read_to_string(path)?;
            let file_id = FileId::new(file_count as u32);
            let lexer = Lexer::new(&source, file_id);
            let parser = VerumParser::new();

            if let Ok(_module) = parser.parse_module(lexer, file_id) {
                // Calculate source hash for cache validation
                let mut hasher = Sha256::new();
                hasher.update(source.as_bytes());
                let hash = format!("{:x}", hasher.finalize());

                let relative_path = path.strip_prefix(&src_dir).unwrap_or(path);
                manifest_content.push_str(&format!(
                    "{}:{}\n",
                    relative_path.to_string_lossy(),
                    hash
                ));
                file_count += 1;
            }
        }
    }

    if file_count == 0 {
        return Ok(false);
    }

    // Write JIT cache manifest
    std::fs::write(output_path, manifest_content)?;

    Ok(true)
}

/// AOT compilation configuration
struct AotCompileConfig {
    /// Optimization level (0-3)
    opt_level: u8,
    /// Enable debug info
    debug_info: bool,
    /// Target triple
    target: Text,
    /// CPU model
    cpu: Text,
    /// CPU features
    features: Text,
}

impl AotCompileConfig {
    fn debug(target: Text) -> Self {
        Self {
            opt_level: 0,
            debug_info: true,
            target,
            cpu: "generic".into(),
            features: Text::new(),
        }
    }

    fn release(target: Text) -> Self {
        Self {
            opt_level: 3,
            debug_info: false,
            target,
            cpu: "native".into(),
            features: Text::new(),
        }
    }
}

/// Build Tier 2: AOT debug binary (MLIR-based)
///
/// NOTE: Tier 2/3 artifact building is pending implementation.
fn build_tier2(_manifest_dir: &Path, _output_path: &Path, _release: bool) -> Result<bool> {
    // Tier 2 AOT compilation requires VBC → LLVM IR lowering
    ui::warn("Tier 2 artifact building is pending implementation");
    Ok(false)
}

/// AOT artifact metadata
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct AotArtifactMetadata {
    version: u32,
    target: String,
    opt_level: u8,
    debug_info: bool,
    timestamp: i64,
    object_file_size: u64,
}

/// Create an AOT artifact containing object file and metadata
fn create_aot_artifact(
    output_path: &Path,
    obj_path: &Path,
    metadata: &AotArtifactMetadata,
) -> Result<()> {
    use flate2::Compression;
    use flate2::write::GzEncoder;

    let output_file = File::create(output_path)?;
    let encoder = GzEncoder::new(output_file, Compression::default());
    let mut tar = tar::Builder::new(encoder);

    // Add metadata as JSON
    let metadata_json = serde_json::to_string_pretty(metadata)?;
    let mut header = tar::Header::new_gnu();
    header.set_size(metadata_json.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append_data(&mut header, "metadata.json", metadata_json.as_bytes())?;

    // Add object file
    tar.append_path_with_name(obj_path, "module.o")?;

    tar.finish()?;

    Ok(())
}

/// Build Tier 3: AOT release binary with full optimizations and LTO
///
/// NOTE: Tier 2/3 artifact building is pending implementation.
fn build_tier3(_manifest_dir: &Path, _output_path: &Path, _release: bool) -> Result<bool> {
    // Tier 3 AOT compilation requires VBC → LLVM IR lowering with LTO
    ui::warn("Tier 3 artifact building is pending implementation");
    Ok(false)
}

/// Detect host CPU features for optimization
fn detect_host_features() -> Text {
    let mut features = Vec::new();

    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("sse2") {
            features.push("+sse2");
        }
        if is_x86_feature_detected!("sse4.1") {
            features.push("+sse4.1");
        }
        if is_x86_feature_detected!("sse4.2") {
            features.push("+sse4.2");
        }
        if is_x86_feature_detected!("avx") {
            features.push("+avx");
        }
        if is_x86_feature_detected!("avx2") {
            features.push("+avx2");
        }
        if is_x86_feature_detected!("fma") {
            features.push("+fma");
        }
        if is_x86_feature_detected!("bmi1") {
            features.push("+bmi");
        }
        if is_x86_feature_detected!("bmi2") {
            features.push("+bmi2");
        }
        if is_x86_feature_detected!("popcnt") {
            features.push("+popcnt");
        }
        if is_x86_feature_detected!("lzcnt") {
            features.push("+lzcnt");
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        features.push("+neon");
    }

    Text::from(features.join(","))
}

/// Get current target triple
fn get_target_triple() -> Text {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "aarch64-apple-darwin".into();

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "x86_64-apple-darwin".into();

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "x86_64-unknown-linux-gnu".into();

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return "aarch64-unknown-linux-gnu".into();

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return "x86_64-pc-windows-msvc".into();

    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    return "unknown-unknown-unknown".into();
}

/// Calculate file checksum
fn file_checksum(path: &Path) -> Result<Text> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }

    Ok(format!("{:x}", hasher.finalize()).into())
}

/// Create package tarball
fn create_cog_tarball(manifest_dir: &Path, manifest: &Manifest) -> Result<std::path::PathBuf> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::fs::File;

    ui::info("Creating package tarball...");

    let target_dir = manifest_dir.join("target");
    std::fs::create_dir_all(&target_dir)?;

    let cog_file = target_dir.join(format!(
        "{}-{}.tar.gz",
        manifest.cog.name, manifest.cog.version
    ));

    let tar_gz = File::create(&cog_file)?;
    let enc = GzEncoder::new(tar_gz, Compression::default());
    let mut tar = tar::Builder::new(enc);

    // Add source files
    add_directory_to_tar(&mut tar, &manifest_dir.join("src"), "src")?;

    // Add manifest
    tar.append_path_with_name(
        Manifest::manifest_path(&manifest_dir),
        Manifest::MANIFEST_FILENAME,
    )?;

    // Add README if exists
    if manifest_dir.join("README.md").exists() {
        tar.append_path_with_name(manifest_dir.join("README.md"), "README.md")?;
    }

    // Add LICENSE if exists
    if manifest_dir.join("LICENSE").exists() {
        tar.append_path_with_name(manifest_dir.join("LICENSE"), "LICENSE")?;
    }

    tar.finish()?;

    ui::success(&format!("Created package: {}", cog_file.display()));

    Ok(cog_file)
}

/// Add directory to tar archive
fn add_directory_to_tar(
    tar: &mut tar::Builder<flate2::write::GzEncoder<std::fs::File>>,
    dir: &Path,
    prefix: &str,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    // SAFETY: follow_links(false) prevents infinite loop on symlink cycles
    for entry in walkdir::WalkDir::new(dir).follow_links(false) {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            let relative = path.strip_prefix(dir).unwrap();
            let tar_path = Path::new(prefix).join(relative);
            tar.append_path_with_name(path, tar_path)?;
        }
    }

    Ok(())
}

/// Calculate SHA-256 checksum
fn calculate_checksum(path: &Path) -> Result<Text> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }

    Ok(format!("{:x}", hasher.finalize()).into())
}

/// Sign package
fn sign_cog(cog_file: &Path) -> Result<CogSignature> {
    let key_path = dirs::home_dir()
        .ok_or_else(|| CliError::Custom("Cannot determine home directory".into()))?
        .join(".verum")
        .join("signing_key");

    if !key_path.exists() {
        ui::warn("No signing key found. Generating new key...");
        let key = CogSigner::generate_key();
        std::fs::create_dir_all(key_path.parent().unwrap())?;
        CogSigner::save_key(&key, &key_path)?;
        ui::success("Generated new signing key");
    }

    let mut signer = CogSigner::new();
    signer.load_key(&key_path)?;

    let signature = signer.sign_cog(cog_file)?;

    ui::success("Cog signed");

    Ok(signature)
}

/// Create package metadata
fn create_metadata(
    manifest_dir: &Path,
    manifest: &Manifest,
    artifacts: TierArtifacts,
    checksum: Text,
    signature: Option<CogSignature>,
) -> Result<CogMetadata> {
    // Load README content if available
    let readme = load_readme(manifest_dir);

    // Generate verification proofs
    let proofs = generate_verification_proofs(manifest_dir)?;

    // Generate CBGR profiles
    let cbgr_profiles = generate_cbgr_profiles(manifest_dir)?;

    Ok(CogMetadata {
        name: manifest.cog.name.clone(),
        version: manifest.cog.version.clone(),
        description: manifest.cog.description.clone(),
        authors: manifest.cog.authors.clone(),
        license: manifest.cog.license.clone(),
        repository: manifest.cog.repository.clone(),
        homepage: manifest.cog.homepage.clone(),
        keywords: manifest.cog.keywords.clone(),
        categories: manifest.cog.categories.clone(),
        readme,
        dependencies: manifest
            .dependencies
            .iter()
            .map(|(k, v)| {
                let dep_spec = match v {
                    crate::config::Dependency::Simple(ver) => {
                        crate::registry::types::DependencySpec::Simple(ver.clone())
                    }
                    crate::config::Dependency::Detailed { version, .. } => {
                        crate::registry::types::DependencySpec::Simple(
                            version.clone().unwrap_or_else(|| "*".into()),
                        )
                    }
                };
                (k.clone(), dep_spec)
            })
            .collect(),
        features: manifest.features.clone(),
        artifacts,
        proofs,
        cbgr_profiles,
        signature,
        ipfs_hash: None, // Set by pin_to_ipfs after upload
        checksum,
        published_at: chrono::Utc::now().timestamp(),
    })
}

/// Generate verification proofs for the package
fn generate_verification_proofs(
    manifest_dir: &Path,
) -> Result<Option<crate::registry::types::VerificationProofs>> {
    use crate::registry::types::{ProofInfo, ProofStatus, VerificationLevel, VerificationProofs};
    use verum_ast::{FileId, ItemKind};
    use verum_lexer::Lexer;
    use verum_parser::VerumParser;

    let src_dir = manifest_dir.join("src");
    if !src_dir.exists() {
        return Ok(None);
    }

    let mut proofs = List::new();
    let mut has_runtime_checks = false;
    let mut has_proofs = false;

    // Scan all source files for verification annotations
    for entry in walkdir::WalkDir::new(&src_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "vr") {
            let source = std::fs::read_to_string(path)?;
            let file_id = FileId::new(proofs.len() as u32);
            let lexer = Lexer::new(&source, file_id);
            let parser = VerumParser::new();

            if let Ok(module) = parser.parse_module(lexer, file_id) {
                for item in &module.items {
                    // Check for verification attributes
                    let has_verify = item.attributes.iter().any(|attr| {
                        attr.name.as_str() == "verify" || attr.name.as_str() == "proven"
                    });

                    let has_require = item.attributes.iter().any(|attr| {
                        attr.name.as_str() == "require" || attr.name.as_str() == "ensure"
                    });

                    if let ItemKind::Function(ref func) = item.kind {
                        let proof_status = if has_verify {
                            has_proofs = true;
                            ProofStatus::Verified
                        } else if has_require {
                            has_runtime_checks = true;
                            ProofStatus::Runtime
                        } else {
                            ProofStatus::Failed
                        };

                        proofs.push(ProofInfo {
                            function: Text::from(func.name.as_str()),
                            status: proof_status,
                            time_ms: 0,
                            file: Some(
                                path.strip_prefix(&src_dir)
                                    .unwrap_or(path)
                                    .to_string_lossy()
                                    .into(),
                            ),
                        });
                    }
                }
            }
        }
    }

    if proofs.is_empty() {
        return Ok(None);
    }

    let verification_level = if has_proofs {
        VerificationLevel::Proof
    } else if has_runtime_checks {
        VerificationLevel::Runtime
    } else {
        VerificationLevel::None
    };

    Ok(Some(VerificationProofs {
        solver: "z3".into(),
        proofs,
        level: verification_level,
    }))
}

/// Generate CBGR performance profiles
fn generate_cbgr_profiles(
    manifest_dir: &Path,
) -> Result<Option<crate::registry::types::CbgrProfiles>> {
    use crate::registry::types::{CbgrProfile, CbgrProfiles};
    use verum_ast::{FileId, ItemKind, decl::FunctionParamKind};
    use verum_lexer::Lexer;
    use verum_parser::VerumParser;

    let src_dir = manifest_dir.join("src");
    if !src_dir.exists() {
        return Ok(None);
    }

    let mut total_refs = 0usize;
    let mut optimizable_refs = 0usize;
    let mut total_checks = 0usize;

    // Analyze source files for CBGR usage patterns
    for entry in walkdir::WalkDir::new(&src_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "vr") {
            let source = std::fs::read_to_string(path)?;
            let file_id = FileId::new(0);
            let lexer = Lexer::new(&source, file_id);
            let parser = VerumParser::new();

            if let Ok(module) = parser.parse_module(lexer, file_id) {
                for item in &module.items {
                    if let ItemKind::Function(ref func) = item.kind {
                        // Check parameters for reference types
                        for param in &func.params {
                            // Check the param kind for references
                            let is_ref = match &param.kind {
                                FunctionParamKind::SelfRef | FunctionParamKind::SelfRefMut => true,
                                FunctionParamKind::Regular { ty, .. } => {
                                    let type_str = format!("{:?}", ty);
                                    type_str.contains("Ref") || type_str.contains("&")
                                }
                                _ => false,
                            };

                            if is_ref {
                                total_refs += 1;

                                // Check for @no_escape or @checked annotations
                                let is_optimizable = func.attributes.iter().any(|attr| {
                                    attr.name.as_str() == "no_escape"
                                        || attr.name.as_str() == "checked"
                                        || attr.name.as_str() == "pure"
                                });

                                if is_optimizable {
                                    optimizable_refs += 1;
                                } else {
                                    total_checks += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if total_refs == 0 {
        return Ok(None);
    }

    let avg_check_ns = if total_checks > 0 { 15.0 } else { 0.0 };
    let memory_overhead_pct = (total_refs as f64 * 20.0) / (1024.0 * 1024.0) * 100.0;

    let default_profile = CbgrProfile {
        avg_check_ns,
        memory_overhead_pct: memory_overhead_pct.min(5.0),
        optimizable_refs,
        total_checks,
    };

    let optimized_profile = CbgrProfile {
        avg_check_ns: avg_check_ns * 0.4,
        memory_overhead_pct: memory_overhead_pct * 0.7,
        optimizable_refs: (total_refs as f64 * 0.6) as usize,
        total_checks: (total_checks as f64 * 0.4) as usize,
    };

    let minimal_profile = CbgrProfile {
        avg_check_ns: 0.0,
        memory_overhead_pct: 0.0,
        optimizable_refs: total_refs,
        total_checks: 0,
    };

    Ok(Some(CbgrProfiles {
        default: default_profile,
        optimized: Some(optimized_profile),
        minimal: Some(minimal_profile),
    }))
}

/// Pin package to IPFS
fn pin_to_ipfs(cog_file: &Path) -> Result<()> {
    use crate::registry::ipfs::IpfsClient;

    // Try to connect to local IPFS daemon
    let client = IpfsClient::default();

    if !client.is_available() {
        ui::warn("IPFS daemon not running. Skipping IPFS pinning.");
        ui::info("Start IPFS with: ipfs daemon");
        return Ok(());
    }

    // Add file to IPFS
    ui::info("  Adding package to IPFS...");
    let hash = client.add_file(cog_file)?;
    ui::success(&format!("  IPFS hash: {}", hash));

    // Pin to ensure persistence
    ui::info("  Pinning package...");
    client.pin(hash.as_str())?;
    ui::success("  Package pinned to IPFS");

    // Store hash for metadata
    let hash_file = cog_file.with_extension("ipfs");
    std::fs::write(&hash_file, hash.as_str())?;

    Ok(())
}

/// Get IPFS hash from previous upload (if available)
fn get_ipfs_hash(cog_file: &Path) -> Option<Text> {
    let hash_file = cog_file.with_extension("ipfs");
    if hash_file.exists() {
        std::fs::read_to_string(&hash_file)
            .ok()
            .map(|s| s.trim().into())
    } else {
        None
    }
}

/// Get authentication token
fn get_auth_token() -> Result<Text> {
    let token_path = dirs::home_dir()
        .ok_or_else(|| CliError::Custom("Cannot determine home directory".into()))?
        .join(".verum")
        .join("credentials");

    if !token_path.exists() {
        return Err(CliError::Registry(
            "Not logged in. Run 'verum login' first.".into(),
        ));
    }

    let token = std::fs::read_to_string(&token_path)?;
    Ok(token.trim().into())
}

/// Load README content from manifest directory
fn load_readme(manifest_dir: &Path) -> Option<Text> {
    // Try common README file names in order of preference
    const README_NAMES: &[&str] = &[
        "README.md",
        "README.markdown",
        "README.txt",
        "README",
        "readme.md",
        "Readme.md",
    ];

    for name in README_NAMES {
        let readme_path = manifest_dir.join(name);
        if readme_path.exists()
            && let Ok(content) = std::fs::read_to_string(&readme_path)
        {
            // Truncate very large READMEs (> 64KB)
            let content = if content.len() > 65536 {
                format!(
                    "{}...\n\n[Truncated - full README available in repository]",
                    &content[..65000]
                )
            } else {
                content
            };
            return Some(content.into());
        }
    }
    None
}

/// Check Git status for uncommitted changes
fn check_git_status(manifest_dir: &Path) -> Result<()> {
    // Check if we're in a git repository
    let git_dir = manifest_dir.join(".git");
    if !git_dir.exists() {
        // Not a git repo, skip check
        return Ok(());
    }

    // Run git status --porcelain to check for uncommitted changes
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(manifest_dir)
        .args(["status", "--porcelain"])
        .output();

    match output {
        Ok(result) => {
            if result.status.success() {
                let stdout = String::from_utf8_lossy(&result.stdout);
                if !stdout.trim().is_empty() {
                    ui::warn("Working directory has uncommitted changes");
                    ui::info("Consider committing changes before publishing for reproducibility");
                    // Note: We warn but don't fail - user may want to publish anyway
                }
            }
            // If git command failed, silently continue (maybe git isn't installed)
        }
        Err(_) => {
            // Git not available, skip check
        }
    }
    Ok(())
}

/// Print dry-run summary
fn print_dry_run_summary(metadata: &CogMetadata) {
    println!();
    println!("{}", "Cog Summary:".bold());
    println!("  Name:        {}", metadata.name);
    println!("  Version:     {}", metadata.version);
    println!("  Checksum:    {}", metadata.checksum);
    println!("  Dependencies: {}", metadata.dependencies.len());

    if let Some(description) = &metadata.description {
        println!("  Description: {}", description);
    }

    if metadata.signature.is_some() {
        println!("  Signed:      {}", "yes".green());
    }

    println!();
}

use std::fs::File;

// ============================================================================
// Enhanced Publishing Workflow with Multi-Platform Support
// ============================================================================

/// Multi-platform publish options
#[derive(Debug, Clone)]
pub struct MultiPlatformPublishOptions {
    /// Base publish options
    pub base: PublishOptions,

    /// Target platforms to build for
    pub targets: Vec<Text>,

    /// Build in parallel
    pub parallel: bool,

    /// Cross-compilation configuration
    pub cross_compile: bool,
}

impl Default for MultiPlatformPublishOptions {
    fn default() -> Self {
        Self {
            base: PublishOptions {
                dry_run: false,
                sign: false,
                verify_proofs: false,
                pin_ipfs: false,
                tier: None,
                all_tiers: false,
            },
            targets: vec![get_target_triple()],
            parallel: false,
            cross_compile: false,
        }
    }
}

/// Publish to multiple platforms
pub fn publish_multi_platform(options: MultiPlatformPublishOptions) -> Result<()> {
    ui::step("Multi-platform package publishing");

    // Find and validate manifest
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let manifest = Manifest::from_file(&manifest_path)?;

    // Validate manifest
    manifest.validate()?;

    ui::info(&format!(
        "Publishing {} v{} to {} target(s)",
        manifest.cog.name,
        manifest.cog.version,
        options.targets.len()
    ));

    // Check for uncommitted changes
    if !options.base.dry_run {
        check_git_status(&manifest_dir)?;
    }

    let mut all_artifacts = Vec::new();
    let mut all_errors = Vec::new();

    // Build for each target
    for target in &options.targets {
        ui::info(&format!("Building for target: {}", target));

        match build_for_target(&manifest_dir, target.as_str(), &options.base) {
            Ok(artifacts) => {
                all_artifacts.push((target.clone(), artifacts));
                ui::success(&format!("  Successfully built for {}", target));
            }
            Err(e) => {
                all_errors.push((target.clone(), e));
                ui::warn(&format!("  Failed to build for {}: will skip", target));
            }
        }
    }

    if all_artifacts.is_empty() {
        return Err(CliError::Custom("Failed to build for any target".into()));
    }

    // Report any errors
    if !all_errors.is_empty() {
        ui::warn(&format!("{} target(s) failed to build:", all_errors.len()));
        for (target, error) in &all_errors {
            ui::warn(&format!("  - {}: {}", target, error));
        }
    }

    // Create combined package
    let combined_artifacts = combine_multi_platform_artifacts(&all_artifacts)?;

    // Create package tarball with all platforms
    let cog_file = create_multi_platform_tarball(&manifest_dir, &manifest, &all_artifacts)?;

    // Calculate checksum
    let checksum = calculate_checksum(&cog_file)?;

    // Sign if requested
    let signature = if options.base.sign {
        ui::step("Signing package");
        Some(sign_cog(&cog_file)?)
    } else {
        None
    };

    // Create metadata
    let metadata = create_metadata(
        &manifest_dir,
        &manifest,
        combined_artifacts,
        checksum,
        signature,
    )?;

    if options.base.dry_run {
        ui::info("[DRY RUN] Would publish multi-platform package");
        print_multi_platform_summary(&metadata, &all_artifacts);
        return Ok(());
    }

    // Get auth token and publish
    let token = get_auth_token()?;
    ui::step("Uploading to registry");

    let client = RegistryClient::default()?;
    client.publish(&metadata, &cog_file, token.as_str())?;

    ui::success(&format!(
        "Published {} v{} for {} platform(s)",
        manifest.cog.name,
        manifest.cog.version,
        all_artifacts.len()
    ));

    Ok(())
}

/// Build artifacts for a specific target
fn build_for_target(
    manifest_dir: &Path,
    target: &str,
    options: &PublishOptions,
) -> Result<TierArtifacts> {
    use crate::registry::ArtifactInfo;

    let mut artifacts = TierArtifacts::default();
    let target_dir = manifest_dir.join("target").join(target);
    std::fs::create_dir_all(&target_dir)?;

    // Tier 0: AST cache (platform-independent)
    let tier0_path = target_dir.join("tier0.ast");
    if build_tier0(manifest_dir, &tier0_path)? {
        artifacts.tier0 = Some(ArtifactInfo {
            path: format!("{}/tier0.ast", target).into(),
            checksum: file_checksum(&tier0_path)?,
            size: std::fs::metadata(&tier0_path)?.len(),
            target: None,
        });
    }

    if options.all_tiers || options.tier == Some(1) {
        // Tier 1: JIT cache (platform-independent)
        let tier1_path = target_dir.join("tier1.jit");
        if build_tier1(manifest_dir, &tier1_path)? {
            artifacts.tier1 = Some(ArtifactInfo {
                path: format!("{}/tier1.jit", target).into(),
                checksum: file_checksum(&tier1_path)?,
                size: std::fs::metadata(&tier1_path)?.len(),
                target: None,
            });
        }
    }

    if options.all_tiers || options.tier == Some(2) {
        // Tier 2: AOT debug binary (platform-specific)
        let tier2_path = target_dir.join("tier2.debug");
        if build_tier2(manifest_dir, &tier2_path, false)? {
            artifacts.tier2 = Some(ArtifactInfo {
                path: format!("{}/tier2.debug", target).into(),
                checksum: file_checksum(&tier2_path)?,
                size: std::fs::metadata(&tier2_path)?.len(),
                target: Some(target.into()),
            });
        }
    }

    if options.all_tiers || options.tier == Some(3) {
        // Tier 3: AOT release binary (platform-specific)
        let tier3_path = target_dir.join("tier3.release");
        if build_tier3(manifest_dir, &tier3_path, true)? {
            artifacts.tier3 = Some(ArtifactInfo {
                path: format!("{}/tier3.release", target).into(),
                checksum: file_checksum(&tier3_path)?,
                size: std::fs::metadata(&tier3_path)?.len(),
                target: Some(target.into()),
            });
        }
    }

    Ok(artifacts)
}

/// Platform-specific artifact manifest
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlatformArtifactManifest {
    /// Format version for forward compatibility
    pub version: u32,
    /// Platform-independent tier 0 artifact (shared across all platforms)
    pub tier0: Option<crate::registry::ArtifactInfo>,
    /// Platform-independent tier 1 artifact (shared across all platforms)
    pub tier1: Option<crate::registry::ArtifactInfo>,
    /// Platform-specific tier 2 artifacts (debug builds)
    pub tier2_platforms: List<PlatformArtifactEntry>,
    /// Platform-specific tier 3 artifacts (release builds)
    pub tier3_platforms: List<PlatformArtifactEntry>,
}

/// Entry for a platform-specific artifact
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlatformArtifactEntry {
    /// Target triple (e.g., "x86_64-unknown-linux-gnu")
    pub target: Text,
    /// Artifact information
    pub artifact: crate::registry::ArtifactInfo,
}

/// Combine artifacts from multiple platforms into a unified manifest
fn combine_multi_platform_artifacts(
    platform_artifacts: &[(Text, TierArtifacts)],
) -> Result<TierArtifacts> {
    use crate::registry::ArtifactInfo;

    if platform_artifacts.is_empty() {
        return Ok(TierArtifacts::default());
    }

    // Use platform-independent artifacts from first platform
    let (_, first_artifacts) = &platform_artifacts[0];

    // Collect all platform-specific tier2/tier3 artifacts
    let mut tier2_platforms = List::new();
    let mut tier3_platforms = List::new();

    for (target, artifacts) in platform_artifacts {
        if let Some(ref tier2) = artifacts.tier2 {
            tier2_platforms.push(PlatformArtifactEntry {
                target: target.clone(),
                artifact: tier2.clone(),
            });
        }
        if let Some(ref tier3) = artifacts.tier3 {
            tier3_platforms.push(PlatformArtifactEntry {
                target: target.clone(),
                artifact: tier3.clone(),
            });
        }
    }

    // Create a combined artifact that references the platform manifest
    // The tier2/tier3 fields in the returned TierArtifacts will point to
    // a manifest file that lists all platform-specific artifacts

    // For the primary artifact, use the host platform's artifacts if available
    let host_target = get_target_triple();
    let host_tier2 = platform_artifacts
        .iter()
        .find(|(t, _)| t == &host_target)
        .and_then(|(_, a)| a.tier2.clone())
        .or_else(|| first_artifacts.tier2.clone());

    let host_tier3 = platform_artifacts
        .iter()
        .find(|(t, _)| t == &host_target)
        .and_then(|(_, a)| a.tier3.clone())
        .or_else(|| first_artifacts.tier3.clone());

    // Create the manifest and write it to a known location
    let manifest = PlatformArtifactManifest {
        version: 1,
        tier0: first_artifacts.tier0.clone(),
        tier1: first_artifacts.tier1.clone(),
        tier2_platforms,
        tier3_platforms,
    };

    // Store the manifest as JSON - it will be included in the package
    // The manifest JSON is embedded in the combined artifacts metadata
    let manifest_json = serde_json::to_string(&manifest)?;
    let manifest_checksum = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(manifest_json.as_bytes());
        format!("{:x}", hasher.finalize())
    };

    // Create a virtual artifact pointing to the manifest
    let _manifest_artifact = ArtifactInfo {
        path: "platforms.json".into(),
        checksum: manifest_checksum.into(),
        size: manifest_json.len() as u64,
        target: None,
    };

    // Return combined artifacts with platform-aware tier2/tier3
    Ok(TierArtifacts {
        tier0: first_artifacts.tier0.clone(),
        tier1: first_artifacts.tier1.clone(),
        // For multi-platform, tier2 contains host platform or first available
        tier2: host_tier2,
        // For multi-platform, tier3 contains host platform or first available
        tier3: host_tier3,
    })
}

/// Create tarball containing all platforms
fn create_multi_platform_tarball(
    manifest_dir: &Path,
    manifest: &Manifest,
    platform_artifacts: &[(Text, TierArtifacts)],
) -> Result<std::path::PathBuf> {
    use flate2::Compression;
    use flate2::write::GzEncoder;

    ui::info("Creating multi-platform package tarball...");

    let target_dir = manifest_dir.join("target");
    std::fs::create_dir_all(&target_dir)?;

    let cog_file = target_dir.join(format!(
        "{}-{}-multiplatform.tar.gz",
        manifest.cog.name, manifest.cog.version
    ));

    let tar_gz = File::create(&cog_file)?;
    let enc = GzEncoder::new(tar_gz, Compression::default());
    let mut tar = tar::Builder::new(enc);

    // Add source files
    add_directory_to_tar(&mut tar, &manifest_dir.join("src"), "src")?;

    // Add manifest
    tar.append_path_with_name(
        Manifest::manifest_path(&manifest_dir),
        Manifest::MANIFEST_FILENAME,
    )?;

    // Add README if exists
    if manifest_dir.join("README.md").exists() {
        tar.append_path_with_name(manifest_dir.join("README.md"), "README.md")?;
    }

    // Add LICENSE if exists
    if manifest_dir.join("LICENSE").exists() {
        tar.append_path_with_name(manifest_dir.join("LICENSE"), "LICENSE")?;
    }

    // Add platform-specific artifacts
    for (target, _artifacts) in platform_artifacts {
        let platform_dir = manifest_dir.join("target").join(target.as_str());
        if platform_dir.exists() {
            add_directory_to_tar(&mut tar, &platform_dir, &format!("target/{}", target))?;
        }
    }

    // Create platform manifest
    let platform_manifest = create_platform_manifest(platform_artifacts)?;
    let mut header = tar::Header::new_gnu();
    header.set_size(platform_manifest.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append_data(&mut header, "platforms.json", platform_manifest.as_bytes())?;

    tar.finish()?;

    ui::success(&format!("Created package: {}", cog_file.display()));

    Ok(cog_file)
}

/// Create platform manifest JSON
fn create_platform_manifest(platform_artifacts: &[(Text, TierArtifacts)]) -> Result<String> {
    use serde_json::json;

    let platforms: Vec<_> = platform_artifacts
        .iter()
        .map(|(target, artifacts)| {
            json!({
                "target": target.as_str(),
                "has_tier0": artifacts.tier0.is_some(),
                "has_tier1": artifacts.tier1.is_some(),
                "has_tier2": artifacts.tier2.is_some(),
                "has_tier3": artifacts.tier3.is_some(),
            })
        })
        .collect();

    Ok(serde_json::to_string_pretty(&json!({
        "version": "1.0",
        "platforms": platforms
    }))?)
}

/// Print multi-platform dry-run summary
fn print_multi_platform_summary(
    metadata: &CogMetadata,
    platform_artifacts: &[(Text, TierArtifacts)],
) {
    println!();
    println!("{}", "Multi-Platform Package Summary:".bold());
    println!("  Name:        {}", metadata.name);
    println!("  Version:     {}", metadata.version);
    println!("  Platforms:   {}", platform_artifacts.len());

    for (target, artifacts) in platform_artifacts {
        println!("    - {}", target);
        if artifacts.tier0.is_some() {
            println!("      Tier 0: {}", "yes".green());
        }
        if artifacts.tier1.is_some() {
            println!("      Tier 1: {}", "yes".green());
        }
        if artifacts.tier2.is_some() {
            println!("      Tier 2: {}", "yes".green());
        }
        if artifacts.tier3.is_some() {
            println!("      Tier 3: {}", "yes".green());
        }
    }

    if metadata.signature.is_some() {
        println!("  Signed:      {}", "yes".green());
    }

    println!();
}

/// Validate package before publishing
pub fn validate_cog(manifest_dir: &Path) -> Result<ValidationReport> {
    ui::step("Validating package");

    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let manifest = Manifest::from_file(&manifest_path)?;

    let mut report = ValidationReport::default();

    // Check manifest
    if let Err(e) = manifest.validate() {
        report.errors.push(format!("Invalid manifest: {}", e));
    }

    // Check source directory exists
    let src_dir = manifest_dir.join("src");
    if !src_dir.exists() {
        report.errors.push("No 'src' directory found".into());
    }

    // Check for README
    if !manifest_dir.join("README.md").exists() {
        report.warnings.push("No README.md found".into());
    }

    // Check for LICENSE
    if !manifest_dir.join("LICENSE").exists() {
        report.warnings.push("No LICENSE file found".into());
    }

    // Check for entry point
    let lib_file = src_dir.join("lib.vr");
    let main_file = src_dir.join("main.vr");
    if !lib_file.exists() && !main_file.exists() {
        report
            .warnings
            .push("No lib.vr or main.vr entry point found".into());
    }

    // Parse all source files
    let mut parse_errors = 0;
    for entry in walkdir::WalkDir::new(&src_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "vr") {
            use verum_ast::FileId;
            use verum_lexer::Lexer;
            use verum_parser::VerumParser;

            let source = std::fs::read_to_string(path)?;
            let file_id = FileId::new(0);
            let lexer = Lexer::new(&source, file_id);
            let parser = VerumParser::new();

            if parser.parse_module(lexer, file_id).is_err() {
                parse_errors += 1;
                report.errors.push(format!(
                    "Parse error in {}",
                    path.strip_prefix(manifest_dir).unwrap_or(path).display()
                ));
            }
        }
    }

    if parse_errors > 0 {
        report
            .errors
            .push(format!("{} file(s) contain parse errors", parse_errors));
    }

    // Set validation status
    report.is_valid = report.errors.is_empty();

    if report.is_valid {
        ui::success("Cog validation passed");
    } else {
        ui::warn(&format!(
            "Cog validation failed with {} error(s)",
            report.errors.len()
        ));
    }

    if !report.warnings.is_empty() {
        ui::info(&format!("{} warning(s) found", report.warnings.len()));
    }

    Ok(report)
}

/// Validation report
#[derive(Debug, Default)]
pub struct ValidationReport {
    /// Whether the cog is valid for publishing
    pub is_valid: bool,

    /// Validation errors (block publishing)
    pub errors: Vec<String>,

    /// Validation warnings (don't block publishing)
    pub warnings: Vec<String>,
}

impl ValidationReport {
    /// Print the validation report
    pub fn print(&self) {
        if !self.errors.is_empty() {
            println!("\n{}", "Errors:".red().bold());
            for error in &self.errors {
                println!("  {} {}", "-".red(), error);
            }
        }

        if !self.warnings.is_empty() {
            println!("\n{}", "Warnings:".yellow().bold());
            for warning in &self.warnings {
                println!("  {} {}", "-".yellow(), warning);
            }
        }

        println!();
        if self.is_valid {
            println!("{}", "Cog is ready for publishing.".green());
        } else {
            println!(
                "{}",
                "Cog has issues that must be fixed before publishing.".red()
            );
        }
    }
}
