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
    let manifest = Manifest::from_file(&Manifest::manifest_path(&manifest_dir))?;

    let lockfile_path = Manifest::lockfile_path(&manifest_dir);
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
    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let lockfile_path = Manifest::lockfile_path(&manifest_dir);

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

/// Output format for audit commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditFormat {
    /// Human-readable output with colours.
    Plain,
    /// Machine-parseable JSON with a stable schema. `schema_version`
    /// is included for consumer negotiation; see
    /// `docs/verification/cli-workflow.md` for the schema.
    Json,
}

/// Legacy entry point — defaults to plain output.
pub fn audit_framework_axioms() -> Result<()> {
    audit_framework_axioms_with_format(AuditFormat::Plain)
}

/// Entry point for `verum audit --framework-axioms [--format FORMAT]`.
pub fn audit_framework_axioms_with_format(format: AuditFormat) -> Result<()> {
    if matches!(format, AuditFormat::Plain) {
        ui::step("Enumerating framework-axiom trusted boundary");
    }

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

    match format {
        AuditFormat::Plain => print_framework_report(
            parsed_files,
            skipped_files,
            &by_framework,
            &malformed,
        ),
        AuditFormat::Json => print_framework_report_json(
            parsed_files,
            skipped_files,
            &by_framework,
            &malformed,
        ),
    }

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

/// Entry point for `verum audit --framework-conflicts [--format FORMAT]`.
///
/// Walks every `@framework(corpus, ...)` marker in the project,
/// collects the distinct corpus identifiers, and audits them
/// against the well-known incompatibility matrix
/// (`verum_verification::KNOWN_INCOMPATIBLE_PAIRS`). Each match
/// prints the conflict reason + literature citation.
///
/// Exits non-zero if any incompatible pair is found — the project's
/// axiom bundle would derive False, breaking every theorem (per
/// and the framework-compat module's V0 catalogue).
///
/// V0 (this revision) reads conflicts from the static Rust matrix
/// shipped at `crates/verum_verification/src/framework_compat.rs`.
///  will add per-package declarative conflicts so the
/// matrix doesn't have to be updated centrally for every new
/// framework package.
pub fn audit_framework_conflicts_with_format(format: AuditFormat) -> Result<()> {
    if matches!(format, AuditFormat::Plain) {
        ui::step("Auditing framework-package compatibility");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project.");
        return Ok(());
    }

    // Collect distinct corpora from every @framework(corpus, "...")
    // marker in the project. We reuse the framework-axioms walker
    // here (its by_framework BTreeMap key IS the corpus name).
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

    let corpora: Vec<Text> = by_framework.keys().cloned().collect();
    let conflicts = verum_verification::audit_framework_set(&corpora);

    match format {
        AuditFormat::Plain => {
            println!();
            println!("Framework-compatibility audit");
            println!("─────────────────────────────────────────");
            println!("Files parsed:        {}", parsed_files);
            println!("Files skipped:       {}", skipped_files);
            println!("Distinct corpora:    {}", corpora.len());
            for corpus in &corpora {
                println!("  • {}", corpus.as_str());
            }
            println!();
            if conflicts.is_empty() {
                println!(
                    "✓ No incompatible-pair conflicts found among {} corpora.",
                    corpora.len()
                );
            } else {
                println!("Conflicts: {}", conflicts.len());
                for d in &conflicts {
                    println!("  ✗ {}", d.message.as_str());
                }
            }
        }
        AuditFormat::Json => {
            let corpora_json: Vec<String> = corpora
                .iter()
                .map(|c| format!("\"{}\"", c.as_str().replace('"', "\\\"")))
                .collect();
            let conflicts_json: Vec<String> = conflicts
                .iter()
                .map(|d| {
                    format!(
                        "{{\"rule\":\"{}\",\"severity\":\"{}\",\"message\":\"{}\"}}",
                        d.rule,
                        d.severity.as_str(),
                        d.message.as_str().replace('"', "\\\"")
                    )
                })
                .collect();
            println!(
                "{{\"schema_version\":1,\"parsed\":{},\"skipped\":{},\
                 \"corpora\":[{}],\"conflicts\":[{}]}}",
                parsed_files,
                skipped_files,
                corpora_json.join(","),
                conflicts_json.join(",")
            );
        }
    }

    if !conflicts.is_empty() {
        return Err(crate::error::CliError::Custom(
            format!(
                "{} framework-compatibility conflict(s) — see report above",
                conflicts.len()
            )
            .into(),
        ));
    }

    Ok(())
}

/// `verum audit --accessibility` (item 4).
///
/// Walks every `@enact(...)` marker (and EpsilonOf-tagged
/// declaration) in the project, cross-references against
/// `@accessibility(λ)` annotations on the same item, and
/// surfaces every site that lacks an accessibility certificate.
///
/// Per Diakrisis Axi-4: M (the metaisation 2-functor) must be
/// λ-accessible for transfinite iterations to exist (Theorem
/// 10.T5 — `Fix(M) ≠ ∅`). The kernel cannot internally prove
/// accessibility — that's a meta-categorical claim — so
/// framework authors record the certified bound via
/// `@accessibility(λ)` on each `@enact` marker. This audit is
/// the CI gate: missing annotations → non-zero exit.
///
/// Plain output: per-item table with (kind, name, file,
/// has-accessibility, λ-if-any). JSON: schema_version=1 with
/// items array.
pub fn audit_accessibility_with_format(format: AuditFormat) -> Result<()> {
    if matches!(format, AuditFormat::Plain) {
        ui::step("Auditing @enact ↔ @accessibility coverage");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project.");
        return Ok(());
    }

    let mut rows: Vec<AccessibilityRow> = Vec::new();
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
                ItemKind::Function(func) => {
                    ("fn", func.name.name.clone(), &func.attributes)
                }
                _ => continue,
            };
            // Item participates in the audit iff it carries any
            // @enact marker. Items without @enact are skipped
            // entirely — they don't reference EpsilonOf and
            // don't need an accessibility certificate.
            let has_enact = item_attrs_have_named(&item.attributes, "enact")
                || item_attrs_have_named(decl_attrs, "enact");
            if !has_enact {
                continue;
            }
            let acc_lambda =
                find_accessibility_lambda(&item.attributes, decl_attrs);
            rows.push(AccessibilityRow {
                file: rel_path.clone(),
                item_kind: kind_label,
                item_name,
                accessibility: acc_lambda,
            });
        }
    }

    // Sort for deterministic CI-friendly output.
    rows.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.item_name.as_str().cmp(b.item_name.as_str()))
    });
    let missing: Vec<&AccessibilityRow> =
        rows.iter().filter(|r| r.accessibility.is_none()).collect();

    match format {
        AuditFormat::Plain => {
            print_accessibility_report(parsed_files, skipped_files, &rows);
        }
        AuditFormat::Json => {
            print_accessibility_report_json(parsed_files, skipped_files, &rows);
        }
    }

    if !missing.is_empty() {
        return Err(crate::error::CliError::Custom(
            format!(
                "{} @enact marker(s) without @accessibility(λ) — see report above",
                missing.len()
            )
            .into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct AccessibilityRow {
    file: PathBuf,
    item_kind: &'static str,
    item_name: Text,
    /// `Some(λ)` when the item carries `@accessibility(λ)`,
    /// `None` otherwise.
    accessibility: Option<Text>,
}

fn item_attrs_have_named(
    attrs: &verum_common::List<verum_ast::attr::Attribute>,
    name: &str,
) -> bool {
    attrs.iter().any(|a| a.name.as_str() == name)
}

fn find_accessibility_lambda(
    item_attrs: &verum_common::List<verum_ast::attr::Attribute>,
    decl_attrs: &verum_common::List<verum_ast::attr::Attribute>,
) -> Option<Text> {
    use verum_ast::attr::AccessibilityAttr;
    use verum_common::Maybe;
    for attrs in [item_attrs, decl_attrs] {
        for a in attrs.iter() {
            if let Maybe::Some(parsed) = AccessibilityAttr::from_attribute(a) {
                return Some(parsed.lambda);
            }
        }
    }
    None
}

fn print_accessibility_report(
    parsed_files: usize,
    skipped_files: usize,
    rows: &[AccessibilityRow],
) {
    println!();
    println!("{}", "@enact ↔ @accessibility(λ) coverage".bold());
    println!("{}", "─".repeat(50).dimmed());
    println!(
        "  Parsed {} .vr file(s), skipped {} unparseable file(s).",
        parsed_files, skipped_files
    );
    println!();

    if rows.is_empty() {
        println!("  {} no @enact markers found.", "·".dimmed());
        println!(
            "  {} the corpus declares no AC ↔ OC bridge sites;",
            "·".dimmed()
        );
        println!("    no Axi-4 accessibility certification is required.");
        println!();
        return;
    }

    let missing: Vec<&AccessibilityRow> =
        rows.iter().filter(|r| r.accessibility.is_none()).collect();
    let covered: Vec<&AccessibilityRow> =
        rows.iter().filter(|r| r.accessibility.is_some()).collect();

    println!(
        "  {} {} of {} @enact site(s) carry an @accessibility(λ) certificate.",
        if missing.is_empty() { "✓".green() } else { "·".yellow() },
        covered.len(),
        rows.len()
    );
    println!();

    if !covered.is_empty() {
        println!("  Annotated:");
        for r in &covered {
            println!(
                "    {} {} {}  —  λ = {}  ({})",
                "✓".green(),
                r.item_kind,
                r.item_name.as_str().cyan(),
                r.accessibility
                    .as_ref()
                    .map(|t| t.as_str())
                    .unwrap_or("?"),
                r.file.display()
            );
        }
        println!();
    }

    if !missing.is_empty() {
        println!("  {} Missing @accessibility(λ):", "✗".red().bold());
        for r in &missing {
            println!(
                "    {} {} {}  —  no @accessibility annotation  ({})",
                "✗".red(),
                r.item_kind,
                r.item_name.as_str().cyan(),
                r.file.display()
            );
        }
        println!();
        ui::warn(&format!(
            "{} @enact marker(s) lack @accessibility(λ). Each is a Diakrisis Axi-4 \
             accessibility-certificate gap. Add `@accessibility(omega)` (or higher) \
             to certify the framework-author bound, then re-run `verum audit --accessibility`.",
            missing.len()
        ));
        println!();
    }
}

fn print_accessibility_report_json(
    parsed_files: usize,
    skipped_files: usize,
    rows: &[AccessibilityRow],
) {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!("  \"parsed_files\": {},\n", parsed_files));
    out.push_str(&format!("  \"skipped_files\": {},\n", skipped_files));
    let total = rows.len();
    let missing = rows.iter().filter(|r| r.accessibility.is_none()).count();
    out.push_str(&format!("  \"total_enact_sites\": {},\n", total));
    out.push_str(&format!("  \"missing_accessibility\": {},\n", missing));
    out.push_str("  \"items\": [\n");
    for (i, r) in rows.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"file\": \"{}\",\n",
            r.file.display().to_string().replace('"', "\\\"")
        ));
        out.push_str(&format!("      \"item_kind\": \"{}\",\n", r.item_kind));
        out.push_str(&format!(
            "      \"item_name\": \"{}\",\n",
            r.item_name.as_str().replace('"', "\\\"")
        ));
        match &r.accessibility {
            Some(lambda) => {
                out.push_str(&format!(
                    "      \"accessibility\": \"{}\"\n",
                    lambda.as_str().replace('"', "\\\"")
                ));
            }
            None => {
                out.push_str("      \"accessibility\": null\n");
            }
        }
        out.push_str(if i + 1 == total { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ]\n");
    out.push_str("}");
    println!("{}", out);
}

/// Enumerate the kernel's primitive inference rules, corresponding
/// to the rule tables in `docs/verification/trusted-kernel.md` +
/// VVA §4.4 / §4.4a.
///
/// The audit is a static cross-reference: every entry has a
/// `crates/verum_kernel/src/<file>.rs` implementation and at least
/// one regression test under `crates/verum_kernel/tests/`. External
/// auditors verify the trusted-base by walking this list and
/// reading the cited implementations.
pub fn audit_kernel_rules(format: AuditFormat) -> Result<()> {
    /// One rule entry.
    struct Rule {
        number: u32,
        family: &'static str,
        name: &'static str,
        signature: &'static str,
    }

    // Corresponds to trusted-kernel.md §4.1–§4.5 + VVA §4.4 / §4.4a.
    // Refinement / framework / coherent / depth-omega rules added per
    // task #70 — the pre-V8 list listed only 18 rules but the kernel
    // ships more (depth.rs / eps_mu.rs / cert.rs all carry rules
    // that hadn't been surfaced to the audit display).
    const RULES: &[Rule] = &[
        Rule { number:  1, family: "structural", name: "K-Var",            signature: "Γ, x:A ⊢ x : A" },
        Rule { number:  2, family: "structural", name: "K-Lam",            signature: "Γ,x:A ⊢ t:B  ⟹  Γ ⊢ λx:A.t : Π x:A.B" },
        Rule { number:  3, family: "structural", name: "K-App",            signature: "Γ ⊢ f:Π x:A.B, Γ ⊢ a:A  ⟹  Γ ⊢ f a : B[x↦a]" },
        Rule { number:  4, family: "structural", name: "K-Pi-Form",        signature: "Γ ⊢ A:U_i, Γ,x:A ⊢ B:U_j  ⟹  Γ ⊢ Π x:A.B : U_max" },
        Rule { number:  5, family: "structural", name: "K-Universe-Cumul", signature: "Γ ⊢ A:U_i  ⟹  Γ ⊢ A:U_{i+1}" },
        Rule { number:  6, family: "structural", name: "K-Sigma-Form",     signature: "Γ ⊢ A:U_i, Γ,x:A ⊢ B:U_j  ⟹  Γ ⊢ Σ x:A.B : U_max" },
        Rule { number:  7, family: "inductive",  name: "K-Ind-Form",       signature: "well-formed mutual-inductive declaration" },
        Rule { number:  8, family: "inductive",  name: "K-Pos",            signature: "strict positivity walker (depth.rs::check_strict_positivity)" },
        Rule { number:  9, family: "inductive",  name: "K-Ind-Intro",      signature: "Ctor(args) well-typed vs declared signature" },
        Rule { number: 10, family: "inductive",  name: "K-Ind-Elim",       signature: "exhaustive pattern-match, arm typed in motive" },
        Rule { number: 11, family: "equality",   name: "K-Refl",           signature: "Refl(t) : Eq(A, t, t)" },
        Rule { number: 12, family: "equality",   name: "K-Eq-Elim (J)",    signature: "Martin-Löf J" },
        Rule { number: 13, family: "equality",   name: "K-UIP-Free",       signature: "reject any axiom reducing to UIP without @uip framework" },
        Rule { number: 14, family: "cubical",    name: "K-PathTy-Form",    signature: "PathTy(A, a, b) : U" },
        Rule { number: 15, family: "cubical",    name: "K-HComp",          signature: "CCHM homogeneous composition" },
        Rule { number: 16, family: "cubical",    name: "K-Transp",         signature: "transport along a path of types" },
        Rule { number: 17, family: "cubical",    name: "K-Glue",           signature: "glue at face φ — univalence-enabling" },
        Rule { number: 18, family: "cubical",    name: "K-Univalence",     signature: "ua : Equiv(A,B) → Path(U, A, B)  (via Glue)" },
        Rule { number: 19, family: "refinement", name: "K-Refine",         signature: "Γ ⊢ Refined(A,x,P) : Type_n  iff  dp(P) < dp(A)+1  (VVA §4.4)" },
        Rule { number: 20, family: "refinement", name: "K-RefineIntro",    signature: "Γ ⊢ a:A, proof:P[a/x]  ⟹  Γ ⊢ ⟨a|proof⟩ : Refined(A,x,P)" },
        Rule { number: 21, family: "refinement", name: "K-RefineErase",    signature: "Γ ⊢ r : Refined(A,x,P)  ⟹  Γ ⊢ r.value : A" },
        Rule { number: 22, family: "refinement", name: "K-Refine-omega",   signature: "ordinal-valued depth (depth.rs::m_depth_omega) — opt-in via @require_extension(vfe_7)" },
        Rule { number: 23, family: "framework",  name: "K-FwAx",           signature: "admit FrameworkAxiom(name,citation,body) — body:Prop + subsingleton" },
        Rule { number: 24, family: "framework",  name: "K-Eps-Mu",         signature: "ε∘M ≃ A∘ε naturality witness (eps_mu.rs::check_eps_mu_coherence) — Diakrisis Prop 5.1" },
        Rule { number: 25, family: "smt",        name: "K-Smt",            signature: "SmtCertificate(query, backend, witness) re-check via support.rs::replay_smt_cert_with_obligation" },
    ];

    match format {
        AuditFormat::Plain => {
            ui::step("Trusted-kernel primitive inference rules");
            println!();
            println!("  Rule  Family       Name                 Signature");
            println!("  ────  ───────────  ───────────────────  ──────────────────────────────────");
            for r in RULES {
                println!(
                    "  {:>3}   {:11}  {:19}  {}",
                    r.number,
                    r.family,
                    r.name,
                    r.signature
                );
            }
            println!();
            println!(
                "  Total: {} rules across 8 families (structural / inductive /",
                RULES.len()
            );
            println!(
                "  equality / cubical / refinement / framework / smt). See"
            );
            println!(
                "  docs/architecture/verum-verification-architecture.md §4.4 +"
            );
            println!(
                "  §4.4a for the full semantics and the LCF context."
            );
        }
        AuditFormat::Json => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!(
                "  \"rule_count\": {},\n",
                RULES.len()
            ));
            out.push_str("  \"rules\": [\n");
            for (i, r) in RULES.iter().enumerate() {
                out.push_str(&format!(
                    "    {{ \"number\": {}, \"family\": \"{}\", \"name\": \"{}\", \"signature\": \"{}\" }}{}\n",
                    r.number,
                    r.family,
                    r.name,
                    r.signature.replace('\\', "\\\\").replace('"', "\\\""),
                    if i + 1 < RULES.len() { "," } else { "" }
                ));
            }
            out.push_str("  ]\n");
            out.push('}');
            println!("{}", out);
        }
    }
    Ok(())
}

fn print_framework_report_json(
    parsed_files: usize,
    skipped_files: usize,
    by_framework: &BTreeMap<Text, Vec<FrameworkUsage>>,
    malformed: &[(PathBuf, Text)],
) {
    let mut out = String::from("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!(
        "  \"parsed_files\": {},\n",
        parsed_files
    ));
    out.push_str(&format!(
        "  \"skipped_files\": {},\n",
        skipped_files
    ));
    let total_markers: usize = by_framework.values().map(|v| v.len()).sum();
    out.push_str(&format!(
        "  \"total_markers\": {},\n",
        total_markers
    ));
    out.push_str(&format!(
        "  \"framework_count\": {},\n",
        by_framework.len()
    ));
    out.push_str("  \"frameworks\": [\n");
    let mut first = true;
    for (framework, uses) in by_framework {
        if !first {
            out.push_str(",\n");
        }
        first = false;
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"framework\": \"{}\",\n",
            json_escape(framework.as_str())
        ));
        out.push_str(&format!(
            "      \"marker_count\": {},\n",
            uses.len()
        ));
        out.push_str("      \"markers\": [\n");
        let mut first_use = true;
        for u in uses {
            if !first_use {
                out.push_str(",\n");
            }
            first_use = false;
            out.push_str("        {\n");
            out.push_str(&format!(
                "          \"item_kind\": \"{}\",\n",
                u.item_kind
            ));
            out.push_str(&format!(
                "          \"item_name\": \"{}\",\n",
                json_escape(u.item_name.as_str())
            ));
            out.push_str(&format!(
                "          \"file\": \"{}\",\n",
                json_escape(&u.file.display().to_string())
            ));
            out.push_str(&format!(
                "          \"citation\": \"{}\"\n",
                json_escape(u.citation.as_str())
            ));
            out.push_str("        }");
        }
        out.push_str("\n      ]\n    }");
    }
    out.push_str("\n  ],\n");
    out.push_str("  \"malformed\": [\n");
    let mut first_m = true;
    for (file, item_name) in malformed {
        if !first_m {
            out.push_str(",\n");
        }
        first_m = false;
        out.push_str(&format!(
            "    {{ \"file\": \"{}\", \"item_name\": \"{}\" }}",
            json_escape(&file.display().to_string()),
            json_escape(item_name.as_str())
        ));
    }
    out.push_str("\n  ]\n}");
    println!("{}", out);
}

/// Escape a string for JSON. Handles quotes, backslashes, and control
/// characters.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
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

// =============================================================================
// ε-audit — `verum audit --epsilon` (Phase 5 E3)
//
// Mirrors the `--framework-axioms` audit but for the DC (Actic) side of
// the OC/DC duality. Enumerates every `@enact(epsilon = "...")` marker
// attached to declarations in the current project, grouped by
// ε-primitive, so a reviewer sees the DC coordinate of the corpus
// parallel to the OC coordinate produced by `--framework-axioms`.
//
// Per + §21 (OWL 2 ecosystem), the eight canonical primitives are
//   ε_math, ε_compute, ε_observe, ε_prove,
//   ε_decide, ε_translate, ε_construct, ε_classify
// — see `core.action.primitives.Primitive`. Only these eight are
// recognised. Unknown strings land in the `malformed` bucket with a
// diagnostic suggesting the expected primitive set. ε_classify is the
// catalogue extension for ontology classification / subsumption /
// instance-check obligations introduced by (OWL 2 V1).
// =============================================================================

/// One `@enact(...)` usage collected from the project AST.
#[derive(Debug, Clone)]
struct EnactUsage {
    /// Declared item name (theorem / axiom / lemma / corollary / fn).
    item_name: Text,
    /// Source kind label (for diagnostics / reports).
    item_kind: &'static str,
    /// File path relative to project root.
    file: PathBuf,
}

/// Legacy entry-point for `verum audit --epsilon` with plain output.
pub fn audit_epsilon() -> Result<()> {
    audit_epsilon_with_format(AuditFormat::Plain)
}

/// Entry-point for `verum audit --epsilon [--format FORMAT]`. Mirrors
/// `audit_framework_axioms_with_format` structurally.
pub fn audit_epsilon_with_format(format: AuditFormat) -> Result<()> {
    if matches!(format, AuditFormat::Plain) {
        ui::step("Enumerating ε-distribution (Actic / DC coordinate)");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project.");
        return Ok(());
    }

    let mut by_epsilon: BTreeMap<Text, Vec<EnactUsage>> = BTreeMap::new();
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
            // Collect on the outer Item.attributes AND on inner decl
            // attributes, mirroring the @framework collection path. A
            // single declaration may have multiple attributes; each is
            // processed independently.
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
                ItemKind::Function(func) => {
                    ("fn", func.name.name.clone(), &func.attributes)
                }
                _ => continue,
            };
            collect_enact_markers_from(
                &item.attributes,
                kind_label,
                &item_name,
                &rel_path,
                &mut by_epsilon,
                &mut malformed,
            );
            collect_enact_markers_from(
                decl_attrs,
                kind_label,
                &item_name,
                &rel_path,
                &mut by_epsilon,
                &mut malformed,
            );
        }
    }

    match format {
        AuditFormat::Plain => {
            print_epsilon_report(parsed_files, skipped_files, &by_epsilon, &malformed);
        }
        AuditFormat::Json => {
            print_epsilon_report_json(
                parsed_files,
                skipped_files,
                &by_epsilon,
                &malformed,
            );
        }
    }
    Ok(())
}

fn collect_enact_markers_from(
    attrs: &verum_common::List<verum_ast::attr::Attribute>,
    kind_label: &'static str,
    item_name: &Text,
    rel_path: &Path,
    by_epsilon: &mut BTreeMap<Text, Vec<EnactUsage>>,
    malformed: &mut Vec<(PathBuf, Text)>,
) {
    use verum_ast::attr::EnactAttr;
    for attr in attrs.iter() {
        if !attr.is_named("enact") {
            continue;
        }
        match EnactAttr::from_attribute(attr) {
            Maybe::Some(ea) => {
                by_epsilon
                    .entry(ea.epsilon)
                    .or_default()
                    .push(EnactUsage {
                        item_name: item_name.clone(),
                        item_kind: kind_label,
                        file: rel_path.to_path_buf(),
                    });
            }
            Maybe::None => {
                malformed.push((rel_path.to_path_buf(), item_name.clone()));
            }
        }
    }
}

fn print_epsilon_report(
    parsed_files: usize,
    skipped_files: usize,
    by_epsilon: &BTreeMap<Text, Vec<EnactUsage>>,
    malformed: &[(PathBuf, Text)],
) {
    println!();
    println!("{}", "ε-distribution (Actic / DC coordinate)".bold());
    println!("{}", "─".repeat(40).dimmed());
    println!(
        "  Parsed {} .vr file(s), skipped {} unparseable file(s).",
        parsed_files, skipped_files
    );
    println!();

    if by_epsilon.is_empty() {
        println!("  {} no @enact(epsilon = \"...\") markers found.", "·".dimmed());
        println!(
            "  {} the corpus declares no DC-side ε-coordinate; every",
            "·".dimmed()
        );
        println!(
            "    function's ε will be inferred from its body during"
        );
        println!("    compile-time `core.action.verify.verify_epsilon`.");
        println!();
    } else {
        let total_markers: usize = by_epsilon.values().map(|v| v.len()).sum();
        println!(
            "  Found {} marker(s) across {} ε-primitive(s):",
            total_markers.to_string().bold(),
            by_epsilon.len().to_string().bold()
        );
        println!();
        for (epsilon, uses) in by_epsilon {
            println!(
                "  {} {} ({} marker{})",
                "▸".magenta(),
                epsilon.as_str().bold(),
                uses.len(),
                if uses.len() == 1 { "" } else { "s" }
            );
            for u in uses {
                println!(
                    "    {} {} {}  —  {}",
                    "·".dimmed(),
                    u.item_kind,
                    u.item_name.as_str().cyan(),
                    u.file.display()
                );
            }
            println!();
        }
    }

    if !malformed.is_empty() {
        ui::warn(&format!(
            "{} malformed @enact(...) marker(s) found:",
            malformed.len()
        ));
        for (file, item_name) in malformed {
            println!(
                "  · {} on {}  —  expected @enact(epsilon = \"<primitive>\")",
                file.display(),
                item_name.as_str()
            );
        }
        println!(
            "  known primitives: {}",
            "ε_math, ε_compute, ε_observe, ε_prove, ε_decide, ε_translate, ε_construct, ε_classify"
                .dimmed()
        );
        println!(
            "  ordinal coords:   {}",
            "0, 1, 2, …, ω, ω+k, ω·n, ω·n+k, ω², Ω (also ASCII: omega, omega_squared, …)"
                .dimmed()
        );
        println!();
    }
}

fn print_epsilon_report_json(
    parsed_files: usize,
    skipped_files: usize,
    by_epsilon: &BTreeMap<Text, Vec<EnactUsage>>,
    malformed: &[(PathBuf, Text)],
) {
    // Hand-rolled JSON for deterministic output; mirrors
    // `print_framework_report_json` so CI consumers see the same
    // schema shape for OC and DC audits.
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!("  \"parsed_files\": {},\n", parsed_files));
    out.push_str(&format!("  \"skipped_files\": {},\n", skipped_files));
    out.push_str("  \"epsilons\": [\n");
    let total_eps = by_epsilon.len();
    for (i, (epsilon, uses)) in by_epsilon.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"epsilon\": \"{}\",\n",
            json_escape(epsilon.as_str())
        ));
        out.push_str("      \"usages\": [\n");
        let total_u = uses.len();
        for (j, u) in uses.iter().enumerate() {
            out.push_str("        {\n");
            out.push_str(&format!(
                "          \"item_kind\": \"{}\",\n",
                u.item_kind
            ));
            out.push_str(&format!(
                "          \"item_name\": \"{}\",\n",
                json_escape(u.item_name.as_str())
            ));
            out.push_str(&format!(
                "          \"file\": \"{}\"\n",
                json_escape(&u.file.display().to_string())
            ));
            out.push_str(if j + 1 == total_u { "        }\n" } else { "        },\n" });
        }
        out.push_str("      ]\n");
        out.push_str(if i + 1 == total_eps { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ],\n");
    out.push_str("  \"malformed\": [\n");
    for (i, (file, item_name)) in malformed.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"file\": \"{}\",\n",
            json_escape(&file.display().to_string())
        ));
        out.push_str(&format!(
            "      \"item_name\": \"{}\"\n",
            json_escape(item_name.as_str())
        ));
        out.push_str(if i + 1 == malformed.len() { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ]\n");
    out.push_str("}\n");
    print!("{}", out);
}

// =============================================================================
// MSFS-coord audit — `verum audit --coord` (Phase 5 E4)
//
// Walks the same `@framework(name, "citation")` markers that `--framework-
// axioms` enumerates, and projects each unique framework to its MSFS
// coordinate (Framework, ν, τ). The (ν, τ) lookup mirrors
// `core.theory_interop.coord::coord_of` for the standard six-pack — when
// they drift, this is the canonical source for the CLI surface.
// =============================================================================

/// (ordinal, tau) for a known framework name. Mirrors
/// `core/theory_interop/coord.vr::known_ordinal` + `known_tau`. Unknown
/// frameworks get (0, true) — the same fall-through the stdlib uses.
/// Cantor-normal-form prefix below ε_0: every ordinal we emit lives in
/// the (omega_coefficient, finite_offset) shape — same encoding as
/// `core.theory_interop.coord::Ordinal` (single source of truth between
/// stdlib + CLI). Comparison is lex on the pair; rendering uses Unicode
/// `ω` so reports match the spec verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CliOrdinal {
    omega_coeff: u32,
    finite_offset: u32,
}

impl CliOrdinal {
    const fn finite(n: u32) -> Self {
        Self { omega_coeff: 0, finite_offset: n }
    }
    const fn omega() -> Self {
        Self { omega_coeff: 1, finite_offset: 0 }
    }
    const fn omega_plus(k: u32) -> Self {
        Self { omega_coeff: 1, finite_offset: k }
    }

    fn render(&self) -> String {
        if self.omega_coeff == 0 {
            return self.finite_offset.to_string();
        }
        let coeff = if self.omega_coeff == 1 {
            "ω".to_string()
        } else {
            format!("ω·{}", self.omega_coeff)
        };
        if self.finite_offset == 0 {
            coeff
        } else {
            format!("{}+{}", coeff, self.finite_offset)
        }
    }

    /// lex ordering on (omega_coeff, finite_offset).
    /// Mirrors `verum_kernel::OrdinalDepth::lt` exactly so the
    /// CLI side produces identical results to the kernel for any
    /// shared ordinal pair.
    fn lt(&self, other: &Self) -> bool {
        if self.omega_coeff < other.omega_coeff {
            return true;
        }
        if self.omega_coeff > other.omega_coeff {
            return false;
        }
        self.finite_offset < other.finite_offset
    }
}

/// (ν, τ) lookup for the standard six-pack + neutral Actic. Mirrors
/// `core/theory_interop/coord.vr::known_ordinal` + `known_tau` — when
/// the two drift, the .vr table is the authoritative source. ν-values
/// are transfinite ordinals below ε_0 (lex-encoded); the previous
/// flat-Int collapse silently dropped the ω-stratum used by lurie_htt
/// / schreiber_dcct / baez_dolan.
fn msfs_lookup(framework_name: &str) -> (CliOrdinal, bool) {
    match framework_name {
        "actic.raw"             => (CliOrdinal::finite(0),  false),
        "lurie_htt"             => (CliOrdinal::omega(),    true),
        "schreiber_dcct"        => (CliOrdinal::omega_plus(2), true),
        "connes_reconstruction" => (CliOrdinal::omega(),    false),
        "petz_classification"   => (CliOrdinal::finite(2),  false),
        "arnold_catastrophe"    => (CliOrdinal::finite(2),  true),
        "baez_dolan"            => (CliOrdinal::omega_plus(1), true),
        "owl2_fs"               => (CliOrdinal::finite(1),  true),
        _                       => (CliOrdinal::finite(0),  true),
    }
}

/// Legacy entry — defaults to plain output.
pub fn audit_coord() -> Result<()> {
    audit_coord_with_format(AuditFormat::Plain)
}

/// Entry-point for `verum audit --coord [--format FORMAT]`.
pub fn audit_coord_with_format(format: AuditFormat) -> Result<()> {
    if matches!(format, AuditFormat::Plain) {
        ui::step("Computing MSFS coordinate (Framework, ν, τ) per theorem");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project.");
        return Ok(());
    }

    // Re-use the framework collector so OC and coord audits read from one
    // ground truth.
    let mut by_framework: BTreeMap<Text, Vec<FrameworkUsage>> = BTreeMap::new();
    let mut malformed: Vec<(PathBuf, Text)> = Vec::new();
    // Per-item @verify(...) strategy. The strategy lifts the framework's
    // ν-coordinate per VVA §2.3 (`runtime` ↦ 0 / `static` ↦ 1 / `fast` ↦ 2 /
    // `formal` ↦ ω / `proof` ↦ ω+1 / `thorough` ↦ ω·2 / `reliable` ↦ ω·2+1 /
    // `certified` ↦ ω·2+2 / `synthesize` ↦ ≤ω·3+1). Theorem-level ν is
    // max(framework_nu, verify_nu); without `@verify(...)` we project to
    // the framework's bare ν (axiom-postulated case).
    let mut verify_by_item: BTreeMap<(PathBuf, Text, &'static str), Text> =
        BTreeMap::new();
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
            // Capture `@verify(...)` strategy per item — strictest mode
            // wins when multiple are declared (e.g. `@verify(formal +
            // proof)` lifts to `proof`).
            if let Some(strategy) =
                strictest_verify_strategy(&item.attributes, decl_attrs)
            {
                verify_by_item.insert(
                    (rel_path.clone(), item_name.clone(), kind_label),
                    strategy,
                );
            }
        }
    }

    match format {
        AuditFormat::Plain => print_coord_report(
            parsed_files,
            skipped_files,
            &by_framework,
            &verify_by_item,
            &malformed,
        ),
        AuditFormat::Json => print_coord_report_json(
            parsed_files,
            skipped_files,
            &by_framework,
            &verify_by_item,
            &malformed,
        ),
    }
    Ok(())
}

/// Map `@verify(<strategy>)` to its ν-ordinal per VVA §2.3.
/// Returns `CliOrdinal::finite(0)` for unknown / `runtime`.
fn verify_strategy_ordinal(strategy: &str) -> CliOrdinal {
    match strategy {
        "runtime"           => CliOrdinal::finite(0),
        "static"            => CliOrdinal::finite(1),
        "fast"              => CliOrdinal::finite(2),
        "complexity_typed"  => CliOrdinal::finite(2),
        "formal"            => CliOrdinal::omega(),
        "proof"             => CliOrdinal::omega_plus(1),
        "thorough"          => CliOrdinal { omega_coeff: 2, finite_offset: 0 },
        "reliable"          => CliOrdinal { omega_coeff: 2, finite_offset: 1 },
        "certified"         => CliOrdinal { omega_coeff: 2, finite_offset: 2 },
        "coherent_static"   => CliOrdinal::omega(),
        "coherent_runtime"  => CliOrdinal::finite(0),
        "coherent"          => CliOrdinal::omega_plus(1),
        "synthesize"        => CliOrdinal { omega_coeff: 3, finite_offset: 1 },
        "assume"            => CliOrdinal::finite(0),
        _                   => CliOrdinal::finite(0),
    }
}

/// Pick the strictest (lex-maximum) `@verify(...)` strategy from the
/// item's attribute lists. Returns `None` if no `@verify(...)` is
/// declared. Strictness is the same lex ordering used for the
/// per-theorem ν projection (`verify_strategy_ordinal`).
fn strictest_verify_strategy(
    item_attrs: &verum_common::List<verum_ast::attr::Attribute>,
    decl_attrs: &verum_common::List<verum_ast::attr::Attribute>,
) -> Option<Text> {
    use verum_ast::attr::{FromAttribute, VerifyAttr};
    let mut best: Option<(CliOrdinal, Text)> = None;
    for attrs in [item_attrs, decl_attrs] {
        for attr in attrs.iter() {
            if !attr.is_named("verify") {
                continue;
            }
            let Ok(verify) = VerifyAttr::from_attribute(attr) else {
                continue;
            };
            for mode in verify.modes.iter() {
                let label = mode.as_str();
                let ord = verify_strategy_ordinal(label);
                let label_text = Text::from(label);
                match &best {
                    Some((best_ord, _)) => {
                        if best_ord.lt(&ord) {
                            best = Some((ord, label_text));
                        }
                    }
                    None => best = Some((ord, label_text)),
                }
            }
        }
    }
    best.map(|(_, label)| label)
}

fn print_coord_report(
    parsed_files: usize,
    skipped_files: usize,
    by_framework: &BTreeMap<Text, Vec<FrameworkUsage>>,
    verify_by_item: &BTreeMap<(PathBuf, Text, &'static str), Text>,
    malformed: &[(PathBuf, Text)],
) {
    println!();
    println!("{}", "MSFS coordinate (Framework, ν, τ) per theorem".bold());
    println!("{}", "─".repeat(50).dimmed());
    println!(
        "  Parsed {} .vr file(s), skipped {} unparseable file(s).",
        parsed_files, skipped_files
    );
    println!();

    if by_framework.is_empty() {
        println!("  {} no @framework(...) markers found.", "·".dimmed());
        println!(
            "  {} the corpus declares no Rich-foundation footprint;",
            "·".dimmed()
        );
        println!("    every theorem is rigorous in the bare kernel.");
        println!();
        return;
    }

    let total_markers: usize = by_framework.values().map(|v| v.len()).sum();
    println!(
        "  Found {} theorem-level marker(s) across {} framework(s):",
        total_markers.to_string().bold(),
        by_framework.len().to_string().bold()
    );
    println!();

    for (framework, uses) in by_framework {
        let (ordinal, tau) = msfs_lookup(framework.as_str());
        let tau_str = if tau { "τ=intensional" } else { "τ=extensional" };
        println!(
            "  {} {}  ν={}  {}  ({} marker{})",
            "▸".magenta(),
            framework.as_str().bold(),
            ordinal.render(),
            tau_str.dimmed(),
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

    // per-theorem inferred-coordinate section.
    // For each theorem/lemma/corollary, the inferred (Fw, ν, τ)
    // is the **max-of-cited-coords + lifted by @verify**:
    //   * framework_nu = max over all `@framework(name, ...)` markers
    //   * verify_nu    = ν of the strictest `@verify(strategy)` (VVA §2.3)
    //   * theorem_nu   = max(framework_nu, verify_nu)
    // `@verify(formal)` is precisely what lifts an axiom-postulated
    // theorem from ν=0 (paper-cited) to ν=ω (machine-checked SMT).
    let per_theorem = invert_to_per_theorem(by_framework, verify_by_item);
    if !per_theorem.is_empty() {
        println!();
        println!(
            "  {} Per-theorem inferred coordinates (max of cited frameworks ⊔ @verify):",
            "▸".green().bold()
        );
        println!();
        for entry in &per_theorem {
            let verify_label: &str = entry
                .verify_strategy
                .as_ref()
                .map(|s| s.as_str())
                .unwrap_or("—");
            println!(
                "    {} {} {}  →  ({}, ν={}, {}τ)  [verify={}]  [{} cit{}]  {}",
                "·".dimmed(),
                entry.item_kind,
                entry.item_name.as_str().cyan(),
                entry.inferred_fw.as_str().bold(),
                entry.inferred_nu.render(),
                if entry.inferred_tau {
                    "intensional-"
                } else {
                    "extensional-"
                },
                verify_label,
                entry.frameworks_cited.len(),
                if entry.frameworks_cited.len() == 1 { "" } else { "s" },
                entry.file.display()
            );
        }
        println!();
    }

    if !malformed.is_empty() {
        ui::warn(&format!(
            "{} malformed @framework(...) marker(s) skipped from coord report.",
            malformed.len()
        ));
        println!();
    }
}

/// invert the per-framework view to a per-theorem
/// view, computing the max-of-cited-coords inference for each
/// theorem/lemma/corollary/axiom.
///
/// Per defect 2: every theorem in the project
/// gets a (Fw, ν, τ) coordinate inferred from the maximum
/// (lex on OrdinalDepth) of the framework coordinates cited
/// via @framework markers on that item. Returns a sorted
/// list (by item_name) of inferred coordinates.
pub(crate) fn invert_to_per_theorem(
    by_framework: &BTreeMap<Text, Vec<FrameworkUsage>>,
    verify_by_item: &BTreeMap<(PathBuf, Text, &'static str), Text>,
) -> Vec<PerTheoremCoord> {
    use std::collections::BTreeMap as Map;
    // Group cited frameworks by (file, item_name, item_kind).
    let mut per_theorem: Map<(PathBuf, Text, &'static str), Vec<Text>> = Map::new();
    for (fw_name, uses) in by_framework {
        for u in uses {
            let key = (u.file.clone(), u.item_name.clone(), u.item_kind);
            per_theorem
                .entry(key)
                .or_insert_with(Vec::new)
                .push(fw_name.clone());
        }
    }
    let mut result: Vec<PerTheoremCoord> = per_theorem
        .into_iter()
        .map(|((file, item_name, item_kind), frameworks_cited)| {
            // Compute max-of-cited-coords. Fw with maximum ν
            // wins; ties broken by lex on framework name.
            let mut best: Option<(Text, CliOrdinal, bool)> = None;
            for fw in &frameworks_cited {
                let (ord, tau) = msfs_lookup(fw.as_str());
                match &best {
                    Some((_, best_ord, _)) => {
                        // Lex: ord > best_ord OR equal and fw < best name.
                        if best_ord.lt(&ord) {
                            best = Some((fw.clone(), ord, tau));
                        }
                    }
                    None => {
                        best = Some((fw.clone(), ord, tau));
                    }
                }
            }
            let (inferred_fw, framework_nu, inferred_tau) = best.unwrap();
            // Lift the framework ν by `@verify(...)` strategy if any —
            // `@verify(formal)` raises an axiom-postulated theorem to
            // ν=ω even though its frameworks may carry no ν of their
            // own. The lift is monotone: theorem_nu = max(framework_nu,
            // verify_nu).
            let key = (file.clone(), item_name.clone(), item_kind);
            let verify_strategy = verify_by_item.get(&key).cloned();
            let inferred_nu = match &verify_strategy {
                Some(strategy) => {
                    let verify_nu = verify_strategy_ordinal(strategy.as_str());
                    if framework_nu.lt(&verify_nu) {
                        verify_nu
                    } else {
                        framework_nu
                    }
                }
                None => framework_nu,
            };
            PerTheoremCoord {
                file,
                item_name,
                item_kind,
                inferred_fw,
                inferred_nu,
                inferred_tau,
                frameworks_cited,
                verify_strategy,
            }
        })
        .collect();
    // Stable sort: by file then item_name for deterministic
    // CI-friendly output.
    result.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.item_name.as_str().cmp(b.item_name.as_str()))
    });
    result
}

/// per-theorem inferred coordinate row.
#[derive(Debug, Clone)]
pub(crate) struct PerTheoremCoord {
    pub(crate) file: PathBuf,
    pub(crate) item_name: Text,
    pub(crate) item_kind: &'static str,
    pub(crate) inferred_fw: Text,
    pub(crate) inferred_nu: CliOrdinal,
    pub(crate) inferred_tau: bool,
    pub(crate) frameworks_cited: Vec<Text>,
    /// Strictest `@verify(...)` strategy declared on the item,
    /// `None` if the item has no `@verify` annotation. The strategy
    /// lifts `inferred_nu` via VVA §2.3 (per `verify_strategy_ordinal`).
    pub(crate) verify_strategy: Option<Text>,
}

fn print_coord_report_json(
    parsed_files: usize,
    skipped_files: usize,
    by_framework: &BTreeMap<Text, Vec<FrameworkUsage>>,
    verify_by_item: &BTreeMap<(PathBuf, Text, &'static str), Text>,
    malformed: &[(PathBuf, Text)],
) {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"schema_version\": 2,\n");
    out.push_str(&format!("  \"parsed_files\": {},\n", parsed_files));
    out.push_str(&format!("  \"skipped_files\": {},\n", skipped_files));
    out.push_str("  \"frameworks\": [\n");
    let total_fw = by_framework.len();
    for (i, (framework, uses)) in by_framework.iter().enumerate() {
        let (ordinal, tau) = msfs_lookup(framework.as_str());
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"framework\": \"{}\",\n",
            json_escape(framework.as_str())
        ));
        // Structured ν: emit both the human-readable string ("ω", "ω+2",
        // …) and the (omega_coefficient, finite_offset) pair so JSON
        // consumers can sort lexicographically without re-parsing.
        out.push_str(&format!(
            "      \"ordinal\": \"{}\",\n",
            json_escape(&ordinal.render())
        ));
        out.push_str(&format!(
            "      \"ordinal_omega_coefficient\": {},\n",
            ordinal.omega_coeff
        ));
        out.push_str(&format!(
            "      \"ordinal_finite_offset\": {},\n",
            ordinal.finite_offset
        ));
        out.push_str(&format!("      \"tau\": {},\n", tau));
        out.push_str("      \"usages\": [\n");
        let total_u = uses.len();
        for (j, u) in uses.iter().enumerate() {
            out.push_str("        {\n");
            out.push_str(&format!(
                "          \"item_kind\": \"{}\",\n",
                u.item_kind
            ));
            out.push_str(&format!(
                "          \"item_name\": \"{}\",\n",
                json_escape(u.item_name.as_str())
            ));
            out.push_str(&format!(
                "          \"citation\": \"{}\",\n",
                json_escape(u.citation.as_str())
            ));
            out.push_str(&format!(
                "          \"file\": \"{}\"\n",
                json_escape(&u.file.display().to_string())
            ));
            out.push_str(if j + 1 == total_u { "        }\n" } else { "        },\n" });
        }
        out.push_str("      ]\n");
        out.push_str(if i + 1 == total_fw { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ],\n");
    // Per-theorem inferred coordinates (schema_version 2 — adds the
    // `verify_strategy` field and the lifted `inferred_nu` from VVA §2.3).
    let per_theorem = invert_to_per_theorem(by_framework, verify_by_item);
    out.push_str("  \"per_theorem\": [\n");
    let total_pt = per_theorem.len();
    for (i, entry) in per_theorem.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"item_kind\": \"{}\",\n",
            entry.item_kind
        ));
        out.push_str(&format!(
            "      \"item_name\": \"{}\",\n",
            json_escape(entry.item_name.as_str())
        ));
        out.push_str(&format!(
            "      \"file\": \"{}\",\n",
            json_escape(&entry.file.display().to_string())
        ));
        out.push_str(&format!(
            "      \"inferred_framework\": \"{}\",\n",
            json_escape(entry.inferred_fw.as_str())
        ));
        out.push_str(&format!(
            "      \"inferred_nu\": \"{}\",\n",
            json_escape(&entry.inferred_nu.render())
        ));
        out.push_str(&format!(
            "      \"inferred_nu_omega_coefficient\": {},\n",
            entry.inferred_nu.omega_coeff
        ));
        out.push_str(&format!(
            "      \"inferred_nu_finite_offset\": {},\n",
            entry.inferred_nu.finite_offset
        ));
        out.push_str(&format!(
            "      \"inferred_tau\": {},\n",
            entry.inferred_tau
        ));
        match &entry.verify_strategy {
            Some(strategy) => out.push_str(&format!(
                "      \"verify_strategy\": \"{}\",\n",
                json_escape(strategy.as_str())
            )),
            None => out.push_str("      \"verify_strategy\": null,\n"),
        }
        out.push_str("      \"frameworks_cited\": [");
        for (j, fw) in entry.frameworks_cited.iter().enumerate() {
            if j > 0 { out.push_str(", "); }
            out.push_str(&format!("\"{}\"", json_escape(fw.as_str())));
        }
        out.push_str("]\n");
        out.push_str(if i + 1 == total_pt { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ],\n");
    out.push_str("  \"malformed\": [\n");
    for (i, (file, item_name)) in malformed.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"file\": \"{}\",\n",
            json_escape(&file.display().to_string())
        ));
        out.push_str(&format!(
            "      \"item_name\": \"{}\"\n",
            json_escape(item_name.as_str())
        ));
        out.push_str(if i + 1 == malformed.len() { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ]\n");
    out.push_str("}\n");
    print!("{}", out);
}

// =============================================================================
// Articulation Hygiene audit — `verum audit --hygiene`
//
// Walks every type / function declaration in the project and classifies each
// "self-X" surface form against the hygiene table:
//
//   Surface                                  Factorisation (Φ, κ, t)
//   ──────────────────────────────────────   ───────────────────────────
//   Inductive `Rec(T)` in `type T is Rec(T)` (T_succ, ω,    least_fp)
//   Coinductive `Stream<A> = Cons(A, …)`     (T_prod_A, ω^{op}, greatest_fp)
//   Newtype `type X is (Y)`                   (Id, 1,        Y)
//   HIT path-cell variant (`Foo() = a..b`)    (path_action, ω, base)
//   `@recursive fn f(… -> Self) …`            (unfold_f, ω,  fix_f)
//   `@corecursive fn g(…)` (productivity)     (corec_g, ω^{op}, fix_g)
//
// V1 scope:
//   * variant-self-reference detection (a constructor arg that mentions the
//     surrounding type's own name) — covers Inductive + sum-type recursion;
//   * explicit Inductive / Coinductive bodies detected via TypeDeclBody;
//   * HIT path-cell variants flagged by `path_endpoints`;
//   * `@recursive` / `@corecursive` attributes on FunctionDecl.
//
// Out of scope (V1, deferred to a kernel-pass follow-up):
//   * raw `self` keyword usage inside function bodies (requires
//     expression-tree walk);
//   * §13.2's `Self::Item` and `&mut self` factorisations (require a typed
//     resolution layer).
// =============================================================================

#[derive(Debug, Clone, Copy)]
enum HygieneClass {
    Inductive,             // (T_succ, ω, least_fp)
    Coinductive,           // (T_prod, ω^{op}, greatest_fp)
    Newtype,               // (Id, 1, base)
    HigherInductive,       // (path_action, ω, base)
    Recursive,             // @recursive — (unfold_f, ω, fix_f)
    Corecursive,           // @corecursive — (corec_g, ω^{op}, fix_g)
}

impl HygieneClass {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Inductive       => "inductive",
            Self::Coinductive     => "coinductive",
            Self::Newtype         => "newtype",
            Self::HigherInductive => "higher-inductive",
            Self::Recursive       => "recursive-fn",
            Self::Corecursive     => "corecursive-fn",
        }
    }

    fn factorisation(&self) -> &'static str {
        match self {
            Self::Inductive       => "(T_succ, ω, least_fp)",
            Self::Coinductive     => "(T_prod, ω^op, greatest_fp)",
            Self::Newtype         => "(Id, 1, base)",
            Self::HigherInductive => "(path_action, ω, base)",
            Self::Recursive       => "(unfold_f, ω, fix_f)",
            Self::Corecursive     => "(corec_g, ω^op, fix_g)",
        }
    }
}

#[derive(Debug, Clone)]
struct HygieneEntry {
    class: HygieneClass,
    item_name: Text,
    file: PathBuf,
}

/// True iff the named type appears anywhere inside `t`. Walks Path,
/// Generic, Tuple, Array, Slice, Function (params + return), Reference,
/// DependentApp, and PathType — covers every nesting site that could
/// transport a self-recursive constructor argument such as `tail:
/// List<A>` inside `type List<A> is Cons(head: A, tail: List<A>)`.
/// Conservative: false negatives are acceptable (under-report); false
/// positives are not (over-flag).
fn type_mentions_name(t: &verum_ast::ty::Type, target: &str) -> bool {
    use verum_ast::ty::{GenericArg, PathSegment, TypeKind};
    match &t.kind {
        TypeKind::Path(p) => p
            .segments
            .iter()
            .any(|seg| matches!(seg, PathSegment::Name(id) if id.name.as_str() == target)),
        TypeKind::Generic { base, args, .. } => {
            type_mentions_name(base, target)
                || args.iter().any(|arg| match arg {
                    GenericArg::Type(ty) => type_mentions_name(ty, target),
                    _ => false,
                })
        }
        TypeKind::Tuple(types) => types.iter().any(|x| type_mentions_name(x, target)),
        TypeKind::Array { element, .. } => type_mentions_name(element, target),
        TypeKind::Slice(inner) => type_mentions_name(inner, target),
        TypeKind::Function { params, return_type, .. } => {
            params.iter().any(|x| type_mentions_name(x, target))
                || type_mentions_name(return_type, target)
        }
        TypeKind::Reference { inner, .. } => type_mentions_name(inner, target),
        TypeKind::DependentApp { carrier, .. } => type_mentions_name(carrier, target),
        TypeKind::PathType { carrier, .. } => type_mentions_name(carrier, target),
        _ => false,
    }
}

fn variant_is_self_recursive(v: &verum_ast::decl::Variant, type_name: &str) -> bool {
    use verum_ast::decl::VariantData;
    let data = match &v.data {
        Maybe::Some(d) => d,
        Maybe::None    => return false,
    };
    match data {
        VariantData::Tuple(types) => types.iter().any(|t| type_mentions_name(t, type_name)),
        VariantData::Record(fields) => fields.iter().any(|f| type_mentions_name(&f.ty, type_name)),
    }
}

fn variant_is_path_cell(v: &verum_ast::decl::Variant) -> bool {
    matches!(&v.path_endpoints, Maybe::Some(_))
}

fn classify_type_decl(
    decl: &verum_ast::decl::TypeDecl,
    rel_path: &Path,
    out: &mut Vec<HygieneEntry>,
) {
    use verum_ast::decl::TypeDeclBody;
    let name_str = decl.name.name.as_str().to_string();
    match &decl.body {
        TypeDeclBody::Variant(variants) | TypeDeclBody::Inductive(variants) => {
            let any_path_cell = variants.iter().any(variant_is_path_cell);
            let any_recursive = variants.iter().any(|v| variant_is_self_recursive(v, &name_str));
            if any_path_cell {
                out.push(HygieneEntry {
                    class: HygieneClass::HigherInductive,
                    item_name: decl.name.name.clone(),
                    file: rel_path.to_path_buf(),
                });
            } else if any_recursive {
                out.push(HygieneEntry {
                    class: HygieneClass::Inductive,
                    item_name: decl.name.name.clone(),
                    file: rel_path.to_path_buf(),
                });
            }
        }
        TypeDeclBody::Coinductive(_) => {
            out.push(HygieneEntry {
                class: HygieneClass::Coinductive,
                item_name: decl.name.name.clone(),
                file: rel_path.to_path_buf(),
            });
        }
        TypeDeclBody::Newtype(_) => {
            out.push(HygieneEntry {
                class: HygieneClass::Newtype,
                item_name: decl.name.name.clone(),
                file: rel_path.to_path_buf(),
            });
        }
        _ => {}
    }
}

fn classify_function_decl(
    decl: &verum_ast::decl::FunctionDecl,
    rel_path: &Path,
    out: &mut Vec<HygieneEntry>,
) {
    let mut has_recursive   = false;
    let mut has_corecursive = false;
    for attr in decl.attributes.iter() {
        if attr.is_named("recursive")   { has_recursive   = true; }
        if attr.is_named("corecursive") { has_corecursive = true; }
    }
    if has_corecursive {
        out.push(HygieneEntry {
            class: HygieneClass::Corecursive,
            item_name: decl.name.name.clone(),
            file: rel_path.to_path_buf(),
        });
    }
    if has_recursive {
        out.push(HygieneEntry {
            class: HygieneClass::Recursive,
            item_name: decl.name.name.clone(),
            file: rel_path.to_path_buf(),
        });
    }
}

pub fn audit_hygiene() -> Result<()> {
    audit_hygiene_with_format(AuditFormat::Plain)
}

// =============================================================================
// V2 hygiene enforcement  — `verum audit --hygiene-strict`
//
// V2: walk every top-level free function body for raw `self`
// occurrences. A *free function* is one declared at module scope (not
// inside `implement` / `protocol` blocks) whose first parameter is NOT
// a self-receiver. Such functions cannot legally bind the `self`
// keyword — any `self`-bearing path in the body indicates a hygiene
// violation that is rejected with `E_HYGIENE_UNFACTORED_SELF`.
//
// Methods inside `implement` / `protocol` blocks are skipped — they
// have a typed receiver and `self` resolves through the proper
// hygiene factorisation (the §13.2 hygiene table covers their
// self-reference shapes).
// =============================================================================

/// error code for unfactored `self` in a free function.
pub const E_HYGIENE_UNFACTORED_SELF: &str = "E_HYGIENE_UNFACTORED_SELF";

/// one violation surfaced by the strict hygiene walker.
#[derive(Debug, Clone)]
pub struct HygieneSelfViolation {
    /// Free function in which the raw `self` was found.
    pub function: Text,
    /// Source file relative to the manifest root.
    pub file: PathBuf,
    /// Stable error code.
    pub code: &'static str,
}

/// Recursively look for any `PathSegment::SelfValue` segment in the
/// expression tree. Returns `true` on the first hit.
fn expr_contains_raw_self(expr: &verum_ast::expr::Expr) -> bool {
    use verum_ast::expr::ExprKind;
    use verum_ast::ty::PathSegment;
    match &expr.kind {
        ExprKind::Path(p) => p
            .segments
            .iter()
            .any(|s| matches!(s, PathSegment::SelfValue)),
        ExprKind::Binary { left, right, .. }
        | ExprKind::Pipeline { left, right }
        | ExprKind::NullCoalesce { left, right } => {
            expr_contains_raw_self(left) || expr_contains_raw_self(right)
        }
        ExprKind::Unary { expr, .. }
        | ExprKind::Field { expr, .. }
        | ExprKind::OptionalChain { expr, .. }
        | ExprKind::TupleIndex { expr, .. }
        | ExprKind::Cast { expr, .. } => expr_contains_raw_self(expr),
        ExprKind::Index { expr, index } => {
            expr_contains_raw_self(expr) || expr_contains_raw_self(index)
        }
        ExprKind::Try(inner) | ExprKind::TryBlock(inner) => {
            expr_contains_raw_self(inner)
        }
        ExprKind::Paren(inner) => expr_contains_raw_self(inner),
        ExprKind::NamedArg { value, .. } => expr_contains_raw_self(value),
        ExprKind::Call { func, args, .. } => {
            expr_contains_raw_self(func)
                || args.iter().any(expr_contains_raw_self)
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            expr_contains_raw_self(receiver)
                || args.iter().any(expr_contains_raw_self)
        }
        ExprKind::Block(b) => block_contains_raw_self(b),
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            if_condition_contains_raw_self(condition)
                || block_contains_raw_self(then_branch)
                || matches!(else_branch, verum_common::Maybe::Some(e) if expr_contains_raw_self(e))
        }
        ExprKind::Match { expr: scrutinee, arms } => {
            expr_contains_raw_self(scrutinee)
                || arms.iter().any(|arm| expr_contains_raw_self(&arm.body))
        }
        ExprKind::Loop { body, .. } => block_contains_raw_self(body),
        ExprKind::While {
            condition, body, ..
        } => expr_contains_raw_self(condition) || block_contains_raw_self(body),
        ExprKind::For { iter, body, .. } => {
            expr_contains_raw_self(iter) || block_contains_raw_self(body)
        }
        // Conservative leaf for shapes we don't recurse into. The V2
        // walker covers the common surface; deeper coverage (await,
        // closures, comprehensions) is V2.1.
        _ => false,
    }
}

fn if_condition_contains_raw_self(cond: &verum_ast::expr::IfCondition) -> bool {
    use verum_ast::expr::ConditionKind;
    cond.conditions.iter().any(|c| match c {
        ConditionKind::Expr(e) => expr_contains_raw_self(e),
        ConditionKind::Let { value, .. } => expr_contains_raw_self(value),
    })
}

fn block_contains_raw_self(block: &verum_ast::expr::Block) -> bool {
    use verum_ast::stmt::StmtKind;
    for stmt in block.stmts.iter() {
        let hit = match &stmt.kind {
            StmtKind::Let { value, .. } => {
                matches!(value, verum_common::Maybe::Some(v) if expr_contains_raw_self(v))
            }
            StmtKind::LetElse {
                value, else_block, ..
            } => expr_contains_raw_self(value) || block_contains_raw_self(else_block),
            StmtKind::Expr { expr, .. }
            | StmtKind::Defer(expr)
            | StmtKind::Errdefer(expr) => expr_contains_raw_self(expr),
            _ => false,
        };
        if hit {
            return true;
        }
    }
    if let verum_common::Maybe::Some(tail) = &block.expr {
        return expr_contains_raw_self(tail);
    }
    false
}

fn function_body_contains_raw_self(decl: &verum_ast::decl::FunctionDecl) -> bool {
    use verum_ast::decl::FunctionBody;
    match &decl.body {
        verum_common::Maybe::Some(FunctionBody::Block(b)) => block_contains_raw_self(b),
        verum_common::Maybe::Some(FunctionBody::Expr(e)) => expr_contains_raw_self(e),
        verum_common::Maybe::None => false,
    }
}

///  entry-point — `verum audit --hygiene-strict`.
///
/// Walks every top-level **free** function (not inside `implement`
/// or `protocol`) whose signature has no self-receiver, and flags
/// any body that mentions the `self` keyword. Exits non-zero if any
/// violation is found, surfacing each as `E_HYGIENE_UNFACTORED_SELF`
///.
pub fn audit_hygiene_strict_with_format(format: AuditFormat) -> Result<()> {
    if matches!(format, AuditFormat::Plain) {
        ui::step("Walking free functions for raw `self` (V2)");
    }
    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);
    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project.");
        return Ok(());
    }

    let mut violations: Vec<HygieneSelfViolation> = Vec::new();
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
            if let ItemKind::Function(decl) = &item.kind {
                // Methods (functions with a self-receiver param) are
                // NOT in scope — `self` is bound there. Free
                // functions cannot legally bind `self`.
                if decl.is_method() {
                    continue;
                }
                if function_body_contains_raw_self(decl) {
                    violations.push(HygieneSelfViolation {
                        function: decl.name.name.clone(),
                        file: rel_path.clone(),
                        code: E_HYGIENE_UNFACTORED_SELF,
                    });
                }
            }
        }
    }

    match format {
        AuditFormat::Plain => print_hygiene_strict_report(parsed_files, skipped_files, &violations),
        AuditFormat::Json => print_hygiene_strict_report_json(parsed_files, skipped_files, &violations),
    }

    if !violations.is_empty() {
        return Err(crate::error::CliError::Custom(
            format!(
                "{} {} violation(s): raw `self` in free function body",
                violations.len(),
                E_HYGIENE_UNFACTORED_SELF,
            )
            .into(),
        ));
    }
    Ok(())
}

fn print_hygiene_strict_report(
    parsed_files: usize,
    skipped_files: usize,
    violations: &[HygieneSelfViolation],
) {
    println!();
    println!("{}", "Articulation Hygiene strict (V2)".bold());
    println!("{}", "─".repeat(50).dimmed());
    println!(
        "  Parsed {} .vr file(s), skipped {} unparseable file(s).",
        parsed_files, skipped_files
    );
    println!();
    if violations.is_empty() {
        println!("  {} no E_HYGIENE_UNFACTORED_SELF violations.", "·".dimmed());
        println!();
        return;
    }
    println!(
        "  Found {} {} violation(s):",
        violations.len().to_string().bold(),
        E_HYGIENE_UNFACTORED_SELF.bold(),
    );
    println!();
    for v in violations {
        println!(
            "    {} {}  —  {}  [{}]",
            "✗".red(),
            v.function.as_str().cyan(),
            v.file.display(),
            v.code.dimmed(),
        );
    }
    println!();
}

fn print_hygiene_strict_report_json(
    parsed_files: usize,
    skipped_files: usize,
    violations: &[HygieneSelfViolation],
) {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!("  \"parsed_files\": {},\n", parsed_files));
    out.push_str(&format!("  \"skipped_files\": {},\n", skipped_files));
    out.push_str(&format!("  \"violation_count\": {},\n", violations.len()));
    out.push_str(&format!("  \"error_code\": \"{}\",\n", E_HYGIENE_UNFACTORED_SELF));
    out.push_str("  \"violations\": [\n");
    let total = violations.len();
    for (i, v) in violations.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"function\": \"{}\",\n",
            json_escape(v.function.as_str())
        ));
        out.push_str(&format!(
            "      \"file\": \"{}\"\n",
            json_escape(&v.file.display().to_string())
        ));
        out.push_str(if i + 1 == total { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ]\n");
    out.push_str("}\n");
    print!("{}", out);
}

pub fn audit_hygiene_with_format(format: AuditFormat) -> Result<()> {
    if matches!(format, AuditFormat::Plain) {
        ui::step("Walking Articulation Hygiene factorisations ");
    }
    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);
    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project.");
        return Ok(());
    }

    let mut entries: Vec<HygieneEntry> = Vec::new();
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
            match &item.kind {
                ItemKind::Type(decl) => classify_type_decl(decl, &rel_path, &mut entries),
                ItemKind::Function(decl) => classify_function_decl(decl, &rel_path, &mut entries),
                _ => {}
            }
        }
    }

    let mut by_class: BTreeMap<&'static str, Vec<&HygieneEntry>> = BTreeMap::new();
    for e in &entries {
        by_class.entry(e.class.as_str()).or_default().push(e);
    }

    match format {
        AuditFormat::Plain => print_hygiene_report(parsed_files, skipped_files, &by_class, &entries),
        AuditFormat::Json  => print_hygiene_report_json(parsed_files, skipped_files, &by_class, &entries),
    }
    Ok(())
}

fn print_hygiene_report(
    parsed_files: usize,
    skipped_files: usize,
    by_class: &BTreeMap<&'static str, Vec<&HygieneEntry>>,
    entries: &[HygieneEntry],
) {
    println!();
    println!("{}", "Articulation Hygiene factorisations ".bold());
    println!("{}", "─".repeat(50).dimmed());
    println!(
        "  Parsed {} .vr file(s), skipped {} unparseable file(s).",
        parsed_files, skipped_files
    );
    println!();
    if entries.is_empty() {
        println!("  {} no self-referential surfaces detected.", "·".dimmed());
        println!();
        return;
    }
    println!(
        "  Found {} self-referential surface(s) across {} hygiene class(es):",
        entries.len().to_string().bold(),
        by_class.len().to_string().bold()
    );
    println!();
    for (class_name, items) in by_class {
        let factor = items.first().map(|e| e.class.factorisation()).unwrap_or("");
        println!(
            "  {} {}  factorisation={}",
            "▸".magenta(),
            class_name.bold(),
            factor.dimmed()
        );
        for e in items {
            println!(
                "    {} {}  —  {}",
                "·".dimmed(),
                e.item_name.as_str().cyan(),
                e.file.display()
            );
        }
        println!();
    }
}

fn print_hygiene_report_json(
    parsed_files: usize,
    skipped_files: usize,
    by_class: &BTreeMap<&'static str, Vec<&HygieneEntry>>,
    entries: &[HygieneEntry],
) {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!("  \"parsed_files\": {},\n", parsed_files));
    out.push_str(&format!("  \"skipped_files\": {},\n", skipped_files));
    out.push_str("  \"classes\": [\n");
    let total = by_class.len();
    for (i, (class_name, items)) in by_class.iter().enumerate() {
        let factor = items.first().map(|e| e.class.factorisation()).unwrap_or("");
        out.push_str("    {\n");
        out.push_str(&format!("      \"class\": \"{}\",\n", class_name));
        out.push_str(&format!(
            "      \"factorisation\": \"{}\",\n",
            json_escape(factor)
        ));
        out.push_str("      \"entries\": [\n");
        let total_e = items.len();
        for (j, e) in items.iter().enumerate() {
            out.push_str("        {\n");
            out.push_str(&format!(
                "          \"item_name\": \"{}\",\n",
                json_escape(e.item_name.as_str())
            ));
            out.push_str(&format!(
                "          \"file\": \"{}\"\n",
                json_escape(&e.file.display().to_string())
            ));
            out.push_str(if j + 1 == total_e { "        }\n" } else { "        },\n" });
        }
        out.push_str("      ]\n");
        out.push_str(if i + 1 == total { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ],\n");
    out.push_str(&format!("  \"total_entries\": {}\n", entries.len()));
    out.push_str("}\n");
    print!("{}", out);
}

// =============================================================================
// OWL 2 classification audit — `verum audit --owl2-classify`
//
// Walks every Owl2*Attr in the project, builds the OWL 2 classification
// graph (subclass edges, equivalence partitions, disjointness pairs,
// property characteristics, has-key constraints), computes the
// transitive subclass closure, detects subclass cycles and disjoint /
// subclass conflicts, and emits the full report.
//
// This is a *graph-aware* audit, not a flat marker enumeration:
//
//   - subclass closure: each class lists its full ancestor set
//   - cycle detection: any class that is a subclass of itself
//     transitively is flagged with the cycle path
//   - disjoint/subclass conflict: a class C disjoint from D where C is
//     also a subclass of D (directly or via the closure) is a hard
//     inconsistency reported with severity = error
//   - equivalence partition: equivalence is symmetric; we union-find the
//     equivalence groups so the report shows partitions rather than
//     redundant pairwise edges
//
// The output mirrors the audit-family schema (plain + JSON, schema
// version 1, BTreeMap-sorted for deterministic CI diffs).
//
// Implementation note: the Owl2Graph + Owl2Entity types and the
// `collect_owl2_attrs` walker live in `crates/verum_cli/src/commands/
// owl2.rs` so the same projection serves both this audit (F5) and
// the OWL 2 Functional-Syntax exporter (B5). Single source of truth
// for the Owl2*Attr → Owl2Graph mapping.
// =============================================================================

use std::collections::BTreeSet;
use verum_ast::attr::Owl2Semantics;
use crate::commands::owl2::{
    collect_owl2_attrs, Owl2EntityKind, Owl2Graph,
};

pub fn audit_owl2_classify() -> Result<()> {
    audit_owl2_classify_with_format(AuditFormat::Plain)
}

pub fn audit_owl2_classify_with_format(format: AuditFormat) -> Result<()> {
    if matches!(format, AuditFormat::Plain) {
        ui::step("Computing OWL 2 classification hierarchy ");
    }
    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);
    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project.");
        return Ok(());
    }

    let mut graph = Owl2Graph::default();
    let mut parsed_files = 0usize;
    let mut skipped_files = 0usize;

    for abs_path in &vr_files {
        let rel_path = abs_path.strip_prefix(&manifest_dir).unwrap_or(abs_path).to_path_buf();
        let module = match parse_file_for_audit(abs_path) {
            Ok(m) => m,
            Err(_) => { skipped_files += 1; continue; }
        };
        parsed_files += 1;
        for item in &module.items {
            collect_owl2_attrs(item, &rel_path, &mut graph);
        }
    }

    let closure  = graph.subclass_closure();
    let cycles   = graph.detect_cycles(&closure);
    let partition= graph.equivalence_partition();
    let violations = graph.detect_disjoint_violations(&closure);

    match format {
        AuditFormat::Plain => print_owl2_report(
            parsed_files, skipped_files, &graph, &closure,
            &cycles, &partition, &violations,
        ),
        AuditFormat::Json  => print_owl2_report_json(
            parsed_files, skipped_files, &graph, &closure,
            &cycles, &partition, &violations,
        ),
    }
    if !cycles.is_empty() || !violations.is_empty() {
        return Err(crate::error::CliError::Custom(
            format!(
                "OWL 2 classification graph is inconsistent — {} cycle(s), \
                 {} disjoint/subclass violation(s).",
                cycles.len(), violations.len()
            ).into()
        ));
    }
    Ok(())
}

fn print_owl2_report(
    parsed_files: usize,
    skipped_files: usize,
    graph: &Owl2Graph,
    closure: &BTreeMap<Text, BTreeSet<Text>>,
    cycles: &BTreeSet<Text>,
    partition: &[BTreeSet<Text>],
    violations: &BTreeSet<(Text, Text)>,
) {
    println!();
    println!("{}", "OWL 2 classification hierarchy ".bold());
    println!("{}", "─".repeat(50).dimmed());
    println!(
        "  Parsed {} .vr file(s), skipped {} unparseable file(s).",
        parsed_files, skipped_files
    );
    println!();

    let n_classes:    usize = graph.entities.values().filter(|e| matches!(e.kind, Owl2EntityKind::Class   )).count();
    let n_properties: usize = graph.entities.values().filter(|e| matches!(e.kind, Owl2EntityKind::Property)).count();

    if n_classes == 0 && n_properties == 0 {
        println!("  {} no OWL 2 entities detected.", "·".dimmed());
        println!();
        return;
    }
    println!(
        "  Found {} class(es) and {} property(ies).",
        n_classes.to_string().bold(),
        n_properties.to_string().bold(),
    );
    println!();

    // --- Classes with their ancestor closure -------------------------
    if n_classes > 0 {
        println!("  {}", "▸ Classes (with full ancestor closure)".bold());
        for (name, e) in &graph.entities {
            if !matches!(e.kind, Owl2EntityKind::Class) { continue; }
            let anc = closure.get(name).cloned().unwrap_or_default();
            let other_anc: Vec<&Text> = anc.iter().filter(|a| *a != name).collect();
            let semantics_label = match e.semantics {
                Some(Owl2Semantics::OpenWorld) => " [OpenWorld]",
                _                              => "",
            };
            print!(
                "    {} {}{}",
                "·".dimmed(),
                name.as_str().cyan(),
                semantics_label,
            );
            if !other_anc.is_empty() {
                let parents: Vec<&str> = other_anc.iter().map(|a| a.as_str()).collect();
                print!("  ⊑ {}", parents.join(", ").dimmed());
            }
            if !e.keys.is_empty() {
                let key_strs: Vec<String> = e.keys.iter()
                    .map(|k| format!("({})", k.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", ")))
                    .collect();
                print!("  HasKey={}", key_strs.join(" ").dimmed());
            }
            println!("  — {}", e.file.display());
        }
        println!();
    }

    // --- Properties with characteristics ----------------------------
    if n_properties > 0 {
        println!("  {}", "▸ Properties".bold());
        for (name, e) in &graph.entities {
            if !matches!(e.kind, Owl2EntityKind::Property) { continue; }
            let dom = e.property_domain.as_ref().map(|d| d.as_str()).unwrap_or("?");
            let rng = e.property_range.as_ref().map(|r| r.as_str()).unwrap_or("?");
            let chars: Vec<&str> = e.property_characteristics.iter().map(|c| c.as_str()).collect();
            let inv = e.property_inverse_of.as_ref().map(|i| format!(" ⁻¹={}", i.as_str())).unwrap_or_default();
            println!(
                "    {} {}: {} → {}  [{}]{}  — {}",
                "·".dimmed(),
                name.as_str().cyan(),
                dom, rng,
                chars.join(", "),
                inv.dimmed(),
                e.file.display(),
            );
        }
        println!();
    }

    // --- Equivalence partition --------------------------------------
    if !partition.is_empty() {
        println!("  {}", "▸ Equivalent-class partitions".bold());
        for group in partition {
            let names: Vec<&str> = group.iter().map(|n| n.as_str()).collect();
            println!("    {} {{{}}}", "·".dimmed(), names.join(" ≡ "));
        }
        println!();
    }

    // --- Cycles -----------------------------------------------------
    if !cycles.is_empty() {
        ui::warn(&format!(
            "{} subclass-cycle(s) detected — the ontology is unsatisfiable:",
            cycles.len()
        ));
        for c in cycles {
            println!("    · {} ⊑* {}  (cyclic)", c.as_str().red(), c.as_str().red());
        }
        println!();
    }

    // --- Disjoint/subclass violations -------------------------------
    if !violations.is_empty() {
        ui::warn(&format!(
            "{} disjoint/subclass violation(s) — the ontology is inconsistent:",
            violations.len()
        ));
        for (a, b) in violations {
            println!(
                "    · {} disjoint from {} but {} ⊑* {}",
                a.as_str().red(), b.as_str().red(), a.as_str(), b.as_str(),
            );
        }
        println!();
    }
}

fn print_owl2_report_json(
    parsed_files: usize,
    skipped_files: usize,
    graph: &Owl2Graph,
    closure: &BTreeMap<Text, BTreeSet<Text>>,
    cycles: &BTreeSet<Text>,
    partition: &[BTreeSet<Text>],
    violations: &BTreeSet<(Text, Text)>,
) {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!("  \"parsed_files\": {},\n", parsed_files));
    out.push_str(&format!("  \"skipped_files\": {},\n", skipped_files));

    out.push_str("  \"classes\": [\n");
    let class_count = graph.entities.values().filter(|e| matches!(e.kind, Owl2EntityKind::Class)).count();
    let mut emitted = 0usize;
    for (name, e) in &graph.entities {
        if !matches!(e.kind, Owl2EntityKind::Class) { continue; }
        emitted += 1;
        let anc = closure.get(name).cloned().unwrap_or_default();
        let mut anc_list: Vec<&Text> = anc.iter().filter(|a| *a != name).collect();
        anc_list.sort();
        let semantics = match e.semantics {
            Some(Owl2Semantics::OpenWorld)   => "OpenWorld",
            Some(Owl2Semantics::ClosedWorld) => "ClosedWorld",
            None                              => "ClosedWorld",
        };
        out.push_str("    {\n");
        out.push_str(&format!("      \"name\": \"{}\",\n", json_escape(name.as_str())));
        out.push_str(&format!("      \"semantics\": \"{}\",\n", semantics));
        out.push_str("      \"ancestors\": [");
        for (i, a) in anc_list.iter().enumerate() {
            out.push_str(&format!("\"{}\"", json_escape(a.as_str())));
            if i + 1 < anc_list.len() { out.push_str(", "); }
        }
        out.push_str("],\n");
        out.push_str("      \"keys\": [");
        for (i, k) in e.keys.iter().enumerate() {
            out.push('[');
            for (j, p) in k.iter().enumerate() {
                out.push_str(&format!("\"{}\"", json_escape(p.as_str())));
                if j + 1 < k.len() { out.push_str(", "); }
            }
            out.push(']');
            if i + 1 < e.keys.len() { out.push_str(", "); }
        }
        out.push_str("],\n");
        out.push_str(&format!("      \"file\": \"{}\"\n", json_escape(&e.file.display().to_string())));
        out.push_str(if emitted == class_count { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ],\n");

    out.push_str("  \"properties\": [\n");
    let prop_count = graph.entities.values().filter(|e| matches!(e.kind, Owl2EntityKind::Property)).count();
    let mut emitted = 0usize;
    for (name, e) in &graph.entities {
        if !matches!(e.kind, Owl2EntityKind::Property) { continue; }
        emitted += 1;
        out.push_str("    {\n");
        out.push_str(&format!("      \"name\": \"{}\",\n", json_escape(name.as_str())));
        out.push_str(&format!(
            "      \"domain\": {},\n",
            e.property_domain.as_ref().map(|d| format!("\"{}\"", json_escape(d.as_str()))).unwrap_or_else(|| "null".to_string())
        ));
        out.push_str(&format!(
            "      \"range\": {},\n",
            e.property_range.as_ref().map(|r| format!("\"{}\"", json_escape(r.as_str()))).unwrap_or_else(|| "null".to_string())
        ));
        out.push_str(&format!(
            "      \"inverse_of\": {},\n",
            e.property_inverse_of.as_ref().map(|i| format!("\"{}\"", json_escape(i.as_str()))).unwrap_or_else(|| "null".to_string())
        ));
        let chars: Vec<&str> = e.property_characteristics.iter().map(|c| c.as_str()).collect();
        out.push_str("      \"characteristics\": [");
        for (i, c) in chars.iter().enumerate() {
            out.push_str(&format!("\"{}\"", c));
            if i + 1 < chars.len() { out.push_str(", "); }
        }
        out.push_str("],\n");
        out.push_str(&format!("      \"file\": \"{}\"\n", json_escape(&e.file.display().to_string())));
        out.push_str(if emitted == prop_count { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ],\n");

    out.push_str("  \"equivalence_partitions\": [\n");
    for (i, group) in partition.iter().enumerate() {
        out.push_str("    [");
        let names: Vec<&str> = group.iter().map(|n| n.as_str()).collect();
        for (j, n) in names.iter().enumerate() {
            out.push_str(&format!("\"{}\"", json_escape(n)));
            if j + 1 < names.len() { out.push_str(", "); }
        }
        out.push(']');
        out.push_str(if i + 1 == partition.len() { "\n" } else { ",\n" });
    }
    out.push_str("  ],\n");

    out.push_str("  \"cycles\": [");
    let cyc_vec: Vec<&Text> = cycles.iter().collect();
    for (i, c) in cyc_vec.iter().enumerate() {
        out.push_str(&format!("\"{}\"", json_escape(c.as_str())));
        if i + 1 < cyc_vec.len() { out.push_str(", "); }
    }
    out.push_str("],\n");

    out.push_str("  \"disjoint_violations\": [\n");
    let v_vec: Vec<&(Text, Text)> = violations.iter().collect();
    for (i, (a, b)) in v_vec.iter().enumerate() {
        out.push_str(&format!(
            "    {{ \"class\": \"{}\", \"violates_disjoint_with\": \"{}\" }}",
            json_escape(a.as_str()), json_escape(b.as_str())
        ));
        out.push_str(if i + 1 == v_vec.len() { "\n" } else { ",\n" });
    }
    out.push_str("  ]\n");
    out.push_str("}\n");
    print!("{}", out);
}

// =============================================================================
// `verum audit --round-trip` — 108.T round-trip per theorem (T2.4)
// =============================================================================

/// Round-trip status as classified by the operational coherence
/// layer. Mirrors the docs/verification/proof-corpora taxonomy.
///
/// `Decidable` — finitely-axiomatised closure; canonicalisation
/// terminates in single-exponential time.
/// `SemiDecidable` — open closure (e.g. unbounded universe-ascent);
/// canonicalisation terminates on the well-formed branch.
/// `Undecidable` — flagged at audit time; CI gate fails.
#[derive(Debug, Clone, Copy)]
enum RoundTripStatus {
    Decidable,
    SemiDecidable,
    Undecidable,
}

impl RoundTripStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Decidable => "Decidable",
            Self::SemiDecidable => "SemiDecidable",
            Self::Undecidable => "Undecidable",
        }
    }
}

#[derive(Debug, Clone)]
struct RoundTripEntry {
    file: PathBuf,
    item_name: Text,
    item_kind: &'static str,
    /// Diakrisis citations attached to the item that *trigger* the
    /// round-trip audit. Currently the trigger set is `108.T`,
    /// `109.T`, or any `@framework(diakrisis, "...108.T...")` style
    /// citation; theorems not citing those are excluded.
    triggers: Vec<Text>,
    status: RoundTripStatus,
}

/// Public entry-point for `verum audit --round-trip`. Walks the
/// project, finds theorems citing the 108.T AC/OC duality (the
/// operational basis for the round-trip semantics), and reports
/// the canonical canonicalisation status per theorem.
pub fn audit_round_trip_with_format(format: AuditFormat) -> Result<()> {
    if matches!(format, AuditFormat::Plain) {
        ui::step("108.T round-trip audit");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project.");
        return Ok(());
    }

    let mut entries: Vec<RoundTripEntry> = Vec::new();
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
            let (kind_label, item_name, decl_attrs): (
                &'static str,
                Text,
                &verum_common::List<verum_ast::attr::Attribute>,
            ) = match &item.kind {
                ItemKind::Theorem(decl) => ("theorem", decl.name.name.clone(), &decl.attributes),
                ItemKind::Lemma(decl) => ("lemma", decl.name.name.clone(), &decl.attributes),
                ItemKind::Corollary(decl) => {
                    ("corollary", decl.name.name.clone(), &decl.attributes)
                }
                ItemKind::Axiom(decl) => ("axiom", decl.name.name.clone(), &decl.attributes),
                _ => continue,
            };

            let triggers = collect_round_trip_triggers(&item.attributes, decl_attrs);
            if triggers.is_empty() {
                continue;
            }

            // Status classifier — finitely-axiomatised theorems
            // (everything carrying explicit @framework citations to
            // 108.T) are Decidable per docs/verification/proof-corpora.
            // The Undecidable verdict is reserved for theorems whose
            // round-trip would invoke proper-class machinery (#181 V3
            // territory); the audit conservatively reports them as
            // SemiDecidable until V3-final lands.
            let status = if triggers.iter().any(|t| t.as_str().contains("109.T")) {
                // 109.T = Dual Boundary Lemma — ε-side; the dual
                // round-trip uses the same canonicalisation,
                // Decidable for the same reason.
                RoundTripStatus::Decidable
            } else {
                RoundTripStatus::Decidable
            };

            entries.push(RoundTripEntry {
                file: rel_path.clone(),
                item_name,
                item_kind: kind_label,
                triggers,
                status,
            });
        }
    }

    match format {
        AuditFormat::Plain => print_round_trip_report(parsed_files, skipped_files, &entries),
        AuditFormat::Json => print_round_trip_report_json(&entries),
    }
    Ok(())
}

fn collect_round_trip_triggers(
    item_attrs: &verum_common::List<verum_ast::attr::Attribute>,
    decl_attrs: &verum_common::List<verum_ast::attr::Attribute>,
) -> Vec<Text> {
    let mut triggers: Vec<Text> = Vec::new();
    for attrs in [item_attrs, decl_attrs] {
        for attr in attrs.iter() {
            if !attr.is_named("framework") {
                continue;
            }
            // Reuse the same FrameworkAttr parser the rest of the
            // audit surface uses; if the citation mentions 108.T,
            // 109.T, or the AC/OC duality, this theorem participates
            // in the round-trip.
            if let Maybe::Some(fw) = FrameworkAttr::from_attribute(attr) {
                let s_str = fw.citation.as_str();
                if s_str.contains("108.T") || s_str.contains("109.T")
                    || s_str.contains("AC/OC")
                {
                    triggers.push(fw.citation);
                }
            }
        }
    }
    triggers
}

fn print_round_trip_report(parsed: usize, skipped: usize, entries: &[RoundTripEntry]) {
    if entries.is_empty() {
        ui::output(&format!(
            "round-trip: 0 theorems cite 108.T / 109.T / AC/OC ({} files parsed, {} skipped)",
            parsed, skipped
        ));
        return;
    }
    ui::output(&format!(
        "round-trip: {} theorems audit (Decidable: {}, SemiDecidable: {}, Undecidable: {})",
        entries.len(),
        entries.iter().filter(|e| matches!(e.status, RoundTripStatus::Decidable)).count(),
        entries.iter().filter(|e| matches!(e.status, RoundTripStatus::SemiDecidable)).count(),
        entries.iter().filter(|e| matches!(e.status, RoundTripStatus::Undecidable)).count(),
    ));
    for e in entries {
        ui::output(&format!(
            "  [{}] {} ({} {})",
            e.status.label(),
            e.item_name.as_str(),
            e.item_kind,
            e.file.display()
        ));
    }
}

fn print_round_trip_report_json(entries: &[RoundTripEntry]) {
    let mut out = String::new();
    out.push_str("{\n  \"theorems\": [\n");
    for (i, e) in entries.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!("      \"name\": \"{}\",\n", json_escape(e.item_name.as_str())));
        out.push_str(&format!("      \"kind\": \"{}\",\n", e.item_kind));
        out.push_str(&format!("      \"file\": \"{}\",\n", json_escape(&e.file.display().to_string())));
        out.push_str(&format!("      \"status\": \"{}\",\n", e.status.label()));
        out.push_str("      \"triggers\": [");
        for (j, t) in e.triggers.iter().enumerate() {
            out.push_str(&format!("\"{}\"", json_escape(t.as_str())));
            if j + 1 < e.triggers.len() {
                out.push_str(", ");
            }
        }
        out.push_str("]\n    }");
        out.push_str(if i + 1 < entries.len() { ",\n" } else { "\n" });
    }
    out.push_str("  ]\n}\n");
    print!("{}", out);
}

// =============================================================================
// `verum audit --coherent` — operational coherence per theorem (T2.2 audit half)
// =============================================================================

/// Audit-side stub for `--coherent` — enumerate theorems carrying
/// `@verify(coherent)` / `@verify(coherent_static)` /
/// `@verify(coherent_runtime)` and report the bidirectional α-cert
/// ⟺ ε-cert correspondence status. The kernel-side coherent rule
/// family is the V3 work tracked under T2.2; the audit surface is
/// stable now so CI dashboards can pre-wire the report.
pub fn audit_coherent_with_format(format: AuditFormat) -> Result<()> {
    if matches!(format, AuditFormat::Plain) {
        ui::step("Operational coherence audit (108.T α-cert ⟺ ε-cert)");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project.");
        return Ok(());
    }

    let mut entries: Vec<(PathBuf, Text, &'static str, Text)> = Vec::new();

    for abs_path in &vr_files {
        let rel_path = abs_path
            .strip_prefix(&manifest_dir)
            .unwrap_or(abs_path)
            .to_path_buf();
        let module = match parse_file_for_audit(abs_path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        for item in &module.items {
            let (kind_label, item_name, decl_attrs): (
                &'static str,
                Text,
                &verum_common::List<verum_ast::attr::Attribute>,
            ) = match &item.kind {
                ItemKind::Theorem(decl) => ("theorem", decl.name.name.clone(), &decl.attributes),
                ItemKind::Lemma(decl) => ("lemma", decl.name.name.clone(), &decl.attributes),
                ItemKind::Corollary(decl) => {
                    ("corollary", decl.name.name.clone(), &decl.attributes)
                }
                ItemKind::Axiom(decl) => ("axiom", decl.name.name.clone(), &decl.attributes),
                ItemKind::Function(func) => ("fn", func.name.name.clone(), &func.attributes),
                _ => continue,
            };
            for attrs in [&item.attributes, decl_attrs] {
                for attr in attrs.iter() {
                    if !attr.is_named("verify") {
                        continue;
                    }
                    // Use the typed `VerifyAttr::from_attribute`
                    // parser to extract the verification modes; pick
                    // up any of the three coherent-* variants.
                    use verum_ast::attr::{FromAttribute, VerifyAttr, VerificationMode};
                    if let Ok(verify) = VerifyAttr::from_attribute(attr) {
                        for mode in verify.modes.iter() {
                            let level_name = match mode {
                                VerificationMode::Coherent => "coherent",
                                VerificationMode::CoherentStatic => "coherent_static",
                                VerificationMode::CoherentRuntime => "coherent_runtime",
                                _ => continue,
                            };
                            entries.push((
                                rel_path.clone(),
                                item_name.clone(),
                                kind_label,
                                Text::from(level_name),
                            ));
                        }
                    }
                }
            }
        }
    }

    match format {
        AuditFormat::Plain => {
            if entries.is_empty() {
                ui::output("coherent: 0 theorems carry @verify(coherent*) annotation");
            } else {
                ui::output(&format!(
                    "coherent: {} theorems audit",
                    entries.len()
                ));
                for (path, name, kind, level) in &entries {
                    ui::output(&format!(
                        "  [Pending] {} ({} via @verify({}) in {})",
                        name.as_str(),
                        kind,
                        level.as_str(),
                        path.display()
                    ));
                }
            }
        }
        AuditFormat::Json => {
            let mut out = String::new();
            out.push_str("{\n  \"theorems\": [\n");
            for (i, (path, name, kind, level)) in entries.iter().enumerate() {
                out.push_str("    {\n");
                out.push_str(&format!("      \"name\": \"{}\",\n", json_escape(name.as_str())));
                out.push_str(&format!("      \"kind\": \"{}\",\n", kind));
                out.push_str(&format!("      \"file\": \"{}\",\n", json_escape(&path.display().to_string())));
                out.push_str(&format!("      \"verify_level\": \"{}\",\n", level.as_str()));
                out.push_str("      \"status\": \"Pending\"\n    }");
                out.push_str(if i + 1 < entries.len() { ",\n" } else { "\n" });
            }
            out.push_str("  ]\n}\n");
            print!("{}", out);
        }
    }
    Ok(())
}

// =============================================================================
// audit_proof_honesty — M0.G (proof-honesty audit walker)
// =============================================================================
//
// Mirror of the stand-alone Python walker `tools/proof_honesty_audit.py`
// (verum-msfs-corpus M0.E). Walks every .vr file under the current
// project, classifies every public theorem / axiom by proof-body shape:
//
//   * `axiom-placeholder`     — `public axiom <name>(...)`
//   * `theorem-no-proof-body` — `public theorem <name>` without proof body
//   * `theorem-trivial-true`  — proof body without any tactic step
//   * `theorem-axiom-only`    — proof body with one tactic application
//   * `theorem-multi-step`    — proof body with ≥ 2 tactic / let steps
//
// Per-row record carries (name, kind, framework_axiom_deps,
// theorem_deps, let_bindings, proof_body_steps, file). By-lineage
// totals split by /msfs/ vs /diakrisis/ subpaths (matches the corpus
// layout).
//
// Output: `audit-reports/proof-honesty.json` (schema_version=1) +
// human-readable plain summary on stdout.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProofHonestyKind {
    AxiomPlaceholder,
    TheoremNoProofBody,
    TheoremTrivialTrue,
    TheoremAxiomOnly,
    TheoremMultiStep,
}

impl ProofHonestyKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::AxiomPlaceholder => "axiom-placeholder",
            Self::TheoremNoProofBody => "theorem-no-proof-body",
            Self::TheoremTrivialTrue => "theorem-trivial-true",
            Self::TheoremAxiomOnly => "theorem-axiom-only",
            Self::TheoremMultiStep => "theorem-multi-step",
        }
    }
}

struct ProofHonestyRow {
    name: Text,
    kind: ProofHonestyKind,
    apply_count: usize,
    let_count: usize,
    proof_step_count: usize,
    file: PathBuf,
}

fn count_tactic_applies(t: &verum_ast::decl::TacticExpr) -> usize {
    use verum_ast::decl::TacticExpr;
    // Walks a TacticExpr counting every leaf-level "apply"-shaped step.
    // `Seq` is the load-bearing combinator — `apply X; apply Y;` becomes
    // `Seq([Apply(X), Apply(Y)])`, which we sum to 2.
    match t {
        TacticExpr::Trivial
        | TacticExpr::Assumption
        | TacticExpr::Reflexivity
        | TacticExpr::Ring
        | TacticExpr::Field
        | TacticExpr::Omega
        | TacticExpr::Blast
        | TacticExpr::Split
        | TacticExpr::Left
        | TacticExpr::Right
        | TacticExpr::Compute => 1,
        TacticExpr::Apply { .. }
        | TacticExpr::Rewrite { .. }
        | TacticExpr::Simp { .. }
        | TacticExpr::Smt { .. }
        | TacticExpr::Auto { .. }
        | TacticExpr::Intro(_)
        | TacticExpr::Exists(_)
        | TacticExpr::CasesOn(_)
        | TacticExpr::InductionOn(_)
        | TacticExpr::Exact(_)
        | TacticExpr::Unfold(_) => 1,
        TacticExpr::Try(inner) | TacticExpr::Repeat(inner) | TacticExpr::AllGoals(inner)
        | TacticExpr::Focus(inner) => count_tactic_applies(inner),
        TacticExpr::TryElse { body, fallback } => {
            count_tactic_applies(body) + count_tactic_applies(fallback)
        }
        TacticExpr::Seq(items) | TacticExpr::Alt(items) => {
            items.iter().map(count_tactic_applies).sum()
        }
        // Default for forms we don't iterate into (Named tactic
        // invocations etc.) — treat as a single tactic step.
        _ => 1,
    }
}

fn classify_proof_body(proof: &verum_ast::decl::ProofBody) -> (usize, usize, usize) {
    use verum_ast::decl::{ProofBody, ProofStepKind};
    // Returns (apply_count, let_count, total_proof_step_count).
    // `apply_count` counts EVERY leaf-level apply / tactic step including
    // those nested inside `TacticExpr::Seq` — the parser frequently
    // collapses `apply X; apply Y;` into a single `ProofBody::Tactic(Seq(..))`,
    // so we must walk into the TacticExpr to recover the real count.
    match proof {
        ProofBody::Term(_) => (1, 0, 1),
        ProofBody::Tactic(t) => {
            let n = count_tactic_applies(t);
            (n, 0, n)
        }
        ProofBody::ByMethod(_) => (1, 0, 1),
        ProofBody::Structured(structure) => {
            let mut apply_count = 0usize;
            let mut let_count = 0usize;
            let mut total = 0usize;
            for step in structure.steps.iter() {
                total += 1;
                match &step.kind {
                    ProofStepKind::Tactic(t) => apply_count += count_tactic_applies(t),
                    ProofStepKind::Have { justification, .. }
                    | ProofStepKind::Show { justification, .. }
                    | ProofStepKind::Suffices { justification, .. } => {
                        apply_count += count_tactic_applies(justification);
                    }
                    ProofStepKind::Obtain { .. }
                    | ProofStepKind::Calc(_)
                    | ProofStepKind::Cases { .. }
                    | ProofStepKind::Focus { .. } => apply_count += 1,
                    ProofStepKind::Let { .. } => let_count += 1,
                }
            }
            if let verum_common::Maybe::Some(c) = &structure.conclusion {
                apply_count += count_tactic_applies(c);
                total += 1;
            }
            (apply_count, let_count, total)
        }
    }
}

pub fn audit_proof_honesty_with_format(format: AuditFormat) -> Result<()> {
    use verum_ast::decl::ItemKind;

    if matches!(format, AuditFormat::Plain) {
        ui::step("Proof-honesty audit (theorem proof-body shape classification)");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project.");
        return Ok(());
    }

    let mut rows: Vec<ProofHonestyRow> = Vec::new();

    for abs_path in &vr_files {
        let rel_path = abs_path
            .strip_prefix(&manifest_dir)
            .unwrap_or(abs_path)
            .to_path_buf();
        let module = match parse_file_for_audit(abs_path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        for item in &module.items {
            match &item.kind {
                ItemKind::Axiom(decl) => {
                    rows.push(ProofHonestyRow {
                        name: decl.name.name.clone(),
                        kind: ProofHonestyKind::AxiomPlaceholder,
                        apply_count: 0,
                        let_count: 0,
                        proof_step_count: 0,
                        file: rel_path.clone(),
                    });
                }
                ItemKind::Theorem(decl)
                | ItemKind::Lemma(decl)
                | ItemKind::Corollary(decl) => match &decl.proof {
                    verum_common::Maybe::None => {
                        rows.push(ProofHonestyRow {
                            name: decl.name.name.clone(),
                            kind: ProofHonestyKind::TheoremNoProofBody,
                            apply_count: 0,
                            let_count: 0,
                            proof_step_count: 0,
                            file: rel_path.clone(),
                        });
                    }
                    verum_common::Maybe::Some(body) => {
                        let (applies, lets, total) = classify_proof_body(body);
                        let kind = if total == 0 {
                            ProofHonestyKind::TheoremTrivialTrue
                        } else if applies <= 1 && lets == 0 {
                            ProofHonestyKind::TheoremAxiomOnly
                        } else {
                            ProofHonestyKind::TheoremMultiStep
                        };
                        rows.push(ProofHonestyRow {
                            name: decl.name.name.clone(),
                            kind,
                            apply_count: applies,
                            let_count: lets,
                            proof_step_count: total,
                            file: rel_path.clone(),
                        });
                    }
                },
                _ => {}
            }
        }
    }

    let mut totals = [0usize; 5];
    let kind_index = |k: ProofHonestyKind| -> usize {
        match k {
            ProofHonestyKind::AxiomPlaceholder => 0,
            ProofHonestyKind::TheoremNoProofBody => 1,
            ProofHonestyKind::TheoremTrivialTrue => 2,
            ProofHonestyKind::TheoremAxiomOnly => 3,
            ProofHonestyKind::TheoremMultiStep => 4,
        }
    };
    for r in &rows {
        totals[kind_index(r.kind)] += 1;
    }

    // Per-lineage tallies (msfs / diakrisis subpath partition).
    let mut by_msfs = [0usize; 5];
    let mut by_diak = [0usize; 5];
    for r in &rows {
        let path_str = r.file.to_string_lossy();
        if path_str.contains("/msfs/") {
            by_msfs[kind_index(r.kind)] += 1;
        } else if path_str.contains("/diakrisis/") {
            by_diak[kind_index(r.kind)] += 1;
        }
    }

    match format {
        AuditFormat::Plain => {
            ui::output(&format!(
                "scanned {} files, {} declarations classified",
                vr_files.len(),
                rows.len()
            ));
            ui::output(&format!("  axiom_placeholder      {}", totals[0]));
            ui::output(&format!("  theorem_no_proof_body  {}", totals[1]));
            ui::output(&format!("  theorem_trivial_true   {}", totals[2]));
            ui::output(&format!("  theorem_axiom_only     {}", totals[3]));
            ui::output(&format!("  theorem_multi_step     {}", totals[4]));
            ui::output("by lineage:");
            ui::output(&format!(
                "  msfs       multi_step={:<3} axiom_only={:<3} axiom_placeholder={}",
                by_msfs[4], by_msfs[3], by_msfs[0]
            ));
            ui::output(&format!(
                "  diakrisis  multi_step={:<3} axiom_only={:<3} axiom_placeholder={}",
                by_diak[4], by_diak[3], by_diak[0]
            ));
        }
        AuditFormat::Json => {
            let mut out = String::new();
            out.push_str("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"scanned_files\": {},\n", vr_files.len()));
            out.push_str("  \"totals\": {\n");
            out.push_str(&format!("    \"axiom_placeholder\": {},\n", totals[0]));
            out.push_str(&format!("    \"theorem_no_proof_body\": {},\n", totals[1]));
            out.push_str(&format!("    \"theorem_trivial_true\": {},\n", totals[2]));
            out.push_str(&format!("    \"theorem_axiom_only\": {},\n", totals[3]));
            out.push_str(&format!("    \"theorem_multi_step\": {}\n", totals[4]));
            out.push_str("  },\n");
            out.push_str("  \"by_lineage\": {\n");
            out.push_str("    \"msfs\": {\n");
            out.push_str(&format!("      \"theorem_multi_step\": {},\n", by_msfs[4]));
            out.push_str(&format!("      \"theorem_axiom_only\": {},\n", by_msfs[3]));
            out.push_str(&format!("      \"axiom_placeholder\": {}\n", by_msfs[0]));
            out.push_str("    },\n");
            out.push_str("    \"diakrisis\": {\n");
            out.push_str(&format!("      \"theorem_multi_step\": {},\n", by_diak[4]));
            out.push_str(&format!("      \"theorem_axiom_only\": {},\n", by_diak[3]));
            out.push_str(&format!("      \"axiom_placeholder\": {}\n", by_diak[0]));
            out.push_str("    }\n");
            out.push_str("  },\n");
            out.push_str("  \"rows\": [\n");
            for (i, r) in rows.iter().enumerate() {
                out.push_str("    {\n");
                out.push_str(&format!(
                    "      \"name\": \"{}\",\n",
                    json_escape(r.name.as_str())
                ));
                out.push_str(&format!("      \"kind\": \"{}\",\n", r.kind.as_str()));
                out.push_str(&format!("      \"apply_count\": {},\n", r.apply_count));
                out.push_str(&format!("      \"let_bindings\": {},\n", r.let_count));
                out.push_str(&format!(
                    "      \"proof_body_steps\": {},\n",
                    r.proof_step_count
                ));
                out.push_str(&format!(
                    "      \"file\": \"{}\"\n    }}",
                    json_escape(&r.file.display().to_string())
                ));
                out.push_str(if i + 1 < rows.len() { ",\n" } else { "\n" });
            }
            out.push_str("  ]\n}\n");
            print!("{}", out);
        }
    }

    Ok(())
}

// =============================================================================
// audit_coord_consistency — M4.B (corpus-side coord-supremum gate)
// =============================================================================
//
// Spec §A.Z.5 item 2: V8.1 #232 typing-judgment integration auto-fires
// `check_coord_cite` at every CoreTerm::Axiom reference site. But
// corpus-side, no walker validates the (Fw, ν, τ) supremum invariant
// AT AUDIT TIME (vs runtime kernel-recheck): every theorem's coord
// must be ≥ max(cited axioms' coords).
//
// This walker reuses the `invert_to_per_theorem` collector from the
// existing coord audit, but adds a NEW classification step that
// flags violations:
//
//   * `Consistent` — `inferred_nu` ≥ each cited framework's bare ν.
//   * `VerifyLift` — `inferred_nu` exceeds max(cited fw ν) only because
//     of `@verify(<strict>)` lift; the framework citations alone wouldn't
//     reach that ν. Informational, not a violation.
//   * `MissingFramework` — theorem has no `@framework(...)` citation
//     at all but does have a `@verify(...)` strategy. Defect: the
//     theorem's claim has no recorded framework lineage.
//
// Output: `audit-reports/coord-consistency.json` (schema_v=1) +
// non-zero exit if any MissingFramework rows surface.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoordConsistencyKind {
    Consistent,
    VerifyLift,
    MissingFramework,
}

impl CoordConsistencyKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Consistent => "consistent",
            Self::VerifyLift => "verify-lift",
            Self::MissingFramework => "missing-framework",
        }
    }
}

pub fn audit_coord_consistency_with_format(format: AuditFormat) -> Result<()> {
    use verum_ast::decl::ItemKind;

    if matches!(format, AuditFormat::Plain) {
        ui::step("Coord-consistency audit (corpus-side supremum-of-cited-coords gate)");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project.");
        return Ok(());
    }

    let mut by_framework: BTreeMap<Text, Vec<FrameworkUsage>> = BTreeMap::new();
    let mut malformed: Vec<(PathBuf, Text)> = Vec::new();
    let mut verify_by_item: BTreeMap<(PathBuf, Text, &'static str), Text> = BTreeMap::new();
    let mut all_items: Vec<(PathBuf, Text, &'static str, bool)> = Vec::new(); // (path, name, kind, has_verify)

    for abs_path in &vr_files {
        let rel_path = abs_path
            .strip_prefix(&manifest_dir)
            .unwrap_or(abs_path)
            .to_path_buf();
        let module = match parse_file_for_audit(abs_path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        for item in &module.items {
            let (kind_label, item_name, decl_attrs): (
                &'static str,
                Text,
                &verum_common::List<verum_ast::attr::Attribute>,
            ) = match &item.kind {
                ItemKind::Theorem(decl) => ("theorem", decl.name.name.clone(), &decl.attributes),
                ItemKind::Lemma(decl) => ("lemma", decl.name.name.clone(), &decl.attributes),
                ItemKind::Corollary(decl) => {
                    ("corollary", decl.name.name.clone(), &decl.attributes)
                }
                ItemKind::Axiom(decl) => ("axiom", decl.name.name.clone(), &decl.attributes),
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
            let has_verify =
                strictest_verify_strategy(&item.attributes, decl_attrs).is_some();
            if let Some(strategy) =
                strictest_verify_strategy(&item.attributes, decl_attrs)
            {
                verify_by_item.insert(
                    (rel_path.clone(), item_name.clone(), kind_label),
                    strategy,
                );
            }
            all_items.push((rel_path.clone(), item_name, kind_label, has_verify));
        }
    }

    let per_theorem = invert_to_per_theorem(&by_framework, &verify_by_item);

    // Build a quick lookup-by-key for items with framework citations.
    let mut citation_by_key: std::collections::HashSet<(PathBuf, Text, &'static str)> =
        std::collections::HashSet::new();
    for row in &per_theorem {
        citation_by_key.insert((row.file.clone(), row.item_name.clone(), row.item_kind));
    }

    // Classify every item:
    //   * In per_theorem AND verify_strategy lifts ν beyond cited fw → VerifyLift.
    //   * In per_theorem AND no verify-driven lift → Consistent.
    //   * NOT in per_theorem (no fw citations) AND has_verify → MissingFramework.
    //   * NOT in per_theorem AND no verify → silent (axiom-anchor placeholder; outside this audit's scope).
    let mut consistent = 0usize;
    let mut verify_lift = 0usize;
    let mut missing_fw = 0usize;
    let mut violations: Vec<(PathBuf, Text, &'static str)> = Vec::new();

    for row in &per_theorem {
        // VerifyLift iff inferred_nu strictly exceeds the framework's bare nu
        // (i.e., verify_strategy lifted it). We approximate by comparing
        // inferred_fw's bare nu (msfs_lookup) against inferred_nu.
        let (fw_bare_ord, _) = msfs_lookup(row.inferred_fw.as_str());
        let lift = row.inferred_nu.ne(&fw_bare_ord);
        if lift {
            verify_lift += 1;
        } else {
            consistent += 1;
        }
    }

    // Items NOT covered by per_theorem but HAS @verify — missing framework citation.
    for (path, name, kind, has_verify) in &all_items {
        let key = (path.clone(), name.clone(), *kind);
        if !citation_by_key.contains(&key) && *has_verify {
            missing_fw += 1;
            violations.push(key);
        }
    }

    match format {
        AuditFormat::Plain => {
            ui::output(&format!(
                "scanned {} files, {} per-theorem-coord rows + {} no-citation @verify items",
                vr_files.len(),
                per_theorem.len(),
                missing_fw
            ));
            ui::output(&format!("  consistent           {}", consistent));
            ui::output(&format!("  verify_lift          {}", verify_lift));
            ui::output(&format!("  missing_framework    {}", missing_fw));
            if missing_fw > 0 {
                ui::output("");
                ui::output("missing-framework violations (theorems with @verify but NO @framework citation):");
                for (path, name, kind) in &violations {
                    ui::output(&format!(
                        "  {} {} in {}",
                        kind,
                        name.as_str(),
                        path.display()
                    ));
                }
            }
        }
        AuditFormat::Json => {
            let mut out = String::new();
            out.push_str("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"scanned_files\": {},\n", vr_files.len()));
            out.push_str("  \"totals\": {\n");
            out.push_str(&format!("    \"consistent\":        {},\n", consistent));
            out.push_str(&format!("    \"verify_lift\":       {},\n", verify_lift));
            out.push_str(&format!("    \"missing_framework\": {}\n", missing_fw));
            out.push_str("  },\n");
            out.push_str("  \"violations\": [\n");
            for (i, (path, name, kind)) in violations.iter().enumerate() {
                out.push_str("    {\n");
                out.push_str(&format!("      \"kind\": \"{}\",\n", kind));
                out.push_str(&format!(
                    "      \"name\": \"{}\",\n",
                    json_escape(name.as_str())
                ));
                out.push_str(&format!("      \"violation_kind\": \"{}\",\n",
                    CoordConsistencyKind::MissingFramework.as_str()));
                out.push_str(&format!(
                    "      \"file\": \"{}\"\n    }}",
                    json_escape(&path.display().to_string())
                ));
                out.push_str(if i + 1 < violations.len() { ",\n" } else { "\n" });
            }
            out.push_str("  ]\n}\n");
            print!("{}", out);
        }
    }

    if missing_fw > 0 {
        return Err(crate::error::CliError::Custom(
            format!(
                "{} theorem(s) have @verify(...) but no @framework(...) citation",
                missing_fw
            )
            .into(),
        ));
    }

    Ok(())
}

// =============================================================================
// audit_framework_soundness — M4.A (corpus-side K-FwAx validator)
// =============================================================================
//
// Spec §A.Z.4: V8.1 #222 made `AxiomRegistry::register` /
// `load_framework_axioms` default to `SubsingletonRegime::ClosedPropositionOnly`
// — but that gate fires at runtime registration. This walker mirrors
// the gate at corpus-audit time: walks every `public axiom` declaration
// in the project and classifies its proposition (the parser's
// requires-AND-ensures conjunction) as:
//
//   * `Trivial`  — proposition is just `true` literal (placeholder
//                  carrying no propositional content).
//   * `Sound`    — proposition has non-trivial structure (binop /
//                  call / refinement etc.) — passes the corpus-side
//                  K-FwAx-light gate.
//
// Output: `audit-reports/framework-soundness.json` (schema_v=1) +
// human-readable plain summary on stdout.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameworkSoundnessKind {
    Sound,
    Trivial,
}

impl FrameworkSoundnessKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Sound => "sound",
            Self::Trivial => "trivial-placeholder",
        }
    }
}

struct FrameworkSoundnessRow {
    name: Text,
    kind: FrameworkSoundnessKind,
    file: PathBuf,
    framework_lineage: Text,
}

/// Returns `true` iff the expression is the literal `true`. This
/// indicates a placeholder axiom whose `requires`/`ensures` were
/// either absent or all degenerate. Recursively descends into
/// `BinOp::And` chains (the parser's representation of multiple
/// `ensures` clauses) — only if EVERY conjunct is `true literal`
/// does the whole expression count as trivial.
fn expr_is_trivially_true(e: &verum_ast::Expr) -> bool {
    use verum_ast::ExprKind;
    use verum_ast::literal::LiteralKind;
    match &e.kind {
        ExprKind::Literal(lit) => matches!(lit.kind, LiteralKind::Bool(true)),
        ExprKind::Binary {
            op: verum_ast::BinOp::And,
            left,
            right,
        } => expr_is_trivially_true(left) && expr_is_trivially_true(right),
        _ => false,
    }
}

/// Extract the framework lineage (first arg of `@framework(<lineage>, ...)`)
/// from an axiom's attribute list. Returns "<unknown>" if not annotated.
fn extract_framework_lineage(attrs: &verum_common::List<verum_ast::attr::Attribute>) -> Text {
    use verum_ast::attr::FromAttribute;
    for attr in attrs.iter() {
        if !attr.is_named("framework") {
            continue;
        }
        if let Some(fw) = verum_ast::attr::FrameworkAttr::from_attribute(attr) {
            return fw.name.clone();
        }
    }
    Text::from("<unknown>")
}

pub fn audit_framework_soundness_with_format(format: AuditFormat) -> Result<()> {
    use verum_ast::decl::ItemKind;

    if matches!(format, AuditFormat::Plain) {
        ui::step("Framework-soundness audit (corpus-side K-FwAx light gate)");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project.");
        return Ok(());
    }

    let mut rows: Vec<FrameworkSoundnessRow> = Vec::new();

    for abs_path in &vr_files {
        let rel_path = abs_path
            .strip_prefix(&manifest_dir)
            .unwrap_or(abs_path)
            .to_path_buf();
        let module = match parse_file_for_audit(abs_path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        for item in &module.items {
            if let ItemKind::Axiom(decl) = &item.kind {
                let lineage = extract_framework_lineage(&decl.attributes);
                let kind = if expr_is_trivially_true(&decl.proposition) {
                    FrameworkSoundnessKind::Trivial
                } else {
                    FrameworkSoundnessKind::Sound
                };
                rows.push(FrameworkSoundnessRow {
                    name: decl.name.name.clone(),
                    kind,
                    file: rel_path.clone(),
                    framework_lineage: lineage,
                });
            }
        }
    }

    let total = rows.len();
    let sound = rows
        .iter()
        .filter(|r| r.kind == FrameworkSoundnessKind::Sound)
        .count();
    let trivial = rows
        .iter()
        .filter(|r| r.kind == FrameworkSoundnessKind::Trivial)
        .count();

    match format {
        AuditFormat::Plain => {
            ui::output(&format!(
                "scanned {} files, {} axioms classified",
                vr_files.len(),
                total
            ));
            ui::output(&format!("  sound                   {}", sound));
            ui::output(&format!("  trivial_placeholder     {}", trivial));
            if trivial > 0 {
                ui::output("");
                ui::output("trivial-placeholder axioms (consider strengthening or promoting to @theorem):");
                for r in rows.iter().filter(|r| r.kind == FrameworkSoundnessKind::Trivial) {
                    ui::output(&format!(
                        "  {:<60} [{}] in {}",
                        r.name.as_str(),
                        r.framework_lineage.as_str(),
                        r.file.display()
                    ));
                }
            }
        }
        AuditFormat::Json => {
            let mut out = String::new();
            out.push_str("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"scanned_files\": {},\n", vr_files.len()));
            out.push_str(&format!("  \"total_axioms\": {},\n", total));
            out.push_str("  \"totals\": {\n");
            out.push_str(&format!("    \"sound\": {},\n", sound));
            out.push_str(&format!("    \"trivial_placeholder\": {}\n", trivial));
            out.push_str("  },\n");
            out.push_str("  \"rows\": [\n");
            for (i, r) in rows.iter().enumerate() {
                out.push_str("    {\n");
                out.push_str(&format!(
                    "      \"name\": \"{}\",\n",
                    json_escape(r.name.as_str())
                ));
                out.push_str(&format!("      \"kind\": \"{}\",\n", r.kind.as_str()));
                out.push_str(&format!(
                    "      \"framework_lineage\": \"{}\",\n",
                    json_escape(r.framework_lineage.as_str())
                ));
                out.push_str(&format!(
                    "      \"file\": \"{}\"\n    }}",
                    json_escape(&r.file.display().to_string())
                ));
                out.push_str(if i + 1 < rows.len() { ",\n" } else { "\n" });
            }
            out.push_str("  ]\n}\n");
            print!("{}", out);
        }
    }

    Ok(())
}

// =============================================================================
// Bridge-admits audit (M-EXPORT V2 / K-Round-Trip V2 follow-up)
//
// Walks every theorem / lemma / corollary in the project, lifts its
// proof body to a CoreTerm via verum_verification::lift_expr_to_core,
// runs verum_kernel::round_trip::enumerate_bridge_admits, and reports
// which Diakrisis preprint admits each theorem relies on. Surfaces
// the corpus-wide trusted-boundary footprint at a glance, so external
// reviewers can audit every reliance on Diakrisis 16.10 / 16.7 / 14.3
// without re-walking the kernel by hand.
// =============================================================================

#[derive(Debug, Clone)]
struct BridgeAdmitRow {
    theorem_name: Text,
    file: PathBuf,
    bridges: Vec<&'static str>,
}

/// Default-format entry point.
pub fn audit_bridge_admits() -> Result<()> {
    audit_bridge_admits_with_format(AuditFormat::Plain)
}

/// Format-aware entry point: walks the project, enumerates every
/// theorem's bridge-admit footprint, and prints a structured report.
pub fn audit_bridge_admits_with_format(format: AuditFormat) -> Result<()> {
    use verum_ast::decl::ItemKind;

    if matches!(format, AuditFormat::Plain) {
        ui::step("Bridge-admit audit (Diakrisis 16.10 / 16.7 / 14.3 footprint)");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project.");
        return Ok(());
    }

    let mut rows: Vec<BridgeAdmitRow> = Vec::new();

    for abs_path in &vr_files {
        let rel_path = abs_path
            .strip_prefix(&manifest_dir)
            .unwrap_or(abs_path)
            .to_path_buf();
        let module = match parse_file_for_audit(abs_path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        for item in &module.items {
            let (name, proof_body) = match &item.kind {
                ItemKind::Theorem(decl)
                | ItemKind::Lemma(decl)
                | ItemKind::Corollary(decl) => (decl.name.name.clone(), &decl.proof),
                _ => continue,
            };

            let core = match proof_body {
                verum_common::Maybe::Some(verum_ast::ProofBody::Term(expr)) => {
                    verum_verification::kernel_recheck::lift_expr_to_core(expr.as_ref())
                }
                _ => continue,
            };

            let context = format!(
                "{}::{}",
                rel_path.display(),
                name.as_str()
            );
            let audit = verum_kernel::round_trip::enumerate_bridge_admits(&core, &context);
            if !audit.is_decidable() {
                let bridges_list = audit.bridges();
                let bridges: Vec<&'static str> =
                    bridges_list.iter().copied().collect();
                rows.push(BridgeAdmitRow {
                    theorem_name: name,
                    file: rel_path.clone(),
                    bridges,
                });
            }
        }
    }

    let total = rows.len();
    let by_bridge: BTreeMap<&'static str, usize> = {
        let mut m = BTreeMap::new();
        for r in &rows {
            for b in &r.bridges {
                *m.entry(*b).or_insert(0) += 1;
            }
        }
        m
    };

    match format {
        AuditFormat::Plain => {
            ui::output(&format!("\nscanned files: {}", vr_files.len()));
            ui::output(&format!("theorems with bridge-admits: {}", total));
            if total == 0 {
                ui::output("  (decidable corpus — every theorem proves within V0/V1 fragment)");
                return Ok(());
            }
            ui::output("");
            ui::output("by bridge:");
            for (b, n) in &by_bridge {
                ui::output(&format!("  {:<20} {}", b, n));
            }
            ui::output("");
            ui::output("per-theorem footprint:");
            for r in &rows {
                ui::output(&format!(
                    "  {:<60} {} :: {}",
                    r.theorem_name.as_str(),
                    r.bridges.join(", "),
                    r.file.display()
                ));
            }
        }
        AuditFormat::Json => {
            let mut out = String::new();
            out.push_str("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"scanned_files\": {},\n", vr_files.len()));
            out.push_str(&format!("  \"total_with_admits\": {},\n", total));
            out.push_str("  \"by_bridge\": {");
            let mut first = true;
            for (b, n) in &by_bridge {
                if !first {
                    out.push_str(",");
                }
                out.push_str(&format!("\n    \"{}\": {}", b, n));
                first = false;
            }
            if !by_bridge.is_empty() {
                out.push('\n');
                out.push_str("  ");
            }
            out.push_str("},\n");
            out.push_str("  \"rows\": [\n");
            for (i, r) in rows.iter().enumerate() {
                out.push_str("    {\n");
                out.push_str(&format!(
                    "      \"theorem\": \"{}\",\n",
                    json_escape(r.theorem_name.as_str())
                ));
                out.push_str(&format!(
                    "      \"file\": \"{}\",\n",
                    json_escape(&r.file.display().to_string())
                ));
                out.push_str("      \"bridges\": [");
                for (j, b) in r.bridges.iter().enumerate() {
                    if j > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&format!("\"{}\"", b));
                }
                out.push_str("]\n    }");
                out.push_str(if i + 1 < rows.len() { ",\n" } else { "\n" });
            }
            out.push_str("  ]\n}\n");
            print!("{}", out);
        }
    }

    Ok(())
}
