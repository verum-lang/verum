// Cog registry operations
// Publishing, searching, and installing cogs
//
// This module provides full cog registry functionality using the
// comprehensive registry infrastructure in crates/verum_cli/src/registry/

use crate::config::Manifest;
use crate::error::{CliError, Result};
use crate::registry::{
    CacheManager, LockedCog, Lockfile, CogMetadata, CogSigner, RegistryClient,
    SearchResult,
};
use crate::ui;
use colored::Colorize;
use semver::VersionReq;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use verum_common::{List, Map, Text};

/// Publish a cog to the Verum registry
///
/// # Implementation
///
/// This function:
/// 1. Validates the manifest (Verum.toml)
/// 2. Builds the cog tarball (.vr archive)
/// 3. Signs the cog with Ed25519 (if key available)
/// 4. Uploads to the central registry
/// 5. Updates the registry index
///
/// # Arguments
///
/// * `dry_run` - If true, performs all steps except actual upload
/// * `allow_dirty` - If true, allows publishing with uncommitted changes
///
/// # Examples
///
/// ```no_run
/// use verum_cli::cog::publish;
///
/// # fn main() -> anyhow::Result<()> {
/// // Normal publish
/// publish(false, false)?;
///
/// // Dry run (preview what would be published)
/// publish(true, false)?;
/// # Ok(())
/// # }
/// ```
pub fn publish(dry_run: bool, allow_dirty: bool) -> Result<()> {
    ui::step("Publishing cog");

    // Find and validate manifest
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = manifest_dir.join("Verum.toml");
    let manifest = Manifest::from_file(&manifest_path)?;

    // Validate manifest
    manifest.validate()?;

    ui::info(&format!(
        "Publishing {} v{}",
        manifest.cog.name.as_str().cyan(),
        manifest.cog.version.as_str().green()
    ));

    // Check for uncommitted changes (unless allow_dirty or dry_run)
    if !allow_dirty && !dry_run {
        check_git_status(&manifest_dir)?;
    }

    // Create cog tarball
    let cog_file = create_cog_tarball(&manifest_dir, &manifest)?;
    ui::success("Created cog archive");

    // Calculate checksum
    let checksum = calculate_checksum(&cog_file)?;
    ui::info(&format!("  SHA-256: {}", &checksum[..16]));

    // Sign cog if key is available
    let signature = sign_cog_if_key_exists(&cog_file)?;
    if signature.is_some() {
        ui::success("Cog signed with Ed25519");
    }

    // Create metadata
    let metadata = create_metadata(&manifest, checksum, signature)?;

    if dry_run {
        ui::info("");
        ui::info(&format!("{}", "[DRY RUN] Would publish:".bold()));
        ui::info(&format!("  Name: {}", manifest.cog.name));
        ui::info(&format!("  Version: {}", manifest.cog.version));
        ui::info(&format!(
            "  Size: {} bytes",
            fs::metadata(&cog_file)?.len()
        ));
        ui::info(&format!("  Dependencies: {}", metadata.dependencies.len()));
        ui::info("");
        ui::success("Dry run complete - cog is valid for publishing");

        // Clean up temp file
        let _ = fs::remove_file(&cog_file);
        return Ok(());
    }

    // Get authentication token
    let token = get_auth_token()?;

    // Upload to registry
    ui::step("Uploading to registry");
    let client = RegistryClient::default()?;
    client.publish(&metadata, &cog_file, token.as_str())?;

    // Clean up temp file
    let _ = fs::remove_file(&cog_file);

    ui::success(&format!(
        "Published {} v{}",
        manifest.cog.name, manifest.cog.version
    ));

    println!();
    ui::info(&format!(
        "Cog URL: {}",
        format!(
            "https://packages.verum.lang/cogs/{}/{}",
            manifest.cog.name, manifest.cog.version
        )
        .cyan()
    ));

    Ok(())
}

/// Search for cogs in the Verum registry
///
/// # Implementation
///
/// Uses the registry API to perform fuzzy search on cog names,
/// descriptions, keywords, and categories.
///
/// # Arguments
///
/// * `query` - Search query string
/// * `limit` - Maximum number of results to return
///
/// # Examples
///
/// ```no_run
/// use verum_cli::cog::search;
///
/// # fn main() -> anyhow::Result<()> {
/// // Search for HTTP-related cogs
/// search("http", 10)?;
///
/// // Search for crypto cogs
/// search("crypto", 20)?;
/// # Ok(())
/// # }
/// ```
pub fn search(query: &str, limit: usize) -> Result<()> {
    ui::step(&format!("Searching for: {}", query.cyan()));

    let client = RegistryClient::default()?;
    let results = client.search(query, limit)?;

    if results.is_empty() {
        ui::warn(&format!("No cogs found matching '{}'", query));
        println!();
        ui::info("Try a different search term or browse:");
        ui::info(&format!(
            "  • https://packages.verum.lang/search?q={}",
            query
        ));
        ui::info("  • https://github.com/topics/verum-lang");
        return Ok(());
    }

    println!();
    ui::info(&format!(
        "Found {} cog{}:",
        results.len(),
        if results.len() == 1 { "" } else { "s" }
    ));
    println!();

    for result in results.iter() {
        print_search_result(result);
    }

    println!();
    ui::info(&format!(
        "To install: {}",
        "verum install <cog>".to_string().cyan()
    ));

    Ok(())
}

/// Install a cog from the Verum registry
///
/// # Implementation
///
/// This function:
/// 1. Resolves the cog version (latest if not specified)
/// 2. Downloads the cog from the registry
/// 3. Verifies the cog signature
/// 4. Extracts to the local cache
/// 5. Updates Verum.toml with the dependency
/// 6. Updates the lockfile (Verum.lock)
///
/// # Arguments
///
/// * `name` - Cog name to install
/// * `version` - Optional version requirement (uses latest if None)
///
/// # Examples
///
/// ```no_run
/// use verum_cli::cog::install;
/// use verum_common::Text;
///
/// # fn main() -> anyhow::Result<()> {
/// // Install latest version
/// install("http-client", None)?;
///
/// // Install specific version
/// install("http-client", Some(Text::from("1.2.3")))?;
/// # Ok(())
/// # }
/// ```
pub fn install(name: &str, version: Option<Text>) -> Result<()> {
    let version_str = version.as_ref().map(|v| v.as_str()).unwrap_or("latest");
    ui::step(&format!("Installing {} {}", name.cyan(), version_str));

    let client = RegistryClient::default()?;

    // Resolve version
    let resolved_version = if let Some(ref v) = version {
        // Parse as version requirement
        let _req = VersionReq::parse(v.as_str())
            .map_err(|e| CliError::Custom(format!("Invalid version requirement: {}", e)))?;
        // For now, use exact version if specified
        v.clone()
    } else {
        // Get latest version
        ui::info("Resolving latest version...");
        client.get_latest_version(name)?
    };

    ui::info(&format!("  Version: {}", resolved_version.as_str().green()));

    // Get package metadata
    let metadata = client.get_metadata(name, resolved_version.as_str())?;

    // Check for vulnerabilities
    ui::info("Checking for vulnerabilities...");
    let vuln_report = client.check_vulnerabilities(name, resolved_version.as_str())?;
    if !vuln_report.vulnerabilities.is_empty() {
        ui::warn(&format!(
            "Found {} vulnerabilities:",
            vuln_report.vulnerabilities.len()
        ));
        for vuln in vuln_report.vulnerabilities.iter() {
            ui::warn(&format!(
                "  • {:?}: {}",
                vuln.severity,
                vuln.description.as_str()
            ));
        }
        println!();
    } else {
        ui::success("  No known vulnerabilities");
    }

    // Download cog
    ui::step("Downloading cog");
    let cache_dir = CacheManager::default_cache_dir()?;
    let cache_manager = CacheManager::new(cache_dir)?;

    // Get download URL
    let download_url = format!(
        "{}/cogs/{}/{}/download",
        crate::registry::DEFAULT_REGISTRY,
        name,
        resolved_version.as_str()
    );

    let cog_path = cache_manager.get_or_download(
        name,
        resolved_version.as_str(),
        &download_url,
        metadata.checksum.as_str(),
    )?;

    // Verify signature if present
    if let Some(ref sig) = metadata.signature {
        ui::info("Verifying cog signature...");
        let valid = CogSigner::verify_signature(&cog_path, sig)?;
        if valid {
            ui::success("  Signature verified");
        } else {
            return Err(CliError::Custom(
                "Cog signature verification failed".into(),
            ));
        }
    }

    // Update Verum.toml
    ui::step("Updating Verum.toml");
    update_manifest_dependency(name, resolved_version.as_str())?;

    // Update lockfile
    ui::step("Updating Verum.lock");
    update_lockfile(name, &metadata)?;

    ui::success(&format!(
        "Installed {} v{}",
        name,
        resolved_version.as_str()
    ));

    println!();
    ui::info("Run 'verum build' to compile with the new dependency");

    Ok(())
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Check for uncommitted git changes
fn check_git_status(dir: &Path) -> Result<()> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            if !stdout.trim().is_empty() {
                return Err(CliError::Custom(
                    "Working directory has uncommitted changes. \
                     Commit your changes or use --allow-dirty to publish anyway."
                        .into(),
                ));
            }
            Ok(())
        }
        _ => {
            // Not a git repo or git not available - that's fine
            Ok(())
        }
    }
}

/// Create cog tarball (.vr archive)
fn create_cog_tarball(manifest_dir: &Path, manifest: &Manifest) -> Result<PathBuf> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use tar::Builder;

    let cog_name = format!("{}-{}.vr", manifest.cog.name, manifest.cog.version);
    let cog_path = std::env::temp_dir().join(&cog_name);

    let file = fs::File::create(&cog_path)?;
    let encoder = GzEncoder::new(file, Compression::best());
    let mut archive = Builder::new(encoder);

    // Add Verum.toml
    archive.append_path_with_name(manifest_dir.join("Verum.toml"), "Verum.toml")?;

    // Add src directory
    let src_dir = manifest_dir.join("src");
    if src_dir.exists() {
        add_directory_to_archive(&mut archive, &src_dir, "src")?;
    }

    // Add README if exists
    for readme in &["README.md", "README.txt", "README"] {
        let readme_path = manifest_dir.join(readme);
        if readme_path.exists() {
            archive.append_path_with_name(&readme_path, readme)?;
            break;
        }
    }

    // Add LICENSE if exists
    for license in &["LICENSE", "LICENSE.md", "LICENSE.txt"] {
        let license_path = manifest_dir.join(license);
        if license_path.exists() {
            archive.append_path_with_name(&license_path, license)?;
            break;
        }
    }

    archive.finish()?;

    Ok(cog_path)
}

/// Recursively add directory to tar archive
fn add_directory_to_archive<W: IoWrite>(
    archive: &mut tar::Builder<W>,
    dir: &Path,
    prefix: &str,
) -> Result<()> {
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        let relative = path.strip_prefix(dir).unwrap_or(path);
        let archive_path = format!("{}/{}", prefix, relative.display());

        if path.is_file() {
            archive.append_path_with_name(path, &archive_path)?;
        }
    }
    Ok(())
}

/// Calculate SHA-256 checksum of a file
fn calculate_checksum(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

/// Sign cog if signing key exists
fn sign_cog_if_key_exists(
    cog_path: &Path,
) -> Result<Option<crate::registry::CogSignature>> {
    // Look for signing key in standard locations
    let key_paths = [
        dirs::config_dir()
            .map(|d| d.join("verum").join("signing.key"))
            .unwrap_or_default(),
        dirs::home_dir()
            .map(|d| d.join(".verum").join("signing.key"))
            .unwrap_or_default(),
    ];

    for key_path in key_paths.iter() {
        if key_path.exists() {
            let mut signer = CogSigner::new();
            signer.load_key(key_path)?;
            return Ok(Some(signer.sign_cog(cog_path)?));
        }
    }

    Ok(None)
}

/// Create cog metadata from manifest
fn create_metadata(
    manifest: &Manifest,
    checksum: String,
    signature: Option<crate::registry::CogSignature>,
) -> Result<CogMetadata> {
    use crate::config::Dependency;

    let mut dependencies = Map::new();
    for (name, dep) in manifest.dependencies.iter() {
        let version = match dep {
            Dependency::Simple(v) => v.clone(),
            Dependency::Detailed { version, .. } => {
                version.clone().unwrap_or_else(|| Text::from("*"))
            }
        };
        dependencies.insert(
            name.clone(),
            crate::registry::DependencySpec::Simple(version),
        );
    }

    let mut features = Map::new();
    for (name, feature_deps) in manifest.features.iter() {
        features.insert(name.clone(), feature_deps.clone());
    }

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
        readme: None, // Readme content loaded separately if needed
        dependencies,
        features,
        artifacts: crate::registry::TierArtifacts::default(),
        proofs: None,
        cbgr_profiles: None,
        signature,
        ipfs_hash: None,
        checksum: checksum.into(),
        published_at: chrono::Utc::now().timestamp(),
    })
}

/// Get authentication token from environment or credential store
fn get_auth_token() -> Result<String> {
    // Check environment variable first
    if let Ok(token) = std::env::var("VERUM_REGISTRY_TOKEN") {
        return Ok(token);
    }

    // Check credentials file
    let creds_path = dirs::config_dir()
        .map(|d| d.join("verum").join("credentials"))
        .ok_or_else(|| CliError::Custom("Cannot find config directory".into()))?;

    if creds_path.exists() {
        let content = fs::read_to_string(&creds_path)?;
        for line in content.lines() {
            if let Some(token) = line.strip_prefix("token = ") {
                return Ok(token.trim_matches('"').to_string());
            }
        }
    }

    Err(CliError::Custom(
        "No authentication token found. Run 'verum login' or set VERUM_REGISTRY_TOKEN".into(),
    ))
}

/// Print a search result
fn print_search_result(result: &SearchResult) {
    println!(
        "  {} {} - {}",
        result.name.as_str().cyan().bold(),
        result.version.as_str().green(),
        result
            .description
            .as_ref()
            .map(|d| d.as_str())
            .unwrap_or("No description")
    );

    // Show verification status
    let status = if result.verified {
        "✓ verified".green()
    } else {
        "unverified".dimmed()
    };

    let cbgr = if result.cbgr_optimized {
        " | CBGR optimized".cyan()
    } else {
        "".normal()
    };

    println!(
        "    Downloads: {} | {}{}",
        format_downloads(result.downloads),
        status,
        cbgr
    );
    println!();
}

/// Format download count
fn format_downloads(downloads: u64) -> String {
    if downloads >= 1_000_000 {
        format!("{:.1}M", downloads as f64 / 1_000_000.0)
    } else if downloads >= 1_000 {
        format!("{:.1}K", downloads as f64 / 1_000.0)
    } else {
        downloads.to_string()
    }
}

/// Update Verum.toml with new dependency
fn update_manifest_dependency(name: &str, version: &str) -> Result<()> {
    let manifest_path = Manifest::find_manifest_dir()?.join("Verum.toml");
    let content = fs::read_to_string(&manifest_path)?;

    // Parse TOML
    let mut manifest: toml::Value = toml::from_str(&content)
        .map_err(|e| CliError::Custom(format!("Failed to parse Verum.toml: {}", e)))?;

    // Ensure dependencies section exists and add dependency
    if let Some(table) = manifest.as_table_mut() {
        let deps = table
            .entry("dependencies")
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));

        if let Some(deps_table) = deps.as_table_mut() {
            deps_table.insert(name.to_string(), toml::Value::String(version.to_string()));
        }
    }

    // Write back
    let updated = toml::to_string_pretty(&manifest)
        .map_err(|e| CliError::Custom(format!("Failed to serialize Verum.toml: {}", e)))?;
    fs::write(&manifest_path, updated)?;

    Ok(())
}

/// Update lockfile with new cog
fn update_lockfile(name: &str, metadata: &CogMetadata) -> Result<()> {
    let manifest_dir = Manifest::find_manifest_dir()?;
    let lockfile_path = manifest_dir.join("Verum.lock");

    // Get root cog name from manifest
    let manifest = Manifest::from_file(&manifest_dir.join("Verum.toml"))?;
    let root_name = manifest.cog.name.clone();

    let mut lockfile = if lockfile_path.exists() {
        Lockfile::from_file(&lockfile_path)?
    } else {
        Lockfile::new(root_name)
    };

    // Build dependencies map (name -> version)
    let mut deps_map = Map::new();
    for (dep_name, dep_spec) in metadata.dependencies.iter() {
        let version = match dep_spec {
            crate::registry::DependencySpec::Simple(v) => v.clone(),
            crate::registry::DependencySpec::Detailed { version, .. } => {
                version.clone().unwrap_or_else(|| Text::from("*"))
            }
        };
        deps_map.insert(dep_name.clone(), version);
    }

    // Create locked cog entry
    let locked = LockedCog {
        name: name.into(),
        version: metadata.version.clone(),
        source: crate::registry::CogSource::Registry {
            registry: crate::registry::DEFAULT_REGISTRY.into(),
            version: metadata.version.clone(),
        },
        checksum: metadata.checksum.clone(),
        dependencies: deps_map,
        features: List::new(),
        optional: false,
    };

    lockfile.add_cog(locked);
    lockfile.to_file(&lockfile_path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_downloads() {
        assert_eq!(format_downloads(500), "500");
        assert_eq!(format_downloads(1500), "1.5K");
        assert_eq!(format_downloads(1_500_000), "1.5M");
    }

    #[test]
    fn test_calculate_checksum() {
        use std::io::Write;

        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        let mut file = fs::File::create(&file_path).unwrap();
        file.write_all(b"test content").unwrap();

        let checksum = calculate_checksum(&file_path).unwrap();
        assert_eq!(checksum.len(), 64); // SHA-256 produces 64 hex chars
    }
}
