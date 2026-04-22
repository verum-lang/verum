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

// =============================================================================
// Framework-axiom audit
//
// `verum audit --framework-axioms` enumerates every `@framework(name, "cite")`
// marker attached to axiom / theorem / lemma / corollary declarations in the
// current project. The report groups citations by framework name and prints
// a structured trusted-boundary view so external reviewers see exactly which
// external results (Lurie HTT, Schreiber DCCT, Connes reconstruction, Petz
// classification, Arnold-Mather catastrophe, Baez-Dolan, …) a proof relies
// on before they inspect the proofs themselves.
// =============================================================================

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use verum_ast::attr::FrameworkAttr;
use verum_ast::decl::ItemKind;
use verum_ast::Item;
use verum_common::Maybe;
use verum_compiler::pipeline::CompilationPipeline;
use verum_compiler::session::Session;
use verum_compiler::CompilerOptions;

/// One framework-axiom usage point.
#[derive(Debug, Clone)]
struct FrameworkUsage {
    /// Item name (theorem / axiom / lemma).
    item_name: Text,
    /// Kind of item the marker was attached to.
    item_kind: &'static str,
    /// File path relative to project root.
    file: PathBuf,
    /// Citation string from the second argument.
    citation: Text,
}

/// Entry point for `verum audit --framework-axioms`.
pub fn audit_framework_axioms() -> Result<()> {
    ui::step("Enumerating framework-axiom trusted boundary");

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project.");
        return Ok(());
    }

    let mut by_framework: BTreeMap<Text, Vec<FrameworkUsage>> = BTreeMap::new();
    let mut malformed: Vec<(PathBuf, Text)> = Vec::new();
    let mut parsed_files = 0usize;
    let mut skipped_files = 0usize;

    for abs_path in &vr_files {
        let rel_path = abs_path
            .strip_prefix(&manifest_dir)
            .unwrap_or(abs_path)
            .to_path_buf();
        let module = match parse_file_for_audit(abs_path) {
            Ok(m) => m,
            Err(_) => {
                skipped_files += 1;
                continue;
            }
        };
        parsed_files += 1;

        for item in &module.items {
            // The parser can place `@framework(...)` either on the outer
            // Item.attributes or on the inner decl.attributes list (both
            // storage sites exist across TheoremDecl / AxiomDecl / …), so
            // we walk both.
            let (kind_label, item_name, decl_attrs): (
                &'static str,
                Text,
                &verum_common::List<verum_ast::attr::Attribute>,
            ) = match &item.kind {
                ItemKind::Theorem(decl) => {
                    ("theorem", decl.name.name.clone(), &decl.attributes)
                }
                ItemKind::Lemma(decl) => {
                    ("lemma", decl.name.name.clone(), &decl.attributes)
                }
                ItemKind::Corollary(decl) => {
                    ("corollary", decl.name.name.clone(), &decl.attributes)
                }
                ItemKind::Axiom(decl) => {
                    ("axiom", decl.name.name.clone(), &decl.attributes)
                }
                _ => continue,
            };
            collect_framework_markers_from(
                &item.attributes,
                kind_label,
                &item_name,
                &rel_path,
                &mut by_framework,
                &mut malformed,
            );
            collect_framework_markers_from(
                decl_attrs,
                kind_label,
                &item_name,
                &rel_path,
                &mut by_framework,
                &mut malformed,
            );
        }
    }

    print_framework_report(
        parsed_files,
        skipped_files,
        &by_framework,
        &malformed,
    );

    if !malformed.is_empty() {
        return Err(crate::error::CliError::Custom(
            format!(
                "{} malformed @framework(...) attribute(s) — expected \
                 @framework(<ident>, \"<citation>\")",
                malformed.len()
            )
            .into(),
        ));
    }

    Ok(())
}

/// Walk every `.vr` file under `root`, skipping target/ and hidden dirs.
fn discover_vr_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.') && name != "target" && name != "node_modules"
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_type().is_file()
            && entry.path().extension().map_or(false, |e| e == "vr")
        {
            out.push(entry.into_path());
        }
    }
    out
}

/// Parse a single `.vr` file without running semantic analysis. We only need
/// the top-level item list + attributes.
fn parse_file_for_audit(path: &Path) -> std::result::Result<verum_ast::Module, String> {
    let mut options = CompilerOptions::default();
    options.input = path.to_path_buf();
    let mut session = Session::new(options);
    let file_id = session
        .load_file(path)
        .map_err(|e| format!("load: {}", e))?;
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    pipeline
        .phase_parse(file_id)
        .map_err(|e| format!("parse: {}", e))
}

fn collect_framework_markers_from(
    attrs: &verum_common::List<verum_ast::attr::Attribute>,
    kind_label: &'static str,
    item_name: &Text,
    rel_path: &Path,
    by_framework: &mut BTreeMap<Text, Vec<FrameworkUsage>>,
    malformed: &mut Vec<(PathBuf, Text)>,
) {
    for attr in attrs.iter() {
        if !attr.is_named("framework") {
            continue;
        }
        match FrameworkAttr::from_attribute(attr) {
            Maybe::Some(fw) => {
                by_framework
                    .entry(fw.name)
                    .or_default()
                    .push(FrameworkUsage {
                        item_name: item_name.clone(),
                        item_kind: kind_label,
                        file: rel_path.to_path_buf(),
                        citation: fw.citation,
                    });
            }
            Maybe::None => {
                malformed.push((rel_path.to_path_buf(), item_name.clone()));
            }
        }
    }
}

fn print_framework_report(
    parsed_files: usize,
    skipped_files: usize,
    by_framework: &BTreeMap<Text, Vec<FrameworkUsage>>,
    malformed: &[(PathBuf, Text)],
) {
    println!();
    println!("{}", "Framework-axiom trusted boundary".bold());
    println!("{}", "─".repeat(40).dimmed());
    println!(
        "  Parsed {} .vr file(s), skipped {} unparseable file(s).",
        parsed_files, skipped_files
    );
    println!();

    if by_framework.is_empty() {
        println!("  {} no @framework(...) markers found.", "·".dimmed());
        println!(
            "  {} the proof corpus declares no dependency on external",
            "·".dimmed()
        );
        println!("    mathematical frameworks.");
        println!();
    } else {
        let total_markers: usize = by_framework.values().map(|v| v.len()).sum();
        println!(
            "  Found {} marker(s) across {} framework(s):",
            total_markers.to_string().bold(),
            by_framework.len().to_string().bold()
        );
        println!();

        for (framework, uses) in by_framework {
            println!(
                "  {} {} ({} marker{})",
                "▸".blue(),
                framework.as_str().bold(),
                uses.len(),
                if uses.len() == 1 { "" } else { "s" }
            );
            for u in uses {
                println!(
                    "    {} {} {}  —  {}  ({})",
                    "·".dimmed(),
                    u.item_kind,
                    u.item_name.as_str().cyan(),
                    u.citation.as_str(),
                    u.file.display()
                );
            }
            println!();
        }
    }

    if !malformed.is_empty() {
        ui::warn(&format!(
            "{} malformed @framework(...) marker(s) found:",
            malformed.len()
        ));
        for (file, item_name) in malformed {
            println!(
                "  · {} on {}  —  expected @framework(<ident>, \"<citation>\")",
                file.display(),
                item_name.as_str()
            );
        }
        println!();
    }
}
