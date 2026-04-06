// Security and verification audit of dependencies.
// Checks for known vulnerabilities, signature validity, and supply chain integrity.

use crate::config::Manifest;
use crate::error::Result;
use crate::registry::{Lockfile, RegistryClient, Severity};
use crate::ui;
use colored::Colorize;
use verum_common::{List, Text};

/// Audit options
pub struct AuditOptions {
    pub verify_checksums: bool,
    pub verify_signatures: bool,
    pub verify_proofs: bool,
    pub cbgr_profiles: bool,
    pub fix: bool,
    /// Only audit direct dependencies, not transitive ones
    pub direct_only: bool,
}

impl Default for AuditOptions {
    fn default() -> Self {
        Self {
            verify_checksums: true,
            verify_signatures: false,
            verify_proofs: false,
            cbgr_profiles: false,
            fix: false,
            direct_only: false,
        }
    }
}

/// Audit dependencies for security and verification
pub fn audit(options: AuditOptions) -> Result<()> {
    if options.direct_only {
        ui::step("Auditing direct dependencies only");
    } else {
        ui::step("Auditing all dependencies");
    }

    // Find manifest and lockfile
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest = Manifest::from_file(&manifest_dir.join("Verum.toml"))?;

    let lockfile_path = manifest_dir.join("Verum.lock");
    if !lockfile_path.exists() {
        ui::warn("No lockfile found. Run 'verum build' first.");
        return Ok(());
    }

    let lockfile = Lockfile::from_file(&lockfile_path)?;

    // Build set of direct dependencies if filtering
    let direct_deps: std::collections::HashSet<&str> = if options.direct_only {
        manifest
            .dependencies
            .keys()
            .chain(manifest.dev_dependencies.keys())
            .chain(manifest.build_dependencies.keys())
            .map(|s| s.as_str())
            .collect()
    } else {
        std::collections::HashSet::new()
    };

    // Collect audit results
    let mut vulnerabilities = List::new();
    let mut checksum_failures = List::new();
    let mut signature_failures = List::new();
    let mut cbgr_info = List::new();

    // Check for vulnerabilities
    ui::info("Checking for vulnerabilities...");
    let client = RegistryClient::default()?;

    for package in &lockfile.packages {
        // Skip transitive dependencies if direct_only is set
        if options.direct_only && !direct_deps.contains(package.name.as_str()) {
            continue;
        }

        match client.check_vulnerabilities(package.name.as_str(), package.version.as_str()) {
            Ok(report) => {
                if !report.vulnerabilities.is_empty() {
                    vulnerabilities.push((package.name.clone(), report));
                }
            }
            Err(e) => {
                ui::warn(&format!("Failed to check {}: {}", package.name, e));
            }
        }
    }

    // Verify checksums
    if options.verify_checksums {
        ui::info("Verifying checksums...");
        let cache_dir = crate::registry::cache_dir()?;

        match lockfile.verify_checksums(&cache_dir) {
            Ok(failures) => {
                checksum_failures = failures;
            }
            Err(e) => {
                ui::warn(&format!("Checksum verification failed: {}", e));
            }
        }
    }

    // Verify signatures
    if options.verify_signatures {
        ui::info("Verifying signatures...");
        signature_failures = verify_signatures(&lockfile)?;
    }

    // Check CBGR profiles
    if options.cbgr_profiles {
        ui::info("Analyzing CBGR profiles...");
        cbgr_info = analyze_cbgr_profiles(&lockfile)?;
    }

    // Print report
    print_audit_report(
        &vulnerabilities,
        &checksum_failures,
        &signature_failures,
        &cbgr_info,
    );

    // Fix vulnerabilities if requested
    if options.fix && !vulnerabilities.is_empty() {
        ui::step("Fixing vulnerabilities");
        fix_vulnerabilities(&vulnerabilities)?;
    }

    // Determine exit status
    let has_critical = vulnerabilities.iter().any(|(_, report)| {
        report
            .vulnerabilities
            .iter()
            .any(|v| matches!(v.severity, Severity::Critical | Severity::High))
    });

    if has_critical {
        return Err(crate::error::CliError::Custom(
            "Critical vulnerabilities found".into(),
        ));
    }

    Ok(())
}

/// Verify package signatures
fn verify_signatures(lockfile: &Lockfile) -> Result<List<Text>> {
    let mut failures = List::new();
    let client = RegistryClient::default()?;
    let cache_dir = crate::registry::cache_dir()?;

    for package in &lockfile.packages {
        let metadata = match client.get_metadata(package.name.as_str(), package.version.as_str()) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if let Some(signature) = metadata.signature {
            let cog_path = cache_dir
                .join(package.name.as_str())
                .join(package.version.as_str())
                .join(format!("{}-{}.tar.gz", package.name, package.version));

            if cog_path.exists() {
                use crate::registry::CogSigner;

                match CogSigner::verify_signature(&cog_path, &signature) {
                    Ok(valid) => {
                        if !valid {
                            failures.push(format!("{} {}", package.name, package.version).into());
                        }
                    }
                    Err(_) => {
                        failures.push(format!("{} {}", package.name, package.version).into());
                    }
                }
            }
        }
    }

    Ok(failures)
}

/// Analyze CBGR profiles
fn analyze_cbgr_profiles(lockfile: &Lockfile) -> Result<List<(Text, Text)>> {
    let mut info: List<(Text, Text)> = List::new();
    let client = RegistryClient::default()?;

    for package in &lockfile.packages {
        let metadata = match client.get_metadata(package.name.as_str(), package.version.as_str()) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if let Some(profiles) = metadata.cbgr_profiles {
            let overhead: Text = format!(
                "avg check: {:.1}ns, memory: {:.1}%",
                profiles.default.avg_check_ns, profiles.default.memory_overhead_pct
            )
            .into();

            info.push((package.name.clone(), overhead));
        }
    }

    Ok(info)
}

/// Print audit report
fn print_audit_report(
    vulnerabilities: &List<(Text, crate::registry::types::VulnerabilityReport)>,
    checksum_failures: &List<Text>,
    signature_failures: &List<Text>,
    cbgr_info: &List<(Text, Text)>,
) {
    println!();
    println!("{}", "═".repeat(80));
    println!("{}", "Audit Report".bold());
    println!("{}", "═".repeat(80));

    // Vulnerabilities
    if vulnerabilities.is_empty() {
        println!("{} No known vulnerabilities found", "✓".green());
    } else {
        println!(
            "{} Found {} vulnerabilities",
            "!".red(),
            vulnerabilities.len()
        );
        println!();

        for (package, report) in vulnerabilities {
            for vuln in &report.vulnerabilities {
                let severity_str = match vuln.severity {
                    Severity::Critical => "CRITICAL".red().bold(),
                    Severity::High => "HIGH".red(),
                    Severity::Medium => "MEDIUM".yellow(),
                    Severity::Low => "LOW".dimmed(),
                };

                println!("  {} {}", severity_str, vuln.id);
                println!("    Package: {} {}", package, report.version);
                println!("    Title:   {}", vuln.title);
                println!("    Patched: {}", vuln.patched_versions.join(", "));
                println!();
            }
        }
    }

    // Checksums
    if !checksum_failures.is_empty() {
        println!("{} Checksum verification failed:", "!".red());
        for failure in checksum_failures {
            println!("  {}", failure);
        }
        println!();
    }

    // Signatures
    if !signature_failures.is_empty() {
        println!("{} Signature verification failed:", "!".red());
        for failure in signature_failures {
            println!("  {}", failure);
        }
        println!();
    }

    // CBGR profiles
    if !cbgr_info.is_empty() {
        println!("{}", "CBGR Performance Profiles:".bold());
        for (package, info) in cbgr_info {
            println!("  {}: {}", package.as_str().cyan(), info);
        }
        println!();
    }

    println!("{}", "═".repeat(80));
}

/// Fix vulnerabilities by updating packages
fn fix_vulnerabilities(
    vulnerabilities: &List<(Text, crate::registry::types::VulnerabilityReport)>,
) -> Result<()> {
    use std::fs;

    // Find manifest
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = manifest_dir.join("Verum.toml");
    let lockfile_path = manifest_dir.join("Verum.lock");

    let mut manifest = Manifest::from_file(&manifest_path)?;
    let mut lockfile = if lockfile_path.exists() {
        Lockfile::from_file(&lockfile_path)?
    } else {
        ui::warn("No lockfile found. Cannot fix vulnerabilities without lockfile.");
        return Ok(());
    };

    let client = RegistryClient::default()?;
    let mut fixed_count = 0;
    let mut failed_fixes = List::new();

    for (package, report) in vulnerabilities {
        for vuln in &report.vulnerabilities {
            if vuln.patched_versions.is_empty() {
                ui::warn(&format!(
                    "No patched version available for {} ({})",
                    package, vuln.id
                ));
                failed_fixes.push(package.clone());
                continue;
            }

            // Find best patched version (prefer patch/minor updates over major)
            let patched_versions: Vec<String> = vuln
                .patched_versions
                .iter()
                .map(|t| t.to_string())
                .collect();
            let best_patch = find_best_patched_version(&patched_versions, report.version.as_str())?;

            ui::info(&format!(
                "Fixing {}: {} → {} ({})",
                package, report.version, best_patch, vuln.id
            ));

            // Update manifest dependencies
            let updated = update_manifest_dependency(&mut manifest, package.as_str(), &best_patch);

            if !updated {
                ui::warn(&format!(
                    "Cog {} not found in manifest dependencies",
                    package
                ));
                failed_fixes.push(package.clone());
                continue;
            }

            // Update lockfile
            match client.get_metadata(package.as_str(), &best_patch) {
                Ok(metadata) => {
                    lockfile.update_cog(
                        package.as_str(),
                        best_patch.clone().into(),
                        metadata.checksum,
                    );
                    fixed_count += 1;
                }
                Err(e) => {
                    ui::warn(&format!("Failed to fetch metadata for {}: {}", package, e));
                    failed_fixes.push(package.clone());
                }
            }
        }
    }

    // Write updated files
    if fixed_count > 0 {
        // Write manifest
        let manifest_content =
            toml::to_string_pretty(&manifest).map_err(crate::error::CliError::ConfigSerialize)?;
        fs::write(&manifest_path, manifest_content)?;

        // Write lockfile
        lockfile.to_file(&lockfile_path)?;

        println!();
        ui::success(&format!("Fixed {} vulnerabilities", fixed_count));

        if !failed_fixes.is_empty() {
            println!();
            ui::warn(&format!(
                "Failed to fix {} packages - manual intervention required:",
                failed_fixes.len()
            ));
            for pkg in failed_fixes {
                println!("  • {}", pkg);
            }
        }

        println!();
        ui::info("Run 'verum build' to download updated packages");
    } else {
        ui::warn("No vulnerabilities could be automatically fixed");
    }

    Ok(())
}

/// Find the best patched version (prefer semver-compatible updates)
fn find_best_patched_version(patched_versions: &[String], current: &str) -> Result<String> {
    use semver::Version;

    let current_ver = Version::parse(current).map_err(|_| {
        crate::error::CliError::Custom(format!("Invalid current version: {}", current))
    })?;

    let mut candidates: Vec<Version> = patched_versions
        .iter()
        .filter_map(|v| Version::parse(v).ok())
        .collect();

    if candidates.is_empty() {
        return Err(crate::error::CliError::Custom(
            "No valid semver patched versions found".into(),
        ));
    }

    // Sort by preference: same major > same minor > any
    candidates.sort_by(|a, b| {
        // Prefer same major version
        if a.major == current_ver.major && b.major != current_ver.major {
            return std::cmp::Ordering::Less;
        }
        if a.major != current_ver.major && b.major == current_ver.major {
            return std::cmp::Ordering::Greater;
        }

        // Then prefer same minor version
        if a.major == current_ver.major {
            if a.minor == current_ver.minor && b.minor != current_ver.minor {
                return std::cmp::Ordering::Less;
            }
            if a.minor != current_ver.minor && b.minor == current_ver.minor {
                return std::cmp::Ordering::Greater;
            }
        }

        // Finally, prefer higher version
        a.cmp(b).reverse()
    });

    Ok(candidates[0].to_string())
}

/// Update dependency version in manifest
fn update_manifest_dependency(manifest: &mut Manifest, package: &str, new_version: &str) -> bool {
    use crate::config::Dependency;

    // Try regular dependencies
    let package_key = Text::from(package);
    if let Some(dep) = manifest.dependencies.get_mut(&package_key) {
        match dep {
            Dependency::Simple(v) => {
                *v = new_version.into();
                return true;
            }
            Dependency::Detailed { version: ver, .. } => {
                *ver = Some(new_version.into());
                return true;
            }
        }
    }

    // Try dev dependencies
    if let Some(dep) = manifest.dev_dependencies.get_mut(&package_key) {
        match dep {
            Dependency::Simple(v) => {
                *v = new_version.into();
                return true;
            }
            Dependency::Detailed { version: ver, .. } => {
                *ver = Some(new_version.into());
                return true;
            }
        }
    }

    // Try build dependencies
    if let Some(dep) = manifest.build_dependencies.get_mut(&package_key) {
        match dep {
            Dependency::Simple(v) => {
                *v = new_version.into();
                return true;
            }
            Dependency::Detailed { version: ver, .. } => {
                *ver = Some(new_version.into());
                return true;
            }
        }
    }

    false
}
