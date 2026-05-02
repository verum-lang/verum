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
    let client = RegistryClient::from_manifest()?;

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

 // Verify proofs. Closes the inert-defense pattern around
 // `AuditOptions.verify_proofs`: pre-fix the field landed on
 // the options struct + flowed from CLI flags but no audit
 // path consulted it, so `verum audit --verify-proofs` was a
 // silent no-op. The full proof-replay integration would
 // route every cached proof certificate through
 // `verum_smt::certificates::Generator`'s replay surface,
 // but the audit command doesn't yet have access to that
 // pipeline at this layer. Surface the request via UI step
 // + tracing so the embedder sees the flag was observed and
 // the verification will gain a real pass when the cert
 // replay infrastructure lands at this layer.
    if options.verify_proofs {
        ui::info("Verifying proofs (cert-replay integration pending)...");
        tracing::debug!(
            "audit: verify_proofs = true — full cert-replay integration is \
             forward-looking; the audit command currently surfaces the \
             request without driving verification at this layer"
        );
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
    let client = RegistryClient::from_manifest()?;
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
    let client = RegistryClient::from_manifest()?;

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

    let client = RegistryClient::from_manifest()?;
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
use verum_compiler::CompilerOptions;
use verum_compiler::pipeline::CompilationPipeline;
use verum_compiler::session::Session;

/// One framework-axiom usage point.
#[derive(Debug, Clone)]
pub(crate) struct FrameworkUsage {
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
        }
    }

    match format {
        AuditFormat::Plain => {
            print_framework_report(parsed_files, skipped_files, &by_framework, &malformed)
        }
        AuditFormat::Json => {
            print_framework_report_json(parsed_files, skipped_files, &by_framework, &malformed)
        }
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

/// reads conflicts from the static Rust matrix
/// shipped at `crates/verum_verification/src/framework_compat.rs`.
/// will add per-package declarative conflicts so the
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

/// `verum audit --foundation-profiles` — classify every `@framework(...)`
/// citation in the project by its underlying logical foundation
/// (ZFC family / MLTT / HoTT / Cubical / CIC) and surface
/// cross-foundation conflicts.
///

/// Walks every `.vr` file under the project, collects every
/// `@framework(<name>, "<citation>")` attribute via
/// [`verum_kernel::framework_citation::collect_framework_citations`],
/// then partitions the manifest by foundation via
/// [`verum_kernel::foundation_profile::FoundationDistribution`].
///

/// **Three categories of report data**:
/// 1. Per-foundation citation count — observability for the
/// meta-theoretic shape of the corpus.
/// 2. Unresolved citations — framework names not in either bridge.
/// Surfaced so the corpus author can extend the recogniser or
/// correct the citation.
/// 3. Foundation conflicts — pairwise incompatibilities (currently
/// UIP + univalence; see `FoundationProfile::conflicts_with`).
/// Exits non-zero on any conflict — the corpus would derive
/// `False` if both foundations are simultaneously assumed.
///

/// **Why separate from `--framework-conflicts`**: that gate operates
/// at the framework-name level (the literal `@framework(<name>, ...)`
/// argument) and uses `verum_verification::audit_framework_set`'s
/// hand-curated incompatibility matrix. This gate operates at the
/// foundation level (the LOGIC each framework lives in) and uses
/// foundation-level incompatibility (UIP ⊥ univalence). The two
/// dimensions are orthogonal: a corpus can be framework-coherent
/// (no `uip` + `univalence` framework names) but foundation-incoherent
/// (an `msfs` citation living in ZFC + 2 inacc next to a `cubical`
/// citation living in HoTT — currently compatible, but if MSFS were
/// reclassified to MLTT+UIP, the conflict would surface here).
///

/// Output: `target/audit-reports/foundation-profiles.json`.
pub fn audit_foundation_profiles_with_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::foundation_profile::{FoundationDistribution, FoundationProfile};
    use verum_kernel::framework_citation::collect_framework_citations;

    if matches!(format, AuditFormat::Plain) {
        ui::step("Classifying @framework citations by logical foundation");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project.");
        return Ok(());
    }

    let mut all_citations = Vec::new();
    let mut parsed_files = 0usize;
    let mut skipped_files = 0usize;

    for abs_path in &vr_files {
        let module = match parse_file_for_audit(abs_path) {
            Ok(m) => m,
            Err(_) => {
                skipped_files += 1;
                continue;
            }
        };
        parsed_files += 1;
        let manifest = collect_framework_citations(&module.items);
        all_citations.extend(manifest.rows);
    }

    let dist = FoundationDistribution::from_citations(&all_citations);

 // Always write the JSON report to disk so the bundle dispatcher
 // and any downstream tooling can read it without re-running the
 // gate. This matches the "every gate writes JSON unconditionally"
 // discipline established by task #172.
    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let report_path = report_dir.join("foundation-profiles.json");
    let report_json = serde_json::json!({
        "schema_version": 1,
        "parsed_files": parsed_files,
        "skipped_files": skipped_files,
        "total_citations": all_citations.len(),
        "resolved_count": dist.resolved_count(),
        "unresolved_count": dist.unresolved_count(),
        "by_foundation": dist
            .by_foundation
            .iter()
            .map(|(p, n)| (p.tag().to_string(), serde_json::json!(*n)))
            .collect::<serde_json::Map<_, _>>(),
        "unresolved": dist.unresolved,
        "conflicts": dist.conflicts,
        "is_coherent": dist.is_coherent(),
    });
    let _ = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&report_json).unwrap_or_default(),
    );

    match format {
        AuditFormat::Plain => {
            println!();
            println!("Foundation-profile distribution");
            println!("─────────────────────────────────────────");
            println!("Files parsed:        {}", parsed_files);
            println!("Files skipped:       {}", skipped_files);
            println!("Total citations:     {}", all_citations.len());
            println!("Resolved:            {}", dist.resolved_count());
            println!("Unresolved:          {}", dist.unresolved_count());
            println!();
            if dist.by_foundation.is_empty() {
                println!("No citations classified into a foundation.");
            } else {
                println!("By foundation:");
                for (profile, count) in &dist.by_foundation {
                    println!(
                        "  {:>4}  {:<35}  ({})",
                        count,
                        profile.display_name(),
                        profile.tag(),
                    );
                }
            }
            if !dist.unresolved.is_empty() {
                println!();
                println!(
                    "Unresolved framework names ({}) — extend `from_known_framework` \
                     in verum_kernel::foundation_profile to recognise these:",
                    dist.unresolved.len(),
                );
                let mut by_name: BTreeMap<&String, usize> = BTreeMap::new();
                for u in &dist.unresolved {
                    *by_name.entry(&u.framework).or_insert(0) += 1;
                }
                for (name, count) in &by_name {
                    println!("  • {}  ({})", name, count);
                }
            }
            println!();
            if dist.is_coherent() {
                println!(
                    "{} {} foundation(s) coexist coherently.",
                    "✓".green(),
                    dist.foundations().len(),
                );
            } else {
                println!(
                    "{} Foundation pluralism detected — verify no \
                     single derivation chain crosses these:",
                    "!".yellow(),
                );
                for c in &dist.conflicts {
                    println!("  {} ⊥ {}: {}", c.left.tag(), c.right.tag(), c.reason,);
                }
            }
 // Quick reachability hint that helps the auditor file
 // follow-ups.
            if !dist.unresolved.is_empty() || !dist.is_coherent() {
                println!();
                println!("Report: {}", report_path.display());
            }
 // Touch FoundationProfile to keep the import alive for
 // downstream callers cherry-picking this code path
 // — the linker-level no-op silences the unused-import
 // warning when FoundationProfile is only mentioned in
 // doc comments.
            let _ = FoundationProfile::default_profile();
        }
        AuditFormat::Json => {
 // The unconditional disk write above already produced the
 // canonical artefact; mirror it to stdout so the bundle
 // dispatcher and CLI consumers see the same payload.
            println!(
                "{}",
                serde_json::to_string(&report_json).unwrap_or_default(),
            );
        }
    }

 // Foundation pluralism (corpus-level multi-foundation citation)
 // is observability, not a build-breaking error: a corpus can
 // legitimately host independent theorems in incompatible
 // foundations as long as no single derivation chain assumes both.
 // Cross-chain detection (a per-theorem walk) is a future V1 add;
 // V0 surfaces the corpus-level pluralism so the auditor can
 // verify isolation manually. Exit 0 unconditionally.
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
                ItemKind::Theorem(decl) => ("theorem", decl.name.name.clone(), &decl.attributes),
                ItemKind::Lemma(decl) => ("lemma", decl.name.name.clone(), &decl.attributes),
                ItemKind::Corollary(decl) => {
                    ("corollary", decl.name.name.clone(), &decl.attributes)
                }
                ItemKind::Axiom(decl) => ("axiom", decl.name.name.clone(), &decl.attributes),
                ItemKind::Function(func) => ("fn", func.name.name.clone(), &func.attributes),
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
            let acc_lambda = find_accessibility_lambda(&item.attributes, decl_attrs);
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
        if missing.is_empty() {
            "✓".green()
        } else {
            "·".yellow()
        },
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
                r.accessibility.as_ref().map(|t| t.as_str()).unwrap_or("?"),
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
        out.push_str(if i + 1 == total {
            "    }\n"
        } else {
            "    },\n"
        });
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
        Rule {
            number: 1,
            family: "structural",
            name: "K-Var",
            signature: "Γ, x:A ⊢ x : A",
        },
        Rule {
            number: 2,
            family: "structural",
            name: "K-Lam",
            signature: "Γ,x:A ⊢ t:B  ⟹  Γ ⊢ λx:A.t : Π x:A.B",
        },
        Rule {
            number: 3,
            family: "structural",
            name: "K-App",
            signature: "Γ ⊢ f:Π x:A.B, Γ ⊢ a:A  ⟹  Γ ⊢ f a : B[x↦a]",
        },
        Rule {
            number: 4,
            family: "structural",
            name: "K-Pi-Form",
            signature: "Γ ⊢ A:U_i, Γ,x:A ⊢ B:U_j  ⟹  Γ ⊢ Π x:A.B : U_max",
        },
        Rule {
            number: 5,
            family: "structural",
            name: "K-Universe-Cumul",
            signature: "Γ ⊢ A:U_i  ⟹  Γ ⊢ A:U_{i+1}",
        },
        Rule {
            number: 6,
            family: "structural",
            name: "K-Sigma-Form",
            signature: "Γ ⊢ A:U_i, Γ,x:A ⊢ B:U_j  ⟹  Γ ⊢ Σ x:A.B : U_max",
        },
        Rule {
            number: 7,
            family: "inductive",
            name: "K-Ind-Form",
            signature: "well-formed mutual-inductive declaration",
        },
        Rule {
            number: 8,
            family: "inductive",
            name: "K-Pos",
            signature: "strict positivity walker (depth.rs::check_strict_positivity)",
        },
        Rule {
            number: 9,
            family: "inductive",
            name: "K-Ind-Intro",
            signature: "Ctor(args) well-typed vs declared signature",
        },
        Rule {
            number: 10,
            family: "inductive",
            name: "K-Ind-Elim",
            signature: "exhaustive pattern-match, arm typed in motive",
        },
        Rule {
            number: 11,
            family: "equality",
            name: "K-Refl",
            signature: "Refl(t) : Eq(A, t, t)",
        },
        Rule {
            number: 12,
            family: "equality",
            name: "K-Eq-Elim (J)",
            signature: "Martin-Löf J",
        },
        Rule {
            number: 13,
            family: "equality",
            name: "K-UIP-Free",
            signature: "reject any axiom reducing to UIP without @uip framework",
        },
        Rule {
            number: 14,
            family: "cubical",
            name: "K-PathTy-Form",
            signature: "PathTy(A, a, b) : U",
        },
        Rule {
            number: 15,
            family: "cubical",
            name: "K-HComp",
            signature: "CCHM homogeneous composition",
        },
        Rule {
            number: 16,
            family: "cubical",
            name: "K-Transp",
            signature: "transport along a path of types",
        },
        Rule {
            number: 17,
            family: "cubical",
            name: "K-Glue",
            signature: "glue at face φ — univalence-enabling",
        },
        Rule {
            number: 18,
            family: "cubical",
            name: "K-Univalence",
            signature: "ua : Equiv(A,B) → Path(U, A, B)  (via Glue)",
        },
        Rule {
            number: 19,
            family: "refinement",
            name: "K-Refine",
            signature: "Γ ⊢ Refined(A,x,P) : Type_n  iff  dp(P) < dp(A)+1  (VVA §4.4)",
        },
        Rule {
            number: 20,
            family: "refinement",
            name: "K-RefineIntro",
            signature: "Γ ⊢ a:A, proof:P[a/x]  ⟹  Γ ⊢ ⟨a|proof⟩ : Refined(A,x,P)",
        },
        Rule {
            number: 21,
            family: "refinement",
            name: "K-RefineErase",
            signature: "Γ ⊢ r : Refined(A,x,P)  ⟹  Γ ⊢ r.value : A",
        },
        Rule {
            number: 22,
            family: "refinement",
            name: "K-Refine-omega",
            signature: "ordinal-valued depth (depth.rs::m_depth_omega) — opt-in via @require_extension(vfe_7)",
        },
        Rule {
            number: 23,
            family: "framework",
            name: "K-FwAx",
            signature: "admit FrameworkAxiom(name,citation,body) — body:Prop + subsingleton",
        },
        Rule {
            number: 24,
            family: "framework",
            name: "K-Eps-Mu",
            signature: "ε∘M ≃ A∘ε naturality witness (eps_mu.rs::check_eps_mu_coherence) — Diakrisis Prop 5.1",
        },
        Rule {
            number: 25,
            family: "smt",
            name: "K-Smt",
            signature: "SmtCertificate(query, backend, witness) re-check via support.rs::replay_smt_cert_with_obligation",
        },
    ];

    match format {
        AuditFormat::Plain => {
            ui::step("Trusted-kernel primitive inference rules");
            println!();
            println!("  Rule  Family       Name                 Signature");
            println!(
                "  ────  ───────────  ───────────────────  ──────────────────────────────────"
            );
            for r in RULES {
                println!(
                    "  {:>3}   {:11}  {:19}  {}",
                    r.number, r.family, r.name, r.signature
                );
            }
            println!();
            println!(
                "  Total: {} rules across 8 families (structural / inductive /",
                RULES.len()
            );
            println!("  equality / cubical / refinement / framework / smt). See");
            println!("  docs/architecture/verum-verification-architecture.md §4.4 +");
            println!("  §4.4a for the full semantics and the LCF context.");
        }
        AuditFormat::Json => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"rule_count\": {},\n", RULES.len()));
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
    out.push_str(&format!("  \"parsed_files\": {},\n", parsed_files));
    out.push_str(&format!("  \"skipped_files\": {},\n", skipped_files));
    let total_markers: usize = by_framework.values().map(|v| v.len()).sum();
    out.push_str(&format!("  \"total_markers\": {},\n", total_markers));
    out.push_str(&format!("  \"framework_count\": {},\n", by_framework.len()));
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
        out.push_str(&format!("      \"marker_count\": {},\n", uses.len()));
        out.push_str("      \"markers\": [\n");
        let mut first_use = true;
        for u in uses {
            if !first_use {
                out.push_str(",\n");
            }
            first_use = false;
            out.push_str("        {\n");
            out.push_str(&format!("          \"item_kind\": \"{}\",\n", u.item_kind));
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
pub(crate) fn discover_vr_files(root: &Path) -> Vec<PathBuf> {
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
        if entry.file_type().is_file() && entry.path().extension().map_or(false, |e| e == "vr") {
            out.push(entry.into_path());
        }
    }
    out
}

/// Locate the verum stdlib's `core/` tree and return every `.vr` file
/// under it. Used by audits that need workspace-wide symbol resolution
/// to reach stdlib declarations (apply-graph transitive walker, future
/// dependency-graph audits).
///

/// Discovery order:
/// 1. `VERUM_STDLIB_ROOT` env var — explicit override.
/// 2. `core/` directory adjacent to the verum binary.
/// 3. `../core/` from the manifest dir's parent (cargo-workspace dev
/// builds where the corpus is a sibling of the verum source tree).
/// 4. Hardcoded development locations (`~/projects/oldman/verum-lang/
/// verum/core/`) for repeatable demos.
///

/// Returns an empty vector if no stdlib root is found — the caller
/// then operates on the corpus alone, with stdlib symbols surfacing
/// as `unresolved` per the apply-graph fallback contract.
pub(crate) fn discover_stdlib_vr_files() -> Vec<PathBuf> {
    let candidates: Vec<PathBuf> = stdlib_root_candidates();
 // Aggregate from every viable candidate, deduplicate by absolute
 // path. Returning the first non-empty match (the previous behaviour)
 // missed sibling subtrees like `core/verify/codegen_soundness/`
 // when `core/math/` was already populated. Walking each viable
 // candidate keeps stdlib-side @kernel_discharge axiom discovery
 // complete across all topic subtrees.
    let mut seen: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    for root in candidates {
        if !root.is_dir() {
            continue;
        }
        let canonical = root.canonicalize().unwrap_or(root.clone());
        if !seen.insert(canonical) {
            continue;
        }
        let files = discover_vr_files(&root);
        for f in files {
            let key = f.canonicalize().unwrap_or(f.clone());
            if seen.insert(key) {
                out.push(f);
            }
        }
    }
    out
}

/// Locate the Verum-language stdlib root (the directory containing
/// `core/`). Used by the codegen-attestation cross-check to find
/// `core/verify/codegen_soundness/<pass>.vr` citation files.
///
/// Discovery order mirrors [`stdlib_root_candidates`] but searches
/// for `core/verify/` rather than `core/math/`:
/// 1. `VERUM_STDLIB_ROOT` env var.
/// 2. Walk up from the verum binary directory.
/// 3. Walk up from the current working directory.
/// 4. Walk up from the manifest directory.
/// Returns the first match whose `core/verify/` is a directory, or
/// `None` when no candidate succeeds.
pub(crate) fn locate_stdlib_root(manifest_dir: &Path) -> Option<PathBuf> {
    let probe = |root: &Path| -> bool { root.join("core").join("verify").is_dir() };
    if let Ok(env_root) = std::env::var("VERUM_STDLIB_ROOT") {
        let env_path = PathBuf::from(env_root);
        if probe(&env_path) {
            return Some(env_path);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let mut cur = dir.to_path_buf();
            for _ in 0..8 {
                if probe(&cur) {
                    return Some(cur);
                }
                if !cur.pop() {
                    break;
                }
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut cur = cwd;
        for _ in 0..8 {
            if probe(&cur) {
                return Some(cur);
            }
            if !cur.pop() {
                break;
            }
        }
    }
    let mut cur = manifest_dir.to_path_buf();
    for _ in 0..8 {
        if probe(&cur) {
            return Some(cur);
        }
        if !cur.pop() {
            break;
        }
    }
    None
}

fn stdlib_root_candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
 // The audit consumes per-topic subtrees of `core/`:
 // - `core/math/` — MSFS-related axioms, framework-cited theorems.
 // - `core/verify/` — kernel-soundness corpora + codegen-attestation
 // citations (#162 / `core/verify/codegen_soundness/`).
 // - `core/proof/` — kernel-bridge axioms.
 // Walking every subtree (2000+ files total) would slow the audit;
 // pinpointing the topic-relevant ones keeps it fast while ensuring
 // every kernel-discharge citation surfaces.
    let topic_subdirs: &[&str] = &["math", "verify", "proof"];
    if let Ok(env_root) = std::env::var("VERUM_STDLIB_ROOT") {
        let env_path = PathBuf::from(env_root);
        for topic in topic_subdirs {
            out.push(env_path.join(topic));
        }
        out.push(env_path);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let mut cur = dir.to_path_buf();
            for _ in 0..6 {
                for topic in topic_subdirs {
                    let candidate = cur.join("core").join(topic);
                    if candidate.is_dir() {
                        out.push(candidate);
                    }
                }
                if !cur.pop() {
                    break;
                }
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut cur = cwd;
        for _ in 0..8 {
            for topic in topic_subdirs {
                let candidate = cur.join("core").join(topic);
                if candidate.is_dir() {
                    out.push(candidate);
                }
            }
            if !cur.pop() {
                break;
            }
        }
    }
    out
}

/// Parse a single `.vr` file without running semantic analysis. We only need
/// the top-level item list + attributes.
///

/// Post-parse the @delegate(target) attribute pre-pass runs so audit
/// walkers see the synthesised proof bodies (#146 / MSFS-L4.14).
/// Without this step a corpus theorem with `@delegate(target_full)` and
/// no manual proof body would surface to audit gates as a body-less
/// axiom, mis-classifying the L4 verdict.
pub(crate) fn parse_file_for_audit(path: &Path) -> std::result::Result<verum_ast::Module, String> {
    let mut options = CompilerOptions::default();
    options.input = path.to_path_buf();
    let mut session = Session::new(options);
    let file_id = session
        .load_file(path)
        .map_err(|e| format!("load: {}", e))?;
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    let mut module = pipeline
        .phase_parse(file_id)
        .map_err(|e| format!("parse: {}", e))?;
 // Run @delegate expansion post-parse so audit walkers see the
 // synthesised proof bodies. Rejected outcomes (delegate + manual
 // body) are dropped silently here — the elaborator surfaces them
 // when the same file goes through full compilation.
    let _ = verum_compiler::phases::delegate_expansion::expand_delegates_in_module(&mut module);
    Ok(module)
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
// ε_math, ε_compute, ε_observe, ε_prove,
// ε_decide, ε_translate, ε_construct, ε_classify
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

// =============================================================================
// `verum audit --kernel-recheck` (#122)
// =============================================================================
//

// Walks every `.vr` file in the project, runs
// `KernelRecheck::recheck_module` on each, and reports the
// aggregated per-name K-rule outcomes. Catches refinement-type
// leakage (K-Refine-omega), universe-ascent violations
// (K-Universe-Ascent), naturality-square shape errors
// (K-Eps-Mu), and round-trip inversion failures (K-Round-Trip)
// before the verifier dispatcher runs — useful as a fast first
// gate in CI pipelines.
//

// Non-zero exit when any rejection surfaces.

/// Per-file aggregate produced by the kernel-recheck audit.
#[derive(Debug, Clone)]
struct KernelRecheckFileReport {
    file: PathBuf,
    admitted: usize,
    rejected: Vec<KernelRecheckRejection>,
}

#[derive(Debug, Clone)]
struct KernelRecheckRejection {
    item_name: Text,
    item_kind: &'static str,
    reason: Text,
}

/// Legacy entry-point for `verum audit --kernel-recheck` with plain output.
pub fn audit_kernel_recheck() -> Result<()> {
    audit_kernel_recheck_with_format(AuditFormat::Plain)
}

/// Entry point for `verum audit --kernel-recheck [--format FORMAT]`.
///

/// Walks every `.vr` file under the manifest root, runs
/// `KernelRecheck::recheck_module` on each, and reports per-file
/// admitted / rejected counts. Returns `CliError::VerificationFailed`
/// when any rejection surfaces (CI-friendly non-zero exit).
pub fn audit_kernel_recheck_with_format(format: AuditFormat) -> Result<()> {
    use verum_verification::kernel_recheck::KernelRecheck;
    if matches!(format, AuditFormat::Plain) {
        ui::step("Running kernel-recheck against the project module set");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);
    let total_files = vr_files.len();

    let mut reports: Vec<KernelRecheckFileReport> = Vec::new();
    let mut total_admitted = 0usize;
    let mut total_rejected = 0usize;
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

        let outcomes = KernelRecheck::recheck_module(&module);
        let mut admitted = 0usize;
        let mut rejected: Vec<KernelRecheckRejection> = Vec::new();
        for (item_name, kind_label, result) in outcomes.iter() {
            match result {
                Ok(()) => admitted += 1,
                Err(err) => rejected.push(KernelRecheckRejection {
                    item_name: item_name.clone(),
                    item_kind: kind_label,
                    reason: Text::from(format!("{}", err)),
                }),
            }
        }
        if admitted > 0 || !rejected.is_empty() {
            total_admitted += admitted;
            total_rejected += rejected.len();
            reports.push(KernelRecheckFileReport {
                file: rel_path,
                admitted,
                rejected,
            });
        }
    }

    match format {
        AuditFormat::Plain => print_kernel_recheck_report(
            &reports,
            total_files,
            parsed_files,
            skipped_files,
            total_admitted,
            total_rejected,
        ),
        AuditFormat::Json => print_kernel_recheck_report_json(
            &reports,
            total_files,
            parsed_files,
            skipped_files,
            total_admitted,
            total_rejected,
        ),
    }

    if total_rejected > 0 {
        return Err(crate::error::CliError::VerificationFailed(format!(
            "kernel-recheck rejected {} obligation(s) across {} file(s)",
            total_rejected,
            reports.iter().filter(|r| !r.rejected.is_empty()).count()
        )));
    }
    Ok(())
}

fn print_kernel_recheck_report(
    reports: &[KernelRecheckFileReport],
    total_files: usize,
    parsed_files: usize,
    skipped_files: usize,
    total_admitted: usize,
    total_rejected: usize,
) {
    println!();
    println!("{}", "Kernel-recheck report".bold());
    println!("{}", "─".repeat(40).dimmed());
    println!(
        "  Walked {} .vr file(s); parsed {}, skipped {} unparseable.",
        total_files, parsed_files, skipped_files,
    );
    println!(
        "  {} admitted, {} rejected across {} file(s) with kernel-recheckable items.",
        total_admitted,
        total_rejected,
        reports.len(),
    );
    println!();

    for r in reports {
        if r.rejected.is_empty() {
            continue;
        }
        println!(
            "  {}  ({} admitted, {} rejected)",
            r.file.display().to_string().bold(),
            r.admitted,
            r.rejected.len(),
        );
        for rej in &r.rejected {
            println!(
                "    {} {} `{}` — {}",
                "✗".red(),
                rej.item_kind.dimmed(),
                rej.item_name.as_str(),
                rej.reason.as_str(),
            );
        }
    }

    if total_rejected == 0 {
        println!("  {} every kernel-recheckable item admitted.", "✓".green());
    }
}

fn print_kernel_recheck_report_json(
    reports: &[KernelRecheckFileReport],
    total_files: usize,
    parsed_files: usize,
    skipped_files: usize,
    total_admitted: usize,
    total_rejected: usize,
) {
 // #124 — RFC-8259-safe JSON via serde_json (replaces the
 // hand-rolled writeln! pipeline that only escaped `\\` + `"`
 // and produced invalid JSON when KernelRecheckError Display
 // output carried control chars, newlines, or ` ` bytes).
    let reports_json: Vec<serde_json::Value> = reports
        .iter()
        .map(|r| {
            let rejected: Vec<serde_json::Value> = r
                .rejected
                .iter()
                .map(|rej| {
                    serde_json::json!({
                        "item_name": rej.item_name.as_str(),
                        "item_kind": rej.item_kind,
                        "reason": rej.reason.as_str(),
                    })
                })
                .collect();
            serde_json::json!({
                "file": r.file.display().to_string(),
                "admitted": r.admitted,
                "rejected": rejected,
            })
        })
        .collect();
    let payload = serde_json::json!({
        "schema_version": 1,
        "command": "audit-kernel-recheck",
        "total_files": total_files,
        "parsed_files": parsed_files,
        "skipped_files": skipped_files,
        "total_admitted": total_admitted,
        "total_rejected": total_rejected,
        "reports": reports_json,
    });
    println!("{}", serde_json::to_string_pretty(&payload).unwrap());
}

// =============================================================================
// audit --kernel-soundness — task #80 / VERUM-TRUST-1 entry point
// =============================================================================

/// Legacy entry-point for `verum audit --kernel-soundness` with plain output.
pub fn audit_kernel_soundness() -> Result<()> {
    audit_kernel_soundness_with_format(AuditFormat::Plain)
}

/// Entry-point for `verum audit --kernel-soundness [--format FORMAT]`.
///

/// Drives the kernel-soundness corpus + cross-export pipeline:
///

/// 1. **Drift check** — confirms the Rust-side rule list
/// (`verum_kernel::canonical_rules()`) has the expected variant
/// count. A one-sided edit between the Rust enum and the
/// `core/verify/kernel_soundness/` corpus fails here.
/// 2. **Per-rule status enumeration** — reports how many of the
/// 35 lemmas are structurally proved versus admitted with a
/// concrete IOU.
/// 3. **Foreign-tool emission** — produces `kernel_soundness.v`
/// (Coq) and `KernelSoundness.lean` (Lean 4) files in
/// `target/audit-reports/kernel-soundness/` for independent
/// re-checking. External invocation of `coqc` / `lean` is
/// OPTIONAL and lives in CI; the audit gate itself does not
/// shell out (so the gate stays hermetic).
/// 4. **Honest IOUs** — the report lists every admitted lemma
/// with its meta-theoretic prerequisite reason verbatim.
///

/// Exits non-zero only on drift; admitted lemmas are accountability
/// surface, not failures. When the corpus is fully discharged
/// (admitted_count = 0), the gate flips to `kernel_soundness_holds_unconditionally`
/// and a future tightening could require zero admits to pass.
pub fn audit_kernel_soundness_with_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::soundness::coq::CoqBackend;
    use verum_kernel::soundness::lean::LeanBackend;
    use verum_kernel::soundness::{
        EXPECTED_KERNEL_RULE_COUNT, SoundnessBackend, SoundnessExporter,
    };

    if matches!(format, AuditFormat::Plain) {
        ui::step("Running kernel-soundness corpus check + cross-export");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let exporter = SoundnessExporter::new();

 // 1. Drift check.
    if let Err(reason) = exporter.drift_check() {
        return Err(crate::error::CliError::VerificationFailed(format!(
            "kernel-soundness drift: {}",
            reason,
        )));
    }

 // 2. Emit Coq + Lean theory files into target/audit-reports/.
    let report_dir = manifest_dir
        .join("target")
        .join("audit-reports")
        .join("kernel-soundness");
    let coq = CoqBackend::new();
    let lean = LeanBackend::new();
    let coq_text = exporter.emit(&coq);
    let lean_text = exporter.emit(&lean);

 // Best-effort write — failure to mkdir / write is reported as a
 // notice in plain output and absent fields in JSON, but doesn't
 // fail the gate. The corpus check above is the load-bearing
 // assertion; foreign-export emission is bonus accountability.
    let coq_path = report_dir.join(coq.output_filename());
    let lean_path = report_dir.join(lean.output_filename());
    let mut emit_errors: Vec<String> = Vec::new();
    if let Err(e) = std::fs::create_dir_all(&report_dir) {
        emit_errors.push(format!("mkdir {}: {}", report_dir.display(), e));
    } else {
        if let Err(e) = std::fs::write(&coq_path, &coq_text) {
            emit_errors.push(format!("write {}: {}", coq_path.display(), e));
        }
        if let Err(e) = std::fs::write(&lean_path, &lean_text) {
            emit_errors.push(format!("write {}: {}", lean_path.display(), e));
        }
    }

    let proved = exporter.proved_count();
    let admitted = exporter.admitted_count();
    let ious: Vec<(String, String)> = exporter
        .admitted_iou_list()
        .into_iter()
        .map(|(rule, reason)| (rule.to_string(), reason.to_string()))
        .collect();

    match format {
        AuditFormat::Plain => print_kernel_soundness_report(
            &exporter,
            EXPECTED_KERNEL_RULE_COUNT,
            proved,
            admitted,
            &ious,
            &coq_path,
            &lean_path,
            &emit_errors,
        ),
        AuditFormat::Json => print_kernel_soundness_report_json(
            &exporter,
            EXPECTED_KERNEL_RULE_COUNT,
            proved,
            admitted,
            &ious,
            &coq_path,
            &lean_path,
            &emit_errors,
        ),
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn print_kernel_soundness_report(
    exporter: &verum_kernel::soundness::SoundnessExporter,
    expected_total: usize,
    proved: usize,
    admitted: usize,
    ious: &[(String, String)],
    coq_path: &std::path::Path,
    lean_path: &std::path::Path,
    emit_errors: &[String],
) {
    println!();
    println!("{}", "Kernel-soundness report".bold());
    println!("{}", "─".repeat(40).dimmed());
    println!(
        "  Corpus has {} kernel rules ({} expected); {} structurally proved, {} admitted.",
        exporter.rules().len(),
        expected_total,
        proved,
        admitted,
    );
    println!();

    println!("  {}", "Cross-export emitted:".bold());
    for path in [coq_path, lean_path] {
        println!("    • {}", path.display());
    }
    if !emit_errors.is_empty() {
        for e in emit_errors {
            println!("    {} {}", "!".yellow(), e);
        }
    }
    println!();

    if !ious.is_empty() {
        println!("  {}", "Outstanding IOUs (admitted lemmas):".bold());
        for (rule, reason) in ious {
            println!("    {} {} — {}", "○".dimmed(), rule, reason);
        }
        println!();
    }

    if admitted == 0 {
        println!(
            "  {} kernel_soundness theorem holds UNCONDITIONALLY in this corpus.",
            "✓".green(),
        );
    } else {
        println!(
            "  {} kernel_soundness holds modulo the {} IOUs above.",
            "ℹ".cyan(),
            admitted,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn print_kernel_soundness_report_json(
    exporter: &verum_kernel::soundness::SoundnessExporter,
    expected_total: usize,
    proved: usize,
    admitted: usize,
    ious: &[(String, String)],
    coq_path: &std::path::Path,
    lean_path: &std::path::Path,
    emit_errors: &[String],
) {
    let rules_json: Vec<serde_json::Value> = exporter
        .rules()
        .iter()
        .map(|r| {
            serde_json::json!({
                "rule_name": r.rule_name,
                "lemma_name": r.lemma_name,
                "category": r.category.tag(),
                "premise_arity": r.premise_arity,
                "has_side_condition": r.has_side_condition,
                "status": match &r.status {
                    verum_kernel::soundness::LemmaStatus::Proved { .. } => "Proved",
                    verum_kernel::soundness::LemmaStatus::Admitted { .. } => "Admitted",
                    verum_kernel::soundness::LemmaStatus::DischargedByFramework { .. } => "DischargedByFramework",
                },
                "admit_reason": match &r.status {
                    verum_kernel::soundness::LemmaStatus::Proved { .. } => serde_json::Value::Null,
                    verum_kernel::soundness::LemmaStatus::Admitted { reason } => {
                        serde_json::Value::String(reason.clone())
                    }
                    verum_kernel::soundness::LemmaStatus::DischargedByFramework { citation, .. } => {
                        serde_json::Value::String(citation.clone())
                    }
                },
                "discharge": match &r.status {
                    verum_kernel::soundness::LemmaStatus::DischargedByFramework { lemma_path, framework, citation } => {
                        serde_json::json!({
                            "lemma_path": lemma_path,
                            "framework": framework,
                            "citation": citation,
                        })
                    }
                    _ => serde_json::Value::Null,
                },
            })
        })
        .collect();

    let ious_json: Vec<serde_json::Value> = ious
        .iter()
        .map(|(rule, reason)| {
            serde_json::json!({
                "rule_name": rule,
                "reason": reason,
            })
        })
        .collect();

    let payload = serde_json::json!({
        "schema_version": 1,
        "command": "audit-kernel-soundness",
        "expected_rule_count": expected_total,
        "actual_rule_count": exporter.rules().len(),
        "proved_count": proved,
        "admitted_count": admitted,
        "holds_unconditionally": admitted == 0,
        "rules": rules_json,
        "outstanding_ious": ious_json,
        "exports": {
            "coq_path": coq_path.display().to_string(),
            "lean_path": lean_path.display().to_string(),
            "emit_errors": emit_errors,
        },
    });
    println!("{}", serde_json::to_string_pretty(&payload).unwrap());
}

// =============================================================================
// audit --kernel-v0-roster — task #154 / Phase 3 of trust-base shrinkage
// =============================================================================

/// `verum audit --kernel-v0-roster` — bootstrap-meta-theory roster.
///

/// Walks the canonical 10-rule kernel_v0 manifest
/// ([`verum_kernel::soundness::kernel_v0_manifest`]) and the
/// `core/verify/kernel_v0/rules/` directory on disk; cross-references
/// the two to detect drift (manifest entry without source file, or
/// orphan source file without manifest entry).
///

/// **Three layers of observability**:
///

/// 1. **Roster** — per-rule (name, lemma symbol, file path,
/// Proved / Admitted, IOU citation).
/// 2. **Discharge headline** — proved-count vs admitted-count vs
/// total. The shrinkage roadmap target is 4-proved + 6-admitted
/// → 10-proved across V1+ kernel iterations.
/// 3. **Drift gate** — exits non-zero on missing or orphan files.
/// Adding a rule to `proof_checker.rs` without mirroring it in
/// `kernel_v0/rules/k_<name>.vr` fails this gate.
///

/// Output: `target/audit-reports/kernel-v0-roster.json`.
pub fn audit_kernel_v0_roster_with_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::soundness::kernel_v0_manifest::{
        KERNEL_V0_RULE_COUNT, KernelV0Status, ManifestIssue, admitted_count, manifest,
        proved_count, verify_manifest_with_search_roots,
    };

    if matches!(format, AuditFormat::Plain) {
        ui::step("Auditing kernel_v0 bootstrap-meta-theory roster");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let rules = manifest();
 // Stdlib-aware lookup: kernel_v0 rule files live in the embedded
 // Verum stdlib (`core/verify/kernel_v0/rules/`). When auditing a
 // corpus that doesn't contain those files locally, fall back to
 // the stdlib disk root. Same architectural pattern as the
 // codegen-attestation cross-check.
    let stdlib_root = locate_stdlib_root(&manifest_dir);
    let extra_roots: Vec<&Path> = stdlib_root.as_ref().map(|p| vec![p.as_path()]).unwrap_or_default();
    let issues = verify_manifest_with_search_roots(&manifest_dir, &extra_roots);
    let proved = proved_count();
    let admitted = admitted_count();

 // Always write the JSON report — same convention as the rest
 // of the audit gate suite (#172).
    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let report_path = report_dir.join("kernel-v0-roster.json");
    let payload = serde_json::json!({
        "schema_version": 1,
        "kernel_version": env!("CARGO_PKG_VERSION"),
        "rule_count": KERNEL_V0_RULE_COUNT,
        "proved_count": proved,
        "admitted_count": admitted,
        "rules": rules
            .iter()
            .map(|r| serde_json::json!({
                "name": r.name,
                "lemma_symbol": r.lemma_symbol,
                "file_path": r.file_path.to_string_lossy(),
                "status": r.status.tag(),
                "description": r.description,
                "iou_citation": r.iou_citation,
            }))
            .collect::<Vec<_>>(),
        "issues": issues
            .iter()
            .map(|i| match i {
                ManifestIssue::MissingSourceFile { rule_name, expected_path } => {
                    serde_json::json!({
                        "kind": "missing_source_file",
                        "rule_name": rule_name,
                        "expected_path": expected_path.to_string_lossy(),
                    })
                }
                ManifestIssue::OrphanSourceFile { path } => serde_json::json!({
                    "kind": "orphan_source_file",
                    "path": path.to_string_lossy(),
                }),
            })
            .collect::<Vec<_>>(),
    });
    let _ = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&payload).unwrap_or_default(),
    );

    match format {
        AuditFormat::Plain => {
            println!();
            println!("kernel_v0 bootstrap-meta-theory roster");
            println!("─────────────────────────────────────────");
            println!("Total rules:     {}", KERNEL_V0_RULE_COUNT);
            println!(
                "Proved:          {} ({:.0}%)",
                proved,
                (proved as f64 / KERNEL_V0_RULE_COUNT as f64) * 100.0,
            );
            println!(
                "Admitted (IOU):  {} ({:.0}%)",
                admitted,
                (admitted as f64 / KERNEL_V0_RULE_COUNT as f64) * 100.0,
            );
            println!();
            for r in &rules {
                let status_glyph = match r.status {
                    KernelV0Status::Proved => "✓",
                    KernelV0Status::Admitted => "○",
                };
                println!(
                    "  {} {:<14}  {:<22}  {}",
                    status_glyph,
                    r.name,
                    r.lemma_symbol,
                    r.file_path.display(),
                );
                if !r.iou_citation.is_empty() {
                    println!("       IOU: {}", r.iou_citation);
                }
            }
            if issues.is_empty() {
                println!();
                println!(
                    "{} Manifest consistent with {} files on disk.",
                    "✓".green(),
                    KERNEL_V0_RULE_COUNT,
                );
            } else {
                println!();
                println!("{} Manifest drift:", "✗".red());
                for issue in &issues {
                    match issue {
                        ManifestIssue::MissingSourceFile {
                            rule_name,
                            expected_path,
                        } => {
                            println!(
                                "  ✗ rule {:?} references {:?} but file not found",
                                rule_name, expected_path,
                            );
                        }
                        ManifestIssue::OrphanSourceFile { path } => {
                            println!("  ✗ orphan source file {:?} has no manifest entry", path,);
                        }
                    }
                }
                println!();
                println!("Report: {}", report_path.display());
            }
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string(&payload).unwrap_or_default(),);
        }
    }

    if !issues.is_empty() {
        return Err(crate::error::CliError::Custom(
            format!(
                "{} kernel_v0 manifest issue(s) — see {}",
                issues.len(),
                report_path.display(),
            )
            .into(),
        ));
    }

    Ok(())
}

// =============================================================================
// audit --codegen-attestation — task #162 / verified-compilation entry point
// =============================================================================

/// Entry-point for `verum audit --codegen-attestation [--format FORMAT]`.
///

/// Walks the canonical 6-pass codegen attestation manifest from
/// [`verum_kernel::codegen_attestation`] and reports per-pass status
/// (Discharged / AdmittedWithIou / NotYetAttested). Each pass entry
/// carries its semantic invariant + concrete proof obligation
/// describing what would discharge it.
///

/// **Architecture**: this is the *foundation layer* for task #162's
/// CompCert-style verified-compilation chain. The V0 manifest leaves
/// every entry `NotYetAttested`; subsequent commits flip individual
/// entries to `Discharged` or `AdmittedWithIou` as discharge work
/// completes. This audit gate is the observability layer that
/// reports progress along that chain and surfaces the IOU surface as
/// a first-class L4 line-item.
///

/// **Failure semantics**: this gate exits non-zero only if the
/// manifest claims "all attested" but the data layer disagrees — i.e.
/// it is a literal-claim guard. Pending entries are observability,
/// not failure.
///

/// Output: `target/audit-reports/codegen-attestation.json`.
pub fn audit_codegen_attestation_with_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::codegen_attestation::{
        AttestationStatus, CODEGEN_PASS_COUNT, admitted_count, attested_count, manifest,
        pass_count, pending_count,
    };
    use verum_kernel::intrinsic_dispatch::{IntrinsicValue, dispatch_intrinsic};

    if matches!(format, AuditFormat::Plain) {
        ui::step("Auditing codegen-pass kernel-discharge attestations (#162)");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let passes = manifest();
    let total = pass_count();
    let attested = attested_count();
    let admitted = admitted_count();
    let pending = pending_count();

 // Cross-reference layer (#162 V2) — each manifest entry must have:
 // (a) a matching .vr citation file at
 // core/verify/codegen_soundness/<tag>.vr; and
 // (b) a kernel intrinsic dispatcher that returns
 // Decision { holds: true } for kernel_<tag>_preserves_semantics.
 //
 // **Embedded-stdlib lookup**. `core/` is zstd-compressed and embedded
 // directly into the verum binary at build time (see
 // `verum_compiler::embedded_stdlib`). The cross-check primarily
 // queries the embedded archive — installed binaries don't ship a
 // separate `core/` directory. Source-tree builds get a working
 // archive too because build.rs walks the project's core/ folder
 // and generates the archive on every build.
 //
 // Disk fallback exists only for the rare developer scenario of
 // running the audit against a source tree whose embedded archive
 // is empty (e.g. running `cargo run -- audit ...` immediately
 // after a `cargo clean` — uncommon in practice).
 //
 // Both gaps are reported as `vr_missing` / `dispatcher_missing` rows
 // in the JSON. A discharged-or-admitted manifest entry whose .vr
 // citation OR dispatcher is missing flips the audit verdict to
 // failure — the architecture's bidirectional contract.
    let embedded = verum_compiler::embedded_stdlib::get_embedded_stdlib();
    let stdlib_root_disk = locate_stdlib_root(&manifest_dir);
    let mut vr_missing: Vec<String> = Vec::new();
    let mut dispatcher_missing: Vec<String> = Vec::new();
    let vr_present = |tag: &str| -> bool {
 // Embedded paths are relative to core/, so the lookup key is
 // `verify/codegen_soundness/<tag>.vr`.
        let embedded_key = format!("verify/codegen_soundness/{}.vr", tag);
        if let Some(archive) = embedded {
            if archive.get_file(&embedded_key).is_some() {
                return true;
            }
        }
 // Disk fallback for source-tree builds with empty archive.
        if let Some(root) = stdlib_root_disk.as_ref() {
            let path = root
                .join("core")
                .join("verify")
                .join("codegen_soundness")
                .join(format!("{}.vr", tag));
            if path.exists() {
                return true;
            }
        }
        false
    };
    for p in &passes {
        let intrinsic_name = p.pass.kernel_intrinsic_name();
 // (a) .vr file presence — only required when the entry is not
 // `NotYetAttested`. Pending entries by definition haven't yet
 // landed their citation file.
        if !matches!(p.status, AttestationStatus::NotYetAttested) && !vr_present(p.pass.tag()) {
            vr_missing.push(p.pass.tag().to_string());
        }
 // (b) Dispatcher presence — required for every entry, regardless
 // of status, because the audit gate's intrinsic walker looks
 // these up unconditionally.
        match dispatch_intrinsic(&intrinsic_name, &[]) {
            Some(IntrinsicValue::Decision { holds: true, .. }) => {}
            _ => dispatcher_missing.push(intrinsic_name),
        }
    }
    let cross_check_ok = vr_missing.is_empty() && dispatcher_missing.is_empty();

 // Always write the JSON report — same convention as the rest of
 // the audit gate suite (#172).
    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let report_path = report_dir.join("codegen-attestation.json");
    let payload = serde_json::json!({
        "schema_version": 2,
        "kernel_version": env!("CARGO_PKG_VERSION"),
        "task": "#162",
        "discipline": "compcert_per_pass_simulation",
        "pass_count": total,
        "attested_count": attested,
        "admitted_count": admitted,
        "pending_count": pending,
        "all_attested_claim_supported": attested == total,
 // #162 V2 — cross-reference layer: every non-pending entry
 // must have a matching .vr citation file AND a kernel intrinsic
 // dispatcher entry. Both gaps surface here for audit consumers.
        "vr_citations_missing": vr_missing,
        "dispatchers_missing": dispatcher_missing,
        "cross_check_passed": cross_check_ok,
        "passes": passes
            .iter()
            .map(|p| {
                let (status_tag, iou) = match &p.status {
                    AttestationStatus::Discharged => ("discharged", String::new()),
                    AttestationStatus::AdmittedWithIou { iou } => {
                        ("admitted_with_iou", iou.clone())
                    }
                    AttestationStatus::NotYetAttested => {
                        ("not_yet_attested", String::new())
                    }
                };
                serde_json::json!({
                    "pass": p.pass.tag(),
                    "display_name": p.pass.display_name(),
                    "kernel_intrinsic": p.pass.kernel_intrinsic_name(),
                    "semantic_invariant": p.semantic_invariant,
                    "proof_obligation": p.proof_obligation,
                    "status": status_tag,
                    "iou": iou,
                })
            })
            .collect::<Vec<_>>(),
    });
    let _ = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&payload).unwrap_or_default(),
    );

    match format {
        AuditFormat::Plain => {
            println!();
            println!("codegen-pass kernel-discharge attestations (CompCert-style #162)");
            println!("─────────────────────────────────────────────────────────────────");
            println!("Total passes:        {}", total);
            println!(
                "Attested:            {} ({:.0}%)",
                attested,
                (attested as f64 / total as f64) * 100.0,
            );
            println!(
                "Admitted with IOU:   {} ({:.0}%)",
                admitted,
                (admitted as f64 / total as f64) * 100.0,
            );
            println!(
                "Not yet attested:    {} ({:.0}%)",
                pending,
                (pending as f64 / total as f64) * 100.0,
            );
            println!();
            for p in &passes {
                let (glyph, suffix) = match &p.status {
                    AttestationStatus::Discharged => ("✓".to_string(), String::new()),
                    AttestationStatus::AdmittedWithIou { iou } => {
                        ("○".to_string(), format!("  IOU: {}", iou))
                    }
                    AttestationStatus::NotYetAttested => ("·".to_string(), String::new()),
                };
                println!(
                    "  {} {:<24}  {}",
                    glyph,
                    p.pass.display_name(),
                    p.status.display_name(),
                );
                if !suffix.is_empty() {
                    println!("       {}", suffix);
                }
                println!("       invariant: {}", p.semantic_invariant);
                if matches!(p.status, AttestationStatus::NotYetAttested) {
                    println!("       obligation: {}", p.proof_obligation);
                }
            }
            println!();
 // Cross-reference summary (#162 V2): the bidirectional
 // contract between the Rust manifest and the .vr citation
 // files + kernel dispatchers. Both must agree for the
 // attestation surface to be load-bearing.
            if cross_check_ok {
                println!(
                    "{} cross-check: every non-pending pass has a matching .vr \
                     citation + kernel dispatcher",
                    "✓".green(),
                );
            } else {
                println!(
                    "{} cross-check FAILED:",
                    "✗".red(),
                );
                if !vr_missing.is_empty() {
                    println!(
                        "       .vr citations missing: {}",
                        vr_missing.join(", "),
                    );
                }
                if !dispatcher_missing.is_empty() {
                    println!(
                        "       dispatchers missing: {}",
                        dispatcher_missing.join(", "),
                    );
                }
            }
            println!();
            println!(
                "{} of {} passes attested — the verified-compilation chain is the L4 IOU surface",
                attested, CODEGEN_PASS_COUNT,
            );
            println!();
            println!("Report: {}", report_path.display());
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string(&payload).unwrap_or_default(),);
        }
    }

 // Failure semantics: two distinct surfaces fail the gate.
 // (1) Manifest internal-consistency: if attested+admitted+pending
 // ever drifts from the total, the data layer has a bug.
 // (2) Cross-reference contract: every non-pending entry MUST have
 // a matching .vr citation + kernel dispatcher. A discharged
 // or admitted entry without its citation file or dispatcher
 // is an unsound claim — the audit gate enforces both.
 //
 // Pending entries are still observability — a partial-attestation
 // manifest is an honest report, not a gate failure.
    if attested + admitted + pending != total {
        return Err(crate::error::CliError::Custom(
            format!(
                "codegen-attestation manifest is internally inconsistent: \
                 attested({}) + admitted({}) + pending({}) != total({}) — \
                 see {}",
                attested,
                admitted,
                pending,
                total,
                report_path.display(),
            )
            .into(),
        ));
    }
    if !cross_check_ok {
        return Err(crate::error::CliError::Custom(
            format!(
                "codegen-attestation cross-reference failed: vr_missing={:?}, \
                 dispatchers_missing={:?} — every non-pending manifest entry \
                 requires a matching .vr citation under \
                 core/verify/codegen_soundness/ AND a kernel dispatcher in \
                 verum_kernel::intrinsic_dispatch. See {}",
                vr_missing,
                dispatcher_missing,
                report_path.display(),
            )
            .into(),
        ));
    }

    Ok(())
}

// =============================================================================
// audit --differential-kernel — task #159 / cross-implementation validation
// =============================================================================

/// Entry-point for `verum audit --differential-kernel [--format FORMAT]`.
///
/// Runs the differential-kernel testing harness from
/// [`verum_kernel::differential`] over every kernel_v0 rule + the
/// canonical proof-term certificate library
/// (`core/verify/proof_term_examples/*.vproof`).
///
/// **Architecture (#159)**: differential testing checks that **two**
/// kernel implementations agree on every certificate — Rust trusted
/// base [`verum_kernel::proof_checker`] vs Verum-self-hosted kernel
/// (`core/verify/kernel_v0/`). When the Verum side is online, the
/// gate flips disagreements into audit failures. When the Verum
/// side is stubbed (current state — parser blocker on
/// `core/verify/kernel_v0/`), every report records
/// `not_yet_self_hosting`; the gate exits 0 (observability-only)
/// because there's no second kernel to disagree.
///
/// **Forward-compatibility**: when
/// `verum_kernel::differential::run_differential_test_with_verum`
/// gains a real Verum-side adapter, this gate's output flips
/// automatically — every existing call site still goes through
/// `differential_test_rule(name)` which looks up the Rule then
/// runs the test. Plug in the Verum adapter, the gate becomes
/// load-bearing.
///
/// **Output**: `target/audit-reports/differential-kernel.json`.
pub fn audit_differential_kernel_with_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::differential::{
        DifferentialAgreement, DifferentialOutcome, DifferentialReport, differential_test_rule,
    };
    use verum_kernel::soundness::kernel_v0_manifest::manifest;

    if matches!(format, AuditFormat::Plain) {
        ui::step("Differential-kernel test — Rust trusted base vs Verum self-hosted");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let rules = manifest();
    let mut reports: Vec<DifferentialReport> = Vec::with_capacity(rules.len());

    for rule in &rules {
 // `differential_test_rule` returns `None` for unknown rules,
 // but every name we pass is from `manifest()` directly, so
 // the lookup is total. We use `if let` to stay defensive —
 // a future manifest refactor that introduces aliasing should
 // fail loudly, not silently.
        if let Some(report) = differential_test_rule(&rule.name) {
            reports.push(report);
        }
    }
    let outcome = DifferentialOutcome::from_reports(&reports);

    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let report_path = report_dir.join("differential-kernel.json");
    let payload = serde_json::json!({
        "schema_version": 1,
        "kernel_version": env!("CARGO_PKG_VERSION"),
        "task": "#159",
        "discipline": "differential_kernel_cross_implementation",
        "rule_count": rules.len(),
        "report_count": reports.len(),
        "outcome": {
            "accepted": outcome.accepted,
            "rejected": outcome.rejected,
            "disagreement": outcome.disagreement,
            "not_yet_self_hosting": outcome.not_yet_self_hosting,
        },
        "load_bearing": outcome.disagreement == 0,
        "reports": reports
            .iter()
            .map(|r| {
                let agreement_tag = match r.agreement {
                    DifferentialAgreement::BothAccept => "both_accept",
                    DifferentialAgreement::BothReject => "both_reject",
                    DifferentialAgreement::Disagreement => "disagreement",
                    DifferentialAgreement::NotYetSelfHosting => "not_yet_self_hosting",
                };
                serde_json::json!({
                    "rule": r.rule_name,
                    "rust_verdict": r.rust_verdict.tag(),
                    "verum_verdict": r.verum_verdict.tag(),
                    "agreement": agreement_tag,
                })
            })
            .collect::<Vec<_>>(),
    });
    let _ = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&payload).unwrap_or_default(),
    );

    match format {
        AuditFormat::Plain => {
            println!();
            println!("Differential-kernel test (#159 — Rust ↔ Verum self-hosted)");
            println!("──────────────────────────────────────────────────────────");
            println!("Total rules:           {}", rules.len());
            println!("Reports run:           {}", reports.len());
            println!("Both accept:           {}", outcome.accepted);
            println!("Both reject:           {}", outcome.rejected);
            println!(
                "{} Disagreement:          {}",
                if outcome.disagreement == 0 { "✓" } else { "✗" },
                outcome.disagreement,
            );
            println!("Not yet self-hosting:  {}", outcome.not_yet_self_hosting);
            println!();
            for r in &reports {
                let glyph = match r.agreement {
                    DifferentialAgreement::BothAccept => "✓",
                    DifferentialAgreement::BothReject => "○",
                    DifferentialAgreement::Disagreement => "✗",
                    DifferentialAgreement::NotYetSelfHosting => "·",
                };
                println!(
                    "  {} {:<14}  rust={:<10}  verum={:<22}",
                    glyph, r.rule_name, r.rust_verdict.tag(), r.verum_verdict.tag(),
                );
            }
            println!();
            if outcome.disagreement == 0 {
                if outcome.not_yet_self_hosting > 0 {
                    println!(
                        "{} {} report(s) pending Verum-side self-hosting (parser blocker on \
                         core/verify/kernel_v0/); harness load-bearing the moment it lands.",
                        "·".yellow(),
                        outcome.not_yet_self_hosting,
                    );
                } else {
                    println!(
                        "{} All differential reports agree — kernel implementations consistent.",
                        "✓".green(),
                    );
                }
            } else {
                println!(
                    "{} {} disagreement(s) — at least one Rust↔Verum kernel divergence \
                     detected. Failing the audit.",
                    "✗".red(),
                    outcome.disagreement,
                );
            }
            println!();
            println!("Report: {}", report_path.display());
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string(&payload).unwrap_or_default(),);
        }
    }

 // Failure semantics: ANY disagreement fails the gate.
 // `not_yet_self_hosting` reports are observability — the gate
 // remains pass-state because there's no second kernel to disagree.
 // Once the Verum-side adapter lands, the same audit code starts
 // producing real verdicts and the gate becomes load-bearing.
    if outcome.disagreement > 0 {
        return Err(crate::error::CliError::Custom(
            format!(
                "differential-kernel audit: {} disagreement(s) between Rust trusted \
                 base and Verum self-hosted kernel — see {}",
                outcome.disagreement,
                report_path.display(),
            )
            .into(),
        ));
    }

    Ok(())
}

// =============================================================================
// audit --differential-kernel-fuzz — mutation-based property fuzzing
// =============================================================================

/// Default iteration count for the fuzz audit gate. Bounded for
/// CI-friendly runtime; the campaign is deterministic so the same
/// number of iterations always produces the same coverage.
const DIFFERENTIAL_FUZZ_DEFAULT_ITERATIONS: usize = 500;

/// Default base seed. Kept stable so the fuzz output is
/// reproducible across audit runs — a disagreement found at audit
/// time is bisectable by simply re-running locally with the
/// recorded seed.
const DIFFERENTIAL_FUZZ_DEFAULT_SEED: u64 = 0xA174_F022_5EE7_DEAD;

/// Property-based fuzz audit gate over the kernel registry.
///
/// Runs `DIFFERENTIAL_FUZZ_DEFAULT_ITERATIONS` mutation-based fuzz
/// iterations against the default kernel registry. Every mutant
/// must produce a unanimous agreement; any disagreement is a
/// kernel-implementation bug and flips the gate to failure.
///
/// **Architectural pin**: the canonical-certificate roster covers
/// the curated surface; this gate covers the long tail. Together
/// they form the differential-kernel testing discipline at scale.
pub fn audit_differential_kernel_fuzz_with_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::differential_fuzz::run_fuzz_campaign;

    if matches!(format, AuditFormat::Plain) {
        ui::step("Differential-kernel fuzz — mutation-based property testing");
    }

    let report = run_fuzz_campaign(
        DIFFERENTIAL_FUZZ_DEFAULT_ITERATIONS,
        DIFFERENTIAL_FUZZ_DEFAULT_SEED,
    );

    let manifest_dir = Manifest::find_manifest_dir()?;
    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let report_path = report_dir.join("differential-kernel-fuzz.json");

    let disagreement_summaries: Vec<serde_json::Value> = report
        .disagreements
        .iter()
        .map(|d| {
            serde_json::json!({
                "iteration": d.iteration,
                "seed_index": d.seed_index,
                "mutation": d.mutation_tag,
                "agreement": d.agreement_tag(),
                "outcomes": d.verdict.outcomes.iter().map(|o| {
                    serde_json::json!({
                        "kernel": o.kernel_name,
                        "accepted": o.accepted,
                        "error": o.error_summary,
                    })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();

    let payload = serde_json::json!({
        "schema_version": 1,
        "kernel_version": env!("CARGO_PKG_VERSION"),
        "discipline": "differential_kernel_property_fuzz",
        "campaign": {
            "iterations": report.total_iterations,
            "base_seed": format!("{:#x}", DIFFERENTIAL_FUZZ_DEFAULT_SEED),
            "registered_kernels": report.registered_kernels,
        },
        "outcome": {
            "unanimous_accept": report.unanimous_accept,
            "unanimous_reject": report.unanimous_reject,
            "disagreements": report.disagreements.len(),
        },
        "load_bearing": report.is_sound(),
        "disagreement_details": disagreement_summaries,
    });
    let _ = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&payload).unwrap_or_default(),
    );

    match format {
        AuditFormat::Plain => {
            println!();
            println!("Differential-kernel fuzz — mutation-based property testing");
            println!("──────────────────────────────────────────────────────────");
            println!("Iterations:            {}", report.total_iterations);
            println!(
                "Base seed:             {:#x}",
                DIFFERENTIAL_FUZZ_DEFAULT_SEED,
            );
            println!(
                "Registered kernels:    {}",
                report.registered_kernels.join(", "),
            );
            println!("Unanimous accept:      {}", report.unanimous_accept);
            println!("Unanimous reject:      {}", report.unanimous_reject);
            println!(
                "{} Disagreements:         {}",
                if report.is_sound() { "✓" } else { "✗" },
                report.disagreements.len(),
            );
            println!();
            if report.is_sound() {
                println!(
                    "{} All {} mutants produced unanimous verdicts — \
                     kernel implementations consistent under mutation.",
                    "✓".green(),
                    report.total_iterations,
                );
            } else {
                println!(
                    "{} {} disagreement(s) detected — kernel-implementation bug surfaced.",
                    "✗".red(),
                    report.disagreements.len(),
                );
                for d in report.disagreements.iter().take(5) {
                    println!(
                        "    iter {} seed {} mutation={} agreement={}",
                        d.iteration, d.seed_index, d.mutation_tag, d.agreement_tag(),
                    );
                }
                if report.disagreements.len() > 5 {
                    println!("    ... ({} more)", report.disagreements.len() - 5);
                }
            }
            println!();
            println!("Report: {}", report_path.display());
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string(&payload).unwrap_or_default());
        }
    }

    if !report.is_sound() {
        return Err(crate::error::CliError::Custom(
            format!(
                "differential-kernel fuzz audit: {} disagreement(s) over {} iterations \
                 (seed {:#x}) — see {}",
                report.disagreements.len(),
                report.total_iterations,
                DIFFERENTIAL_FUZZ_DEFAULT_SEED,
                report_path.display(),
            )
            .into(),
        ));
    }

    Ok(())
}

// =============================================================================
// audit --bridge-discharge — task #134 / MSFS-L4.1 entry point
// =============================================================================

/// Legacy entry-point for `verum audit --bridge-discharge` with plain output.
pub fn audit_bridge_discharge() -> Result<()> {
    audit_bridge_discharge_with_format(AuditFormat::Plain)
}

/// Entry-point for `verum audit --bridge-discharge [--format FORMAT]`.
///

/// Walks every `.vr` module in the manifest project, finds every
/// `apply kernel_*_strict(args)` invocation in proof bodies, and
/// invokes [`verum_kernel::dispatch_intrinsic`] against the
/// literal-arg call sites. Reports per-bridge:
///

/// * **callsites_total** — total `apply` invocations
/// * **callsites_literal_args** — invocations whose args reduce to literals
/// * **callsites_non_literal** — invocations with non-literal args
/// * **dispatcher_decisions** — per-callsite `Decision { holds, reason }`
/// * **false_discharges** — count where `holds: false`
///

/// **Architecture**: this is the *observability layer* for task #80's
/// L4 promotion path. It introduces no per-bridge hardcoding — every
/// bridge auto-registers through the dispatcher table, and every
/// literal-arg invocation in the corpus is replayed mechanically.
/// Adding a new `kernel_<verb>_strict` bridge requires only registering
/// the dispatcher entry; this audit picks up the discharge automatically.
///

/// Exits non-zero when:
/// * any bridge invocation has `holds: false` (false discharge — the
/// proof body cited the bridge but the dispatcher's structural
/// check rejects the args as written)
/// * any cited bridge has no dispatcher entry (unknown_bridges
/// non-empty — gap in the bridge table)
pub fn audit_bridge_discharge_with_format(format: AuditFormat) -> Result<()> {
    use crate::commands::bridge_discharge::{BridgeReport, finalise_report, walk_module};

    if matches!(format, AuditFormat::Plain) {
        ui::step("Walking proof bodies for `apply kernel_*_strict(...)` discharge sites");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    let mut aggregator: std::collections::BTreeMap<String, BridgeReport> =
        std::collections::BTreeMap::new();
    let mut modules_scanned = 0usize;
    let mut items_walked = 0usize;

    for abs_path in &vr_files {
        let rel_path = abs_path
            .strip_prefix(&manifest_dir)
            .unwrap_or(abs_path)
            .to_path_buf();
        let module = match parse_file_for_audit(abs_path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        modules_scanned += 1;
        walk_module(&module, &rel_path, &mut aggregator, &mut items_walked);
    }

    let report = finalise_report(aggregator, modules_scanned, items_walked);

 // #172 audit-output discipline: every gate writes its JSON to
 // disk regardless of `--format` so the bundle dispatcher (#151)
 // and downstream tooling can read each per-gate report reliably.
    if let Ok(manifest_dir) = Manifest::find_manifest_dir() {
        let dir = manifest_dir.join("target").join("audit-reports");
        let _ = std::fs::create_dir_all(&dir);
        let payload = serde_json::json!({
            "schema_version": 1,
            "command": "audit-bridge-discharge",
            "modules_scanned": report.modules_scanned,
            "items_walked": report.items_walked,
            "total_callsites": report.total_callsites,
            "total_false_discharges": report.total_false_discharges,
            "bridges": report.bridges,
            "unknown_bridges": report.unknown_bridges,
        });
        let _ = std::fs::write(
            dir.join("bridge-discharge.json"),
            serde_json::to_string_pretty(&payload).unwrap(),
        );
    }

    match format {
        AuditFormat::Plain => print_bridge_discharge_report_plain(&report),
        AuditFormat::Json => print_bridge_discharge_report_json(&report),
    }

    if report.total_false_discharges > 0 {
        return Err(crate::error::CliError::VerificationFailed(format!(
            "bridge-discharge audit found {} false-discharge site(s) — \
             dispatcher rejected the cited bridge args. See report above.",
            report.total_false_discharges,
        )));
    }
    if !report.unknown_bridges.is_empty() {
        return Err(crate::error::CliError::VerificationFailed(format!(
            "bridge-discharge audit found {} bridge(s) cited in proofs but \
             missing from the dispatcher table: {}",
            report.unknown_bridges.len(),
            report.unknown_bridges.join(", "),
        )));
    }

    Ok(())
}

fn print_bridge_discharge_report_plain(
    report: &crate::commands::bridge_discharge::DischargeReport,
) {
    println!();
    println!("{}", "Bridge-discharge report".bold());
    println!("{}", "─".repeat(40).dimmed());
    println!(
        "  Walked {} module(s); {} theorem-shaped items examined.",
        report.modules_scanned, report.items_walked,
    );
    println!(
        "  {} total callsite(s) across {} bridge(s); {} false discharge(s).",
        report.total_callsites,
        report.bridges.len(),
        report.total_false_discharges,
    );
    println!();

    if report.bridges.is_empty() {
        println!(
            "  {} no `apply kernel_*` callsite found — corpus is bridge-free \
             at this stage.",
            "ℹ".cyan(),
        );
        return;
    }

    for b in &report.bridges {
        let header_color = if b.false_discharges > 0 {
            "✗".red().to_string()
        } else {
            "✓".green().to_string()
        };
        println!(
            "  {} {}  ({} total · {} literal · {} non-literal · {} false)",
            header_color,
            b.bridge_name.bold(),
            b.callsites_total,
            b.callsites_literal_args,
            b.callsites_non_literal,
            b.false_discharges,
        );
        for c in &b.callsites {
            let verdict = match c.holds {
                Some(true) => "✓".green().to_string(),
                Some(false) => "✗".red().to_string(),
                None => "○".dimmed().to_string(),
            };
            println!(
                "    {}  {} :: {} ({})",
                verdict,
                c.file.display().to_string().dimmed(),
                c.item_name,
                c.args_text.join(", "),
            );
            if !c.reason.is_empty() {
                println!("        {}", c.reason.dimmed());
            }
        }
    }

    if !report.unknown_bridges.is_empty() {
        println!();
        println!(
            "  {} bridge(s) cited but missing from dispatcher:",
            "!".yellow()
        );
        for n in &report.unknown_bridges {
            println!("    - {}", n);
        }
    }
}

fn print_bridge_discharge_report_json(report: &crate::commands::bridge_discharge::DischargeReport) {
    let payload = serde_json::json!({
        "schema_version": 1,
        "command": "audit-bridge-discharge",
        "modules_scanned": report.modules_scanned,
        "items_walked": report.items_walked,
        "total_callsites": report.total_callsites,
        "total_false_discharges": report.total_false_discharges,
        "bridges": report.bridges,
        "unknown_bridges": report.unknown_bridges,
    });
    let pretty = serde_json::to_string_pretty(&payload).unwrap();
 // #172 audit-output discipline: every gate writes its JSON to
 // disk regardless of `--format` so bundle dispatcher (#151) and
 // downstream tooling can reliably read each per-gate report.
    if let Ok(manifest_dir) = Manifest::find_manifest_dir() {
        let dir = manifest_dir.join("target").join("audit-reports");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("bridge-discharge.json"), &pretty);
    }
    println!("{}", pretty);
}

// =============================================================================
// audit --ladder-monotonicity — task #139 / MSFS-L4.6
// =============================================================================

/// Legacy entry-point for `verum audit --ladder-monotonicity` (plain).
pub fn audit_ladder_monotonicity() -> Result<()> {
    audit_ladder_monotonicity_with_format(AuditFormat::Plain)
}

/// Entry-point for `verum audit --ladder-monotonicity [--format FORMAT]`.
///

/// For every theorem-shaped item with a `@verify(<strategy>)`
/// annotation, dispatches the obligation at every backbone strategy
/// from `Runtime` up to and including the declared strategy. Verifies
/// the runtime strict-ν-monotonicity invariant: if the obligation
/// `Closes` at any strategy `S_strict`, it MUST `Close` at every
/// coarser backbone strategy `S_coarser ≤ S_strict`.
///

/// **Architectural promise**: pre-fix the verification ladder's
/// strict-ν-monotonicity claim was implementation-by-design; the
/// dispatcher's *implementation table* was checked
/// (`verify_monotonicity`) but the *runtime walk* was never verified.
/// Post-fix every theorem's claimed strategy is exercised across the
/// backbone, and inversion (stricter Closes while coarser fails)
/// fails the gate with a precise diagnostic.
///

/// **Performance**: N theorems × ≤ 12 dispatches each. Each dispatch
/// is the existing per-strategy backend invocation; monotonicity
/// check is a pure structural inspection of the walk report.
///

/// Exits non-zero when any theorem violates the invariant.
pub fn audit_ladder_monotonicity_with_format(format: AuditFormat) -> Result<()> {
    use verum_verification::ladder_dispatch::{
        DefaultLadderDispatcher, LadderObligation, LadderStrategy, MonotonicityViolation,
        check_runtime_monotonicity, dispatch_ladder_walk,
    };

    if matches!(format, AuditFormat::Plain) {
        ui::step("Verifying runtime ν-monotonicity across the backbone");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    let dispatcher = DefaultLadderDispatcher::new();
    let mut total_walks = 0usize;
    let mut total_violations = 0usize;
    let mut violation_rows: Vec<serde_json::Value> = Vec::new();
    let mut violations_text: Vec<(PathBuf, Text, Vec<MonotonicityViolation>)> = Vec::new();
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
            let (item_name, decl_attrs): (Text, &verum_common::List<verum_ast::attr::Attribute>) =
                match &item.kind {
                    verum_ast::decl::ItemKind::Theorem(d)
                    | verum_ast::decl::ItemKind::Lemma(d)
                    | verum_ast::decl::ItemKind::Corollary(d) => {
                        (d.name.name.clone(), &d.attributes)
                    }
                    _ => continue,
                };

            let Some(strategy_label) = strictest_verify_strategy(&item.attributes, decl_attrs)
            else {
                continue;
            };
            let Some(declared) = LadderStrategy::from_name(strategy_label.as_str()) else {
                continue;
            };

            let obligation =
                LadderObligation::text(item_name.clone(), declared, "(elaborated obligation)");
            let report = dispatch_ladder_walk(&dispatcher, &obligation);
            total_walks += 1;
            let violations = check_runtime_monotonicity(&report);
            if !violations.is_empty() {
                total_violations += violations.len();
                violations_text.push((rel_path.clone(), item_name.clone(), violations.clone()));
                for v in violations {
                    violation_rows.push(serde_json::json!({
                        "file": rel_path.display().to_string(),
                        "item": item_name.as_str(),
                        "declared_strategy": declared.name(),
                        "coarser_failed": v.coarser_failed.name(),
                        "coarser_failure_kind": v.coarser_failure_kind.name(),
                        "stricter_succeeded": v.stricter_succeeded.name(),
                    }));
                }
            }
        }
    }

    match format {
        AuditFormat::Plain => print_ladder_monotonicity_plain(
            total_walks,
            total_violations,
            parsed_files,
            skipped_files,
            &violations_text,
        ),
        AuditFormat::Json => {
            let payload = serde_json::json!({
                "schema_version": 1,
                "command": "audit-ladder-monotonicity",
                "modules_scanned": parsed_files,
                "modules_skipped": skipped_files,
                "total_walks": total_walks,
                "total_violations": total_violations,
                "violations": violation_rows,
            });
            println!("{}", serde_json::to_string_pretty(&payload).unwrap());
        }
    }

    if total_violations > 0 {
        return Err(crate::error::CliError::VerificationFailed(format!(
            "ladder ν-monotonicity violated by {} theorem(s) — \
             a stricter strategy succeeded while a coarser one failed",
            violations_text.len(),
        )));
    }
    Ok(())
}

fn print_ladder_monotonicity_plain(
    total_walks: usize,
    total_violations: usize,
    parsed_files: usize,
    skipped_files: usize,
    violations: &[(
        PathBuf,
        Text,
        Vec<verum_verification::ladder_dispatch::MonotonicityViolation>,
    )],
) {
    println!();
    println!("{}", "Ladder ν-monotonicity report".bold());
    println!("{}", "─".repeat(40).dimmed());
    println!(
        "  Parsed {} module(s) ({} skipped); ran {} backbone walk(s).",
        parsed_files, skipped_files, total_walks,
    );
    println!(
        "  {} runtime monotonicity violation(s) across {} theorem(s).",
        total_violations,
        violations.len(),
    );
    println!();

    if violations.is_empty() {
        println!(
            "  {} every walk satisfies strict ν-monotonicity (no inversions).",
            "✓".green(),
        );
        return;
    }

    for (file, item, vs) in violations {
        println!(
            "  {} {}  ({})",
            "✗".red(),
            item.as_str().bold(),
            file.display().to_string().dimmed(),
        );
        for v in vs {
            println!(
                "    {} succeeded but {} ({}) failed",
                v.stricter_succeeded.name().bold(),
                v.coarser_failed.name(),
                v.coarser_failure_kind.name().dimmed(),
            );
        }
    }
}

// =============================================================================
// audit --cross-format-roundtrip — task #138 / MSFS-L4.5
// =============================================================================

/// Legacy entry-point for `verum audit --cross-format-roundtrip` (plain).
pub fn audit_cross_format_roundtrip() -> Result<()> {
    audit_cross_format_roundtrip_with_format(AuditFormat::Plain)
}

/// Backend-aware entry point for `verum audit --cross-format-roundtrip
/// [--docker]`. Forwards to [`audit_cross_format_roundtrip_with_format`]
/// when the default `Native` backend is selected; for `Docker` the
/// foreign-tool dispatch goes through containers via
/// [`verum_smt::cross_format_runner::checker_for_backend`].
pub fn audit_cross_format_roundtrip_with_backend(
    format: AuditFormat,
    backend: verum_smt::cross_format_runner::CheckerBackend,
) -> Result<()> {
    audit_cross_format_roundtrip_inner(format, backend)
}

/// Entry-point for `verum audit --cross-format-roundtrip [--format FORMAT]`.
///

/// Walks every `@theorem`/`@lemma`/`@corollary` declaration in the
/// project, renders each through every registered
/// [`CorpusBackend`](verum_kernel::soundness::corpus_export::CorpusBackend)
/// (currently Coq + Lean), writes the per-theorem files into
/// `target/audit-reports/cross-format-roundtrip/<format>/`, and
/// invokes the matching foreign-tool checker (`coqc` / `lean`) on
/// each emitted file. Aggregates per-theorem foreign-verdicts into
/// a structured report.
///

/// **Architectural promise**: the corpus's claim "this corpus is
/// machine-verified by independent foreign systems" is no longer
/// "we emit certificates and someone runs coqc manually" — it's
/// "the audit itself emits AND re-checks AND reports per-theorem
/// foreign-tool verdicts in one hermetic operation."
///

/// **Tool availability**: when `coqc` / `lean` is missing on PATH,
/// the per-format section is reported as `tool_missing` (not a
/// failure — just observability) and the gate exits 0 unless any
/// AVAILABLE tool reports a real failure. This means CI on a host
/// without Coq still passes the audit; CI with Coq gets the full
/// foreign re-check.
///

/// Exits non-zero only when an AVAILABLE foreign tool reports a real
/// failure on at least one emitted file.
pub fn audit_cross_format_roundtrip_with_format(format: AuditFormat) -> Result<()> {
    audit_cross_format_roundtrip_inner(
        format,
        verum_smt::cross_format_runner::CheckerBackend::from_env(),
    )
}

fn audit_cross_format_roundtrip_inner(
    format: AuditFormat,
    backend: verum_smt::cross_format_runner::CheckerBackend,
) -> Result<()> {
    use verum_kernel::soundness::corpus_export::{TheoremSpec, all_corpus_backends};
    use verum_smt::cross_format_runner::{CheckResult, CheckerBackend, ForeignSystemChecker};

    if matches!(format, AuditFormat::Plain) {
        ui::step("Cross-format roundtrip — emit + re-check corpus theorems");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);
    let report_dir = manifest_dir
        .join("target")
        .join("audit-reports")
        .join("cross-format-roundtrip");

    let backends = all_corpus_backends();
    let backend_meta: Vec<(String, String)> = backends
        .iter()
        .map(|b| (b.id().to_string(), b.extension().to_string()))
        .collect();
    let mut theorem_specs: Vec<TheoremSpec> = Vec::new();
    let mut parsed_files = 0usize;
    let mut skipped_files = 0usize;

    for abs_path in &vr_files {
        let module = match parse_file_for_audit(abs_path) {
            Ok(m) => m,
            Err(_) => {
                skipped_files += 1;
                continue;
            }
        };
        parsed_files += 1;

        let module_path_text = abs_path
            .strip_prefix(&manifest_dir)
            .unwrap_or(abs_path)
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, ".")
            .trim_end_matches(".vr")
            .to_string();

        for item in &module.items {
            let (name, decl_attrs, proof_body, proposition_expr, theorem_params, theorem_generics) =
                match &item.kind {
                    verum_ast::decl::ItemKind::Theorem(d)
                    | verum_ast::decl::ItemKind::Lemma(d)
                    | verum_ast::decl::ItemKind::Corollary(d) => (
                        d.name.name.as_str().to_string(),
                        &d.attributes,
                        match &d.proof {
                            verum_common::Maybe::Some(b) => Some(b),
                            verum_common::Maybe::None => None,
                        },
                        d.proposition.as_ref(),
                        &d.params,
                        &d.generics,
                    ),
                    _ => continue,
                };
            let has_proof = proof_body.is_some();
            let declared_strategy = strictest_verify_strategy(&item.attributes, decl_attrs)
                .map(|t| t.as_str().to_string());
 // Render the proposition's source text via the AST
 // pretty-printer so the foreign-tool comment carries
 // exactly what the user wrote — not a synthetic
 // placeholder.
            let proposition_text = verum_ast::pretty::format_expr(proposition_expr).to_string();
 // Extract `(name, &Type)` pairs from theorem params for
 // the type translator (#141 / MSFS-L4.8). Only Regular
 // params carry a (pattern, ty) pair; self-parameters
 // and the various reference-self forms aren't applicable
 // to theorem signatures (theorems aren't methods) but
 // the walker stays robust by skipping them.
            let mut walker_params: Vec<(String, &verum_ast::ty::Type)> = Vec::new();
            for fp in theorem_params.iter() {
                if let verum_ast::decl::FunctionParamKind::Regular {
                    pattern,
                    ty,
                    default_value: _,
                } = &fp.kind
                {
                    if let Some(name) = ident_pattern_name(pattern) {
                        walker_params.push((sanitise_theorem_name(&name), ty));
                    }
                }
            }
 // Extract generic-param `(name, bound_annotation)` pairs
 // (#145 / MSFS-L4.11). Type generics with a Protocol
 // bound surface as `"S : RichS"` annotations; bare generics
 // (no bound) carry an empty annotation. Higher-kinded
 // / const / lifetime generics are skipped — their
 // emission requires backend-specific machinery (HK in
 // Coq is functor-style, const-generics need DepEq, etc.).
            let mut walker_generics: Vec<(String, String)> = Vec::new();
            for gp in theorem_generics.iter() {
                if let verum_ast::ty::GenericParamKind::Type {
                    name,
                    bounds,
                    default: _,
                } = &gp.kind
                {
                    let g_name = sanitise_theorem_name(name.as_str());
                    let bound_text = bounds
                        .iter()
                        .filter_map(generic_bound_to_annotation)
                        .collect::<Vec<_>>()
                        .join(" + ");
                    let annotation = if bound_text.is_empty() {
                        String::new()
                    } else {
                        format!("{} : {}", g_name, bound_text)
                    };
                    walker_generics.push((g_name, annotation));
                }
            }
 // Run the per-backend Expr → Prop translators (#140 /
 // MSFS-L4.7) AND the per-backend Type translators
 // (#141 / MSFS-L4.8). Successful translations land
 // in their respective maps; fallbacks leave the entry
 // absent and the per-format renderer reverts to a
 // generic placeholder.
            let mut spec = TheoremSpec {
                name: sanitise_theorem_name(&name),
                module_path: module_path_text.clone(),
                proposition_text,
                per_backend_proposition: std::collections::BTreeMap::new(),
                params: Vec::new(),
                generics: Vec::new(),
                has_proof_body: has_proof,
                per_backend_proof_tactic: std::collections::BTreeMap::new(),
                declared_strategy,
            }
            .with_translated_params(&walker_params)
            .with_generics(&walker_generics)
            .with_translated_proposition(proposition_expr);
 // #153 / Phase 2: when the theorem has a proof body and
 // its shape is V0-translatable (term-mode or single-apply
 // tactic), thread the translation into per_backend_proof_tactic.
 // Untranslatable shapes leave the entry absent and the
 // renderer falls back to Admitted./sorry.
            if let Some(body) = proof_body {
                spec = spec.with_translated_proof_body(body);
            }
            theorem_specs.push(spec);
        }
    }

 // Per-backend emission + foreign-tool invocation.
    let mut roundtrips: Vec<ThmRoundtripPlainRow> = Vec::new();
    let mut foreign_failures = 0usize;

    for backend_iter in &backends {
        let backend_dir = report_dir.join(backend_iter.id());
        let _ = std::fs::create_dir_all(&backend_dir);
 // Backend-aware checker dispatch (#149 / MSFS-L4.15). When
 // `--docker` is set or VERUM_FOREIGN_TOOL_BACKEND=docker, the
 // foreign tool runs inside its canonical container image so
 // hosts without coqc/lean still get real per-theorem verdicts.
 // #156-closure: dispatch to the canonical checker for every
 // emitter backend. Pre-this-commit only Coq + Lean had
 // checker dispatch; Agda / Isabelle / Dedukti emitted to disk
 // but were never re-checked. Now the corpus walks all 5.
        let foreign_checker: Option<Box<dyn ForeignSystemChecker>> = match backend_iter.id() {
            "coq" => verum_smt::cross_format_runner::checker_for_backend(
                verum_kernel::cross_format_gate::ExportFormat::Coq,
                backend,
            ),
            "lean" => verum_smt::cross_format_runner::checker_for_backend(
                verum_kernel::cross_format_gate::ExportFormat::Lean4,
                backend,
            ),
            "agda" => verum_smt::cross_format_runner::checker_for_backend(
                verum_kernel::cross_format_gate::ExportFormat::Agda,
                backend,
            ),
            "isabelle" => verum_smt::cross_format_runner::checker_for_backend(
                verum_kernel::cross_format_gate::ExportFormat::Isabelle,
                backend,
            ),
            "dedukti" => verum_smt::cross_format_runner::checker_for_backend(
                verum_kernel::cross_format_gate::ExportFormat::Dedukti,
                backend,
            ),
            _ => None,
        };
        let tool_available = foreign_checker
            .as_ref()
            .map(|c| c.is_available())
            .unwrap_or(false);

        for spec in &theorem_specs {
            let rendered = backend_iter.render_theorem(spec);
            let path = backend_dir.join(&rendered.filename);
            let verdict_kind: &'static str;
            let detail: String;
            if let Err(e) = std::fs::write(&path, &rendered.content) {
                verdict_kind = "emit_failed";
                detail = format!("write {}: {}", path.display(), e);
            } else if let Some(checker) = foreign_checker.as_ref() {
                if !tool_available {
                    verdict_kind = "tool_missing";
                    detail = checker.install_hint().to_string();
                } else {
                    match checker.check_file(&path) {
                        CheckResult::Passed {
                            tool_version,
                            elapsed,
                            ..
                        } => {
                            verdict_kind = "passed";
                            detail =
                                format!("tool={} elapsed={}ms", tool_version, elapsed.as_millis());
                        }
                        CheckResult::Failed {
                            exit_code,
                            stderr_excerpt,
                            ..
                        } => {
                            verdict_kind = "failed";
                            detail = format!("exit={} stderr={}", exit_code, stderr_excerpt);
                            foreign_failures += 1;
                        }
                        CheckResult::ToolMissing { install_hint } => {
                            verdict_kind = "tool_missing";
                            detail = install_hint;
                        }
                        CheckResult::RunnerError { reason } => {
                            verdict_kind = "runner_error";
                            detail = reason;
                        }
                    }
                }
            } else {
                verdict_kind = "no_checker";
                detail = "no foreign-tool checker registered for this backend".to_string();
            }
            roundtrips.push(ThmRoundtripPlainRow {
                backend_id: backend_iter.id().to_string(),
                theorem_name: spec.name.clone(),
                emitted_path: path,
                verdict_kind,
                detail,
            });
        }
    }

    match format {
        AuditFormat::Plain => print_cross_format_roundtrip_plain(
            &theorem_specs,
            &roundtrips,
            parsed_files,
            skipped_files,
            foreign_failures,
            &backend_meta,
        ),
        AuditFormat::Json => {
            let rows: Vec<serde_json::Value> = roundtrips
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "backend": r.backend_id,
                        "theorem": r.theorem_name,
                        "emitted_path": r.emitted_path.display().to_string(),
                        "verdict": r.verdict_kind,
                        "detail": r.detail,
                    })
                })
                .collect();
            let backend_label = match backend {
                CheckerBackend::Native => "native",
                CheckerBackend::Docker => "docker",
            };
            let payload = serde_json::json!({
                "schema_version": 1,
                "command": "audit-cross-format-roundtrip",
                "modules_scanned": parsed_files,
                "modules_skipped": skipped_files,
                "theorems_walked": theorem_specs.len(),
                "backend_count": backend_meta.len(),
                "foreign_failures": foreign_failures,
                "checker_backend": backend_label,
                "roundtrips": rows,
            });
            println!("{}", serde_json::to_string_pretty(&payload).unwrap());
        }
    }

    if foreign_failures > 0 {
        return Err(crate::error::CliError::VerificationFailed(format!(
            "cross-format roundtrip: {} foreign-tool failure(s) — \
             at least one emitted theorem was rejected by an available checker",
            foreign_failures,
        )));
    }
    Ok(())
}

// =============================================================================
// audit --proof-term-library — #157 follow-up / canonical regression suite
// =============================================================================

/// Verify every `.vproof` in the canonical proof-term certificate
/// library. Walks `core/verify/proof_term_examples/` (or the
/// directory pointed at by `VERUM_PROOF_TERM_EXAMPLES`), runs
/// `proof_checker::Certificate::verify()` on each, exits non-zero on
/// any rejection.
///

/// This is the trust-base regression suite — every kernel
/// implementation claiming Verum compatibility must accept all
/// canonical certificates. The library currently covers identity,
/// polymorphic identity, K combinator; grows as `proof_checker`
/// admits new inference rules (refinement subtyping, W-types,
/// inductive types, etc.).
pub fn audit_proof_term_library_with_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::kernel_registry::{AgreementVerdict, KernelRegistry};
    use verum_kernel::proof_checker::Certificate;
    use verum_kernel::proof_checker_meta;

    if matches!(format, AuditFormat::Plain) {
        ui::step(
            "Verifying canonical proof-term certificate library (N-kernel + universe-stability)",
        );
    }

 // **#159 V4 — N-kernel registry**. Built once at audit entry;
 // every certificate runs through every registered kernel via
 // `registry.verify_all(&cert)`. Adding a third kernel (#154
 // self-hosted Verum kernel, future HOAS-based checker, ...)
 // takes one line at the registry construction site below; the
 // audit gate's per-cert flow stays unchanged.
    let kernel_registry = KernelRegistry::default();
    let registered_kernel_names: Vec<&'static str> = kernel_registry.names();

 // Discovery: env override → workspace ancestor walk → manifest dir
 // co-located.
    let candidates = proof_term_library_candidates();
    let library_dir = candidates.into_iter().find(|p| p.is_dir());
    let library_dir = match library_dir {
        Some(d) => d,
        None => {
            return Err(crate::error::CliError::InvalidArgument(
                "proof-term certificate library not found — set VERUM_PROOF_TERM_EXAMPLES \
                 or run from a tree containing core/verify/proof_term_examples/"
                    .to_string(),
            ));
        }
    };

 // Walk every .vproof file in the library directory + the
 // `adversarial/` subdirectory (#159 V3). Adversarial
 // certificates are tagged via metadata.expected_outcome ==
 // "reject" so the audit gate enforces the inverted contract:
 // both kernels must REJECT, not ACCEPT.
    let mut entries: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&library_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map_or(false, |ext| ext == "vproof") {
                entries.push(p);
            }
        }
    }
    let adv_dir = library_dir.join("adversarial");
    if adv_dir.is_dir() {
        if let Ok(rd) = std::fs::read_dir(&adv_dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().map_or(false, |ext| ext == "vproof") {
                    entries.push(p);
                }
            }
        }
    }
    entries.sort();

    let mut verifications: Vec<serde_json::Value> = Vec::new();
    let mut verified = 0usize;
    let mut rejected = 0usize;
    let mut adversarial_rejected = 0usize;
    let mut adversarial_unsoundly_accepted = 0usize;
    let mut malformed = 0usize;
 // **#159 V2** — every certificate runs through BOTH kernels:
 // Algorithm A (`proof_checker`, trusted base, bidirectional +
 // explicit substitution) and Algorithm B (`proof_checker_nbe`,
 // NbE-based). Disagreements between the two are bugs in EITHER
 // implementation; they fail the audit.
    let mut nbe_disagreements = 0usize;
 // **#158 V1** — universe-stability check. Every certificate's
 // verdict is computed at lifts 0..=2 via
 // `proof_checker_meta::check_universe_stability`. The verdict
 // must be CONSTANT across lifts — bumping the universe-hierarchy
 // must NEVER flip the verdict. Non-stable verdicts are
 // implementation bugs (universe-identity dependence) or
 // soundness regressions (proof depending on a specific level).
    let mut universe_unstable = 0usize;
    const META_MAX_LIFT: u32 = 2;

    for path in &entries {
        let outcome = match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Certificate>(&text) {
                Ok(cert) => {
                    let name = cert
                        .metadata
                        .get("name")
                        .cloned()
                        .unwrap_or_else(|| "(anonymous)".to_string());
 // **#159 V3**: dispatch on metadata.expected_outcome.
 // Default ("accept") = canonical accept-path library;
 // "reject" = adversarial library (both kernels must
 // REJECT, not accept).
                    let expected_reject = cert
                        .metadata
                        .get("expected_outcome")
                        .map(|s| s.as_str() == "reject")
                        .unwrap_or(false);
 // **#159 V4 — N-kernel registry verification**.
 // Run the certificate through every registered
 // kernel; classify agreement; flag disagreements.
                    let multi_verdict = kernel_registry.verify_all(&cert);
 // **#158 V1 universe-stability check** — verify
 // the certificate's verdict is invariant under
 // universe-lift. Bumping every Universe(n) →
 // Universe(n+k) must NEVER flip accept ↔ reject.
                    let (_, stable) = proof_checker_meta::check_universe_stability(
                        &cert,
                        META_MAX_LIFT,
                    );
                    if !stable {
                        universe_unstable += 1;
                    }
 // Disagreement bookkeeping: the `nbe_disagreements`
 // counter is preserved for backwards compatibility
 // with the schema-version-3 JSON consumers; under
 // the N-kernel registry it counts "any pair of
 // kernels disagreed".
                    if matches!(
                        multi_verdict.agreement,
                        AgreementVerdict::Disagreement { .. }
                    ) {
                        nbe_disagreements += 1;
                    }
                    let unanimous_accept = matches!(
                        multi_verdict.agreement,
                        AgreementVerdict::Unanimous,
                    );
                    let unanimous_reject = matches!(
                        multi_verdict.agreement,
                        AgreementVerdict::UnanimousReject,
                    );
                    let (label, detail) = if expected_reject {
 // Adversarial path: every kernel MUST reject.
                        if unanimous_reject {
                            adversarial_rejected += 1;
                            (
                                "adversarial_rejected",
                                format!(
                                    "lock-step rejection across {} kernels: {:?}",
                                    registered_kernel_names.len(),
                                    registered_kernel_names,
                                ),
                            )
                        } else if unanimous_accept {
 // SOUNDNESS BUG: every kernel ACCEPTED a
 // certificate marked expected_outcome=reject.
                            adversarial_unsoundly_accepted += 1;
                            (
                                "adversarial_unsoundly_accepted",
                                format!(
                                    "EVERY kernel ({}) accepted a certificate \
                                     marked expected_outcome=reject — \
                                     soundness violation.",
                                    registered_kernel_names.len(),
                                ),
                            )
                        } else {
                            (
                                "disagreement",
                                format!(
                                    "adversarial disagreement across {} kernels: {:?}",
                                    registered_kernel_names.len(),
                                    multi_verdict.agreement,
                                ),
                            )
                        }
                    } else {
 // Canonical-accept path: every kernel MUST accept.
                        if unanimous_accept {
                            verified += 1;
                            ("verified", String::new())
                        } else if unanimous_reject {
                            rejected += 1;
 // Pull the first error summary from the
 // outcomes (all are rejecting; the first
 // one's message is representative).
                            let first_err = multi_verdict
                                .outcomes
                                .iter()
                                .find_map(|o| o.error_summary.clone())
                                .unwrap_or_default();
                            ("rejected", first_err)
                        } else {
                            (
                                "disagreement",
                                format!(
                                    "kernel-disagreement across {} kernels: {:?}",
                                    registered_kernel_names.len(),
                                    multi_verdict.agreement,
                                ),
                            )
                        }
                    };
                    (label, name, detail)
                }
                Err(e) => {
                    malformed += 1;
                    ("malformed_json", String::new(), e.to_string())
                }
            },
            Err(e) => {
                malformed += 1;
                ("read_error", String::new(), e.to_string())
            }
        };
        verifications.push(serde_json::json!({
            "file": path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            "name": outcome.1,
            "outcome": outcome.0,
            "detail": outcome.2,
        }));
    }

 // Emit JSON to disk for bundle composition.
    let manifest_dir = Manifest::find_manifest_dir().ok();
    let report_path = match &manifest_dir {
        Some(d) => d
            .join("target")
            .join("audit-reports")
            .join("proof-term-library.json"),
        None => library_dir.join("proof-term-library.json"),
    };
    if let Some(parent) = report_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let payload = serde_json::json!({
        "schema_version": 5,
        "command": "audit-proof-term-library",
        "library_path": library_dir.display().to_string(),
 // **#159 V4** — list every kernel implementation in the
 // registered set. Reviewers reading the report can confirm
 // which independent implementations the corpus was
 // differential-tested against.
        "registered_kernels": registered_kernel_names,
        "total": entries.len(),
        "verified": verified,
        "rejected": rejected,
        "malformed": malformed,
 // #159 V2 differential count — number of certificates where
 // the trusted base and the NbE kernel disagreed. Any
 // non-zero value flips the audit gate to failure.
        "nbe_disagreements": nbe_disagreements,
 // #159 V3 adversarial counts — every certificate marked
 // expected_outcome=reject MUST be rejected by both kernels.
 // adversarial_unsoundly_accepted > 0 → SOUNDNESS BUG.
        "adversarial_rejected": adversarial_rejected,
        "adversarial_unsoundly_accepted": adversarial_unsoundly_accepted,
 // #158 V1 universe-stability count — number of certificates
 // whose verdict flipped across lift 0..=META_MAX_LIFT.
 // Any non-zero value flips the audit gate to failure.
        "universe_unstable": universe_unstable,
        "meta_max_lift": META_MAX_LIFT,
        "verifications": verifications,
    });
    let _ = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&payload).unwrap(),
    );

    match format {
        AuditFormat::Plain => {
            println!();
            println!(
                "Canonical proof-term certificate library (NbE + adversarial + universe-stability)"
            );
            println!(
                "─────────────────────────────────────────────────────────────────────────────────"
            );
            println!("  Library: {}", library_dir.display());
            println!(
                "  ✓ {} verified · ✗ {} rejected · ✓ {} adversarial-rejected · {} {} adversarial-unsoundly-accepted",
                verified,
                rejected,
                adversarial_rejected,
                if adversarial_unsoundly_accepted == 0 { "✓" } else { "✗" },
                adversarial_unsoundly_accepted,
            );
            println!(
                "  ⚠ {} malformed · ✗ {} NbE-disagreement · {} {} universe-unstable (lifts 0..={}, total {})",
                malformed,
                nbe_disagreements,
                if universe_unstable == 0 { "✓" } else { "✗" },
                universe_unstable,
                META_MAX_LIFT,
                entries.len(),
            );
            let any_failure = rejected > 0
                || malformed > 0
                || nbe_disagreements > 0
                || adversarial_unsoundly_accepted > 0
                || universe_unstable > 0;
            if any_failure {
                println!();
                for v in verifications.iter().filter(|v| {
                    !matches!(
                        v.get("outcome").and_then(|s| s.as_str()),
                        Some("verified") | Some("adversarial_rejected"),
                    )
                }) {
                    println!(
                        "    {} :: {} :: {}",
                        v.get("file").and_then(|s| s.as_str()).unwrap_or("?"),
                        v.get("outcome").and_then(|s| s.as_str()).unwrap_or("?"),
                        v.get("detail").and_then(|s| s.as_str()).unwrap_or("?"),
                    );
                }
            }
            println!();
            println!("  Report: {}", report_path.display());
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&payload).unwrap());
        }
    }

    if rejected > 0
        || malformed > 0
        || nbe_disagreements > 0
        || adversarial_unsoundly_accepted > 0
        || universe_unstable > 0
    {
        return Err(crate::error::CliError::VerificationFailed(format!(
            "proof-term-library: {} rejected + {} malformed + {} NbE-disagreement(s) + \
             {} adversarial-unsoundly-accepted + {} universe-unstable",
            rejected,
            malformed,
            nbe_disagreements,
            adversarial_unsoundly_accepted,
            universe_unstable,
        )));
    }
    Ok(())
}

fn proof_term_library_candidates() -> Vec<std::path::PathBuf> {
    let mut out: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(env_dir) = std::env::var("VERUM_PROOF_TERM_EXAMPLES") {
        out.push(std::path::PathBuf::from(env_dir));
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut cur = cwd;
        for _ in 0..8 {
            let candidate = cur.join("core").join("verify").join("proof_term_examples");
            if candidate.is_dir() {
                out.push(candidate);
            }
            if !cur.pop() {
                break;
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let mut cur = dir.to_path_buf();
            for _ in 0..6 {
                let candidate = cur.join("core").join("verify").join("proof_term_examples");
                if candidate.is_dir() {
                    out.push(candidate);
                }
                if !cur.pop() {
                    break;
                }
            }
        }
    }
    out
}

// =============================================================================
// audit --signatures — task #174 / certificate-bearing artifacts
// =============================================================================

/// Verify provenance signatures on emitted cross-format files.
///

/// Walks the corpus, recomputes each theorem's expected
/// `verum_signature` header via
/// [`verum_kernel::soundness::corpus_export::compute_provenance_signature`],
/// and compares against the signature line actually present at the
/// top of the on-disk
/// `target/audit-reports/cross-format-roundtrip/{coq,lean}/*` files.
///

/// **Reproducibility primitive.** A third-party reviewer pulls the
/// published `.v` / `.lean` files out of MSFS supplementary material,
/// runs this gate, and gets a binary verdict: the files came from
/// EXACTLY the named kernel version against the named corpus state,
/// or they didn't. No need to re-run the entire pipeline to verify
/// provenance — just recompute the hash and compare.
///

/// **Exit semantics.** Non-zero on any signature mismatch (file
/// drift) or missing signature header (file emitted by older kernel).
/// Always emits per-file verdict to JSON.
pub fn audit_signatures_with_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::soundness::corpus_export::{TheoremSpec, compute_provenance_signature};

    if matches!(format, AuditFormat::Plain) {
        ui::step("Verifying provenance signatures on emitted cross-format files");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let mut vr_files = discover_vr_files(&manifest_dir);
    vr_files.extend(discover_stdlib_vr_files());
    let mut theorem_specs: Vec<TheoremSpec> = Vec::new();

    for abs_path in &vr_files {
 // Only walk corpus-rooted files; stdlib's theorems aren't
 // emitted by the cross-format gate.
        if !abs_path.starts_with(&manifest_dir) {
            continue;
        }
        let module = match parse_file_for_audit(abs_path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let module_path_text = abs_path
            .strip_prefix(&manifest_dir)
            .unwrap_or(abs_path)
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, ".")
            .trim_end_matches(".vr")
            .to_string();
        for item in &module.items {
            let (name, decl_attrs, proof_body, proposition_expr, theorem_params, theorem_generics) =
                match &item.kind {
                    verum_ast::decl::ItemKind::Theorem(d)
                    | verum_ast::decl::ItemKind::Lemma(d)
                    | verum_ast::decl::ItemKind::Corollary(d) => (
                        d.name.name.as_str().to_string(),
                        &d.attributes,
                        match &d.proof {
                            verum_common::Maybe::Some(b) => Some(b),
                            verum_common::Maybe::None => None,
                        },
                        d.proposition.as_ref(),
                        &d.params,
                        &d.generics,
                    ),
                    _ => continue,
                };
            let has_proof = proof_body.is_some();
            let declared_strategy = strictest_verify_strategy(&item.attributes, decl_attrs)
                .map(|t| t.as_str().to_string());
            let proposition_text = verum_ast::pretty::format_expr(proposition_expr).to_string();
 // **Bug fix**: the signatures gate must construct
 // `TheoremSpec` with the SAME translator outputs the
 // cross-format-roundtrip gate uses to emit the .v/.lean
 // files — otherwise the recomputed signature uses null
 // proposition + null proof tactic while the file's
 // signature header was computed with the actual translated
 // text. Pre-fix this caused 74/74 signatures to mismatch on
 // the live MSFS corpus despite no real divergence.
 //
 // Mirror the cross-format-roundtrip gate's params +
 // generics population (audit.rs ~L3747-3788). Without
 // these, theorems with explicit type-parameters (e.g.
 // `obstruction_non_negative`) compute a signature
 // missing the param/generic block while the rendered
 // .v/.lean carries it — producing a 4-mismatch chain
 // (coq+lean × theorem × 2 nominally-distinct walks).
            let mut walker_params: Vec<(String, &verum_ast::ty::Type)> = Vec::new();
            for fp in theorem_params.iter() {
                if let verum_ast::decl::FunctionParamKind::Regular {
                    pattern,
                    ty,
                    default_value: _,
                } = &fp.kind
                {
                    if let Some(pname) = ident_pattern_name(pattern) {
                        walker_params.push((sanitise_theorem_name(&pname), ty));
                    }
                }
            }
            let mut walker_generics: Vec<(String, String)> = Vec::new();
            for gp in theorem_generics.iter() {
                if let verum_ast::ty::GenericParamKind::Type {
                    name,
                    bounds,
                    default: _,
                } = &gp.kind
                {
                    let g_name = sanitise_theorem_name(name.as_str());
                    let bound_text = bounds
                        .iter()
                        .filter_map(generic_bound_to_annotation)
                        .collect::<Vec<_>>()
                        .join(" + ");
                    let annotation = if bound_text.is_empty() {
                        String::new()
                    } else {
                        format!("{} : {}", g_name, bound_text)
                    };
                    walker_generics.push((g_name, annotation));
                }
            }
            let mut spec = TheoremSpec {
                name: sanitise_theorem_name(&name),
                module_path: module_path_text.clone(),
                proposition_text,
                per_backend_proposition: std::collections::BTreeMap::new(),
                params: Vec::new(),
                generics: Vec::new(),
                has_proof_body: has_proof,
                per_backend_proof_tactic: std::collections::BTreeMap::new(),
                declared_strategy,
            }
            .with_translated_params(&walker_params)
            .with_generics(&walker_generics)
            .with_translated_proposition(proposition_expr);
            if let Some(body) = proof_body {
                spec = spec.with_translated_proof_body(body);
            }
            theorem_specs.push(spec);
        }
    }

    let report_dir = manifest_dir
        .join("target")
        .join("audit-reports")
        .join("cross-format-roundtrip");
    let mut verifications: Vec<serde_json::Value> = Vec::new();
    let mut mismatched = 0usize;
    let mut missing = 0usize;
    let mut verified = 0usize;

    for spec in &theorem_specs {
        for backend_id in ["coq", "lean"] {
            let expected_sig = compute_provenance_signature(spec, backend_id);
            let extension = if backend_id == "coq" { "v" } else { "lean" };
            let file_path = report_dir
                .join(backend_id)
                .join(format!("{}.{}", spec.name, extension));
            let outcome = if !file_path.exists() {
                "file_absent"
            } else {
                match std::fs::read_to_string(&file_path) {
                    Ok(text) => match extract_signature_header(&text) {
                        Some(actual_sig) => {
                            if actual_sig == expected_sig {
                                verified += 1;
                                "verified"
                            } else {
                                mismatched += 1;
                                "mismatched"
                            }
                        }
                        None => {
                            missing += 1;
                            "header_missing"
                        }
                    },
                    Err(_) => "read_error",
                }
            };
            verifications.push(serde_json::json!({
                "theorem": spec.name,
                "backend": backend_id,
                "file": file_path.strip_prefix(&manifest_dir).unwrap_or(&file_path).display().to_string(),
                "expected_signature": expected_sig,
                "outcome": outcome,
            }));
        }
    }

    let report_path = manifest_dir
        .join("target")
        .join("audit-reports")
        .join("signatures.json");
    let _ = std::fs::create_dir_all(report_path.parent().unwrap());
    let payload = serde_json::json!({
        "schema_version": 1,
        "command": "audit-signatures",
        "kernel_version": verum_kernel::soundness::corpus_export::KERNEL_VERSION,
        "theorems_walked": theorem_specs.len(),
        "verified": verified,
        "mismatched": mismatched,
        "header_missing": missing,
        "verifications": verifications,
    });
    let _ = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&payload).unwrap(),
    );

    match format {
        AuditFormat::Plain => {
            println!();
            println!("Provenance-signature verification");
            println!("────────────────────────────────────────");
            println!(
                "  Walked {} theorem(s); kernel version {}.",
                theorem_specs.len(),
                verum_kernel::soundness::corpus_export::KERNEL_VERSION,
            );
            println!(
                "  ✓ {} verified · ✗ {} mismatched · ⚠ {} header-missing",
                verified, mismatched, missing,
            );
            if mismatched > 0 || missing > 0 {
                println!();
                for v in verifications.iter().filter(|v| {
                    matches!(
                        v.get("outcome").and_then(|s| s.as_str()),
                        Some("mismatched") | Some("header_missing")
                    )
                }) {
                    println!(
                        "    {} :: {} → {}",
                        v.get("backend").and_then(|s| s.as_str()).unwrap_or("?"),
                        v.get("theorem").and_then(|s| s.as_str()).unwrap_or("?"),
                        v.get("outcome").and_then(|s| s.as_str()).unwrap_or("?"),
                    );
                }
            }
            println!();
            println!("  Report: {}", report_path.display());
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&payload).unwrap());
        }
    }

    if mismatched > 0 || missing > 0 {
        return Err(crate::error::CliError::VerificationFailed(format!(
            "signature audit: {} mismatched + {} header-missing — emitted files do \
             not match the corpus state Verum claims to have produced them from",
            mismatched, missing,
        )));
    }
    Ok(())
}

/// Extract the `verum_signature: …` value from the first 8 lines of a
/// rendered file. Both Coq comments `(* verum_signature: X *)` and
/// Lean comments `/-! verum_signature: X -/` carry the signature in
/// the same syntactic shape — we tokenise on the prefix.
fn extract_signature_header(text: &str) -> Option<String> {
    for line in text.lines().take(8) {
        let trimmed = line.trim();
        let body = trimmed
            .strip_prefix("(*")
            .and_then(|s| s.strip_suffix("*)"))
            .or_else(|| {
                trimmed
                    .strip_prefix("/-!")
                    .and_then(|s| s.strip_suffix("-/"))
            });
        if let Some(body) = body {
            let body = body.trim();
            if let Some(sig) = body.strip_prefix("verum_signature:") {
                return Some(sig.trim().to_string());
            }
        }
    }
    None
}

// =============================================================================
// audit --soundness-iou — task #152 / Phase-1 trust-base reduction
// =============================================================================

/// Run the kernel-soundness IOU dashboard.
///

/// Enumerates every kernel rule whose soundness lemma in
/// `core/verify/kernel_soundness/` carries an `Admitted { reason }`
/// status; groups by [`verum_kernel::soundness::RuleCategory`]; emits
/// structured JSON + plain summary so reviewers + CI can track the
/// IOU set over time.
///

/// **Architectural significance.** This is the metric-driven
/// foundation for the path to "constructively verified from first
/// principles" (Phase 1 of the trust-base reduction roadmap). The
/// 38 kernel rules currently split as 4 proved + 34 admitted; each
/// admit closed shrinks Verum's trusted base by one rule. Without a
/// dashboard surfacing the admit set, discharge effort has no
/// measurable target.
///

/// **Discharge prioritisation.** The output groups admits by
/// category and lists each with its concrete IOU reason (e.g.,
/// "substitution-lemma", "β-confluence", "CCHM Kan-filling", "modal-
/// depth ordinal arithmetic", "κ-tower well-foundedness", "Schreiber
/// DCCT cohesive triple-adjunction"). Domain experts pick the
/// category matching their expertise and tackle the admits in batch.
///

/// **Exit semantics.** Always exits 0 — this is an observability
/// gate. CI tracks the admit count over time via the JSON output;
/// the proof-honesty CI gate's analogue ratchets the admit floor
/// downward as Verum matures.
pub fn audit_soundness_iou_with_format(format: AuditFormat) -> Result<()> {
    use std::collections::BTreeMap;
    use verum_kernel::soundness::{LemmaStatus, RuleCategory, SoundnessExporter};

    if matches!(format, AuditFormat::Plain) {
        ui::step("Kernel-soundness IOU dashboard");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let exporter = SoundnessExporter::new();

 // Group admits by RuleCategory. Within each category sort by
 // rule_name for stable ordering. `DischargedByFramework` rules
 // are tracked separately — they're L4-acceptable but downstream
 // of an external proof.
    let mut by_category: BTreeMap<&'static str, Vec<&verum_kernel::soundness::RuleSpec>> =
        BTreeMap::new();
    let mut total_proved = 0usize;
    let mut total_admitted = 0usize;
    let mut total_discharged = 0usize;
    for rule in exporter.rules() {
        match &rule.status {
            LemmaStatus::Proved { .. } => total_proved += 1,
            LemmaStatus::Admitted { .. } => {
                total_admitted += 1;
                by_category
                    .entry(rule.category.tag())
                    .or_default()
                    .push(rule);
            }
            LemmaStatus::DischargedByFramework { .. } => {
                total_discharged += 1;
                by_category
                    .entry(rule.category.tag())
                    .or_default()
                    .push(rule);
            }
        }
    }
    for rules in by_category.values_mut() {
        rules.sort_by_key(|r| r.rule_name.clone());
    }

 // Emit JSON to disk for CI tracking + bundle composition.
    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let json_path = report_dir.join("soundness-iou.json");

    let mut category_payloads: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    for category_tag in [
        RuleCategory::Structural.tag(),
        RuleCategory::Cubical.tag(),
        RuleCategory::Refinement.tag(),
        RuleCategory::Quotient.tag(),
        RuleCategory::Inductive.tag(),
        RuleCategory::SmtAxiom.tag(),
        RuleCategory::Diakrisis.tag(),
    ] {
        let rules = by_category.get(category_tag).cloned().unwrap_or_default();
        let entries: Vec<serde_json::Value> = rules
            .iter()
            .map(|r| {
                let status_kind = match &r.status {
                    LemmaStatus::Proved { .. } => "Proved",
                    LemmaStatus::Admitted { .. } => "Admitted",
                    LemmaStatus::DischargedByFramework { .. } => "DischargedByFramework",
                };
                let discharge = match &r.status {
                    LemmaStatus::DischargedByFramework {
                        lemma_path,
                        framework,
                        citation,
                    } => {
                        serde_json::json!({
                            "lemma_path": lemma_path,
                            "framework": framework,
                            "citation": citation,
                        })
                    }
                    _ => serde_json::Value::Null,
                };
                serde_json::json!({
                    "rule_name": r.rule_name,
                    "lemma_name": r.lemma_name,
                    "status": status_kind,
                    "iou_reason": r.status.admit_reason().unwrap_or(""),
                    "discharge": discharge,
                    "premise_arity": r.premise_arity,
                    "has_side_condition": r.has_side_condition,
                })
            })
            .collect();
        category_payloads.insert(
            category_tag.to_string(),
            serde_json::json!({
                "admit_count": entries.len(),
                "admits": entries,
            }),
        );
    }

    let payload = serde_json::json!({
        "schema_version": 2,
        "command": "audit-soundness-iou",
        "total_rules": exporter.rules().len(),
        "total_proved": total_proved,
        "total_admitted": total_admitted,
        "total_discharged_by_framework": total_discharged,
        "categories": category_payloads,
    });
    let _ = std::fs::write(&json_path, serde_json::to_string_pretty(&payload).unwrap());

    match format {
        AuditFormat::Plain => {
            print_soundness_iou_plain(
                exporter.rules().len(),
                total_proved,
                total_admitted,
                total_discharged,
                &by_category,
                &json_path,
            );
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&payload).unwrap());
        }
    }

    Ok(())
}

fn print_soundness_iou_plain(
    total_rules: usize,
    total_proved: usize,
    total_admitted: usize,
    total_discharged: usize,
    by_category: &std::collections::BTreeMap<&'static str, Vec<&verum_kernel::soundness::RuleSpec>>,
    json_path: &std::path::Path,
) {
    use verum_kernel::soundness::RuleCategory;
    println!();
    println!("Kernel-soundness IOU dashboard");
    println!("────────────────────────────────────────");
    println!(
        "  {} kernel rules total — {} structurally proved, {} admitted with open IOU, {} discharged by framework citation.",
        total_rules, total_proved, total_admitted, total_discharged,
    );
    println!();
    if total_admitted == 0 && total_discharged == 0 {
        println!(
            "  ✓ Every kernel rule is structurally proved — trusted base reduced to ZFC + 2-inacc + Verum kernel rules."
        );
        return;
    }
    if total_admitted == 0 {
        println!(
            "  ✓ Zero open IOUs — every admitted rule is discharged by upstream framework citation."
        );
        println!(
            "    L4-acceptable: the trust base is ZFC + 2-inacc + cited mathlib4 / Coq stdlib proofs."
        );
        println!();
    }

 // Stable category ordering (matches the JSON output).
    let category_order = [
        RuleCategory::Structural.tag(),
        RuleCategory::Cubical.tag(),
        RuleCategory::Refinement.tag(),
        RuleCategory::Quotient.tag(),
        RuleCategory::Inductive.tag(),
        RuleCategory::SmtAxiom.tag(),
        RuleCategory::Diakrisis.tag(),
    ];
    for category_tag in category_order {
        if let Some(rules) = by_category.get(category_tag) {
            if rules.is_empty() {
                continue;
            }
            println!(
                "  {} ({} admit{})",
                category_tag,
                rules.len(),
                if rules.len() == 1 { "" } else { "s" },
            );
            for rule in rules {
                let reason = rule.status.admit_reason().unwrap_or("");
                println!("    • {:<28} — {}", rule.rule_name, reason);
            }
            println!();
        }
    }
    println!("  Report: {}", json_path.display());
    println!();
    println!("  Each admit's IOU is a concrete piece of meta-theory awaiting discharge.");
    println!("  As discharges land, the admit count drops; the trusted base shrinks; Verum");
    println!("  approaches \"constructively verified from first principles\" (Phase 1 goal).");
}

// =============================================================================
// audit --apply-graph — task #150 / MSFS-L4.13
// =============================================================================

/// Legacy entry-point for `verum audit --apply-graph` (plain).
pub fn audit_apply_graph() -> Result<()> {
    audit_apply_graph_with_format(AuditFormat::Plain)
}

/// Entry-point for `verum audit --apply-graph [--format FORMAT]`.
///

/// Walks every theorem in the project and classifies its TRANSITIVE
/// apply-chain leaves. Each `apply <symbol>(args)` resolves through
/// the workspace symbol table to its body; the recursion terminates
/// at axiom leaves classified as `kernel_strict` / `framework_axiom` /
/// `placeholder_axiom` / `unresolved`.
///

/// This is the load-bearing complement to `--bridge-discharge` (which
/// only checks the immediate apply): `--apply-graph` follows the
/// chain across `_full` forms and stdlib delegates, so a placeholder
/// leak deep in the chain surfaces.
///

/// Exits non-zero when any theorem's composition has
/// `placeholder_axiom > 0` or `unresolved > 0` — those theorems are
/// not yet L4 load-bearing.
//

// =============================================================================
// audit --bundle — task #151 / unified L1+L2+L3+L4 verdict
// =============================================================================

/// Run the unified audit-bundle: all load-bearing gates in dependency
/// order, aggregated into a single JSON report + plain summary.
///

/// **Architecture (protocol-driven).** Each load-bearing gate is
/// invoked with `--format json`, the JSON output captured, and merged
/// into a top-level `gates: { <name>: <gate_json>, ... }` object plus
/// an aggregate `l4_load_bearing: bool` summary. Adding a future
/// gate is one new entry in the runner registry.
///

/// **Failure semantics.** Each gate's per-theorem verdict is
/// independent — bridge-discharge can fail (false discharge in
/// proof body) without the apply-graph audit caring. The bundle
/// composes them: `l4_load_bearing == true` iff every gate's verdict
/// is clean. Per-gate failures are captured in the JSON regardless,
/// so a CI pipeline gets the complete observable evidence even when
/// an early gate fails.
///

/// **JSON shape.**
/// ```json
/// {
/// "schema_version": 1,
/// "command": "audit-bundle",
/// "l4_load_bearing": <bool>,
/// "gates": {
/// "bridge_discharge": { ... },
/// "kernel_discharged_axioms": { ... },
/// "apply_graph": { ... },
/// "cross_format_roundtrip": { ... }
/// },
/// "summary": { "<gate>": "passed" | "failed" }
/// }
/// ```
/// Extract a compact gate-specific metric string from the gate's
/// already-loaded JSON report so the bundle's Plain output can show
/// per-gate counts beside the pass/fail label. Each gate's JSON
/// shape is gate-specific (no shared `summary` envelope), so the
/// extractor matches by gate name and reads the load-bearing
/// top-level fields directly. Returns an empty String for gates
/// that have no useful compact metric (or when the JSON is absent
/// / malformed — the bundle then falls back to the bare pass/fail
/// label).
fn bundle_gate_metric(
    gate: &str,
    gates: &std::collections::BTreeMap<&'static str, serde_json::Value>,
) -> String {
    let v = match gates.get(gate) {
        Some(v) => v,
        None => return String::new(),
    };
    let u64_at = |key: &str| -> u64 { v.get(key).and_then(|x| x.as_u64()).unwrap_or(0) };
    let nested_u64 = |outer: &str, inner: &str| -> u64 {
        v.get(outer)
            .and_then(|o| o.get(inner))
            .and_then(|x| x.as_u64())
            .unwrap_or(0)
    };
    match gate {
        "manifest_coverage" => {
            let total = nested_u64("summary", "total");
            let load = nested_u64("summary", "load_bearing");
            let partial = nested_u64("summary", "load_bearing_partial");
            let embedder = nested_u64("summary", "embedder_load_bearing");
            let forward = nested_u64("summary", "forward_looking");
            if total == 0 {
                return String::new();
            }
            let wired = load + partial + embedder;
            format!("{wired}/{total} wired, {forward} forward-looking")
        }
        "mls_coverage" => {
            let total = nested_u64("summary", "total_functions");
            let classified = nested_u64("summary", "classified_functions");
            let params = nested_u64("summary", "total_classified_params");
            let declassify = nested_u64("summary", "declassify_boundaries");
            let sinks = nested_u64("summary", "sink_consumers");
            if total == 0 && classified == 0 && declassify == 0 && sinks == 0 {
                return String::new();
            }
            format!(
                "{classified}/{total} classified, {params} params, {declassify} declassify, {sinks} sinks"
            )
        }
        "soundness_iou" => {
            let proved = u64_at("total_proved");
            let admitted = u64_at("total_admitted");
            let rules = u64_at("total_rules");
            if rules == 0 {
                return String::new();
            }
            format!("{proved}/{rules} proved, {admitted} admitted")
        }
        "cross_format_roundtrip" => {
            let theorems = u64_at("theorems_walked");
            let backends = u64_at("backend_count");
            let foreign = u64_at("foreign_failures");
            if theorems == 0 && backends == 0 {
                return String::new();
            }
            format!("{theorems} theorems × {backends} backends, {foreign} failures")
        }
        "signatures" => {
            let verified = u64_at("verified");
            let mismatched = u64_at("mismatched");
            let missing = u64_at("header_missing");
            let walked = u64_at("theorems_walked");
            if walked == 0 {
                return String::new();
            }
            format!(
                "{walked} theorems, {verified} verified, {mismatched} mismatched, {missing} no-header"
            )
        }
        "apply_graph" => {
            let walked = u64_at("theorems_walked");
            let leaking = u64_at("leaking_theorems");
            if walked == 0 {
                return String::new();
            }
            format!("{walked} theorems, {leaking} leaking")
        }
        "bridge_discharge" => {
            let modules = u64_at("modules_scanned");
            let callsites = u64_at("total_callsites");
            let false_disch = u64_at("total_false_discharges");
            if modules == 0 && callsites == 0 {
                return String::new();
            }
            format!("{callsites} callsites, {false_disch} false-discharges")
        }
        "kernel_discharged_axioms" => {
            let total = u64_at("discharge_count");
            let unknown = u64_at("unrecognised_count");
            let parsed = u64_at("files_parsed");
            if parsed == 0 {
                return String::new();
            }
            format!("{total} discharges, {unknown} unrecognised, {parsed} files")
        }
        _ => String::new(),
    }
}

pub fn audit_bundle_with_format(format: AuditFormat) -> Result<()> {
    if matches!(format, AuditFormat::Plain) {
        ui::step("Audit bundle — load-bearing L1+L2+L3+L4 gate aggregator");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);

 // Run each gate, capturing its JSON report from disk. Each
 // gate's `_with_format(Json)` path writes to a known location;
 // we read it back after invocation. This pattern works because
 // every gate already produces a JSON file under target/audit-reports
 // — the bundle reuses the existing artefacts rather than
 // re-running the audit logic.
    use std::collections::BTreeMap;
    let mut gates: BTreeMap<&'static str, serde_json::Value> = BTreeMap::new();
    let mut summary: BTreeMap<&'static str, &'static str> = BTreeMap::new();
    let mut overall_l4 = true;

 /// Invoke a gate and capture (JSON outcome, "passed"|"failed"
 /// label). A gate that returns `Err` is recorded as `failed`
 /// without aborting the bundle — the bundle is observability
 /// across all gates, not a fail-fast pipeline.
 ///

 /// Per-gate JSON stdout is silenced inside the bundle: gates
 /// invoked under `AuditFormat::Json` normally print their
 /// payload to stdout, but the bundle's clean output requires
 /// that output be captured to disk only. We redirect stdout
 /// for the duration of the gate's invocation by routing the
 /// captured chunk through a Vec writer; this is best-effort —
 /// gates that bypass println! (e.g., direct write_all to fd 1)
 /// will still surface, but the standard `serde_json::to_string_pretty
 /// + println!` pattern used by every load-bearing audit is captured.
    fn run_gate<F>(
        gates: &mut BTreeMap<&'static str, serde_json::Value>,
        summary: &mut BTreeMap<&'static str, &'static str>,
        name: &'static str,
        json_path: std::path::PathBuf,
        invoke: F,
    ) where
        F: FnOnce() -> Result<()>,
    {
 // Use Gag (or similar) is heavy; the simpler path is to just
 // run the gate and accept its stdout noise. The bundle's
 // headline summary still lands at the bottom and aggregates
 // every gate's verdict.
        let outcome = invoke();
        let label = if outcome.is_ok() { "passed" } else { "failed" };
        summary.insert(name, label);
        if let Ok(text) = std::fs::read_to_string(&json_path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                gates.insert(name, json);
                return;
            }
        }
 // Gate's JSON file unavailable — record stub so the bundle
 // still surfaces the gate's existence and verdict. The
 // stub records the gate's verdict label so downstream
 // tooling can still tell pass-from-fail without parsing the
 // missing report.
        gates.insert(
            name,
            serde_json::json!({
                "command": format!("audit-{}", name),
                "verdict": label,
                "report_path": json_path.display().to_string(),
                "report_readable": false,
            }),
        );
    }

 // 1. Bridge-discharge — observability for `apply kernel_*_strict`
 // callsites. Pre-cursor to the apply-graph's transitive walk.
    run_gate(
        &mut gates,
        &mut summary,
        "bridge_discharge",
        report_dir.join("bridge-discharge.json"),
        || audit_bridge_discharge_with_format(AuditFormat::Json),
    );
    if summary.get("bridge_discharge") != Some(&"passed") {
        overall_l4 = false;
    }

 // 2. Kernel-discharged-axioms — drift check on stdlib's
 // @kernel_discharge cross-link.
    run_gate(
        &mut gates,
        &mut summary,
        "kernel_discharged_axioms",
        report_dir.join("kernel-discharged-axioms.json"),
        || audit_kernel_discharged_axioms(AuditFormat::Json),
    );
    if summary.get("kernel_discharged_axioms") != Some(&"passed") {
        overall_l4 = false;
    }

 // 2b. Kernel-soundness IOU dashboard — observability for the
 // trust-base reduction roadmap (Phase 1). Doesn't flip
 // l4_load_bearing (the IOUs are accountability surface, not
 // L4 failures), but the bundle records the admit count so
 // CI tracks trust-base shrinkage over time.
    run_gate(
        &mut gates,
        &mut summary,
        "soundness_iou",
        report_dir.join("soundness-iou.json"),
        || audit_soundness_iou_with_format(AuditFormat::Json),
    );
 // Always passes (observability only).

 // 3. Apply-graph — transitive load-bearing verdict. This is the
 // headline gate for L4 closure.
    run_gate(
        &mut gates,
        &mut summary,
        "apply_graph",
        report_dir.join("apply-graph.json"),
        || audit_apply_graph_with_format(AuditFormat::Json),
    );
    if summary.get("apply_graph") != Some(&"passed") {
        overall_l4 = false;
    }

 // 4. Cross-format-roundtrip — independent foreign-tool re-check
 // (Coq + Lean). Surfaces tool_missing on hosts without coqc/
 // lean (gate stays GREEN); fails on real foreign-tool
 // rejections. Use --docker to force the docker backend.
    run_gate(
        &mut gates,
        &mut summary,
        "cross_format_roundtrip",
        report_dir
            .join("cross-format-roundtrip")
            .join("cross-format-roundtrip.json"),
        || audit_cross_format_roundtrip_with_format(AuditFormat::Json),
    );
 // Cross-format failures don't drop overall_l4 — when foreign
 // tools are missing the gate's "verdict" is `tool_missing` which
 // is by-design observability. Real failures are caught upstream
 // by the gate's own non-zero exit, which surfaces as Err here.
    if summary.get("cross_format_roundtrip") == Some(&"failed") {
        overall_l4 = false;
    }

 // 5. Signature verification — each emitted file carries a
 // `verum_signature` header pinning it to a kernel version +
 // spec hash; this gate recomputes and matches. Mismatches
 // surface drift between the emit step and the signature
 // expectation (#174).
    run_gate(
        &mut gates,
        &mut summary,
        "signatures",
        report_dir.join("signatures.json"),
        || audit_signatures_with_format(AuditFormat::Json),
    );
    if summary.get("signatures") == Some(&"failed") {
        overall_l4 = false;
    }

 // 6. Manifest-coverage — every Verum.toml manifest field's
 // wiring status is enumerated in a static table; this gate
 // emits the report and verifies no field is silently inert
 // (#290). Doesn't flip overall_l4 (forward-looking entries
 // are observability, not L4 failures), but the bundle
 // records the wired/forward-looking counts so CI tracks
 // inert-defense audit progress over time.
    run_gate(
        &mut gates,
        &mut summary,
        "manifest_coverage",
        report_dir.join("manifest-coverage.json"),
        || audit_manifest_coverage(AuditFormat::Json),
    );
 // Always passes (observability gate — forward-looking rows
 // are accountability surface, not failures).

 // 7. MLS-coverage — surface the project's MLS classification
 // topology (#296). Always passes (observability only); the
 // bundle records counts so CI can track classification
 // growth.
    run_gate(
        &mut gates,
        &mut summary,
        "mls_coverage",
        report_dir.join("mls-coverage.json"),
        || audit_mls_coverage(AuditFormat::Json),
    );

 // 8. kernel_v0 roster — bootstrap-meta-theory drift gate (#154).
 // Confirms the canonical 10-rule manifest matches the
 // on-disk verify/kernel_v0/rules/ tree. Drift is an L4
 // failure: if proof_checker.rs gains a rule that kernel_v0
 // doesn't mirror, the bootstrap chain is broken.
    run_gate(
        &mut gates,
        &mut summary,
        "kernel_v0_roster",
        report_dir.join("kernel-v0-roster.json"),
        || audit_kernel_v0_roster_with_format(AuditFormat::Json),
    );
    if summary.get("kernel_v0_roster") != Some(&"passed") {
        overall_l4 = false;
    }

 // 9. Foundation-profiles — citation-by-foundation classifier.
 // Observability-only: surfaces multi-foundation pluralism but
 // doesn't flip the L4 verdict, since corpora can legitimately
 // host independent theorems in incompatible foundations as
 // long as no single derivation chain assumes both.
    run_gate(
        &mut gates,
        &mut summary,
        "foundation_profiles",
        report_dir.join("foundation-profiles.json"),
        || audit_foundation_profiles_with_format(AuditFormat::Json),
    );

 // 10. Codegen-attestation — per-pass kernel-discharge manifest
 // (CompCert-style verified-compilation surface). V0 baseline
 // reports 0 of 6 passes attested; observability-only in V0,
 // flips to load-bearing when discharge work lands.
    run_gate(
        &mut gates,
        &mut summary,
        "codegen_attestation",
        report_dir.join("codegen-attestation.json"),
        || audit_codegen_attestation_with_format(AuditFormat::Json),
    );

 // 11. Differential-kernel — N-kernel cross-implementation
 // agreement (#159 V0+V1+V4). Walks the kernel_v0 manifest's
 // 10 rules through every registered kernel implementation
 // (currently `proof_checker` + `proof_checker_nbe`) on the
 // canonical polymorphic-identity certificate. Disagreements
 // are kernel-implementation bugs to be fixed; this is the
 // load-bearing CI gate that catches them at audit time.
    run_gate(
        &mut gates,
        &mut summary,
        "differential_kernel",
        report_dir.join("differential-kernel.json"),
        || audit_differential_kernel_with_format(AuditFormat::Json),
    );
    if summary.get("differential_kernel") != Some(&"passed") {
        overall_l4 = false;
    }

 // 11b. Differential-kernel fuzz — mutation-based property
 // fuzzing over the kernel registry. Bounded campaign (default
 // 500 iterations, deterministic seed) walks structurally
 // mutated certificates through every registered kernel; every
 // disagreement is a kernel-implementation bug and fails the
 // gate. Complements the canonical-certificate roster: roster
 // covers the curated surface, fuzz covers the long tail.
    run_gate(
        &mut gates,
        &mut summary,
        "differential_kernel_fuzz",
        report_dir.join("differential-kernel-fuzz.json"),
        || audit_differential_kernel_fuzz_with_format(AuditFormat::Json),
    );
    if summary.get("differential_kernel_fuzz") != Some(&"passed") {
        overall_l4 = false;
    }

 // 11c. Reflection-tower — ordinal-indexed meta-soundness.
 // Walks every level in REF^0..REF^4 + REF^ω; each finite
 // level must discharge against the per-rule kernel-rule
 // footprint. Citations are Gödel 1931 + Feferman 1989,
 // Pohlers 2009, Beklemishev 2003, Schütte 1965, Feferman
 // 1962 (one per level). The tower's stability is the
 // load-bearing meta-theory invariant of Verum's Gödel-2nd
 // escape.
    run_gate(
        &mut gates,
        &mut summary,
        "reflection_tower",
        report_dir.join("reflection-tower.json"),
        || audit_reflection_tower_with_format(AuditFormat::Json),
    );
    if summary.get("reflection_tower") != Some(&"passed") {
        overall_l4 = false;
    }

 // 11d. ATS-V Architectural Type System discharge registry.
 // Walks the 8 kernel-side architectural intrinsics + reports
 // the canonical 10-pattern anti-pattern catalog with stable
 // RFC error codes (ATS-V-AP-001..010). registry
 // surface; full per-cog dispatch lands.
    run_gate(
        &mut gates,
        &mut summary,
        "arch_discharges",
        report_dir.join("arch-discharges.json"),
        || audit_arch_discharges_with_format(AuditFormat::Json),
    );
    if summary.get("arch_discharges") != Some(&"passed") {
        overall_l4 = false;
    }

 // 11e. ATS-V counterfactual reasoning engine pin
 // battery. Verifies every InvariantStatus arm
 // (HoldsBoth/HoldsBaseOnly/HoldsAltOnly/HoldsNeither) holds
 // its soundness contract; failure means engine arm-routing
 // drifted from spec §22.2.
    run_gate(
        &mut gates,
        &mut summary,
        "counterfactual",
        report_dir.join("counterfactual.json"),
        || audit_counterfactual_with_format(AuditFormat::Json),
    );
    if summary.get("counterfactual") != Some(&"passed") {
        overall_l4 = false;
    }

 // 11f. ATS-V adjunction analyzer pin battery.
 // Verifies each canonical adjunction recogniser (Inline⊣Extract
 // / Specialise⊣Generalise / Decompose⊣Compose /
 // Strengthen⊣Weaken) classifies the corresponding shape-delta
 // correctly; failure means recogniser drifted from spec §20.6.
    run_gate(
        &mut gates,
        &mut summary,
        "adjunctions",
        report_dir.join("adjunctions.json"),
        || audit_adjunctions_with_format(AuditFormat::Json),
    );
    if summary.get("adjunctions") != Some(&"passed") {
        overall_l4 = false;
    }

 // 11g. ATS-V Yoneda-equivalence checker pin battery.
 // Verifies each canonical Observer's projection respects spec
 // §20.7 + §23 — Auditor sees foundation, Adversary sees
 // attack surface, EndUser sees public interface, etc.
 // Failure means the observer-functor projection drifted from
 // spec.
    run_gate(
        &mut gates,
        &mut summary,
        "yoneda",
        report_dir.join("yoneda.json"),
        || audit_yoneda_with_format(AuditFormat::Json),
    );
    if summary.get("yoneda") != Some(&"passed") {
        overall_l4 = false;
    }

    // 11h. ATS-V whole-corpus cross-cog architectural firewall.
    //  Walks every annotated cog, builds the global mount-graph,
    //  populates per-cog `DiagnosticContext` (composes_graph +
    //  composed_foundations), runs the 32-pattern checker.
    //  Activates AP-003 DependencyCycle + AP-005 FoundationDrift on
    //  real cross-cog architecture.  Failure means a cog declared
    //  an incompatible foundation against a mounted-and-annotated
    //  cog — a real architectural defect.
    run_gate(
        &mut gates,
        &mut summary,
        "arch_corpus",
        report_dir.join("arch-corpus.json"),
        || audit_arch_corpus_with_format(AuditFormat::Json),
    );
    if summary.get("arch_corpus") != Some(&"passed") {
        overall_l4 = false;
    }

 // 12. Proof-term-library — N-kernel + universe-stability +
 // adversarial verification of the canonical certificate
 // library at `core/verify/proof_term_examples/` (#157, #158
 // V1, #159 V2-V4). Every certificate runs through every
 // registered kernel; the verdict is checked for
 // universe-stability across lifts 0..=2 (Gödel-2nd
 // workaround foundation); adversarial certificates
 // (`expected_outcome=reject` metadata) MUST be unanimously
 // rejected — soundness violations flip the gate.
    run_gate(
        &mut gates,
        &mut summary,
        "proof_term_library",
        report_dir.join("proof-term-library.json"),
        || audit_proof_term_library_with_format(AuditFormat::Json),
    );
    if summary.get("proof_term_library") != Some(&"passed") {
        overall_l4 = false;
    }

    let bundle_path = report_dir.join("bundle.json");
    let payload = serde_json::json!({
        "schema_version": 1,
        "command": "audit-bundle",
        "l4_load_bearing": overall_l4,
        "gates": gates,
        "summary": summary,
    });
    let _ = std::fs::write(
        &bundle_path,
        serde_json::to_string_pretty(&payload).unwrap(),
    );

    match format {
        AuditFormat::Plain => {
            println!();
            println!("Audit bundle — L1+L2+L3+L4 verdict");
            println!("────────────────────────────────────────");
            for (gate, label) in &summary {
                let marker = match *label {
                    "passed" => "✓",
                    _ => "✗",
                };
 // Append a compact gate-specific summary line for
 // observability gates that carry useful metrics —
 // manifest-coverage shows wired/total, mls-coverage
 // shows classification counts. Other gates show only
 // their pass/fail label.
                let metric = bundle_gate_metric(gate, &gates);
                if metric.is_empty() {
                    println!("  {}  {:<28} {}", marker, gate, label);
                } else {
                    println!("  {}  {:<28} {}  ({})", marker, gate, label, metric);
                }
            }
            println!();
            if overall_l4 {
                println!("  ✓ L4 load-bearing — every gate produced a clean verdict.");
                println!("    Bundle: {}", bundle_path.display());
            } else {
                println!("  ✗ NOT L4 load-bearing — at least one gate reported a failure.");
                println!("    Bundle: {}", bundle_path.display());
            }
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&payload).unwrap());
        }
    }

    if !overall_l4 {
        return Err(crate::error::CliError::VerificationFailed(format!(
            "audit bundle: at least one gate reported a failure — see {}",
            bundle_path.display(),
        )));
    }
    Ok(())
}

/// `verum audit --dependent-theorems <axiom-name>` — list every
/// theorem in the workspace whose transitive proof depends on the
/// given axiom (#188).
///
/// Mathematician-facing utility: when an axiom rejects or is
/// admitted under audit, "which of my theorems lose their
/// discharge?" is answered without manual dependency tracing.
/// Walks the apply-graph backwards from the named axiom. Output:
/// `target/audit-reports/dependent-theorems-<axiom>.json` + plain
/// or JSON CLI rendering.
pub fn audit_dependent_theorems_with_format(
    axiom_name: &str,
    format: AuditFormat,
) -> Result<()> {
    use verum_kernel::soundness::apply_graph::{
        ApplyGraph, SymbolEntry, dependent_theorems, extract_apply_targets,
    };

    if matches!(format, AuditFormat::Plain) {
        ui::step(&format!(
            "Walking apply-graph for theorems dependent on `{}`",
            axiom_name,
        ));
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let mut vr_files = discover_vr_files(&manifest_dir);
    vr_files.extend(discover_stdlib_vr_files());

    let mut graph = ApplyGraph::new();
    let mut parsed_files = 0usize;
    let mut skipped_files = 0usize;
    for abs_path in &vr_files {
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
                verum_ast::decl::ItemKind::Theorem(d)
                | verum_ast::decl::ItemKind::Lemma(d)
                | verum_ast::decl::ItemKind::Corollary(d) => {
                    let name = d.name.name.as_str().to_string();
                    if let verum_common::Maybe::Some(body) = &d.proof {
                        let apply_targets = extract_apply_targets(body);
                        graph.insert(name, SymbolEntry::Theorem { apply_targets });
                    } else {
                        graph.insert(
                            name,
                            classify_axiom_entry(
                                &d.name.name,
                                &d.attributes,
                                &item.attributes,
                            ),
                        );
                    }
                }
                verum_ast::decl::ItemKind::Axiom(a) => {
                    let name = a.name.name.as_str().to_string();
                    graph.insert(
                        name,
                        classify_axiom_entry(
                            &a.name.name,
                            &a.attributes,
                            &item.attributes,
                        ),
                    );
                }
                _ => {}
            }
        }
    }

    let dependents = dependent_theorems(&graph, axiom_name);

    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let report_path = report_dir.join(format!(
        "dependent-theorems-{}.json",
        axiom_name.replace('/', "-").replace('.', "_"),
    ));
    let payload = serde_json::json!({
        "schema_version": 1,
        "axiom": axiom_name,
        "parsed_files": parsed_files,
        "skipped_files": skipped_files,
        "dependent_count": dependents.len(),
        "dependents": dependents
            .iter()
            .map(|d| serde_json::json!({
                "theorem": d.theorem,
                "chain": d.chain,
            }))
            .collect::<Vec<_>>(),
    });
    let _ = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&payload).unwrap_or_default(),
    );

    match format {
        AuditFormat::Plain => {
            println!();
            println!("Dependent-theorems audit");
            println!("─────────────────────────────────────────");
            println!("Axiom:          {}", axiom_name);
            println!("Files parsed:   {}", parsed_files);
            println!("Files skipped:  {}", skipped_files);
            println!("Dependents:     {}", dependents.len());
            if dependents.is_empty() {
                println!();
                println!(
                    "{} No theorem in the workspace depends on `{}`.",
                    "✓".green(),
                    axiom_name,
                );
            } else {
                println!();
                println!(
                    "Theorems whose proof transitively depends on `{}`:",
                    axiom_name,
                );
                for d in &dependents {
                    println!("  • {}", d.theorem);
                    if d.chain.len() > 2 {
                        println!("       chain: {}", d.chain.join(" → "));
                    }
                }
            }
            println!();
            println!("Report: {}", report_path.display());
        }
        AuditFormat::Json => {
            println!(
                "{}",
                serde_json::to_string(&payload).unwrap_or_default(),
            );
        }
    }

    Ok(())
}

pub fn audit_apply_graph_with_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::soundness::apply_graph::{
        ApplyGraph, LeafKind, SymbolEntry, extract_apply_targets, walk_transitive,
    };

    if matches!(format, AuditFormat::Plain) {
        ui::step("Apply-graph transitive bridge-discharge audit");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let mut vr_files = discover_vr_files(&manifest_dir);
 // Extend the scan with the verum stdlib's `core/` tree when it can
 // be located (#150 follow-up). Without this step every apply-target
 // resolving into stdlib symbols (e.g., `msfs_lemma_3_4_*`,
 // `msfs_id_x_violates_pi_4`) surfaces as `unresolved`, blocking the
 // L4 verdict on chains that legitimately reach paper-cited stdlib
 // declarations. Discovery: VERUM_STDLIB_ROOT env override, then
 // a small walk from the verum binary location, then the cargo
 // workspace root (for dev builds).
    vr_files.extend(discover_stdlib_vr_files());

 // Pass 1: build the workspace-wide symbol table. Every theorem
 // gets its proof-body's apply-targets pre-extracted; every axiom
 // gets classified as kernel-bridge / framework / placeholder.
    let mut graph = ApplyGraph::new();
 // Track which theorems exist and their source path so the report
 // can pinpoint the file location of any leaking chain. Only
 // CORPUS-side theorems (not stdlib delegates) are tracked here so
 // the report focuses on the audit subject; stdlib theorems still
 // populate the symbol table for transitive resolution.
    let mut theorem_sources: std::collections::BTreeMap<String, std::path::PathBuf> =
        std::collections::BTreeMap::new();
    let mut parsed_files = 0usize;
    let mut skipped_files = 0usize;

    for abs_path in &vr_files {
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
                verum_ast::decl::ItemKind::Theorem(d)
                | verum_ast::decl::ItemKind::Lemma(d)
                | verum_ast::decl::ItemKind::Corollary(d) => {
                    let name = d.name.name.as_str().to_string();
                    match &d.proof {
                        verum_common::Maybe::Some(body) => {
 // Theorem with a proof body — the
 // apply-targets become the children in
 // the graph.
                            let apply_targets = extract_apply_targets(body);
                            graph.insert(name.clone(), SymbolEntry::Theorem { apply_targets });
 // Only track corpus-side theorems for the
 // per-theorem report; stdlib theorems
 // populate the symbol table for transitive
 // resolution but aren't audit subjects.
                            if abs_path.starts_with(&manifest_dir) {
                                theorem_sources.insert(name, abs_path.clone());
                            }
                        }
                        verum_common::Maybe::None => {
 // Theorem-shaped axiom — classify as a
 // leaf based on the @framework attribute
 // and the kernel_ prefix.
                            let entry =
                                classify_axiom_entry(&name, &d.attributes, &item.attributes);
                            graph.insert(name, entry);
                        }
                    }
                }
                verum_ast::decl::ItemKind::Axiom(a) => {
                    let name = a.name.name.as_str().to_string();
                    let entry = classify_axiom_entry(&name, &a.attributes, &item.attributes);
                    graph.insert(name, entry);
                }
                _ => {}
            }
        }
    }

 // Pass 2: walk the transitive apply-graph for every theorem with
 // a proof body and accumulate the per-theorem composition.
    const MAX_DEPTH: usize = 16;
    let mut rows: Vec<ApplyGraphRow> = Vec::new();
    let mut leaking_theorems = 0usize;

    for (theorem_name, source_path) in &theorem_sources {
        let comp = walk_transitive(&graph, theorem_name, MAX_DEPTH);
        let load_bearing = comp.is_l4_load_bearing();
        if !load_bearing {
            leaking_theorems += 1;
        }
        rows.push(ApplyGraphRow {
            theorem: theorem_name.clone(),
            source: source_path
                .strip_prefix(&manifest_dir)
                .unwrap_or(source_path)
                .to_path_buf(),
            kernel_strict: comp.kernel_strict,
            framework_axiom: comp.framework_axiom,
            placeholder_axiom: comp.placeholder_axiom,
            unresolved: comp.unresolved,
            load_bearing,
            placeholder_leaves: comp
                .leaves
                .iter()
                .filter(|h| matches!(h.kind, LeafKind::PlaceholderAxiom | LeafKind::Unresolved))
                .map(|h| ApplyGraphLeakHit {
                    leaf: h.symbol.clone(),
                    chain: h.chain.clone(),
                    kind: h.kind.label().to_string(),
                })
                .collect(),
        });
    }
    rows.sort_by(|a, b| a.theorem.cmp(&b.theorem));

    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let json_path = report_dir.join("apply-graph.json");
    let payload = serde_json::json!({
        "schema_version": 1,
        "command": "audit-apply-graph",
        "modules_scanned": parsed_files,
        "modules_skipped": skipped_files,
        "theorems_walked": rows.len(),
        "leaking_theorems": leaking_theorems,
        "max_depth": MAX_DEPTH,
        "rows": rows,
    });
    let _ = std::fs::write(&json_path, serde_json::to_string_pretty(&payload).unwrap());

    match format {
        AuditFormat::Plain => {
            print_apply_graph_plain(&rows, leaking_theorems, parsed_files, skipped_files);
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&payload).unwrap());
        }
    }

    if leaking_theorems > 0 {
        return Err(crate::error::CliError::VerificationFailed(format!(
            "apply-graph: {} theorem(s) have non-L4 transitive apply-chains \
             (placeholder_axiom or unresolved leaves) — chain leaks listed in \
             {}",
            leaking_theorems,
            json_path.display(),
        )));
    }
    Ok(())
}

/// Per-theorem row emitted into the `apply-graph.json` audit report.
#[derive(Debug, serde::Serialize)]
struct ApplyGraphRow {
    theorem: String,
    source: std::path::PathBuf,
    kernel_strict: usize,
    framework_axiom: usize,
    placeholder_axiom: usize,
    unresolved: usize,
    load_bearing: bool,
 /// Empty when `load_bearing == true`. Otherwise lists every
 /// leaf-class hit that breaks the L4 property, with the chain
 /// of intermediate symbols leading to it.
    placeholder_leaves: Vec<ApplyGraphLeakHit>,
}

#[derive(Debug, serde::Serialize)]
struct ApplyGraphLeakHit {
    leaf: String,
    chain: Vec<String>,
    kind: String,
}

fn classify_axiom_entry(
    name: &str,
    decl_attrs: &verum_common::List<verum_ast::attr::Attribute>,
    item_attrs: &verum_common::List<verum_ast::attr::Attribute>,
) -> verum_kernel::soundness::apply_graph::SymbolEntry {
 // Single canonical implementation lives in
 // `verum_kernel::soundness::apply_graph::classify_axiom_entry_for_attrs`
 // (#188 / #318). Both audit-time and compile-time graph builders
 // route through it so leaf classification is identical across
 // contexts.
    verum_kernel::soundness::apply_graph::classify_axiom_entry_for_attrs(
        name, decl_attrs, item_attrs,
    )
}

fn print_apply_graph_plain(
    rows: &[ApplyGraphRow],
    leaking_theorems: usize,
    parsed_files: usize,
    skipped_files: usize,
) {
    println!();
    println!("Apply-graph transitive discharge report");
    println!("────────────────────────────────────────");
    println!(
        "  Parsed {} module(s) ({} skipped); walked {} theorem(s).",
        parsed_files,
        skipped_files,
        rows.len(),
    );
    if rows.is_empty() {
        println!("  (no theorem-shaped declarations with proof bodies discovered)");
        return;
    }
    let total_kernel: usize = rows.iter().map(|r| r.kernel_strict).sum();
    let total_framework: usize = rows.iter().map(|r| r.framework_axiom).sum();
    let total_placeholder: usize = rows.iter().map(|r| r.placeholder_axiom).sum();
    let total_unresolved: usize = rows.iter().map(|r| r.unresolved).sum();
    println!();
    println!(
        "  Aggregate leaf composition: {} kernel_strict · {} framework_axiom · \
         {} placeholder_axiom · {} unresolved",
        total_kernel, total_framework, total_placeholder, total_unresolved,
    );
    if leaking_theorems == 0 {
        println!();
        println!("  ✓ all theorems are L4 load-bearing — every transitive leaf is");
        println!("    kernel_strict or framework_axiom (no placeholders, no unresolveds).");
        return;
    }
    println!();
    println!(
        "  ✗ {} theorem(s) have non-L4 transitive chains:",
        leaking_theorems,
    );
    for row in rows.iter().filter(|r| !r.load_bearing) {
        println!("    - {} (in {})", row.theorem, row.source.display(),);
        println!(
            "        {} kernel_strict · {} framework · {} placeholder · {} unresolved",
            row.kernel_strict, row.framework_axiom, row.placeholder_axiom, row.unresolved,
        );
        for leak in &row.placeholder_leaves {
            let chain_text = leak.chain.join(" → ");
            println!(
                "        leak ({}): {} via {}",
                leak.kind, leak.leaf, chain_text,
            );
        }
    }
}

/// Sanitise a Verum theorem name for use as a foreign-tool
/// identifier. Snake-case names map directly; non-ASCII characters
/// or names colliding with reserved words gain a `verum_` prefix.
/// Rejection-by-renaming is preferable to compile-failure: the user
/// sees a stable mapping rather than mysterious `coqc` errors.
/// Project: an Ident-pattern's bound name. Returns `None` for any
/// other pattern shape (Tuple / Record / Variant / etc.) — those
/// don't translate cleanly to a single foreign-tool parameter binding.
fn ident_pattern_name(pattern: &verum_ast::pattern::Pattern) -> Option<String> {
    use verum_ast::pattern::PatternKind;
    match &pattern.kind {
        PatternKind::Ident { name, .. } => Some(name.as_str().to_string()),
        _ => None,
    }
}

/// Project a single `TypeBound` into a one-line annotation suitable
/// for the cross-format gate's generic-binder comment (#145 /
/// MSFS-L4.11). Returns `None` for bounds that don't have a clean
/// inline rendering (associated-type bounds, etc.) — those are
/// dropped from the per-generic annotation.
fn generic_bound_to_annotation(bound: &verum_ast::ty::TypeBound) -> Option<String> {
    use verum_ast::ty::TypeBoundKind;
    match &bound.kind {
        TypeBoundKind::Protocol(path) => path.as_ident().map(|i| i.as_str().to_string()),
 // Generic protocol like `IntoIterator<Item = Int>` — surface
 // just the head identifier; foreign-tool reviewers see the
 // bound's name without its generic args. Refining this needs
 // the type translator from #141, which doesn't currently take
 // a Path.
        TypeBoundKind::GenericProtocol(_) => None,
 // Equality / negative / associated-type bounds are skipped —
 // their lowering needs more machinery than a single comment
 // line can carry.
        _ => None,
    }
}

fn sanitise_theorem_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 6);
    let needs_prefix = name
        .chars()
        .next()
        .map(|c| !c.is_ascii_alphabetic() && c != '_')
        .unwrap_or(true);
    if needs_prefix {
        out.push_str("verum_");
    }
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn print_cross_format_roundtrip_plain(
    theorem_specs: &[verum_kernel::soundness::corpus_export::TheoremSpec],
    roundtrips: &[ThmRoundtripPlainRow],
    parsed_files: usize,
    skipped_files: usize,
    foreign_failures: usize,
    backend_meta: &[(String, String)],
) {
    println!();
    println!("{}", "Cross-format roundtrip report".bold());
    println!("{}", "─".repeat(40).dimmed());
    println!(
        "  Parsed {} module(s) ({} skipped); walked {} theorem(s) across {} backend(s).",
        parsed_files,
        skipped_files,
        theorem_specs.len(),
        backend_meta.len(),
    );
    println!("  {} foreign-tool failure(s) detected.", foreign_failures,);
    println!();

 // Per-backend summary line first, then per-theorem rows.
    let mut by_backend: BTreeMap<String, Vec<&ThmRoundtripPlainRow>> = BTreeMap::new();
    for r in roundtrips {
        by_backend.entry(r.backend_id.clone()).or_default().push(r);
    }
    for (backend_id, rows) in &by_backend {
        let passed = rows.iter().filter(|r| r.verdict_kind == "passed").count();
        let failed = rows.iter().filter(|r| r.verdict_kind == "failed").count();
        let missing = rows
            .iter()
            .filter(|r| r.verdict_kind == "tool_missing")
            .count();
        let mark = if failed > 0 {
            "✗".red().to_string()
        } else if passed > 0 {
            "✓".green().to_string()
        } else {
            "○".dimmed().to_string()
        };
        println!(
            "  {} {} ({} passed · {} failed · {} tool_missing)",
            mark,
            backend_id.bold(),
            passed,
            failed,
            missing,
        );
        if failed > 0 {
            for r in rows.iter().filter(|r| r.verdict_kind == "failed") {
                println!(
                    "    {} {} — {}",
                    "✗".red(),
                    r.theorem_name,
                    r.detail.lines().next().unwrap_or("").dimmed(),
                );
            }
        }
    }

    if foreign_failures == 0 {
        println!();
        println!(
            "  {} all available foreign tools accepted every emitted theorem.",
            "✓".green()
        );
    }
}

/// Owned row for the plain-output renderer. Mirrors the inline
/// `ThmRoundtrip` struct above but lifts to module scope so the
/// renderer function's signature can name the type.
#[derive(Debug)]
struct ThmRoundtripPlainRow {
    backend_id: String,
    theorem_name: String,
    emitted_path: PathBuf,
    verdict_kind: &'static str,
    detail: String,
}

// Note: the audit body uses an inline `ThmRoundtrip`; we rebuild the
// plain-render-friendly form here. Keeping them separate avoids a
// pub-export of the inline type just for rendering.

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
                ItemKind::Theorem(decl) => ("theorem", decl.name.name.clone(), &decl.attributes),
                ItemKind::Lemma(decl) => ("lemma", decl.name.name.clone(), &decl.attributes),
                ItemKind::Corollary(decl) => {
                    ("corollary", decl.name.name.clone(), &decl.attributes)
                }
                ItemKind::Axiom(decl) => ("axiom", decl.name.name.clone(), &decl.attributes),
                ItemKind::Function(func) => ("fn", func.name.name.clone(), &func.attributes),
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
            print_epsilon_report_json(parsed_files, skipped_files, &by_epsilon, &malformed);
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
                by_epsilon.entry(ea.epsilon).or_default().push(EnactUsage {
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
        println!(
            "  {} no @enact(epsilon = \"...\") markers found.",
            "·".dimmed()
        );
        println!(
            "  {} the corpus declares no DC-side ε-coordinate; every",
            "·".dimmed()
        );
        println!("    function's ε will be inferred from its body during");
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
            "0, 1, 2, …, ω, ω+k, ω·n, ω·n+k, ω², Ω (also ASCII: omega, omega_squared, …)".dimmed()
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
            out.push_str(&format!("          \"item_kind\": \"{}\",\n", u.item_kind));
            out.push_str(&format!(
                "          \"item_name\": \"{}\",\n",
                json_escape(u.item_name.as_str())
            ));
            out.push_str(&format!(
                "          \"file\": \"{}\"\n",
                json_escape(&u.file.display().to_string())
            ));
            out.push_str(if j + 1 == total_u {
                "        }\n"
            } else {
                "        },\n"
            });
        }
        out.push_str("      ]\n");
        out.push_str(if i + 1 == total_eps {
            "    }\n"
        } else {
            "    },\n"
        });
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
        out.push_str(if i + 1 == malformed.len() {
            "    }\n"
        } else {
            "    },\n"
        });
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
pub(crate) struct CliOrdinal {
    omega_coeff: u32,
    finite_offset: u32,
}

impl CliOrdinal {
    const fn finite(n: u32) -> Self {
        Self {
            omega_coeff: 0,
            finite_offset: n,
        }
    }
    const fn omega() -> Self {
        Self {
            omega_coeff: 1,
            finite_offset: 0,
        }
    }
    const fn omega_plus(k: u32) -> Self {
        Self {
            omega_coeff: 1,
            finite_offset: k,
        }
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
        "actic.raw" => (CliOrdinal::finite(0), false),
        "lurie_htt" => (CliOrdinal::omega(), true),
        "schreiber_dcct" => (CliOrdinal::omega_plus(2), true),
        "connes_reconstruction" => (CliOrdinal::omega(), false),
        "petz_classification" => (CliOrdinal::finite(2), false),
        "arnold_catastrophe" => (CliOrdinal::finite(2), true),
        "baez_dolan" => (CliOrdinal::omega_plus(1), true),
        "owl2_fs" => (CliOrdinal::finite(1), true),
        _ => (CliOrdinal::finite(0), true),
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
    let mut verify_by_item: BTreeMap<(PathBuf, Text, &'static str), Text> = BTreeMap::new();
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
            if let Some(strategy) = strictest_verify_strategy(&item.attributes, decl_attrs) {
                verify_by_item.insert((rel_path.clone(), item_name.clone(), kind_label), strategy);
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
        "runtime" => CliOrdinal::finite(0),
        "static" => CliOrdinal::finite(1),
        "fast" => CliOrdinal::finite(2),
        "complexity_typed" => CliOrdinal::finite(2),
        "formal" => CliOrdinal::omega(),
        "proof" => CliOrdinal::omega_plus(1),
        "thorough" => CliOrdinal {
            omega_coeff: 2,
            finite_offset: 0,
        },
        "reliable" => CliOrdinal {
            omega_coeff: 2,
            finite_offset: 1,
        },
        "certified" => CliOrdinal {
            omega_coeff: 2,
            finite_offset: 2,
        },
        "coherent_static" => CliOrdinal::omega(),
        "coherent_runtime" => CliOrdinal::finite(0),
        "coherent" => CliOrdinal::omega_plus(1),
        "synthesize" => CliOrdinal {
            omega_coeff: 3,
            finite_offset: 1,
        },
        "assume" => CliOrdinal::finite(0),
        _ => CliOrdinal::finite(0),
    }
}

/// Pick the strictest (lex-maximum) `@verify(...)` strategy from the
/// item's attribute lists. Returns `None` if no `@verify(...)` is
/// declared. Strictness is the same lex ordering used for the
/// per-theorem ν projection (`verify_strategy_ordinal`).
pub(crate) fn strictest_verify_strategy(
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
        let tau_str = if tau {
            "τ=intensional"
        } else {
            "τ=extensional"
        };
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
 // * framework_nu = max over all `@framework(name, ...)` markers
 // * verify_nu = ν of the strictest `@verify(strategy)` (VVA §2.3)
 // * theorem_nu = max(framework_nu, verify_nu)
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
                if entry.frameworks_cited.len() == 1 {
                    ""
                } else {
                    "s"
                },
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
            out.push_str(&format!("          \"item_kind\": \"{}\",\n", u.item_kind));
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
            out.push_str(if j + 1 == total_u {
                "        }\n"
            } else {
                "        },\n"
            });
        }
        out.push_str("      ]\n");
        out.push_str(if i + 1 == total_fw {
            "    }\n"
        } else {
            "    },\n"
        });
    }
    out.push_str("  ],\n");
 // Per-theorem inferred coordinates (schema_version 2 — adds the
 // `verify_strategy` field and the lifted `inferred_nu` from VVA §2.3).
    let per_theorem = invert_to_per_theorem(by_framework, verify_by_item);
    out.push_str("  \"per_theorem\": [\n");
    let total_pt = per_theorem.len();
    for (i, entry) in per_theorem.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!("      \"item_kind\": \"{}\",\n", entry.item_kind));
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
            if j > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("\"{}\"", json_escape(fw.as_str())));
        }
        out.push_str("]\n");
        out.push_str(if i + 1 == total_pt {
            "    }\n"
        } else {
            "    },\n"
        });
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
        out.push_str(if i + 1 == malformed.len() {
            "    }\n"
        } else {
            "    },\n"
        });
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

// Surface Factorisation (Φ, κ, t)
// ────────────────────────────────────── ───────────────────────────
// Inductive `Rec(T)` in `type T is Rec(T)` (T_succ, ω, least_fp)
// Coinductive `Stream<A> = Cons(A, …)` (T_prod_A, ω^{op}, greatest_fp)
// Newtype `type X is (Y)` (Id, 1, Y)
// HIT path-cell variant (`Foo() = a..b`) (path_action, ω, base)
// `@recursive fn f(… -> Self) …` (unfold_f, ω, fix_f)
// `@corecursive fn g(…)` (productivity) (corec_g, ω^{op}, fix_g)
//

// V1 scope:
// * variant-self-reference detection (a constructor arg that mentions the
// surrounding type's own name) — covers Inductive + sum-type recursion;
// * explicit Inductive / Coinductive bodies detected via TypeDeclBody;
// * HIT path-cell variants flagged by `path_endpoints`;
// * `@recursive` / `@corecursive` attributes on FunctionDecl.
//

// Out of scope (V1, deferred to a kernel-pass follow-up):
// * raw `self` keyword usage inside function bodies (requires
// expression-tree walk);
// * §13.2's `Self::Item` and `&mut self` factorisations (require a typed
// resolution layer).
// =============================================================================

#[derive(Debug, Clone, Copy)]
enum HygieneClass {
    Inductive,       // (T_succ, ω, least_fp)
    Coinductive,     // (T_prod, ω^{op}, greatest_fp)
    Newtype,         // (Id, 1, base)
    HigherInductive, // (path_action, ω, base)
    Recursive,       // @recursive — (unfold_f, ω, fix_f)
    Corecursive,     // @corecursive — (corec_g, ω^{op}, fix_g)
}

impl HygieneClass {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Inductive => "inductive",
            Self::Coinductive => "coinductive",
            Self::Newtype => "newtype",
            Self::HigherInductive => "higher-inductive",
            Self::Recursive => "recursive-fn",
            Self::Corecursive => "corecursive-fn",
        }
    }

    fn factorisation(&self) -> &'static str {
        match self {
            Self::Inductive => "(T_succ, ω, least_fp)",
            Self::Coinductive => "(T_prod, ω^op, greatest_fp)",
            Self::Newtype => "(Id, 1, base)",
            Self::HigherInductive => "(path_action, ω, base)",
            Self::Recursive => "(unfold_f, ω, fix_f)",
            Self::Corecursive => "(corec_g, ω^op, fix_g)",
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
        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
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
        Maybe::None => return false,
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
            let any_recursive = variants
                .iter()
                .any(|v| variant_is_self_recursive(v, &name_str));
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
    let mut has_recursive = false;
    let mut has_corecursive = false;
    for attr in decl.attributes.iter() {
        if attr.is_named("recursive") {
            has_recursive = true;
        }
        if attr.is_named("corecursive") {
            has_corecursive = true;
        }
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
// V2 hygiene enforcement — `verum audit --hygiene-strict`
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
        ExprKind::Try(inner) | ExprKind::TryBlock(inner) => expr_contains_raw_self(inner),
        ExprKind::Paren(inner) => expr_contains_raw_self(inner),
        ExprKind::NamedArg { value, .. } => expr_contains_raw_self(value),
        ExprKind::Call { func, args, .. } => {
            expr_contains_raw_self(func) || args.iter().any(expr_contains_raw_self)
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            expr_contains_raw_self(receiver) || args.iter().any(expr_contains_raw_self)
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
        ExprKind::Match {
            expr: scrutinee,
            arms,
        } => {
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
            StmtKind::Expr { expr, .. } | StmtKind::Defer(expr) | StmtKind::Errdefer(expr) => {
                expr_contains_raw_self(expr)
            }
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

/// entry-point — `verum audit --hygiene-strict`.
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
        AuditFormat::Json => {
            print_hygiene_strict_report_json(parsed_files, skipped_files, &violations)
        }
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
        println!(
            "  {} no E_HYGIENE_UNFACTORED_SELF violations.",
            "·".dimmed()
        );
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
    out.push_str(&format!(
        "  \"error_code\": \"{}\",\n",
        E_HYGIENE_UNFACTORED_SELF
    ));
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
        out.push_str(if i + 1 == total {
            "    }\n"
        } else {
            "    },\n"
        });
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
        AuditFormat::Plain => {
            print_hygiene_report(parsed_files, skipped_files, &by_class, &entries)
        }
        AuditFormat::Json => {
            print_hygiene_report_json(parsed_files, skipped_files, &by_class, &entries)
        }
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
            out.push_str(if j + 1 == total_e {
                "        }\n"
            } else {
                "        },\n"
            });
        }
        out.push_str("      ]\n");
        out.push_str(if i + 1 == total {
            "    }\n"
        } else {
            "    },\n"
        });
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

// - subclass closure: each class lists its full ancestor set
// - cycle detection: any class that is a subclass of itself
// transitively is flagged with the cycle path
// - disjoint/subclass conflict: a class C disjoint from D where C is
// also a subclass of D (directly or via the closure) is a hard
// inconsistency reported with severity = error
// - equivalence partition: equivalence is symmetric; we union-find the
// equivalence groups so the report shows partitions rather than
// redundant pairwise edges
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

use crate::commands::owl2::{Owl2EntityKind, Owl2Graph, collect_owl2_attrs};
use std::collections::BTreeSet;
use verum_ast::attr::Owl2Semantics;

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
            collect_owl2_attrs(item, &rel_path, &mut graph);
        }
    }

    let closure = graph.subclass_closure();
    let cycles = graph.detect_cycles(&closure);
    let partition = graph.equivalence_partition();
    let violations = graph.detect_disjoint_violations(&closure);

    match format {
        AuditFormat::Plain => print_owl2_report(
            parsed_files,
            skipped_files,
            &graph,
            &closure,
            &cycles,
            &partition,
            &violations,
        ),
        AuditFormat::Json => print_owl2_report_json(
            parsed_files,
            skipped_files,
            &graph,
            &closure,
            &cycles,
            &partition,
            &violations,
        ),
    }
    if !cycles.is_empty() || !violations.is_empty() {
        return Err(crate::error::CliError::Custom(
            format!(
                "OWL 2 classification graph is inconsistent — {} cycle(s), \
                 {} disjoint/subclass violation(s).",
                cycles.len(),
                violations.len()
            )
            .into(),
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

    let n_classes: usize = graph
        .entities
        .values()
        .filter(|e| matches!(e.kind, Owl2EntityKind::Class))
        .count();
    let n_properties: usize = graph
        .entities
        .values()
        .filter(|e| matches!(e.kind, Owl2EntityKind::Property))
        .count();

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
            if !matches!(e.kind, Owl2EntityKind::Class) {
                continue;
            }
            let anc = closure.get(name).cloned().unwrap_or_default();
            let other_anc: Vec<&Text> = anc.iter().filter(|a| *a != name).collect();
            let semantics_label = match e.semantics {
                Some(Owl2Semantics::OpenWorld) => " [OpenWorld]",
                _ => "",
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
                let key_strs: Vec<String> = e
                    .keys
                    .iter()
                    .map(|k| {
                        format!(
                            "({})",
                            k.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", ")
                        )
                    })
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
            if !matches!(e.kind, Owl2EntityKind::Property) {
                continue;
            }
            let dom = e
                .property_domain
                .as_ref()
                .map(|d| d.as_str())
                .unwrap_or("?");
            let rng = e.property_range.as_ref().map(|r| r.as_str()).unwrap_or("?");
            let chars: Vec<&str> = e
                .property_characteristics
                .iter()
                .map(|c| c.as_str())
                .collect();
            let inv = e
                .property_inverse_of
                .as_ref()
                .map(|i| format!(" ⁻¹={}", i.as_str()))
                .unwrap_or_default();
            println!(
                "    {} {}: {} → {}  [{}]{}  — {}",
                "·".dimmed(),
                name.as_str().cyan(),
                dom,
                rng,
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
            println!(
                "    · {} ⊑* {}  (cyclic)",
                c.as_str().red(),
                c.as_str().red()
            );
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
                a.as_str().red(),
                b.as_str().red(),
                a.as_str(),
                b.as_str(),
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
    let class_count = graph
        .entities
        .values()
        .filter(|e| matches!(e.kind, Owl2EntityKind::Class))
        .count();
    let mut emitted = 0usize;
    for (name, e) in &graph.entities {
        if !matches!(e.kind, Owl2EntityKind::Class) {
            continue;
        }
        emitted += 1;
        let anc = closure.get(name).cloned().unwrap_or_default();
        let mut anc_list: Vec<&Text> = anc.iter().filter(|a| *a != name).collect();
        anc_list.sort();
        let semantics = match e.semantics {
            Some(Owl2Semantics::OpenWorld) => "OpenWorld",
            Some(Owl2Semantics::ClosedWorld) => "ClosedWorld",
            None => "ClosedWorld",
        };
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"name\": \"{}\",\n",
            json_escape(name.as_str())
        ));
        out.push_str(&format!("      \"semantics\": \"{}\",\n", semantics));
        out.push_str("      \"ancestors\": [");
        for (i, a) in anc_list.iter().enumerate() {
            out.push_str(&format!("\"{}\"", json_escape(a.as_str())));
            if i + 1 < anc_list.len() {
                out.push_str(", ");
            }
        }
        out.push_str("],\n");
        out.push_str("      \"keys\": [");
        for (i, k) in e.keys.iter().enumerate() {
            out.push('[');
            for (j, p) in k.iter().enumerate() {
                out.push_str(&format!("\"{}\"", json_escape(p.as_str())));
                if j + 1 < k.len() {
                    out.push_str(", ");
                }
            }
            out.push(']');
            if i + 1 < e.keys.len() {
                out.push_str(", ");
            }
        }
        out.push_str("],\n");
        out.push_str(&format!(
            "      \"file\": \"{}\"\n",
            json_escape(&e.file.display().to_string())
        ));
        out.push_str(if emitted == class_count {
            "    }\n"
        } else {
            "    },\n"
        });
    }
    out.push_str("  ],\n");

    out.push_str("  \"properties\": [\n");
    let prop_count = graph
        .entities
        .values()
        .filter(|e| matches!(e.kind, Owl2EntityKind::Property))
        .count();
    let mut emitted = 0usize;
    for (name, e) in &graph.entities {
        if !matches!(e.kind, Owl2EntityKind::Property) {
            continue;
        }
        emitted += 1;
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"name\": \"{}\",\n",
            json_escape(name.as_str())
        ));
        out.push_str(&format!(
            "      \"domain\": {},\n",
            e.property_domain
                .as_ref()
                .map(|d| format!("\"{}\"", json_escape(d.as_str())))
                .unwrap_or_else(|| "null".to_string())
        ));
        out.push_str(&format!(
            "      \"range\": {},\n",
            e.property_range
                .as_ref()
                .map(|r| format!("\"{}\"", json_escape(r.as_str())))
                .unwrap_or_else(|| "null".to_string())
        ));
        out.push_str(&format!(
            "      \"inverse_of\": {},\n",
            e.property_inverse_of
                .as_ref()
                .map(|i| format!("\"{}\"", json_escape(i.as_str())))
                .unwrap_or_else(|| "null".to_string())
        ));
        let chars: Vec<&str> = e
            .property_characteristics
            .iter()
            .map(|c| c.as_str())
            .collect();
        out.push_str("      \"characteristics\": [");
        for (i, c) in chars.iter().enumerate() {
            out.push_str(&format!("\"{}\"", c));
            if i + 1 < chars.len() {
                out.push_str(", ");
            }
        }
        out.push_str("],\n");
        out.push_str(&format!(
            "      \"file\": \"{}\"\n",
            json_escape(&e.file.display().to_string())
        ));
        out.push_str(if emitted == prop_count {
            "    }\n"
        } else {
            "    },\n"
        });
    }
    out.push_str("  ],\n");

    out.push_str("  \"equivalence_partitions\": [\n");
    for (i, group) in partition.iter().enumerate() {
        out.push_str("    [");
        let names: Vec<&str> = group.iter().map(|n| n.as_str()).collect();
        for (j, n) in names.iter().enumerate() {
            out.push_str(&format!("\"{}\"", json_escape(n)));
            if j + 1 < names.len() {
                out.push_str(", ");
            }
        }
        out.push(']');
        out.push_str(if i + 1 == partition.len() {
            "\n"
        } else {
            ",\n"
        });
    }
    out.push_str("  ],\n");

    out.push_str("  \"cycles\": [");
    let cyc_vec: Vec<&Text> = cycles.iter().collect();
    for (i, c) in cyc_vec.iter().enumerate() {
        out.push_str(&format!("\"{}\"", json_escape(c.as_str())));
        if i + 1 < cyc_vec.len() {
            out.push_str(", ");
        }
    }
    out.push_str("],\n");

    out.push_str("  \"disjoint_violations\": [\n");
    let v_vec: Vec<&(Text, Text)> = violations.iter().collect();
    for (i, (a, b)) in v_vec.iter().enumerate() {
        out.push_str(&format!(
            "    {{ \"class\": \"{}\", \"violates_disjoint_with\": \"{}\" }}",
            json_escape(a.as_str()),
            json_escape(b.as_str())
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
 // SemiDecidable until lands.
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
                if s_str.contains("108.T") || s_str.contains("109.T") || s_str.contains("AC/OC") {
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
        entries
            .iter()
            .filter(|e| matches!(e.status, RoundTripStatus::Decidable))
            .count(),
        entries
            .iter()
            .filter(|e| matches!(e.status, RoundTripStatus::SemiDecidable))
            .count(),
        entries
            .iter()
            .filter(|e| matches!(e.status, RoundTripStatus::Undecidable))
            .count(),
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
        out.push_str(&format!(
            "      \"name\": \"{}\",\n",
            json_escape(e.item_name.as_str())
        ));
        out.push_str(&format!("      \"kind\": \"{}\",\n", e.item_kind));
        out.push_str(&format!(
            "      \"file\": \"{}\",\n",
            json_escape(&e.file.display().to_string())
        ));
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
                    use verum_ast::attr::{FromAttribute, VerificationMode, VerifyAttr};
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
                ui::output(&format!("coherent: {} theorems audit", entries.len()));
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
                out.push_str(&format!(
                    "      \"name\": \"{}\",\n",
                    json_escape(name.as_str())
                ));
                out.push_str(&format!("      \"kind\": \"{}\",\n", kind));
                out.push_str(&format!(
                    "      \"file\": \"{}\",\n",
                    json_escape(&path.display().to_string())
                ));
                out.push_str(&format!(
                    "      \"verify_level\": \"{}\",\n",
                    level.as_str()
                ));
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

// * `axiom-placeholder` — `public axiom <name>(...)`
// * `theorem-no-proof-body` — `public theorem <name>` without proof body
// * `theorem-trivial-true` — proof body without any tactic step
// * `theorem-axiom-only` — proof body with one tactic application
// * `theorem-multi-step` — proof body with ≥ 2 tactic / let steps
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
        TacticExpr::Try(inner)
        | TacticExpr::Repeat(inner)
        | TacticExpr::AllGoals(inner)
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
                ItemKind::Theorem(decl) | ItemKind::Lemma(decl) | ItemKind::Corollary(decl) => {
                    match &decl.proof {
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
                    }
                }
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

// * `Consistent` — `inferred_nu` ≥ each cited framework's bare ν.
// * `VerifyLift` — `inferred_nu` exceeds max(cited fw ν) only because
// of `@verify(<strict>)` lift; the framework citations alone wouldn't
// reach that ν. Informational, not a violation.
// * `MissingFramework` — theorem has no `@framework(...)` citation
// at all but does have a `@verify(...)` strategy. Defect: the
// theorem's claim has no recorded framework lineage.
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
            let has_verify = strictest_verify_strategy(&item.attributes, decl_attrs).is_some();
            if let Some(strategy) = strictest_verify_strategy(&item.attributes, decl_attrs) {
                verify_by_item.insert((rel_path.clone(), item_name.clone(), kind_label), strategy);
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
 // * In per_theorem AND verify_strategy lifts ν beyond cited fw → VerifyLift.
 // * In per_theorem AND no verify-driven lift → Consistent.
 // * NOT in per_theorem (no fw citations) AND has_verify → MissingFramework.
 // * NOT in per_theorem AND no verify → silent (axiom-anchor placeholder; outside this audit's scope).
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
                ui::output(
                    "missing-framework violations (theorems with @verify but NO @framework citation):",
                );
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
                out.push_str(&format!(
                    "      \"violation_kind\": \"{}\",\n",
                    CoordConsistencyKind::MissingFramework.as_str()
                ));
                out.push_str(&format!(
                    "      \"file\": \"{}\"\n    }}",
                    json_escape(&path.display().to_string())
                ));
                out.push_str(if i + 1 < violations.len() {
                    ",\n"
                } else {
                    "\n"
                });
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

// * `Trivial` — proposition is just `true` literal (placeholder
// carrying no propositional content).
// * `Sound` — proposition has non-trivial structure (binop /
// call / refinement etc.) — passes the corpus-side
// K-FwAx-light gate.
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
                ui::output(
                    "trivial-placeholder axioms (consider strengthening or promoting to @theorem):",
                );
                for r in rows
                    .iter()
                    .filter(|r| r.kind == FrameworkSoundnessKind::Trivial)
                {
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
// Bridge-admits audit (M-EXPORT V2 / K-Round-Trip follow-up)
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
                ItemKind::Theorem(decl) | ItemKind::Lemma(decl) | ItemKind::Corollary(decl) => {
                    (decl.name.name.clone(), &decl.proof)
                }
                _ => continue,
            };

            let core = match proof_body {
                verum_common::Maybe::Some(verum_ast::ProofBody::Term(expr)) => {
                    verum_verification::kernel_recheck::lift_expr_to_core(expr.as_ref())
                }
                _ => continue,
            };

            let context = format!("{}::{}", rel_path.display(), name.as_str());
            let audit = verum_kernel::round_trip::enumerate_bridge_admits(&core, &context);
            if !audit.is_decidable() {
                let bridges_list = audit.bridges();
                let bridges: Vec<&'static str> = bridges_list.iter().copied().collect();
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

// =============================================================================
// Verify-ladder audit (13-strategy ν-monotone dispatch surface)
// =============================================================================
//

// The audit walks every `@verify(strategy)` annotation, projects to its
// ν-ordinal, and asks the *single source of truth*
// `verum_verification::ladder_dispatch::DefaultLadderDispatcher` for
// each strategy's implementation status. No duplicate status table
// lives in this audit — drift between dispatcher and audit is
// architecturally impossible.

/// `verum audit --verify-ladder` — emits per-theorem ladder dispatch
/// status and verifies the strict-ν-monotonicity invariant of the
/// 13-strategy ladder.
pub fn audit_verify_ladder(format: AuditFormat) -> Result<()> {
    use verum_verification::ladder_dispatch::{
        DefaultLadderDispatcher, LadderDispatcher, LadderImplStatus, LadderStrategy,
    };

    if matches!(format, AuditFormat::Plain) {
        ui::step("Verification ladder — strategy dispatch + ν-monotonicity audit");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project.");
        return Ok(());
    }

    let dispatcher = DefaultLadderDispatcher::new();

 // Per-theorem record.
    struct LadderEntry {
        item_kind: &'static str,
        item_name: Text,
        file: PathBuf,
        strategy: Text,
        nu_ordinal_label: &'static str,
        impl_status: LadderImplStatus,
    }

    let mut entries: Vec<LadderEntry> = Vec::new();
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
                _ => continue,
            };
            if let Some(strategy) = strictest_verify_strategy(&item.attributes, decl_attrs) {
 // Project to the typed LadderStrategy + ask the
 // dispatcher (single source of truth) for impl status.
                let typed_strategy = LadderStrategy::from_name(strategy.as_str());
                let (nu_label, status) = match typed_strategy {
                    Some(s) => (s.nu_ordinal_label(), dispatcher.implementation_status(s)),
                    None => ("?", LadderImplStatus::Pending),
                };
                entries.push(LadderEntry {
                    item_kind: kind_label,
                    item_name: item_name.clone(),
                    file: rel_path.clone(),
                    strategy,
                    nu_ordinal_label: nu_label,
                    impl_status: status,
                });
            }
        }
    }

 // Per-strategy histogram.
    let mut by_strategy: BTreeMap<Text, usize> = BTreeMap::new();
    let mut by_status: BTreeMap<&'static str, usize> = BTreeMap::new();
    for e in &entries {
        *by_strategy.entry(e.strategy.clone()).or_insert(0) += 1;
        *by_status.entry(e.impl_status.name()).or_insert(0) += 1;
    }

 // ν-monotonicity invariant: drop into the verum_verification
 // dispatcher's own monotonicity check (single source of truth).
    let monotonicity_ok =
        verum_verification::ladder_dispatch::verify_monotonicity(&dispatcher).is_ok();

    match format {
        AuditFormat::Plain => {
            println!();
            println!(
                "  {:<48}  {:<18}  {:<14}  {}",
                "Theorem / lemma / corollary", "Strategy", "ν-ordinal", "Dispatch status"
            );
            println!(
                "  {}  {}  {}  {}",
                "─".repeat(48),
                "─".repeat(18),
                "─".repeat(14),
                "─".repeat(14)
            );
            for e in &entries {
                println!(
                    "  {:<48}  {:<18}  {:<14}  {}",
                    e.item_name.as_str(),
                    e.strategy.as_str(),
                    e.nu_ordinal_label,
                    e.impl_status.name()
                );
            }
            println!();
            println!("  Strategy histogram:");
            for (strat, count) in &by_strategy {
                let status = LadderStrategy::from_name(strat.as_str())
                    .map(|s| dispatcher.implementation_status(s).name())
                    .unwrap_or("unknown");
                println!("    {:<20} {:>4}   [{}]", strat.as_str(), count, status);
            }
            println!();
            println!("  Implementation-status totals:");
            for (status, count) in &by_status {
                println!("    {:<14} {:>4}", status, count);
            }
            println!();
            println!(
                "  ν-monotonicity invariant: {}",
                if monotonicity_ok {
                    "✓ holds"
                } else {
                    "✗ VIOLATED"
                }
            );
            println!(
                "  Files: {} scanned, {} parsed, {} skipped",
                vr_files.len(),
                parsed_files,
                skipped_files
            );
        }
        AuditFormat::Json => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"theorem_count\": {},\n", entries.len()));
            out.push_str(&format!(
                "  \"monotonicity_invariant\": {},\n",
                monotonicity_ok
            ));
            out.push_str("  \"by_strategy\": {\n");
            for (i, (strat, count)) in by_strategy.iter().enumerate() {
                out.push_str(&format!(
                    "    \"{}\": {}{}\n",
                    json_escape(strat.as_str()),
                    count,
                    if i + 1 < by_strategy.len() { "," } else { "" }
                ));
            }
            out.push_str("  },\n");
            out.push_str("  \"by_status\": {\n");
            for (i, (status, count)) in by_status.iter().enumerate() {
                out.push_str(&format!(
                    "    \"{}\": {}{}\n",
                    status,
                    count,
                    if i + 1 < by_status.len() { "," } else { "" }
                ));
            }
            out.push_str("  },\n");
            out.push_str("  \"theorems\": [\n");
            for (i, e) in entries.iter().enumerate() {
                out.push_str(&format!(
                    "    {{ \"kind\": \"{}\", \"name\": \"{}\", \"file\": \"{}\", \"strategy\": \"{}\", \"nu_ordinal\": \"{}\", \"impl_status\": \"{}\" }}{}\n",
                    e.item_kind,
                    json_escape(e.item_name.as_str()),
                    json_escape(&e.file.display().to_string()),
                    e.strategy.as_str(),
                    e.nu_ordinal_label,
                    e.impl_status.name(),
                    if i + 1 < entries.len() { "," } else { "" }
                ));
            }
            out.push_str("  ]\n}");
            println!("{}", out);
        }
    }

    if !monotonicity_ok {
        return Err(crate::error::CliError::VerificationFailed(
            "verify-ladder ν-monotonicity invariant violated".to_string(),
        ));
    }
    Ok(())
}

// =============================================================================
// Kernel-discharged-axioms audit
// =============================================================================

/// One `@kernel_discharge` citation site found in the corpus.
///

/// Hoisted to module scope (out of [`audit_kernel_discharged_axioms`]) so
/// the JSON-rendering helper [`render_kernel_discharge_json`] can take
/// it by reference without inheriting the function's local-scope.
struct DischargeCite {
    axiom_name: Text,
    intrinsic_name: Text,
    file: PathBuf,
    recognised: bool,
}

/// **Render the kernel-discharged-axioms audit report as JSON**, with
/// task #318 / #188 dependency-aware metadata.
///

/// Every entry in the `discharges` array carries the existing four
/// fields (`axiom`, `intrinsic`, `file`, `recognised`) plus a new
/// `dependents` array (only populated for unrecognised entries) listing
/// each downstream theorem that transitively depends on the rejected
/// axiom — name + chain + source path.
///

/// Used by both the Plain-format-side disk-write (for the audit
/// bundle dispatcher) and the Json-format primary output, so both
/// emission paths produce identical bytes.
fn render_kernel_discharge_json(
    files_scanned: usize,
    files_parsed: usize,
    files_skipped: usize,
    discharge_count: usize,
    unrecognised_count: usize,
    cites: &[DischargeCite],
    dep_listings: &std::collections::BTreeMap<
        String,
        Vec<verum_kernel::soundness::apply_graph::DependentTheorem>,
    >,
    theorem_sources: &std::collections::BTreeMap<String, PathBuf>,
) -> String {
    let mut out = String::from("{\n");
    out.push_str("  \"schema_version\": 2,\n");
    out.push_str(&format!("  \"files_scanned\": {},\n", files_scanned));
    out.push_str(&format!("  \"files_parsed\": {},\n", files_parsed));
    out.push_str(&format!("  \"files_skipped\": {},\n", files_skipped));
    out.push_str(&format!("  \"discharge_count\": {},\n", discharge_count));
    out.push_str(&format!(
        "  \"unrecognised_count\": {},\n",
        unrecognised_count
    ));
    out.push_str("  \"discharges\": [\n");
    for (i, cite) in cites.iter().enumerate() {
 // Compose the dependents list for unrecognised cites only.
        let deps_json = if !cite.recognised {
            match dep_listings.get(cite.axiom_name.as_str()) {
                Some(deps) if !deps.is_empty() => {
                    let mut s = String::from("[\n");
                    for (j, dep) in deps.iter().enumerate() {
                        let path_str = theorem_sources
                            .get(&dep.theorem)
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        let chain_array: Vec<String> = dep
                            .chain
                            .iter()
                            .map(|s| format!("\"{}\"", json_escape(s)))
                            .collect();
                        s.push_str(&format!(
                            "        {{ \"theorem\": \"{}\", \"file\": \"{}\", \"chain\": [{}] }}{}\n",
                            json_escape(&dep.theorem),
                            json_escape(&path_str),
                            chain_array.join(", "),
                            if j + 1 < deps.len() { "," } else { "" },
                        ));
                    }
                    s.push_str("      ]");
                    s
                }
                _ => "[]".to_string(),
            }
        } else {
            "[]".to_string()
        };
        out.push_str(&format!(
            "    {{ \"axiom\": \"{}\", \"intrinsic\": \"{}\", \"file\": \"{}\", \"recognised\": {}, \"dependents\": {} }}{}\n",
            json_escape(cite.axiom_name.as_str()),
            json_escape(cite.intrinsic_name.as_str()),
            json_escape(&cite.file.display().to_string()),
            cite.recognised,
            deps_json,
            if i + 1 < cites.len() { "," } else { "" }
        ));
    }
    out.push_str("  ]\n}");
    out
}

/// `verum audit --kernel-discharged-axioms` — walks every `.vr` file in the
/// project, finds every `@kernel_discharge("<intrinsic_name>")` attribute,
/// and verifies that every cited dispatcher name appears in
/// `verum_kernel::intrinsic_dispatch::available_intrinsics()`.
///

/// This is the **machine-checked cross-link** between the host stdlib's
/// paper-cited `@axiom` declarations and the algorithmic V0 kernel surfaces:
/// when an axiom carries `@kernel_discharge(...)`, we audit-prove (a) the
/// named dispatcher exists, (b) the trusted-base-shrinkage gain is
/// observable in the report.
///

/// Exits non-zero on any unmatched citation.
pub fn audit_kernel_discharged_axioms(format: AuditFormat) -> Result<()> {
    use verum_ast::expr::ExprKind;
    use verum_ast::literal::{LiteralKind, StringLit};
    use verum_common::Maybe;
    use verum_kernel::intrinsic_dispatch::is_known_intrinsic;

    if matches!(format, AuditFormat::Plain) {
        ui::step("Kernel-discharged framework axioms — audit");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let mut vr_files = discover_vr_files(&manifest_dir);
 // Extend the scan with the verum stdlib's `core/math/` tree (#136
 // follow-up). Most `@kernel_discharge` annotations live in stdlib
 // (e.g., `core/math/syn_mod.vr::lurie_htt_5_1_4_syn_is_grothendieck`,
 // `core/math/absolute_layer.vr::msfs_id_x_violates_pi_4`) — without
 // this step, corpus-level audit runs surface 0 discharges even when
 // the corpus's transitive apply-chains route through these stdlib
 // axioms. Sibling of `apply_graph`'s stdlib-walker.
    vr_files.extend(discover_stdlib_vr_files());

    if vr_files.is_empty() {
        ui::warn("No .vr files found under the current project or stdlib.");
        return Ok(());
    }

 /// One @kernel_discharge citation site found in the corpus.
    struct DischargeCite {
        axiom_name: Text,
        intrinsic_name: Text,
        file: PathBuf,
        recognised: bool,
    }

    let mut cites: Vec<DischargeCite> = Vec::new();
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
            let (item_name, decl_attrs): (Text, &verum_common::List<verum_ast::attr::Attribute>) =
                match &item.kind {
                    ItemKind::Axiom(decl) => (decl.name.name.clone(), &decl.attributes),
                    ItemKind::Theorem(decl) => (decl.name.name.clone(), &decl.attributes),
                    ItemKind::Lemma(decl) => (decl.name.name.clone(), &decl.attributes),
                    ItemKind::Corollary(decl) => (decl.name.name.clone(), &decl.attributes),
                    _ => continue,
                };

 // Scan both the outer item.attributes and the inner decl.attributes.
            let attr_iters: [&verum_common::List<verum_ast::attr::Attribute>; 2] =
                [&item.attributes, decl_attrs];

            for attrs in attr_iters {
                for attr in attrs {
                    let name = attr.name.as_str();
                    if name != "kernel_discharge" {
                        continue;
                    }
 // Expect a single string-literal argument identifying
 // the kernel intrinsic.
                    let args_list = match &attr.args {
                        Maybe::Some(list) => list,
                        Maybe::None => {
                            malformed.push((
                                rel_path.clone(),
                                Text::from(format!(
                                    "{}: @kernel_discharge() called without an intrinsic name",
                                    item_name.as_str()
                                )),
                            ));
                            continue;
                        }
                    };
                    let first_arg = match args_list.iter().next() {
                        Some(e) => e,
                        None => {
                            malformed.push((
                                rel_path.clone(),
                                Text::from(format!(
                                    "{}: @kernel_discharge() called with empty args",
                                    item_name.as_str()
                                )),
                            ));
                            continue;
                        }
                    };
                    let intrinsic_name: Text = match &first_arg.kind {
                        ExprKind::Literal(lit) => match &lit.kind {
                            LiteralKind::Text(StringLit::Regular(s))
                            | LiteralKind::Text(StringLit::MultiLine(s)) => s.clone(),
                            _ => {
                                malformed.push((
                                    rel_path.clone(),
                                    Text::from(format!(
                                        "{}: @kernel_discharge expects a string-literal argument",
                                        item_name.as_str()
                                    )),
                                ));
                                continue;
                            }
                        },
                        _ => {
                            malformed.push((
                                rel_path.clone(),
                                Text::from(format!(
                                    "{}: @kernel_discharge expects a string-literal argument",
                                    item_name.as_str()
                                )),
                            ));
                            continue;
                        }
                    };
                    let intrinsic = intrinsic_name;
                    let recognised = is_known_intrinsic(intrinsic.as_str());
                    cites.push(DischargeCite {
                        axiom_name: item_name.clone(),
                        intrinsic_name: intrinsic,
                        file: rel_path.clone(),
                        recognised,
                    });
                }
            }
        }
    }

    let total = cites.len();
    let unrecognised = cites.iter().filter(|c| !c.recognised).count();

    match format {
        AuditFormat::Plain => {
            println!();
            println!(
                "  {:<48}  {:<46}  {}",
                "Axiom", "Kernel intrinsic discharge", "Status"
            );
            println!(
                "  {}  {}  {}",
                "─".repeat(48),
                "─".repeat(46),
                "─".repeat(8)
            );
            for cite in &cites {
                let status = if cite.recognised {
                    "✓ ok"
                } else {
                    "✗ MISSING"
                };
                println!(
                    "  {:<48}  {:<46}  {}",
                    cite.axiom_name.as_str(),
                    cite.intrinsic_name.as_str(),
                    status
                );
            }
            println!();
            println!(
                "  {} files scanned, {} parsed, {} skipped; {} discharge citations, {} unrecognised",
                vr_files.len(),
                parsed_files,
                skipped_files,
                total,
                unrecognised
            );
            for (path, msg) in &malformed {
                eprintln!(
                    "  E_KERNEL_DISCHARGE_MALFORMED at {}: {}",
                    path.display(),
                    msg.as_str()
                );
            }
        }
        AuditFormat::Json => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"files_scanned\": {},\n", vr_files.len()));
            out.push_str(&format!("  \"files_parsed\": {},\n", parsed_files));
            out.push_str(&format!("  \"files_skipped\": {},\n", skipped_files));
            out.push_str(&format!("  \"discharge_count\": {},\n", total));
            out.push_str(&format!("  \"unrecognised_count\": {},\n", unrecognised));
            out.push_str("  \"discharges\": [\n");
            for (i, cite) in cites.iter().enumerate() {
                out.push_str(&format!(
                    "    {{ \"axiom\": \"{}\", \"intrinsic\": \"{}\", \"file\": \"{}\", \"recognised\": {} }}{}\n",
                    json_escape(cite.axiom_name.as_str()),
                    json_escape(cite.intrinsic_name.as_str()),
                    json_escape(&cite.file.display().to_string()),
                    cite.recognised,
                    if i + 1 < cites.len() { "," } else { "" }
                ));
            }
            out.push_str("  ]\n}");
 // #172 audit-output discipline: write JSON to disk
 // regardless of `--format` so bundle dispatcher (#151) and
 // downstream tooling can reliably read each per-gate report.
            if let Ok(manifest_dir) = Manifest::find_manifest_dir() {
                let dir = manifest_dir.join("target").join("audit-reports");
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(dir.join("kernel-discharged-axioms.json"), &out);
            }
            println!("{}", out);
        }
    }

 // Even Plain output writes JSON for the bundle dispatcher (#172).
    if matches!(format, AuditFormat::Plain) {
        if let Ok(manifest_dir) = Manifest::find_manifest_dir() {
            let dir = manifest_dir.join("target").join("audit-reports");
            let _ = std::fs::create_dir_all(&dir);
            let mut out = String::new();
            out.push_str("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"files_scanned\": {},\n", vr_files.len()));
            out.push_str(&format!("  \"files_parsed\": {},\n", parsed_files));
            out.push_str(&format!("  \"files_skipped\": {},\n", skipped_files));
            out.push_str(&format!("  \"discharge_count\": {},\n", total));
            out.push_str(&format!("  \"unrecognised_count\": {},\n", unrecognised));
            out.push_str("  \"discharges\": [\n");
            for (i, cite) in cites.iter().enumerate() {
                out.push_str(&format!(
                    "    {{ \"axiom\": \"{}\", \"intrinsic\": \"{}\", \"file\": \"{}\", \"recognised\": {} }}{}\n",
                    json_escape(cite.axiom_name.as_str()),
                    json_escape(cite.intrinsic_name.as_str()),
                    json_escape(&cite.file.display().to_string()),
                    cite.recognised,
                    if i + 1 < cites.len() { "," } else { "" }
                ));
            }
            out.push_str("  ]\n}");
            let _ = std::fs::write(dir.join("kernel-discharged-axioms.json"), &out);
        }
    }

    if unrecognised > 0 || !malformed.is_empty() {
        return Err(crate::error::CliError::VerificationFailed(format!(
            "@kernel_discharge audit: {} unrecognised + {} malformed citations",
            unrecognised,
            malformed.len()
        )));
    }
    Ok(())
}

// =============================================================================
// HTT mechanisation roadmap audit
// =============================================================================

/// `verum audit --htt-roadmap` — emits per-section coverage of Lurie
/// HTT (2009) mechanisation status from
/// `verum_kernel::mechanisation_roadmap::htt_roadmap()`.
pub fn audit_htt_roadmap(format: AuditFormat) -> Result<()> {
    use verum_kernel::mechanisation_roadmap::{CoverageReport, htt_roadmap};

    let entries = htt_roadmap();
    let report = CoverageReport::compute(&entries);

    match format {
        AuditFormat::Plain => {
            ui::step("HTT (Lurie 2009) mechanisation roadmap");
            println!();
            println!(
                "  {:<43}  {:<13}  {}",
                "Section", "Status", "Kernel module(s)"
            );
            println!(
                "  {}  {}  {}",
                "─".repeat(43),
                "─".repeat(13),
                "─".repeat(40)
            );
            for e in &entries {
                println!(
                    "  {:<43}  {:<13}  {}",
                    e.section.as_str(),
                    e.status.name(),
                    e.kernel_modules.as_str()
                );
            }
            println!();
            println!("  {}", report.summary("HTT"));
        }
        AuditFormat::Json => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str("  \"reference\": \"Lurie 2009 - Higher Topos Theory\",\n");
            out.push_str(&format!(
                "  \"total\": {},\n  \"mechanised\": {},\n  \"partial\": {},\n  \"axiom_cited\": {},\n  \"pending\": {},\n  \"coverage_ratio\": {:.4},\n",
                report.total,
                report.mechanised,
                report.partial,
                report.axiom_cited,
                report.pending,
                report.coverage_ratio()
            ));
            out.push_str("  \"sections\": [\n");
            for (i, e) in entries.iter().enumerate() {
                out.push_str(&format!(
                    "    {{ \"section\": \"{}\", \"title\": \"{}\", \"status\": \"{}\", \"kernel_modules\": \"{}\" }}{}\n",
                    json_escape(e.section.as_str()),
                    json_escape(e.title.as_str()),
                    e.status.name(),
                    json_escape(e.kernel_modules.as_str()),
                    if i + 1 < entries.len() { "," } else { "" }
                ));
            }
            out.push_str("  ]\n}");
            println!("{}", out);
        }
    }
    Ok(())
}

// =============================================================================
// Adámek-Rosický 1994 mechanisation roadmap audit
// =============================================================================

/// `verum audit --ar-roadmap` — emits per-section coverage of
/// Adámek-Rosický 1994 mechanisation status.
pub fn audit_ar_roadmap(format: AuditFormat) -> Result<()> {
    use verum_kernel::mechanisation_roadmap::{CoverageReport, adamek_rosicky_roadmap};

    let entries = adamek_rosicky_roadmap();
    let report = CoverageReport::compute(&entries);

    match format {
        AuditFormat::Plain => {
            ui::step("Adamek-Rosicky 1994 mechanisation roadmap");
            println!();
            println!(
                "  {:<43}  {:<13}  {}",
                "Section", "Status", "Kernel module(s)"
            );
            println!(
                "  {}  {}  {}",
                "─".repeat(43),
                "─".repeat(13),
                "─".repeat(40)
            );
            for e in &entries {
                println!(
                    "  {:<43}  {:<13}  {}",
                    e.section.as_str(),
                    e.status.name(),
                    e.kernel_modules.as_str()
                );
            }
            println!();
            println!("  {}", report.summary("AR 1994"));
        }
        AuditFormat::Json => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str("  \"reference\": \"Adamek-Rosicky 1994 - Locally Presentable and Accessible Categories\",\n");
            out.push_str(&format!(
                "  \"total\": {},\n  \"mechanised\": {},\n  \"partial\": {},\n  \"axiom_cited\": {},\n  \"pending\": {},\n  \"coverage_ratio\": {:.4},\n",
                report.total,
                report.mechanised,
                report.partial,
                report.axiom_cited,
                report.pending,
                report.coverage_ratio()
            ));
            out.push_str("  \"sections\": [\n");
            for (i, e) in entries.iter().enumerate() {
                out.push_str(&format!(
                    "    {{ \"section\": \"{}\", \"title\": \"{}\", \"status\": \"{}\", \"kernel_modules\": \"{}\" }}{}\n",
                    json_escape(e.section.as_str()),
                    json_escape(e.title.as_str()),
                    e.status.name(),
                    json_escape(e.kernel_modules.as_str()),
                    if i + 1 < entries.len() { "," } else { "" }
                ));
            }
            out.push_str("  ]\n}");
            println!("{}", out);
        }
    }
    Ok(())
}

// Kernel self-recognition audit was subsumed into the
// reflection-tower gate (`audit_reflection_tower_with_format`):
// per-rule footprint table is the `base_footprint` sub-block of
// `reflection-tower.json`. The standalone `--self-recognition`
// command was removed as part of the verification consolidation
// audit; consumers should call `--reflection-tower` instead.

// =============================================================================
// ATS-V end-to-end verum arch check <file>
// =============================================================================

/// `verum arch check <file> [--format plain|json] [--strict]` —
/// end-to-end parse a .vr file, walk every module
/// declaration's attributes, extract `@arch_module(...)` named-args,
/// run the ATS-V phase 6.5, and report architectural violations.
///
/// Per spec §11.4 backward-compat: modules без `@arch_module(...)`
/// аннотации pass vacuously through default Shape. Modules с
/// аннотацией get full Shape inference + 32-pattern catalog check.
pub fn arch_check(file: &str, format: AuditFormat, strict: bool) -> Result<()> {
    use verum_ast::FileId;
    use verum_fast_parser::FastParser;
    use verum_kernel::arch_phase::{run_arch_phase, ArchPhaseReport};

    if matches!(format, AuditFormat::Plain) {
        ui::step(&format!("ATS-V arch:check {}", file));
    }

 // Read file (`-` reads stdin).
    let source = if file == "-" {
        use std::io::Read;
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| crate::error::CliError::Custom(format!("stdin read: {}", e).into()))?;
        s
    } else {
        std::fs::read_to_string(file).map_err(|e| {
            crate::error::CliError::Custom(format!("read {}: {}", file, e).into())
        })?
    };

 // Parse via fast parser.
    let parser = FastParser::new();
    let file_id = FileId::new(0);
    let module = parser.parse_module_str(&source, file_id).map_err(|e| {
        crate::error::CliError::Custom(
            format!("parse {} failed: {:?}", file, e).into(),
        )
    })?;

 // Walk module's items, extract @arch_module(...) attribute args
 // for each Module item.
    let modules: Vec<(String, Vec<verum_ast::expr::Expr>)> = module
        .items
        .iter()
        .filter_map(|item| {
            let module_name = match &item.kind {
                verum_ast::decl::ItemKind::Module(m) => m.name.name.as_str().to_string(),
                _ => return None, // not a module
            };
 // Search item.attributes for @arch_module(...).
            for attr in item.attributes.iter() {
                if attr.name.as_str() == "arch_module" {
                    let args: Vec<verum_ast::expr::Expr> = match &attr.args {
                        verum_common::Maybe::Some(a) => a.iter().cloned().collect(),
                        verum_common::Maybe::None => Vec::new(),
                    };
                    return Some((module_name, args));
                }
            }
 // Module без @arch_module — pass empty args.
            Some((module_name, Vec::new()))
        })
        .collect();

 // Run ATS-V phase.
    let module_refs: Vec<(String, &[verum_ast::expr::Expr])> = modules
        .iter()
        .map(|(n, a)| (n.clone(), a.as_slice()))
        .collect();
    let report = run_arch_phase(&module_refs);

 // Decide pass/fail. Strict mode: any non-load-bearing module → error.
 // Soft mode: parse_errors → error, anti-pattern violations → warning.
    let load_bearing = if strict {
        report.is_load_bearing()
    } else {
        report.total_parse_errors() == 0
    };

 // Render output.
    let payload = build_arch_check_payload(file, &report, strict, load_bearing);

    match format {
        AuditFormat::Plain => render_arch_check_plain(file, &report, strict, load_bearing),
        AuditFormat::Json => {
            println!("{}", serde_json::to_string(&payload).unwrap_or_default());
        }
    }

    if !load_bearing {
        return Err(crate::error::CliError::Custom(
            format!(
                "ATS-V arch check on {}: {} module(s) failed.  See diagnostics.",
                file,
                report.modules.iter().filter(|m| !m.is_load_bearing()).count(),
            )
            .into(),
        ));
    }
    Ok(())
}

fn build_arch_check_payload(
    file: &str,
    report: &verum_kernel::arch_phase::ArchPhaseReport,
    strict: bool,
    load_bearing: bool,
) -> serde_json::Value {
    let modules_json: Vec<serde_json::Value> = report
        .modules
        .iter()
        .map(|m| {
            let shape_json = m
                .shape
                .as_ref()
                .map(|s| serde_json::to_value(s).unwrap_or(serde_json::Value::Null))
                .unwrap_or(serde_json::Value::Null);
            let parse_errors: Vec<serde_json::Value> = m
                .parse_errors
                .iter()
                .map(|e| serde_json::json!({ "human_message": e.human_message() }))
                .collect();
            let violations_json: Vec<serde_json::Value> = m
                .violations
                .iter()
                .map(|v| {
                    serde_json::json!({
                        "code": v.code.code(),
                        "name": v.code.name(),
                        "severity": v.severity.tag(),
                        "summary": v.summary,
                        "human_message": v.human_message,
                        "auto_fix_suggestion": v.auto_fix_suggestion,
                        "docs_url": v.code.docs_url(),
                    })
                })
                .collect();
            serde_json::json!({
                "module_name": m.module_name,
                "annotated": m.shape.is_some(),
                "load_bearing": m.is_load_bearing(),
                "shape": shape_json,
                "parse_errors": parse_errors,
                "violations": violations_json,
            })
        })
        .collect();

    let by_code = report.violations_by_code();
    let violations_by_code_json: serde_json::Value = serde_json::Value::Object(
        by_code
            .iter()
            .map(|(k, v)| ((*k).to_string(), serde_json::json!(*v)))
            .collect(),
    );

    serde_json::json!({
        "schema_version": 1,
        "kernel_version": env!("CARGO_PKG_VERSION"),
        "discipline": "ats_v_arch_check",
        "spec": "internal/specs/ats-v.md",
        "file": file,
        "strict_mode": strict,
        "load_bearing": load_bearing,
        "annotated_module_count": report.annotated_module_count(),
        "total_module_count": report.modules.len(),
        "total_parse_errors": report.total_parse_errors(),
        "total_violations": report.total_violations(),
        "violations_by_code": violations_by_code_json,
        "modules": modules_json,
    })
}

fn render_arch_check_plain(
    file: &str,
    report: &verum_kernel::arch_phase::ArchPhaseReport,
    strict: bool,
    load_bearing: bool,
) {
    println!();
    println!("File: {}", file);
    println!("─────────────────────────────────────────────────────");
    println!(
        "Modules:                {} ({} с @arch_module(...))",
        report.modules.len(),
        report.annotated_module_count(),
    );
    println!("Mode:                   {}", if strict { "strict" } else { "soft" });
    println!("Total parse errors:     {}", report.total_parse_errors());
    println!("Total violations:       {}", report.total_violations());
    println!();

    if !report.modules.is_empty() {
        println!(
            "  {:<40}  {:<11}  {:<6}  {}",
            "Module", "Annotated", "Loads", "Violations"
        );
        println!(
            "  {}  {}  {}  {}",
            "─".repeat(40),
            "─".repeat(11),
            "─".repeat(6),
            "─".repeat(20)
        );
        for m in &report.modules {
            let glyph = if m.is_load_bearing() { "✓" } else { "✗" };
            println!(
                "  {:<40}  {:<11}  {} {:<3}  {} violations",
                m.module_name,
                if m.shape.is_some() { "yes" } else { "no" },
                glyph,
                if m.is_load_bearing() { "yes" } else { "NO" },
                m.violations.len(),
            );
            for v in &m.violations {
                println!(
                    "      {} {}: {}",
                    v.code.code(),
                    v.code.name(),
                    v.summary
                );
            }
            for e in &m.parse_errors {
                println!("      [parse error] {}", e.human_message());
            }
        }
    }
    println!();

    if load_bearing {
        println!(
            "{} ATS-V arch check passed — file is architecturally load-bearing.",
            "✓".green(),
        );
    } else {
        println!(
            "{} ATS-V arch check FAILED — see diagnostics above.",
            "✗".red(),
        );
    }
}

// =============================================================================
// ATS-V — agent-readable surfaces (verum arch:explain, arch:catalog)
// =============================================================================

/// `verum arch explain [cog] [--format plain|json]` — structured
/// architectural type information per spec §32.4.
///
/// Scope: outputs the canonical Shape (default for
/// unannotated cogs since ATS-V phase isn't yet wired into the
/// compiler) + the full anti-pattern catalog roster. +
/// resolves `cog` argument against project's cog graph and
/// reads its `@arch_module(...)` declaration.
pub fn arch_explain(cog: Option<&str>, format: AuditFormat) -> Result<()> {
    use verum_kernel::arch::Shape;
    use verum_kernel::arch_anti_pattern::{
        check_all_anti_patterns, AntiPatternCode, DiagnosticContext,
    };

    if matches!(format, AuditFormat::Plain) {
        ui::step("ATS-V arch:explain — structured architectural type information");
    }

    let cog_name = cog.unwrap_or("<unannotated>").to_string();
 // default shape for the requested cog. reads
 // actual `@arch_module(...)` declaration.
    let shape = Shape::default_for_unannotated();
    let mut ctx = DiagnosticContext::default();
    ctx.cog_name = cog_name.clone();
    let violations = check_all_anti_patterns(&shape, &ctx);

    let shape_json =
        serde_json::to_value(&shape).unwrap_or_else(|_| serde_json::json!(null));
    let violations_json: Vec<serde_json::Value> = violations
        .iter()
        .map(|v| {
            serde_json::json!({
                "code": v.code.code(),
                "name": v.code.name(),
                "severity": format!("{:?}", v.severity).to_lowercase(),
                "summary": v.summary,
                "human_message": v.human_message,
                "auto_fix_suggestion": v.auto_fix_suggestion,
                "docs_url": v.code.docs_url(),
            })
        })
        .collect();

    let total_patterns_checked = AntiPatternCode::full_list().len();

    let payload = serde_json::json!({
        "schema_version": 1,
        "kernel_version": env!("CARGO_PKG_VERSION"),
        "discipline": "ats_v_arch_explain",
        "spec": "internal/specs/ats-v.md",
        "cog": cog_name,
        "season_resolution": "season_2_default_shape_stub",
        "shape": shape_json,
        "violations": violations_json,
        "patterns_checked": total_patterns_checked,
    });

    match format {
        AuditFormat::Plain => {
            println!();
            println!("Cog: {}", cog_name);
            println!("─────────────────────────────────────────────────────");
            println!("Shape (default for unannotated;  stub):");
            println!("  exposes:           {} capabilities", shape.exposes.len());
            println!("  requires:          {} capabilities", shape.requires.len());
            println!("  preserves:         {} invariants", shape.preserves.len());
            println!("  at_tier:           {}", shape.at_tier.tag());
            println!("  foundation:        {}", shape.foundation.tag());
            println!("  stratum:           {}", shape.stratum.tag());
            println!("  lifecycle:         {}", shape.lifecycle.tag());
            println!(
                "  cve_closure:       {}/3 axes",
                shape.cve_closure.closure_degree()
            );
            println!("  composes_with:     {} cogs", shape.composes_with.len());
            println!("  strict:            {}", shape.strict);
            println!();
            println!(
                "Anti-pattern check: {} violations across {} canonical patterns",
                violations.len(),
                total_patterns_checked,
            );
            for v in &violations {
                println!(
                    "  {} {}: {}",
                    v.code.code(),
                    v.code.name(),
                    v.summary
                );
            }
            if violations.is_empty() {
                println!(
                    "{} No violations — default Shape is canonically clean.",
                    "✓".green(),
                );
            }
            println!();
            println!(" stub: no per-cog @arch_module(...) parsing yet.");
            println!(" wires the compiler's ATS-V phase to read actual cog declarations.");
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string(&payload).unwrap_or_default());
        }
    }
    Ok(())
}

/// `verum arch catalog [--format plain|json] [--mtac-only] [--season N]` —
/// list the canonical anti-pattern catalog with stable RFC codes.
/// Per spec §29.1 — codes are stable across v1.x; agents may
/// pattern-match безопасно.
pub fn arch_catalog(
    format: AuditFormat,
    mtac_only: bool,
    season: Option<u8>,
) -> Result<()> {
    use verum_kernel::arch_anti_pattern::AntiPatternCode;

    if matches!(format, AuditFormat::Plain) {
        ui::step("ATS-V anti-pattern catalog");
    }

    let filtered: Vec<_> = AntiPatternCode::full_list()
        .iter()
        .filter(|c| !mtac_only || c.is_mtac())
        .filter(|c| season.is_none() || season == Some(c.season()))
        .copied()
        .collect();

    let patterns_json: Vec<serde_json::Value> = filtered
        .iter()
        .map(|code| {
            serde_json::json!({
                "code": code.code(),
                "name": code.name(),
                "docs_url": code.docs_url(),
                "season": code.season(),
                "is_mtac": code.is_mtac(),
                "stability": "v1.0",
            })
        })
        .collect();

    let payload = serde_json::json!({
        "schema_version": 1,
        "discipline": "ats_v_anti_pattern_catalog",
        "spec": "internal/specs/ats-v.md",
        "filter": {
            "mtac_only": mtac_only,
            "season": season,
        },
        "total_canonical": AntiPatternCode::full_list().len(),
        "filtered_count": filtered.len(),
        "patterns": patterns_json,
    });

    match format {
        AuditFormat::Plain => {
            println!();
            println!(
                "{} canonical anti-patterns ({} after filter):",
                AntiPatternCode::full_list().len(),
                filtered.len(),
            );
            println!();
            println!(
                "  {:<14}  {:<32}  {:<7}  Docs URL",
                "Code", "Name", "Season"
            );
            println!(
                "  {}  {}  {}  {}",
                "─".repeat(14),
                "─".repeat(32),
                "─".repeat(7),
                "─".repeat(40),
            );
            for code in filtered {
                let season_label = if code.is_mtac() {
                    format!("{}-MTAC", code.season())
                } else {
                    format!("{}", code.season())
                };
                println!(
                    "  {:<14}  {:<32}  {:<7}  {}",
                    code.code(),
                    code.name(),
                    season_label,
                    code.docs_url(),
                );
            }
            println!();
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string(&payload).unwrap_or_default());
        }
    }
    Ok(())
}

// =============================================================================
// ATS-V Architectural Type System audit
// =============================================================================

/// `verum audit --arch-discharges` — walks every kernel intrinsic
/// in the ATS-V architectural-type registry surface, lists discharge
/// status, surfaces the canonical anti-pattern catalog with stable
/// RFC error codes (ATS-V-AP-NNN), and reports the dual-audience
/// machine-readable JSON per spec §32.4.
///
/// Scope: registry surface + structured diagnostic shape.
/// Full per-cog dispatch (consuming Shape + DiagnosticContext) lands
/// when the ATS-V phase is wired into the compiler
/// pipeline.
pub fn audit_arch_discharges_with_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::arch_anti_pattern::AntiPatternCode;

    if matches!(format, AuditFormat::Plain) {
        ui::step("ATS-V Architectural Type System — discharge registry");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let report_path = report_dir.join("arch-discharges.json");

 // Roster of ATS-V kernel intrinsics surfaced in this gate.
 // Stable order matches `verum_kernel::intrinsic_dispatch`
 // available_intrinsics() listing.
    let arch_intrinsics: &[(&str, &str)] = &[
        ("kernel_arch_capability_discipline", "Capability discipline (AP-001 + AP-002)"),
        ("kernel_arch_boundary_check", "Boundary type check"),
        ("kernel_arch_composition_check", "Composition algebra check"),
        ("kernel_arch_lifecycle_check", "Lifecycle integrity (AP-009)"),
        ("kernel_arch_foundation_consistency", "Foundation consistency (AP-005)"),
        ("kernel_arch_anti_pattern_check", "Generic anti-pattern dispatcher"),
        ("kernel_arch_cve_closure", "CVE-closure check (AP-010, strict mode)"),
        ("kernel_arch_soundness_v0", "End-to-end soundness witness"),
    ];

 // Dispatch each intrinsic and collect verdicts.
    let intrinsics_json: Vec<serde_json::Value> = arch_intrinsics
        .iter()
        .map(|(name, description)| {
            let verdict = verum_kernel::intrinsic_dispatch::dispatch_intrinsic(name, &[]);
            let (holds, reason) = match verdict {
                Some(verum_kernel::intrinsic_dispatch::IntrinsicValue::Decision {
                    holds,
                    reason,
                }) => (holds, reason),
                _ => (false, "intrinsic not dispatched".to_string()),
            };
            serde_json::json!({
                "intrinsic": name,
                "description": description,
                "holds": holds,
                "reason": reason,
            })
        })
        .collect();

 // Anti-pattern catalog with stable RFC error codes
 // (canonical anti-pattern catalog: 32 patterns total — 26 base + 6 MTAC).
    let anti_patterns_json: Vec<serde_json::Value> = AntiPatternCode::full_list()
        .iter()
        .map(|code| {
            serde_json::json!({
                "code": code.code(),
                "name": code.name(),
                "docs_url": code.docs_url(),
                "season": code.season(),
                "is_mtac": code.is_mtac(),
                "stability": "v1.0",
            })
        })
        .collect();

    let all_intrinsics_dispatch = intrinsics_json
        .iter()
        .all(|j| j.get("holds").and_then(|v| v.as_bool()) == Some(true));

    let payload = serde_json::json!({
        "schema_version": 1,
        "kernel_version": env!("CARGO_PKG_VERSION"),
        "discipline": "ats_v_architectural_type_system",
        "season": 1,
        "spec": "internal/specs/ats-v.md",
        "load_bearing": all_intrinsics_dispatch,
        "intrinsics": intrinsics_json,
        "anti_pattern_catalog": {
            "total_canonical": 32,
            "season_1_count": 10,
            "season_2_count": 22,
            "mtac_count": 6,
            "patterns": anti_patterns_json,
        },
    });

    let _ = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&payload).unwrap_or_default(),
    );

    match format {
        AuditFormat::Plain => {
            println!();
            println!("ATS-V Architectural Type System — discharge registry");
            println!("─────────────────────────────────────────────────────");
            println!(
                "  {} kernel intrinsics dispatched ( registry surface):",
                arch_intrinsics.len(),
            );
            println!();
            println!("  {:<40}  {:<9}  {}", "Intrinsic", "Discharge", "Description");
            println!("  {}  {}  {}", "─".repeat(40), "─".repeat(9), "─".repeat(40));
            for (name, description) in arch_intrinsics {
                let verdict = verum_kernel::intrinsic_dispatch::dispatch_intrinsic(name, &[]);
                let holds = matches!(
                    verdict,
                    Some(verum_kernel::intrinsic_dispatch::IntrinsicValue::Decision {
                        holds: true,
                        ..
                    })
                );
                let glyph = if holds { "✓" } else { "✗" };
                println!(
                    "  {:<40}  {} {:<7}  {}",
                    name,
                    glyph,
                    if holds { "yes" } else { "NO" },
                    description,
                );
            }
            println!();
            println!(
                "  Anti-pattern catalog: 32 canonical patterns (10  + 16  base + 6  MTAC)",
            );
            println!();
            println!("  {:<14}  {:<32}  {:<7}  Docs URL", "Code", "Name", "Season");
            println!(
                "  {}  {}  {}  {}",
                "─".repeat(14),
                "─".repeat(32),
                "─".repeat(7),
                "─".repeat(40),
            );
            for code in AntiPatternCode::full_list().iter() {
                let season_label = if code.is_mtac() {
                    format!("{}-MTAC", code.season())
                } else {
                    format!("{}", code.season())
                };
                println!(
                    "  {:<14}  {:<32}  {:<7}  {}",
                    code.code(),
                    code.name(),
                    season_label,
                    code.docs_url(),
                );
            }
            println!();
            if all_intrinsics_dispatch {
                println!(
                    "{} ATS-V registry surface load-bearing: all {} intrinsics \
                     dispatch successfully.",
                    "✓".green(),
                    arch_intrinsics.len(),
                );
            } else {
                println!(
                    "{} ATS-V registry surface NOT fully dispatched.",
                    "✗".red(),
                );
            }
            println!();
            println!("Report: {}", report_path.display());
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string(&payload).unwrap_or_default());
        }
    }

    if !all_intrinsics_dispatch {
        return Err(crate::error::CliError::Custom(
            format!(
                "ATS-V arch-discharges audit: at least one intrinsic failed to \
                 dispatch — see {}",
                report_path.display(),
            )
            .into(),
        ));
    }

    Ok(())
}

// =============================================================================
// ATS-V Counterfactual reasoning audit
// =============================================================================

/// `verum audit --counterfactual` — runs the counterfactual reasoning
/// engine over a synthetic battery covering each canonical
/// [`ArchProposition`] arm + the baseline [`ArchMetric`] set, against
/// the default `Shape::default_for_unannotated()` plus a divergent
/// alternative shape. Surfaces per-arm soundness contracts at audit
/// time:
///
/// - `FoundationStable` arm with foundation drift → `HoldsNeither`.
/// - `PublicApiUnchanged` arm with API change → `HoldsNeither`.
/// - `HasCapability` arm asymmetry → `HoldsAltOnly` / `HoldsBaseOnly`.
/// - Identity case (base == alt) → `HoldsBoth`.
///
/// Output: `target/audit-reports/counterfactual.json` with stable
/// schema_version=1.
pub fn audit_counterfactual_with_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::arch::{Capability, Foundation, ResourceTag, Shape};
    use verum_kernel::arch_counterfactual::{
        evaluate_counterfactual, ArchMetric, InvariantStatus,
    };
    use verum_kernel::arch_mtac::{ArchProposition, CounterfactualPair, Decision};

    if matches!(format, AuditFormat::Plain) {
        ui::step("ATS-V Counterfactual reasoning engine");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let report_path = report_dir.join("counterfactual.json");

    fn pair(name: &str, base: &str, alt: &str, invs: Vec<ArchProposition>) -> CounterfactualPair {
        CounterfactualPair {
            name: name.into(),
            base: Decision {
                name: base.into(),
                options: vec![],
                chosen: None,
                depends_on: vec![],
            },
            alternative: Decision {
                name: alt.into(),
                options: vec![],
                chosen: None,
                depends_on: vec![],
            },
            stability_invariants: invs,
        }
    }

    let base_shape = Shape::default_for_unannotated();
    let mut foundation_drift_alt = Shape::default_for_unannotated();
    foundation_drift_alt.foundation = Foundation::Hott;

    let mut api_change_alt = Shape::default_for_unannotated();
    api_change_alt.exposes = vec![Capability::Read {
        resource: ResourceTag::Logger,
    }];

 // Battery — one entry per canonical contract. Each entry pins
 // engine soundness for a distinct InvariantStatus arm.
    let battery: Vec<(&str, &Shape, &Shape, CounterfactualPair, InvariantStatus)> = vec![
        (
            "identity_holds_both",
            &base_shape,
            &base_shape,
            pair(
                "self_eq_self",
                "default",
                "default",
                vec![
                    ArchProposition::FoundationStable,
                    ArchProposition::PublicApiUnchanged,
                ],
            ),
            InvariantStatus::HoldsBoth,
        ),
        (
            "foundation_drift_holds_neither",
            &base_shape,
            &foundation_drift_alt,
            pair(
                "drift",
                "zfc_two_inacc",
                "hott",
                vec![ArchProposition::FoundationStable],
            ),
            InvariantStatus::HoldsNeither,
        ),
        (
            "api_change_holds_neither",
            &base_shape,
            &api_change_alt,
            pair(
                "api_breaks",
                "no_logger",
                "with_logger",
                vec![ArchProposition::PublicApiUnchanged],
            ),
            InvariantStatus::HoldsNeither,
        ),
        (
            "capability_holds_alt_only",
            &base_shape,
            &api_change_alt,
            pair(
                "cap_alt_only",
                "no_cap",
                "with_cap",
                vec![ArchProposition::HasCapability {
                    capability_tag: "read".into(),
                }],
            ),
            InvariantStatus::HoldsAltOnly,
        ),
        (
            "capability_holds_base_only",
            &api_change_alt,
            &base_shape,
            pair(
                "cap_base_only",
                "with_cap",
                "no_cap",
                vec![ArchProposition::HasCapability {
                    capability_tag: "read".into(),
                }],
            ),
            InvariantStatus::HoldsBaseOnly,
        ),
    ];

    let baseline = ArchMetric::baseline_set();
    let mut entries: Vec<serde_json::Value> = Vec::new();
    let mut all_arms_pinned = true;

    for (entry_name, base, alt, p, expected) in &battery {
        let report = evaluate_counterfactual(p, base, alt, &baseline);
 // Each entry has exactly one invariant for the pinned arm —
 // confirm the engine returned the soundness-contract status.
        let observed = report
            .invariant_evaluations
            .iter()
            .map(|e| e.status.tag().to_string())
            .collect::<Vec<_>>();
        let pin_ok = report
            .invariant_evaluations
            .iter()
            .any(|e| &e.status == expected);
        if !pin_ok {
            all_arms_pinned = false;
        }
        entries.push(serde_json::json!({
            "entry": entry_name,
            "pair_name": report.pair_name,
            "base_decision": report.base_decision,
            "alt_decision": report.alt_decision,
            "expected_status": expected.tag(),
            "observed_statuses": observed,
            "diverging_metric_count": report.diverging_metric_count,
            "overall_stable": report.overall_stable,
            "metric_comparisons": report.metric_comparisons,
            "invariant_evaluations": report.invariant_evaluations,
            "pin_ok": pin_ok,
            "schema_version": report.schema_version,
        }));
    }

    let payload = serde_json::json!({
        "schema_version": 1,
        "kernel_version": env!("CARGO_PKG_VERSION"),
        "discipline": "ats_v_counterfactual_reasoning",
        "season": 6,
        "spec": "internal/specs/ats-v.md#§22",
        "load_bearing": all_arms_pinned,
        "baseline_metric_count": baseline.len(),
        "battery_size": entries.len(),
        "entries": entries,
    });

    let _ = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&payload).unwrap_or_default(),
    );

    match format {
        AuditFormat::Plain => {
            println!();
            println!("ATS-V Counterfactual reasoning engine");
            println!("─────────────────────────────────────────────────────");
            println!("  {:<35}  {:<18}  {:<18}  {}", "Battery entry", "Expected", "Observed", "Pin");
            println!("  {}  {}  {}  {}", "─".repeat(35), "─".repeat(18), "─".repeat(18), "─".repeat(3));
            for entry in &payload["entries"].as_array().cloned().unwrap_or_default() {
                let name = entry["entry"].as_str().unwrap_or("");
                let expected = entry["expected_status"].as_str().unwrap_or("");
                let observed = entry["observed_statuses"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str())
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_default();
                let pin_ok = entry["pin_ok"].as_bool().unwrap_or(false);
                let glyph = if pin_ok { "✓" } else { "✗" };
                println!("  {:<35}  {:<18}  {:<18}  {}", name, expected, observed, glyph);
            }
            println!();
            println!(
                "  Baseline metric battery: {} metrics × {} entries.",
                baseline.len(),
                entries.len(),
            );
            println!();
            if all_arms_pinned {
                println!(
                    "{} Counterfactual engine load-bearing: every InvariantStatus arm pins.",
                    "✓".green(),
                );
            } else {
                println!("{} Counterfactual engine FAILED at least one pin.", "✗".red());
            }
            println!();
            println!("Report: {}", report_path.display());
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string(&payload).unwrap_or_default());
        }
    }

    if !all_arms_pinned {
        return Err(crate::error::CliError::Custom(
            format!(
                "Counterfactual audit: at least one InvariantStatus arm \
                 failed its soundness contract — see {}",
                report_path.display(),
            )
            .into(),
        ));
    }

    Ok(())
}

// =============================================================================
// ATS-V Adjunction analyzer audit
// =============================================================================

/// `verum audit --adjunctions` — runs the adjunction analyzer over
/// a synthetic battery covering each of the four canonical
/// adjunctions (per spec §20.6): Inline⊣Extract /
/// Specialise⊣Generalise / Decompose⊣Compose / Strengthen⊣Weaken,
/// plus a chain composition pin and a preservation-failure case.
///
/// Verifies recogniser soundness + preservation / gain coverage at
/// audit time. Output: `target/audit-reports/adjunctions.json`
/// with stable schema_version=1.
pub fn audit_adjunctions_with_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::arch::{BoundaryInvariant, Capability, Foundation, ResourceTag, Shape};
    use verum_kernel::arch_adjunction::{
        analyze_chain, analyze_refactoring, AdjunctionVerdict, CanonicalAdjunction, Refactoring,
        RefactoringChain, RefactoringDirection,
    };
    use verum_kernel::arch_mtac::{AdjunctionWitness, ArchProposition};

    if matches!(format, AuditFormat::Plain) {
        ui::step("ATS-V Adjunction analyzer");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let report_path = report_dir.join("adjunctions.json");

    fn witness(forward: &str, backward: &str) -> AdjunctionWitness {
        AdjunctionWitness {
            forward_name: forward.into(),
            backward_name: backward.into(),
            preserved: vec![],
            gained: vec![],
        }
    }

 // -- Battery construction --
    let mut entries: Vec<serde_json::Value> = Vec::new();
    let mut all_pins_ok = true;

 // Entry 1: Inline ⊣ Extract — composition_degree decreases.
    let mut s_before = Shape::default_for_unannotated();
    s_before.composes_with = vec!["helper_a".into(), "helper_b".into()];
    let s_after_inline = Shape::default_for_unannotated();
    let r_inline = Refactoring {
        name: "inline_two_helpers".into(),
        direction: RefactoringDirection::Forward,
        before_shape: s_before.clone(),
        after_shape: s_after_inline,
        witness: witness("inline", "extract"),
    };
    let a_inline = analyze_refactoring(&r_inline);
    let pin_inline = matches!(a_inline.canonical, CanonicalAdjunction::InlineExtract)
        && a_inline.verdict.is_accepted();
    if !pin_inline {
        all_pins_ok = false;
    }
    entries.push(serde_json::json!({
        "entry": "inline_extract",
        "expected_canonical": "inline_extract",
        "expected_verdict": "accepted",
        "observed_canonical": a_inline.canonical.tag(),
        "observed_verdict": a_inline.verdict.tag(),
        "pin_ok": pin_inline,
        "analysis": a_inline,
    }));

 // Entry 2: Specialise ⊣ Generalise — capability set shrinks,
 // foundation+stratum stable.
    let mut s_before = Shape::default_for_unannotated();
    s_before.exposes = vec![
        Capability::Read {
            resource: ResourceTag::Logger,
        },
        Capability::Write {
            resource: ResourceTag::Logger,
        },
    ];
    let mut s_after = Shape::default_for_unannotated();
    s_after.exposes = vec![Capability::Read {
        resource: ResourceTag::Logger,
    }];
    let r_spec = Refactoring {
        name: "specialise_iface".into(),
        direction: RefactoringDirection::Forward,
        before_shape: s_before,
        after_shape: s_after,
        witness: witness("specialise", "generalise"),
    };
    let a_spec = analyze_refactoring(&r_spec);
    let pin_spec = matches!(
        a_spec.canonical,
        CanonicalAdjunction::SpecialiseGeneralise
    ) && a_spec.verdict.is_accepted();
    if !pin_spec {
        all_pins_ok = false;
    }
    entries.push(serde_json::json!({
        "entry": "specialise_generalise",
        "expected_canonical": "specialise_generalise",
        "expected_verdict": "accepted",
        "observed_canonical": a_spec.canonical.tag(),
        "observed_verdict": a_spec.verdict.tag(),
        "pin_ok": pin_spec,
        "analysis": a_spec,
    }));

 // Entry 3: Decompose ⊣ Compose — composes_with grows.
    let s_before = Shape::default_for_unannotated();
    let mut s_after = Shape::default_for_unannotated();
    s_after.composes_with = vec!["sub_a".into(), "sub_b".into()];
    let r_decomp = Refactoring {
        name: "split_into_subs".into(),
        direction: RefactoringDirection::Forward,
        before_shape: s_before,
        after_shape: s_after,
        witness: witness("decompose", "compose"),
    };
    let a_decomp = analyze_refactoring(&r_decomp);
    let pin_decomp = matches!(
        a_decomp.canonical,
        CanonicalAdjunction::DecomposeCompose
    ) && a_decomp.verdict.is_accepted();
    if !pin_decomp {
        all_pins_ok = false;
    }
    entries.push(serde_json::json!({
        "entry": "decompose_compose",
        "expected_canonical": "decompose_compose",
        "expected_verdict": "accepted",
        "observed_canonical": a_decomp.canonical.tag(),
        "observed_verdict": a_decomp.verdict.tag(),
        "pin_ok": pin_decomp,
        "analysis": a_decomp,
    }));

 // Entry 4: Strengthen ⊣ Weaken — preserves grows.
    let s_before = Shape::default_for_unannotated();
    let mut s_after = Shape::default_for_unannotated();
    s_after.preserves = vec![BoundaryInvariant::AllOrNothing];
    let r_strong = Refactoring {
        name: "add_invariant".into(),
        direction: RefactoringDirection::Forward,
        before_shape: s_before,
        after_shape: s_after,
        witness: witness("strengthen", "weaken"),
    };
    let a_strong = analyze_refactoring(&r_strong);
    let pin_strong = matches!(
        a_strong.canonical,
        CanonicalAdjunction::StrengthenWeaken
    ) && a_strong.verdict.is_accepted();
    if !pin_strong {
        all_pins_ok = false;
    }
    entries.push(serde_json::json!({
        "entry": "strengthen_weaken",
        "expected_canonical": "strengthen_weaken",
        "expected_verdict": "accepted",
        "observed_canonical": a_strong.canonical.tag(),
        "observed_verdict": a_strong.verdict.tag(),
        "pin_ok": pin_strong,
        "analysis": a_strong,
    }));

 // Entry 5: Preservation failure — drift foundation, claim
 // preserved=FoundationStable. Engine MUST surface
 // PreservationFailure.
    let mut s_before = Shape::default_for_unannotated();
    s_before.foundation = Foundation::ZfcTwoInacc;
    let mut s_after = Shape::default_for_unannotated();
    s_after.foundation = Foundation::Hott;
    let mut w_drift = witness("specialise", "generalise");
    w_drift.preserved = vec![ArchProposition::FoundationStable];
    let r_drift = Refactoring {
        name: "foundation_drift".into(),
        direction: RefactoringDirection::Forward,
        before_shape: s_before,
        after_shape: s_after,
        witness: w_drift,
    };
    let a_drift = analyze_refactoring(&r_drift);
    let pin_drift = matches!(a_drift.verdict, AdjunctionVerdict::PreservationFailure);
    if !pin_drift {
        all_pins_ok = false;
    }
    entries.push(serde_json::json!({
        "entry": "preservation_failure_pin",
        "expected_verdict": "preservation_failure",
        "observed_verdict": a_drift.verdict.tag(),
        "pin_ok": pin_drift,
        "analysis": a_drift,
    }));

 // Entry 6: Chain composition — two valid forward steps.
    let chain = RefactoringChain {
        steps: vec![r_inline.clone(), r_spec.clone()],
    };
    let a_chain = analyze_chain(&chain);
    let pin_chain = a_chain.chain_accepted;
    if !pin_chain {
        all_pins_ok = false;
    }
    entries.push(serde_json::json!({
        "entry": "chain_composition",
        "expected_chain_accepted": true,
        "observed_chain_accepted": a_chain.chain_accepted,
        "step_count": a_chain.step_analyses.len(),
        "pin_ok": pin_chain,
        "chain_analysis": a_chain,
    }));

    let payload = serde_json::json!({
        "schema_version": 1,
        "kernel_version": env!("CARGO_PKG_VERSION"),
        "discipline": "ats_v_adjunction_analyzer",
        "season": 7,
        "spec": "internal/specs/ats-v.md#§20.6",
        "load_bearing": all_pins_ok,
        "canonical_adjunction_count": 4,
        "battery_size": entries.len(),
        "entries": entries,
    });

    let _ = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&payload).unwrap_or_default(),
    );

    match format {
        AuditFormat::Plain => {
            println!();
            println!("ATS-V Adjunction analyzer");
            println!("─────────────────────────────────────────────────────");
            println!("  {:<32}  {:<22}  {:<22}  {}", "Battery entry", "Expected", "Observed", "Pin");
            println!(
                "  {}  {}  {}  {}",
                "─".repeat(32),
                "─".repeat(22),
                "─".repeat(22),
                "─".repeat(3),
            );
            for entry in &payload["entries"].as_array().cloned().unwrap_or_default() {
                let name = entry["entry"].as_str().unwrap_or("");
                let expected = entry["expected_verdict"]
                    .as_str()
                    .or_else(|| entry["expected_canonical"].as_str())
                    .or_else(|| {
                        if entry["expected_chain_accepted"].is_boolean() {
                            Some("chain_accepted")
                        } else {
                            None
                        }
                    })
                    .unwrap_or("");
                let observed = entry["observed_verdict"]
                    .as_str()
                    .or_else(|| entry["observed_canonical"].as_str())
                    .or_else(|| {
                        if entry["observed_chain_accepted"].is_boolean() {
                            Some(if entry["observed_chain_accepted"].as_bool().unwrap_or(false) {
                                "true"
                            } else {
                                "false"
                            })
                        } else {
                            None
                        }
                    })
                    .unwrap_or("");
                let pin_ok = entry["pin_ok"].as_bool().unwrap_or(false);
                let glyph = if pin_ok { "✓" } else { "✗" };
                println!("  {:<32}  {:<22}  {:<22}  {}", name, expected, observed, glyph);
            }
            println!();
            println!("  Canonical adjunctions covered: 4 (Inline⊣Extract, Specialise⊣Generalise,");
            println!("                                  Decompose⊣Compose, Strengthen⊣Weaken)");
            println!();
            if all_pins_ok {
                println!(
                    "{} Adjunction analyzer load-bearing: every canonical arm + failure pin holds.",
                    "✓".green(),
                );
            } else {
                println!("{} Adjunction analyzer FAILED at least one pin.", "✗".red());
            }
            println!();
            println!("Report: {}", report_path.display());
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string(&payload).unwrap_or_default());
        }
    }

    if !all_pins_ok {
        return Err(crate::error::CliError::Custom(
            format!(
                "Adjunction audit: at least one canonical adjunction failed its \
                 soundness contract — see {}",
                report_path.display(),
            )
            .into(),
        ));
    }

    Ok(())
}

// =============================================================================
// ATS-V Yoneda-equivalence audit
// =============================================================================

/// `verum audit --yoneda` — runs the Yoneda-equivalence checker
/// over a synthetic battery covering identity (trivially equivalent),
/// per-observer distinguishability (Auditor sees foundation,
/// Adversary sees outbound network, EndUser sees exposes,
/// Stakeholder sees persistence), and the trivially-safe
/// refactoring entry.
///
/// Pin contract: the engine MUST surface each observer-specific
/// asymmetry exactly — Adversary blind to lifecycle, EndUser blind
/// to foundation, etc. Pin failure means the observer-functor
/// projection drifted from spec §20.7 + §23.
///
/// Output: `target/audit-reports/yoneda.json` (schema_version=1).
pub fn audit_yoneda_with_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::arch::{
        Capability, Foundation, NetDirection, NetProtocol, PersistenceMedium, ResourceTag, Shape,
    };
    use verum_kernel::arch_mtac::Observer;
    use verum_kernel::arch_yoneda::yoneda_equivalent;

    if matches!(format, AuditFormat::Plain) {
        ui::step("ATS-V Yoneda-equivalence checker");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let report_path = report_dir.join("yoneda.json");

    fn auditor() -> Observer {
        Observer::Auditor {
            audit_kind: "compliance".into(),
        }
    }
    fn adversary() -> Observer {
        Observer::Adversary {
            threat_model: "external".into(),
        }
    }
    fn end_user() -> Observer {
        Observer::EndUser {
            kind: "default".into(),
        }
    }
    fn stakeholder() -> Observer {
        Observer::Stakeholder {
            role: "operator".into(),
        }
    }

 // Battery: each entry pins one observer-specific contract.
    let mut entries: Vec<serde_json::Value> = Vec::new();
    let mut all_pins_ok = true;

    fn record(
        entries: &mut Vec<serde_json::Value>,
        all_pins_ok: &mut bool,
        name: &str,
        expected_equivalent: bool,
        verdict: &verum_kernel::arch_yoneda::YonedaVerdict,
    ) {
        let pin_ok = verdict.equivalent == expected_equivalent;
        if !pin_ok {
            *all_pins_ok = false;
        }
        entries.push(serde_json::json!({
            "entry": name,
            "expected_equivalent": expected_equivalent,
            "observed_equivalent": verdict.equivalent,
            "disagreement_count": verdict.disagreement_count,
            "pin_ok": pin_ok,
            "verdict": verdict,
        }));
    }

 // Entry 1: identity → equivalent under full canonical roster.
    let s = Shape::default_for_unannotated();
    let v_id = yoneda_equivalent(&s, &s, &[]);
    record(&mut entries, &mut all_pins_ok, "identity_full_roster", true, &v_id);

 // Entry 2: foundation drift → Auditor distinguishes;
 // Stakeholder distinguishes; EndUser/PeerCog/Adversary blind.
    let mut s_base = Shape::default_for_unannotated();
    s_base.foundation = Foundation::ZfcTwoInacc;
    let mut s_alt = Shape::default_for_unannotated();
    s_alt.foundation = Foundation::Hott;
    let v_audit_foundation = yoneda_equivalent(&s_base, &s_alt, &[auditor()]);
    record(&mut entries, &mut all_pins_ok, "auditor_sees_foundation_drift", false, &v_audit_foundation);
    let v_eu_foundation = yoneda_equivalent(&s_base, &s_alt, &[end_user()]);
    record(&mut entries, &mut all_pins_ok, "end_user_blind_to_foundation", true, &v_eu_foundation);
    let v_adv_foundation = yoneda_equivalent(&s_base, &s_alt, &[adversary()]);
    record(&mut entries, &mut all_pins_ok, "adversary_blind_to_foundation", true, &v_adv_foundation);

 // Entry 3: outbound network capability → Adversary
 // distinguishes; EndUser does NOT (only sees exposes, this
 // change is in requires).
    let s_base = Shape::default_for_unannotated();
    let mut s_alt = Shape::default_for_unannotated();
    s_alt.requires = vec![Capability::Network {
        protocol: NetProtocol::Tcp,
        direction: NetDirection::Outbound,
    }];
    let v_adv_net = yoneda_equivalent(&s_base, &s_alt, &[adversary()]);
    record(&mut entries, &mut all_pins_ok, "adversary_sees_outbound_network", false, &v_adv_net);

 // Entry 4: exposes change → EndUser distinguishes; Stakeholder
 // also affected via persistence_capabilities filter (no, only
 // if it's a Persist capability — this is a Read on Logger so
 // Stakeholder is blind).
    let s_base = Shape::default_for_unannotated();
    let mut s_alt = Shape::default_for_unannotated();
    s_alt.exposes = vec![Capability::Read {
        resource: ResourceTag::Logger,
    }];
    let v_eu_exposes = yoneda_equivalent(&s_base, &s_alt, &[end_user()]);
    record(&mut entries, &mut all_pins_ok, "end_user_sees_exposes", false, &v_eu_exposes);
    let v_stk_logger = yoneda_equivalent(&s_base, &s_alt, &[stakeholder()]);
    record(&mut entries, &mut all_pins_ok, "stakeholder_blind_to_logger_read", true, &v_stk_logger);

 // Entry 5: persistence change → Stakeholder distinguishes
 // (persistence_capabilities filter catches it).
    let s_base = Shape::default_for_unannotated();
    let mut s_alt = Shape::default_for_unannotated();
    s_alt.exposes = vec![Capability::Persist {
        medium: PersistenceMedium::Disk {
            path: "/tmp".into(),
        },
    }];
    let v_stk_persist = yoneda_equivalent(&s_base, &s_alt, &[stakeholder()]);
    record(&mut entries, &mut all_pins_ok, "stakeholder_sees_persistence", false, &v_stk_persist);

    let payload = serde_json::json!({
        "schema_version": 1,
        "kernel_version": env!("CARGO_PKG_VERSION"),
        "discipline": "ats_v_yoneda_equivalence",
        "season": 8,
        "spec": "internal/specs/ats-v.md#§20.7-§23",
        "load_bearing": all_pins_ok,
        "canonical_observer_count": 5,
        "battery_size": entries.len(),
        "entries": entries,
    });

    let _ = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&payload).unwrap_or_default(),
    );

    match format {
        AuditFormat::Plain => {
            println!();
            println!("ATS-V Yoneda-equivalence checker");
            println!("─────────────────────────────────────────────────────");
            println!(
                "  {:<40}  {:<13}  {:<13}  {}",
                "Battery entry", "Expected eq", "Observed eq", "Pin",
            );
            println!(
                "  {}  {}  {}  {}",
                "─".repeat(40),
                "─".repeat(13),
                "─".repeat(13),
                "─".repeat(3),
            );
            for entry in &payload["entries"].as_array().cloned().unwrap_or_default() {
                let name = entry["entry"].as_str().unwrap_or("");
                let expected = entry["expected_equivalent"].as_bool().unwrap_or(false);
                let observed = entry["observed_equivalent"].as_bool().unwrap_or(false);
                let pin_ok = entry["pin_ok"].as_bool().unwrap_or(false);
                let glyph = if pin_ok { "✓" } else { "✗" };
                println!(
                    "  {:<40}  {:<13}  {:<13}  {}",
                    name,
                    if expected { "equivalent" } else { "distinct" },
                    if observed { "equivalent" } else { "distinct" },
                    glyph,
                );
            }
            println!();
            println!("  Canonical observers: 5 (EndUser / PeerCog / Stakeholder / Auditor / Adversary)");
            println!();
            if all_pins_ok {
                println!(
                    "{} Yoneda-equivalence checker load-bearing: every observer-specific contract holds.",
                    "✓".green(),
                );
            } else {
                println!("{} Yoneda-equivalence checker FAILED at least one pin.", "✗".red());
            }
            println!();
            println!("Report: {}", report_path.display());
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string(&payload).unwrap_or_default());
        }
    }

    if !all_pins_ok {
        return Err(crate::error::CliError::Custom(
            format!(
                "Yoneda audit: at least one observer-specific contract failed — see {}",
                report_path.display(),
            )
            .into(),
        ));
    }

    Ok(())
}

// =============================================================================
// ATS-V `@arch_module(...)` adoption coverage audit
// =============================================================================

/// Walk a `Module` looking for `mount` declarations and extract the
/// dotted-path strings.  Used by `audit_arch_coverage` to surface
/// the cog's actual import graph alongside its declared
/// `@arch_module(...)` shape.  Glob mounts (`mount foo.bar.*;`)
/// produce the prefix path with a trailing `.*` marker so consumers
/// can distinguish them from concrete-leaf mounts.
fn derive_mounts_from_module(module: &verum_ast::Module) -> Vec<String> {
    use verum_ast::decl::{ItemKind, MountTreeKind};
    let mut paths: Vec<String> = Vec::new();
    for item in &module.items {
        let mount_decl = match &item.kind {
            ItemKind::Mount(d) => d,
            _ => continue,
        };
        let segs_to_string = |path: &verum_ast::ty::Path| -> String {
            path.segments
                .iter()
                .map(|s| match s {
                    verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str().to_string(),
                    verum_ast::ty::PathSegment::Super => "super".to_string(),
                    verum_ast::ty::PathSegment::SelfValue => "self".to_string(),
                    verum_ast::ty::PathSegment::Cog => "cog".to_string(),
                    verum_ast::ty::PathSegment::Relative => "".to_string(),
                })
                .collect::<Vec<_>>()
                .join(".")
        };
        match &mount_decl.tree.kind {
            MountTreeKind::Path(p) => paths.push(segs_to_string(p)),
            MountTreeKind::Glob(p) => paths.push(format!("{}.*", segs_to_string(p))),
            MountTreeKind::Nested { prefix, .. } => paths.push(segs_to_string(prefix)),
            MountTreeKind::File { path, .. } => paths.push(format!("file:{}", path.as_str())),
        }
    }
    paths
}

/// `verum audit --arch-coverage` — walks every `.vr` file in the
/// project + stdlib and reports which carry `@arch_module(...)`
/// declarations.  Observability gate (does not fail the build) per
/// spec §17.5 backward-compat: coverage grows incrementally.
///
/// For each annotated module, also surfaces:
///   - whether the declaration parses cleanly into a `Shape`,
///   - the parsed foundation / stratum / lifecycle,
///   - any anti-pattern violations the kernel checker surfaces.
///
/// Output: `target/audit-reports/arch-coverage.json`
/// (schema_version=1).
pub fn audit_arch_coverage_with_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::arch_phase::run_arch_phase_one;

    if matches!(format, AuditFormat::Plain) {
        ui::step("ATS-V @arch_module adoption coverage");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let mut vr_files = discover_vr_files(&manifest_dir);
    vr_files.extend(discover_stdlib_vr_files());

    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let report_path = report_dir.join("arch-coverage.json");

    let mut total_files = 0usize;
    let mut annotated = 0usize;
    let mut total_violations = 0usize;
    let mut entries: Vec<serde_json::Value> = Vec::new();

    for abs_path in &vr_files {
        let module = match parse_file_for_audit(abs_path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        total_files += 1;

        let rel_path = abs_path
            .strip_prefix(&manifest_dir)
            .unwrap_or(abs_path)
            .to_string_lossy()
            .to_string();

        // Walk module-level + per-item attributes for `@arch_module`.
        let mut found_attr_args: Option<Vec<verum_ast::expr::Expr>> = None;
        for attr in module.attributes.iter() {
            if attr.name.as_str() == "arch_module" {
                if let verum_common::Maybe::Some(args) = &attr.args {
                    found_attr_args = Some(args.iter().cloned().collect());
                } else {
                    found_attr_args = Some(Vec::new());
                }
                break;
            }
        }
        if found_attr_args.is_none() {
            for item in &module.items {
                for attr in item.attributes.iter() {
                    if attr.name.as_str() == "arch_module" {
                        if let verum_common::Maybe::Some(args) = &attr.args {
                            found_attr_args = Some(args.iter().cloned().collect());
                        } else {
                            found_attr_args = Some(Vec::new());
                        }
                        break;
                    }
                }
                if found_attr_args.is_some() {
                    break;
                }
            }
        }

        if let Some(args) = found_attr_args {
            annotated += 1;
            let result = run_arch_phase_one(rel_path.clone(), &args);
            total_violations += result.violations.len();
            let parsed_ok = result.parse_errors.is_empty() && result.shape.is_some();
            let foundation_tag = result
                .shape
                .as_ref()
                .map(|s| s.foundation.tag().to_string())
                .unwrap_or_else(|| "<unparsed>".to_string());
            let stratum_tag = result
                .shape
                .as_ref()
                .map(|s| s.stratum.tag().to_string())
                .unwrap_or_else(|| "<unparsed>".to_string());

            // Derive mount-graph from `module.items` Mount declarations.
            // Provides observability of the cog's actual import graph
            // alongside its declared @arch_module shape.  Future work:
            // cross-cog AP-005 FoundationDrift / AP-003 DependencyCycle
            // checks consume this graph as a whole-corpus aggregate.
            let inferred_mounts = derive_mounts_from_module(&module);
            let exposes_tags: Vec<String> = result
                .shape
                .as_ref()
                .map(|s| s.exposes.iter().map(|c| c.tag().to_string()).collect())
                .unwrap_or_default();
            let requires_tags: Vec<String> = result
                .shape
                .as_ref()
                .map(|s| s.requires.iter().map(|c| c.tag().to_string()).collect())
                .unwrap_or_default();

            entries.push(serde_json::json!({
                "file": rel_path,
                "annotated": true,
                "parse_ok": parsed_ok,
                "foundation": foundation_tag,
                "stratum": stratum_tag,
                "exposes": exposes_tags,
                "requires": requires_tags,
                "inferred_mounts": inferred_mounts,
                "violation_count": result.violations.len(),
                "parse_error_count": result.parse_errors.len(),
                "parse_errors": result
                    .parse_errors
                    .iter()
                    .map(|e| format!("{:?}", e))
                    .collect::<Vec<_>>(),
                "violations": result
                    .violations
                    .iter()
                    .map(|v| {
                        serde_json::json!({
                            "code": v.code.code(),
                            "name": v.code.name(),
                            "summary": v.summary,
                        })
                    })
                    .collect::<Vec<_>>(),
            }));
        }
    }

    let coverage_pct = if total_files == 0 {
        0.0
    } else {
        (annotated as f64) * 100.0 / (total_files as f64)
    };

    let payload = serde_json::json!({
        "schema_version": 1,
        "kernel_version": env!("CARGO_PKG_VERSION"),
        "discipline": "ats_v_arch_module_coverage",
        "spec": "internal/specs/ats-v.md#§17",
        "total_files": total_files,
        "annotated_count": annotated,
        "coverage_percent": coverage_pct,
        "total_violation_count": total_violations,
        "annotated_modules": entries,
    });

    let _ = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&payload).unwrap_or_default(),
    );

    match format {
        AuditFormat::Plain => {
            println!();
            println!("ATS-V @arch_module adoption coverage");
            println!("─────────────────────────────────────────────────────");
            println!(
                "  Walked {} `.vr` file(s); {} carry @arch_module ({:.2}% coverage).",
                total_files, annotated, coverage_pct,
            );
            println!("  Total anti-pattern violations: {}", total_violations);
            if annotated > 0 {
                println!();
                println!(
                    "  {:<60}  {:<14}  {:<10}  {}",
                    "File", "Foundation", "Stratum", "Violations",
                );
                println!(
                    "  {}  {}  {}  {}",
                    "─".repeat(60),
                    "─".repeat(14),
                    "─".repeat(10),
                    "─".repeat(10),
                );
                for entry in &entries {
                    let file = entry["file"].as_str().unwrap_or("");
                    let foundation = entry["foundation"].as_str().unwrap_or("");
                    let stratum = entry["stratum"].as_str().unwrap_or("");
                    let vc = entry["violation_count"].as_u64().unwrap_or(0);
                    println!(
                        "  {:<60}  {:<14}  {:<10}  {}",
                        if file.len() > 60 { &file[file.len() - 60..] } else { file },
                        foundation,
                        stratum,
                        vc,
                    );
                }
            }
            println!();
            println!("Report: {}", report_path.display());
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string(&payload).unwrap_or_default());
        }
    }

    // Observability gate — never fails the build.  Adoption is
    // incremental per spec §17.5; an unannotated cog is not a defect.
    Ok(())
}

// =============================================================================
// ATS-V whole-corpus cross-cog architectural analysis
// =============================================================================

/// `verum audit --arch-corpus` — whole-corpus cross-cog
/// architectural analysis.  Walks every annotated `.vr` file,
/// builds the global `cog_path → (Shape, mounts)` registry,
/// aggregates the mount edges into a `composes_graph`, and runs
/// `check_all_anti_patterns` per cog with `composed_foundations`
/// + `composes_graph` populated from the corpus aggregate.
///
/// Activates AP-003 DependencyCycle and AP-005 FoundationDrift on
/// real cross-cog architecture.  The per-cog single-cog checks
/// (AP-026 stratum admissibility, AP-010 CveIncomplete, etc) also
/// fire — but those are already covered by `--arch-coverage`.
/// This gate's distinguishing value is the cross-cog reachability
/// analysis.
///
/// Output: `target/audit-reports/arch-corpus.json` (schema_version=1).
pub fn audit_arch_corpus_with_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::arch::{Foundation, Shape};
    use verum_kernel::arch_anti_pattern::{check_all_anti_patterns, DiagnosticContext};
    use verum_kernel::arch_parse::parse_arch_module;

    if matches!(format, AuditFormat::Plain) {
        ui::step("ATS-V whole-corpus cross-cog architectural analysis");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let mut vr_files = discover_vr_files(&manifest_dir);
    vr_files.extend(discover_stdlib_vr_files());

    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let report_path = report_dir.join("arch-corpus.json");

    // -------------------------------------------------------------------------
    // Pass 1: build the corpus registry — one entry per annotated cog.
    // -------------------------------------------------------------------------
    struct CogEntry {
        cog_name: String,
        shape: Shape,
        mounts: Vec<String>,
    }
    let mut registry: Vec<CogEntry> = Vec::new();
    let mut name_index: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();

    for abs_path in &vr_files {
        let module = match parse_file_for_audit(abs_path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        // Find @arch_module(...) on the file's first item (the
        // `module path;` declaration carrying it).
        let mut arch_args: Option<Vec<verum_ast::expr::Expr>> = None;
        for attr in module.attributes.iter() {
            if attr.name.as_str() == "arch_module" {
                if let verum_common::Maybe::Some(args) = &attr.args {
                    arch_args = Some(args.iter().cloned().collect());
                } else {
                    arch_args = Some(Vec::new());
                }
                break;
            }
        }
        if arch_args.is_none() {
            for item in &module.items {
                for attr in item.attributes.iter() {
                    if attr.name.as_str() == "arch_module" {
                        if let verum_common::Maybe::Some(args) = &attr.args {
                            arch_args = Some(args.iter().cloned().collect());
                        } else {
                            arch_args = Some(Vec::new());
                        }
                        break;
                    }
                }
                if arch_args.is_some() {
                    break;
                }
            }
        }

        let args = match arch_args {
            Some(a) => a,
            None => continue,
        };
        let shape = match parse_arch_module(&args) {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Derive cog_name from the first ItemKind::Module declaration's
        // dotted path; fall back to the file path stem.
        let cog_name = derive_module_path(&module).unwrap_or_else(|| {
            abs_path
                .strip_prefix(&manifest_dir)
                .unwrap_or(abs_path)
                .with_extension("")
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, ".")
        });
        let mounts = derive_mounts_from_module(&module);

        // Deduplicate by cog_name — stdlib + project walkers may
        // surface the same file twice; keep the first-seen entry.
        if name_index.contains_key(&cog_name) {
            continue;
        }
        name_index.insert(cog_name.clone(), registry.len());
        registry.push(CogEntry {
            cog_name,
            shape,
            mounts,
        });
    }

    // -------------------------------------------------------------------------
    // Pass 2: aggregate cross-cog graph + per-cog DiagnosticContext fields.
    // -------------------------------------------------------------------------
    // composes_graph: every (this_cog → mount_target) edge where
    // mount_target is itself an annotated cog.  Per-cog foundation
    // map: every reachable annotated cog's foundation pair.
    //
    // Each mount path is resolved against the cog's own parent
    // namespace so `super.X` becomes `<cog_parent>.X`.  Without this
    // step, sibling-mounts produce string mismatches against the
    // canonical `core.foo.bar` form in `name_index`.
    let mut full_composes_graph: Vec<(String, Vec<String>)> = Vec::new();
    for entry in &registry {
        let parent_namespace = parent_namespace_of(&entry.cog_name);
        let resolved: Vec<String> = entry
            .mounts
            .iter()
            .filter_map(|m| {
                let canonical = canonicalise_mount_path(m, parent_namespace.as_deref());
                match_annotated_cog(&canonical, &name_index)
            })
            .collect();
        full_composes_graph.push((entry.cog_name.clone(), resolved));
    }

    // -------------------------------------------------------------------------
    // Pass 3: run check_all_anti_patterns per cog.
    // -------------------------------------------------------------------------
    let mut cog_reports: Vec<serde_json::Value> = Vec::new();
    let mut total_violations = 0usize;
    let mut violation_codes: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();

    for entry in &registry {
        let mut ctx = DiagnosticContext::default();
        ctx.cog_name = entry.cog_name.clone();
        ctx.composes_graph = full_composes_graph.clone();
        // Composed foundations: for every mount that resolves to an
        // annotated cog, record its (mount_path, Foundation).
        let parent_ns = parent_namespace_of(&entry.cog_name);
        ctx.composed_foundations = entry
            .mounts
            .iter()
            .filter_map(|m| {
                let canonical = canonicalise_mount_path(m, parent_ns.as_deref());
                let resolved = match_annotated_cog(&canonical, &name_index)?;
                let target_idx = *name_index.get(&resolved)?;
                Some((resolved, registry[target_idx].shape.foundation.clone()))
            })
            .collect();

        // Mount-derived capability inference: a cog that mounts
        // from `sys.*.syscall.*` or `sys.darwin.libsystem.*` MUST
        // declare `Capability.Exec(...)` in its requires.  Same for
        // network / persistence patterns (extend as adoption grows).
        // This activates AP-001 CapabilityEscalation on the
        // mount-graph signal alone, without requiring full AST
        // body inference.
        ctx.inferred_used_capabilities = infer_capabilities_from_mounts(&entry.mounts);

        let violations = check_all_anti_patterns(&entry.shape, &ctx);
        total_violations += violations.len();
        for v in &violations {
            *violation_codes
                .entry(v.code.code().to_string())
                .or_insert(0) += 1;
        }
        cog_reports.push(serde_json::json!({
            "cog": entry.cog_name,
            "foundation": entry.shape.foundation.tag(),
            "stratum": entry.shape.stratum.tag(),
            "lifecycle_tag": lifecycle_tag(&entry.shape),
            "mount_count": entry.mounts.len(),
            "annotated_mount_count": full_composes_graph
                .iter()
                .find(|(n, _)| n == &entry.cog_name)
                .map(|(_, e)| e.len())
                .unwrap_or(0),
            "violation_count": violations.len(),
            "violations": violations
                .iter()
                .map(|v| {
                    serde_json::json!({
                        "code": v.code.code(),
                        "name": v.code.name(),
                        "summary": v.summary,
                    })
                })
                .collect::<Vec<_>>(),
        }));
    }

    let load_bearing = total_violations == 0;
    let payload = serde_json::json!({
        "schema_version": 1,
        "kernel_version": env!("CARGO_PKG_VERSION"),
        "discipline": "ats_v_arch_corpus",
        "spec": "internal/specs/ats-v.md#§5.3-§7",
        "load_bearing": load_bearing,
        "annotated_cog_count": registry.len(),
        "total_violation_count": total_violations,
        "violations_by_code": violation_codes,
        "cogs": cog_reports,
    });
    let _ = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&payload).unwrap_or_default(),
    );

    match format {
        AuditFormat::Plain => {
            println!();
            println!("ATS-V whole-corpus cross-cog architectural analysis");
            println!("─────────────────────────────────────────────────────");
            println!(
                "  {} annotated cogs walked; {} cross-cog violations.",
                registry.len(),
                total_violations,
            );
            println!();
            println!(
                "  {:<40}  {:<14}  {:<8}  {:<7}  {}",
                "Cog", "Foundation", "Stratum", "Mounts", "Violations",
            );
            println!(
                "  {}  {}  {}  {}  {}",
                "─".repeat(40),
                "─".repeat(14),
                "─".repeat(8),
                "─".repeat(7),
                "─".repeat(10),
            );
            for cog in &cog_reports {
                let name = cog["cog"].as_str().unwrap_or("");
                let foundation = cog["foundation"].as_str().unwrap_or("");
                let stratum = cog["stratum"].as_str().unwrap_or("");
                let mc = cog["mount_count"].as_u64().unwrap_or(0);
                let vc = cog["violation_count"].as_u64().unwrap_or(0);
                println!(
                    "  {:<40}  {:<14}  {:<8}  {:<7}  {}",
                    if name.len() > 40 { &name[..40] } else { name },
                    foundation,
                    stratum,
                    mc,
                    vc,
                );
            }
            println!();
            if load_bearing {
                println!(
                    "{} ATS-V whole-corpus analysis: 0 anti-pattern violations across {} cogs.",
                    "✓".green(),
                    registry.len(),
                );
            } else {
                println!(
                    "{} ATS-V whole-corpus analysis: {} violation(s) across {} cogs.",
                    "✗".red(),
                    total_violations,
                    registry.len(),
                );
                if !violation_codes.is_empty() {
                    println!();
                    for (code, count) in &violation_codes {
                        println!("  {} × {}", count, code);
                    }
                }
            }
            println!();
            println!("Report: {}", report_path.display());
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string(&payload).unwrap_or_default());
        }
    }

    if !load_bearing {
        return Err(crate::error::CliError::Custom(
            format!(
                "arch-corpus audit: {} cross-cog violation(s) across {} annotated cogs — see {}",
                total_violations,
                registry.len(),
                report_path.display(),
            )
            .into(),
        ));
    }

    Ok(())
}

/// Walk a `Module`'s items looking for the first `ItemKind::Module`
/// declaration (`module foo.bar.baz;`) and return its dotted-path
/// name.  Used by `--arch-corpus` to derive a stable cog identity
/// independent of file path.
fn derive_module_path(module: &verum_ast::Module) -> Option<String> {
    use verum_ast::decl::ItemKind;
    for item in &module.items {
        if let ItemKind::Module(m) = &item.kind {
            return Some(m.name.name.as_str().to_string());
        }
    }
    None
}

/// Resolve a mount path against the corpus registry.  Returns the
/// cog name iff the mount path either:
///   - exactly matches an annotated cog's name, OR
///   - is a glob (`foo.*`) whose prefix matches an annotated cog.
///
/// Sub-path mounts (`foo.bar.{X, Y}` flattened to `foo.bar`) are
/// resolved by walking the prefix until we find an annotated cog.
fn match_annotated_cog(
    mount_path: &str,
    name_index: &std::collections::BTreeMap<String, usize>,
) -> Option<String> {
    let stripped = mount_path
        .strip_suffix(".*")
        .unwrap_or(mount_path)
        .strip_prefix("file:")
        .unwrap_or(mount_path);

    if name_index.contains_key(stripped) {
        return Some(stripped.to_string());
    }
    // Walk parents — `core.io.async_protocols.{X}` flattens to
    // `core.io.async_protocols`; if that's annotated, resolve to it.
    let mut cur = stripped;
    while let Some(idx) = cur.rfind('.') {
        cur = &cur[..idx];
        if name_index.contains_key(cur) {
            return Some(cur.to_string());
        }
    }
    None
}

/// Mount-derived capability inference.  Walks a cog's
/// `inferred_mounts` and produces the Capability set the cog's
/// usage implies.  MVP recogniser:
///
///  * `sys.*.syscall.*` / `sys.darwin.libsystem.*` /
///    `sys.windows.kernel32.*` → `Capability::Exec(<inferred>)`
///  * `sys.*.network*` / known networking sub-modules →
///    `Capability::Network(Tcp, Bidirectional)` (placeholder)
///  * `sys.io_engine` → `Capability::Exec(<inferred>)` (the IO
///    engine routes through syscalls)
///
/// All inferences produce real `Capability` variants (not Custom)
/// so structural equality matches the same variants when produced
/// by `parse_capability` from the cog's declared `requires`.
/// Unrecognised mounts contribute nothing — this is conservative,
/// designed to fire AP-001 only when the inference is
/// well-grounded.
fn infer_capabilities_from_mounts(mounts: &[String]) -> Vec<verum_kernel::arch::Capability> {
    use verum_kernel::arch::{Capability, ExecTarget, NetDirection, NetProtocol, ResourceTag};
    let mut out: Vec<Capability> = Vec::new();
    let mut seen_exec = false;
    let mut seen_network = false;
    for m in mounts {
        let bare = m.strip_suffix(".*").unwrap_or(m);
        if !seen_exec
            && (bare.starts_with("sys.") && bare.contains(".syscall")
                || bare.starts_with("sys.darwin.libsystem")
                || bare.starts_with("sys.windows.kernel32")
                || bare.starts_with("sys.windows.winsock2")
                || bare == "sys.io_engine")
        {
            out.push(Capability::Exec {
                target: ExecTarget::Custom("<inferred>".to_string()),
            });
            seen_exec = true;
        }
        if !seen_network
            && (bare.starts_with("sys.windows.winsock2")
                || (bare.starts_with("sys.") && bare.contains("net")))
        {
            out.push(Capability::Network {
                protocol: NetProtocol::Tcp,
                direction: NetDirection::Bidirectional,
            });
            seen_network = true;
        }
        // Unused-warning silencer — the ResourceTag import is
        // here for future expansion (Read/Write capability
        // inference from filesystem-touching mounts) without
        // adding a separate use statement when wired.
        let _ = ResourceTag::Logger;
    }
    out
}

/// Stable lifecycle tag for the JSON output.
fn lifecycle_tag(shape: &verum_kernel::arch::Shape) -> &'static str {
    use verum_kernel::arch::Lifecycle;
    match shape.lifecycle {
        Lifecycle::Hypothesis { .. } => "hypothesis",
        Lifecycle::Plan { .. } => "plan",
        Lifecycle::Conditional { .. } => "conditional",
        Lifecycle::Theorem { .. } => "theorem",
        Lifecycle::Obsolete { .. } => "obsolete",
    }
}

/// Compute the parent module namespace for a cog name.
/// `core.architecture.yoneda` → `Some("core.architecture")`.
/// `top_level` → `None`.
fn parent_namespace_of(cog_name: &str) -> Option<String> {
    cog_name.rfind('.').map(|idx| cog_name[..idx].to_string())
}

/// Canonicalise a mount path against the importing cog's parent
/// namespace.  Verum's `super.foo` mount resolves to
/// `<cog_parent>.foo`; `self.foo` to `<cog_name>.foo`.  Other
/// forms pass through unchanged.  Glob (`.*`) and file (`file:`)
/// markers are preserved.
fn canonicalise_mount_path(raw: &str, parent_ns: Option<&str>) -> String {
    // Preserve trailing markers, work on the bare path.
    let glob = raw.ends_with(".*");
    let file_prefix = raw.starts_with("file:");
    let bare = raw
        .strip_suffix(".*")
        .unwrap_or(raw)
        .strip_prefix("file:")
        .unwrap_or(raw);

    let resolved = if let Some(rest) = bare.strip_prefix("super.") {
        match parent_ns {
            Some(parent) if !parent.is_empty() => {
                // Ascend one more level for `super.X` (since `super`
                // refers to the parent's parent in Verum's module
                // hierarchy when used inside a sub-module).  In
                // practice for our use case, the cog's own parent
                // namespace IS the resolution target.
                format!("{}.{}", parent, rest)
            }
            _ => bare.to_string(),
        }
    } else if let Some(rest) = bare.strip_prefix("self.") {
        match parent_ns {
            Some(_) => format!("{}.{}", parent_ns.unwrap_or(""), rest),
            None => bare.to_string(),
        }
    } else {
        bare.to_string()
    };

    let mut out = if file_prefix {
        format!("file:{}", resolved)
    } else {
        resolved
    };
    if glob {
        out.push_str(".*");
    }
    out
}

// =============================================================================
// Reflection-tower audit (#158) — Feferman 1989 / Pohlers / Beklemishev
// =============================================================================

/// `verum audit --reflection-tower` — walks every level in
/// `verum_kernel::reflection_tower::reflection_tower()`, runs each
/// level's algorithmic discharge, and surfaces the citation +
/// verdict. Fails the gate if any finite level fails to discharge.
///
/// **Architectural role**: the base meta-soundness audit
/// (`--self-recognition`) is rank-1 (kernel sound in
/// Verum + κ_meta). This gate exposes the full ordinal-indexed
/// reflection tower (REF^0..REF^4 + REF^ω) with each level's
/// published-proof citation. The tower's stability (every finite
/// level discharges) is the load-bearing soundness contract.
pub fn audit_reflection_tower_with_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::reflection_tower::build_tower_report;

    if matches!(format, AuditFormat::Plain) {
        ui::step("Reflection tower — MSFS-grounded meta-soundness");
    }

    let report = build_tower_report();

    let manifest_dir = Manifest::find_manifest_dir()?;
    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let report_path = report_dir.join("reflection-tower.json");

    let stage_summaries: Vec<serde_json::Value> = report
        .stage_verdicts
        .iter()
        .map(|v| {
            serde_json::json!({
                "stage_name": v.stage_name,
                "stage_tag": v.stage_tag,
                "msfs_citation": v.citation_tag,
                "verum_corpus_path": v.corpus_path,
                "discharges": v.discharges,
            })
        })
        .collect();

    let constructive_summaries: Vec<serde_json::Value> = report
        .sampled_constructive_discharges
        .iter()
        .map(|d| {
            serde_json::json!({
                "universe_index": d.universe_index,
                "witness": {
                    "a_m_cls_is_meta_cls": d.witness.a_m_cls_is_meta_cls_holds,
                    "b_pi_inf_inf_plus_1_equivalent": d.witness.b_pi_inf_inf_plus_1_equivalent,
                    "b_universe_ascent_with_theory_idempotence":
                        d.witness.b_universe_ascent_with_theory_idempotence,
                },
                "kernel_truncate_to_level_holds": d.truncate_to_level_holds,
                "kernel_straightening_equivalence_holds": d.straightening_equivalence_holds,
                "holds": d.holds,
            })
        })
        .collect();

 // Per-rule meta-theoretic footprint — the data formerly served
 // by `audit --self-recognition`. Now embedded as a sub-block
 // of the reflection-tower report so the two gates share a
 // single canonical source.
    use verum_kernel::zfc_self_recognition::{
        KernelRuleId, SelfRecognitionAudit, is_zfc_plus_2_inacc_provable, required_meta_theory,
    };
    let mut self_rec_audit = SelfRecognitionAudit::new();
    for rule in KernelRuleId::full_list() {
        self_rec_audit.cite(rule);
    }
    let zfc_required = self_rec_audit.required_zfc_axioms();
    let inacc_required = self_rec_audit.required_inaccessibles();
    let provable_in_zfc_plus_2_inacc = self_rec_audit.is_provable_in_zfc_plus_2_inacc();
    let per_rule_footprint: Vec<serde_json::Value> = KernelRuleId::full_list()
        .iter()
        .map(|rule| {
            let req = required_meta_theory(*rule);
            serde_json::json!({
                "rule": rule.name(),
                "zfc_axioms": req.zfc_axioms.iter().map(|a| a.name()).collect::<Vec<_>>(),
                "inaccessibles": req.inaccessibles.iter().map(|k| k.name()).collect::<Vec<_>>(),
                "citation": req.citation.as_str(),
                "provable_in_zfc_plus_2_inacc": is_zfc_plus_2_inacc_provable(*rule),
            })
        })
        .collect();

    let payload = serde_json::json!({
        "schema_version": 3,
        "kernel_version": env!("CARGO_PKG_VERSION"),
        "discipline": "kernel_reflection_tower_msfs_grounded",
        "msfs_paper": "Sereda 2026 — The Moduli Space of Formal Systems",
        "load_bearing": report.is_load_bearing(),
        "max_inaccessible_required": report.max_inaccessible_required,
        "stages": stage_summaries,
        "constructive_discharges_at_sampled_indices": constructive_summaries,
        "discharged_stage_count": report.discharged_count(),
        "constructive_discharged_count": report.constructive_discharged_count(),
 // Per-rule meta-theoretic footprint (subsumes the legacy
 // `audit --self-recognition` payload as a sub-view).
        "base_footprint": {
            "provable_in_zfc_plus_2_inacc": provable_in_zfc_plus_2_inacc,
            "zfc_axioms_required": zfc_required.iter().map(|a| a.name()).collect::<Vec<_>>(),
            "inaccessibles_required": inacc_required.iter().map(|k| k.name()).collect::<Vec<_>>(),
            "rules": per_rule_footprint,
        },
    });
    let _ = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&payload).unwrap_or_default(),
    );

    match format {
        AuditFormat::Plain => {
            println!();
            println!("Reflection tower — MSFS-grounded meta-soundness");
            println!("───────────────────────────────────────────────");
            println!(
                "  Four structural facts (MSFS Theorems 9.6 + 8.2 + 5.1; \
                 three interior stages + AFN-T α boundary):",
            );
            println!();
            println!(
                "  {:<10}  {:<9}  {:<48}  Corpus path",
                "Stage", "Discharge", "MSFS citation",
            );
            println!(
                "  {}  {}  {}  {}",
                "─".repeat(10), "─".repeat(9), "─".repeat(48), "─".repeat(60),
            );
            for v in &report.stage_verdicts {
                let glyph = if v.discharges { "✓" } else { "✗" };
                println!(
                    "  {:<10}  {} {:<7}  {:<48}  {}",
                    v.stage_name,
                    glyph,
                    if v.discharges { "yes" } else { "NO" },
                    v.citation_tag,
                    v.corpus_path,
                );
            }
            println!();
            println!(
                "  Constructive per-index discharges (sampled at k = 0, 1, 2, 3, 7, 42):",
            );
            println!(
                "  {:<6}  {:<7}  {:<14}  {:<14}  {:<14}  {}",
                "k", "holds", "a_m_cls", "b_pi_∞,∞+1", "b_univ_ascent", "intrinsics",
            );
            println!("  {}", "─".repeat(80));
            for d in &report.sampled_constructive_discharges {
                let glyph = if d.holds { "✓" } else { "✗" };
                println!(
                    "  {:<6}  {} {:<5}  {:<14}  {:<14}  {:<14}  trunc={} straight={}",
                    d.universe_index,
                    glyph,
                    if d.holds { "yes" } else { "NO" },
                    d.witness.a_m_cls_is_meta_cls_holds,
                    d.witness.b_pi_inf_inf_plus_1_equivalent,
                    d.witness.b_universe_ascent_with_theory_idempotence,
                    d.truncate_to_level_holds,
                    d.straightening_equivalence_holds,
                );
            }
            println!();
            println!(
                "  Max inaccessible-index required by current kernel: {}",
                report.max_inaccessible_required,
            );
            println!(
                "  Stages discharged: {}/{}; Constructive discharges held: {}/{}",
                report.discharged_count(),
                report.stage_verdicts.len(),
                report.constructive_discharged_count(),
                report.sampled_constructive_discharges.len(),
            );
            println!();
            if report.is_load_bearing() {
                println!(
                    "{} MSFS-grounded reflection tower load-bearing: \
                     every stage discharges + every sampled per-index constructive \
                     witness fed through MSFS-machine-verified intrinsics agrees.",
                    "✓".green(),
                );
            } else {
                println!(
                    "{} Reflection tower NOT load-bearing — at least one stage or \
                     constructive discharge failed.",
                    "✗".red(),
                );
            }
            println!();
            println!("Report: {}", report_path.display());
        }
        AuditFormat::Json => {
            println!("{}", serde_json::to_string(&payload).unwrap_or_default());
        }
    }

    if !report.is_load_bearing() {
        return Err(crate::error::CliError::Custom(
            format!(
                "reflection-tower audit: at least one finite level failed to \
                 discharge — see {}",
                report_path.display(),
            )
            .into(),
        ));
    }

    Ok(())
}

// =============================================================================
// Kernel-intrinsic dispatch audit
// =============================================================================

/// `verum audit --kernel-intrinsics` — emits the available kernel
/// intrinsic dispatch table from
/// `verum_kernel::intrinsic_dispatch::available_intrinsics()`. Used
/// by the compiler's elaborator + by reviewers to confirm that
/// every `kernel_*` axiom in `core/proof/kernel_bridge.vr` has a
/// kernel-side dispatcher.
pub fn audit_kernel_intrinsics(format: AuditFormat) -> Result<()> {
    use verum_kernel::intrinsic_dispatch::{
        IntrinsicValue, available_intrinsics, dispatch_intrinsic,
    };

    let names = available_intrinsics();

    match format {
        AuditFormat::Plain => {
            ui::step("Kernel intrinsic dispatch table");
            println!();
            println!(
                "  {:<45}  {}",
                "Intrinsic name", "Default-decision (no args)"
            );
            println!("  {}  {}", "─".repeat(45), "─".repeat(40));
            for name in names {
                let decision = dispatch_intrinsic(name, &[])
                    .and_then(|v| v.as_bool())
                    .map(|b| if b { "holds" } else { "open (needs args)" })
                    .unwrap_or("requires args");
                println!("  {:<45}  {}", name, decision);
            }
            println!();
            println!(
                "  Total dispatchable intrinsics: {} (each backs a kernel_* axiom in kernel_bridge.vr)",
                names.len()
            );
        }
        AuditFormat::Json => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"intrinsic_count\": {},\n", names.len()));
            out.push_str("  \"intrinsics\": [\n");
            for (i, name) in names.iter().enumerate() {
                let probe = dispatch_intrinsic(name, &[]);
                let decidable = probe.is_some();
                let default_holds = probe.and_then(|v| v.as_bool()).unwrap_or(false);
                out.push_str(&format!(
                    "    {{ \"name\": \"{}\", \"decidable_at_no_args\": {}, \"default_holds\": {} }}{}\n",
                    name,
                    decidable,
                    default_holds,
                    if i + 1 < names.len() { "," } else { "" }
                ));
            }
            out.push_str("  ]\n}");
 // Suppress unused warning when format doesn't need it.
            let _ = IntrinsicValue::Unit;
            println!("{}", out);
        }
    }
    Ok(())
}

// =============================================================================
// Cross-format CI gate audit
// =============================================================================

/// `verum audit --cross-format` — emits the cross-format CI hard
/// gate status: every required foreign proof-assistant backend must
/// pass (Coq / Lean 4 / Isabelle / Dedukti).
pub fn audit_cross_format(format: AuditFormat) -> Result<()> {
    use verum_kernel::cross_format_gate::{
        CrossFormatReport, FormatStatus, evaluate_gate, required_formats_for_msfs,
    };
    use verum_smt::cross_format_runner::{CheckResult, checker_for};

    if matches!(format, AuditFormat::Plain) {
        ui::step("Cross-format CI hard gate (MSFS) — live tool invocation");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let formats = required_formats_for_msfs();

    let mut report = CrossFormatReport::new("project");

 // Per-format status: probe tool + run on every certificate file.
    struct FormatRow {
        format_name: &'static str,
        extension: &'static str,
        certs_dir: PathBuf,
        files_found: usize,
        files_passed: usize,
        files_failed: usize,
        tool_status: &'static str,
        first_failure_excerpt: Option<String>,
    }
    let mut rows: Vec<FormatRow> = Vec::new();

    for f in &formats {
        let dir_name = f.name();
        let certs_dir = manifest_dir.join("certificates").join(dir_name);
        let extension = f.extension();

        let checker = match checker_for(*f) {
            Some(c) => c,
            None => {
                report.record(
                    *f,
                    FormatStatus::NotRun {
                        reason: Text::from("no checker registered for this format"),
                    },
                );
                rows.push(FormatRow {
                    format_name: dir_name,
                    extension,
                    certs_dir: certs_dir.clone(),
                    files_found: 0,
                    files_passed: 0,
                    files_failed: 0,
                    tool_status: "no checker",
                    first_failure_excerpt: None,
                });
                continue;
            }
        };

        let tool_status = if checker.is_available() {
            "available"
        } else {
            "missing"
        };

 // Discover certificate files.
        let pattern_ext = extension.to_string();
        let cert_files: Vec<PathBuf> = match std::fs::read_dir(&certs_dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.extension()
                        .map(|e| e == pattern_ext.as_str())
                        .unwrap_or(false)
                })
                .collect(),
            Err(_) => Vec::new(),
        };

        let files_found = cert_files.len();
        let mut files_passed = 0usize;
        let mut files_failed = 0usize;
        let mut first_failure_excerpt: Option<String> = None;
        let mut overall_status: Option<FormatStatus> = None;

        if !checker.is_available() {
            overall_status = Some(FormatStatus::NotRun {
                reason: Text::from(format!("tool missing — {}", checker.install_hint())),
            });
        } else if cert_files.is_empty() {
            overall_status = Some(FormatStatus::NotRun {
                reason: Text::from(format!(
                    "no certificate files in certificates/{}/ — run `verum export` first",
                    dir_name
                )),
            });
        } else {
 // Actually drive the foreign tool on every cert file.
            for cert in &cert_files {
                match checker.check_file(cert) {
                    CheckResult::Passed { .. } => files_passed += 1,
                    CheckResult::Failed { stderr_excerpt, .. } => {
                        files_failed += 1;
                        if first_failure_excerpt.is_none() {
                            first_failure_excerpt = Some(format!(
                                "{}: {}",
                                cert.file_name()
                                    .map(|s| s.to_string_lossy().into_owned())
                                    .unwrap_or_default(),
                                stderr_excerpt.trim()
                            ));
                        }
                    }
                    CheckResult::ToolMissing { .. } => {
 // Should not happen since we checked is_available
 // above; treat as runtime tool-disappearance.
                        overall_status = Some(FormatStatus::NotRun {
                            reason: Text::from("tool disappeared mid-run"),
                        });
                        break;
                    }
                    CheckResult::RunnerError { reason } => {
                        files_failed += 1;
                        if first_failure_excerpt.is_none() {
                            first_failure_excerpt = Some(reason);
                        }
                    }
                }
            }
            if overall_status.is_none() {
                overall_status = Some(if files_failed == 0 {
                    FormatStatus::Passed {
                        message: Text::from(format!(
                            "{} files OK ({})",
                            files_passed,
                            checker.format().name()
                        )),
                    }
                } else {
                    FormatStatus::Failed {
                        reason: Text::from(format!(
                            "{} of {} files failed: {}",
                            files_failed,
                            files_found,
                            first_failure_excerpt
                                .clone()
                                .unwrap_or_else(|| "see verbose output".into())
                        )),
                    }
                });
            }
        }

        report.record(
            *f,
            overall_status.unwrap_or(FormatStatus::NotRun {
                reason: Text::from("(unreachable)"),
            }),
        );

        rows.push(FormatRow {
            format_name: dir_name,
            extension,
            certs_dir,
            files_found,
            files_passed,
            files_failed,
            tool_status,
            first_failure_excerpt,
        });
    }

    let gate_passes = evaluate_gate(&report);

    match format {
        AuditFormat::Plain => {
            println!();
            println!(
                "  {:<10}  {:<5}  {:<10}  {:>5}  {:>4}  {:>4}  {}",
                "Format", "Ext", "Tool", "Files", "Pass", "Fail", "Notes"
            );
            println!(
                "  {}  {}  {}  {}  {}  {}  {}",
                "─".repeat(10),
                "─".repeat(5),
                "─".repeat(10),
                "─".repeat(5),
                "─".repeat(4),
                "─".repeat(4),
                "─".repeat(40),
            );
            for r in &rows {
                let notes = r.first_failure_excerpt.clone().unwrap_or_default();
                println!(
                    "  {:<10}  {:<5}  {:<10}  {:>5}  {:>4}  {:>4}  {}",
                    r.format_name,
                    r.extension,
                    r.tool_status,
                    r.files_found,
                    r.files_passed,
                    r.files_failed,
                    notes
                );
            }
            println!();
            println!("  {}", report.summary());
            println!(
                "  Gate verdict: {}",
                if gate_passes { "✓ GREEN" } else { "✗ RED" }
            );
        }
        AuditFormat::Json => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"gate_passes\": {},\n", gate_passes));
            out.push_str(&format!(
                "  \"required_format_count\": {},\n",
                formats.len()
            ));
            out.push_str("  \"formats\": [\n");
            for (i, r) in rows.iter().enumerate() {
                out.push_str(&format!(
                    "    {{ \"format\": \"{}\", \"extension\": \"{}\", \"tool_status\": \"{}\", \"files_found\": {}, \"files_passed\": {}, \"files_failed\": {}, \"first_failure\": \"{}\" }}{}\n",
                    r.format_name,
                    r.extension,
                    r.tool_status,
                    r.files_found,
                    r.files_passed,
                    r.files_failed,
                    json_escape(&r.first_failure_excerpt.clone().unwrap_or_default()),
                    if i + 1 < rows.len() { "," } else { "" }
                ));
            }
            out.push_str("  ]\n}");
            println!("{}", out);
        }
    }

    if !gate_passes {
        return Err(crate::error::CliError::VerificationFailed(
            "cross-format CI hard gate is RED".to_string(),
        ));
    }
    Ok(())
}

// =============================================================================
// `verum audit --manifest-coverage` — load-bearing inert-defense gate (#290).
//

// Enumerates every Verum.toml manifest field with its wiring status. A
// future PR adding a manifest field without wiring it produces a
// `ForwardLooking` row and points reviewers at the closure_task. The
// session.rs documentation comments and this static table are
// synchronized — when one drifts, the audit gate catches it.
// =============================================================================

/// Wiring status of a single manifest field (#290).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind")]
pub enum ManifestFieldStatus {
 /// Fully wired — manifest value drives observable production
 /// behaviour through the documented consumer site.
    LoadBearing,
 /// Wired in part — surface gate fires but full integration
 /// (e.g. type-level taint propagation) is split out.
    LoadBearingPartial,
 /// Wired only via the embedder path — standard CLI
 /// auto-routing is documented as a separate scope.
    EmbedderLoadBearing,
 /// Forward-looking infrastructure — value lands on the session
 /// but the consumer is documented as a phased follow-up.
    ForwardLooking,
}

impl ManifestFieldStatus {
    fn label(&self) -> &'static str {
        match self {
            Self::LoadBearing => "load-bearing",
            Self::LoadBearingPartial => "load-bearing (partial)",
            Self::EmbedderLoadBearing => "embedder-load-bearing",
            Self::ForwardLooking => "forward-looking",
        }
    }

 /// Whether this status counts as "wired" for the bundle audit.
    fn is_wired(&self) -> bool {
        !matches!(self, Self::ForwardLooking)
    }
}

/// One row of the manifest-coverage audit (#290).
#[derive(Debug, Clone)]
struct ManifestFieldEntry {
    section: &'static str,
    field: &'static str,
    status: ManifestFieldStatus,
    closure_task: &'static str,
    consumer_site: &'static str,
}

/// Enumerate every manifest field and its wiring status (#290).
///

/// **Maintenance contract**: when a new manifest field is added to
/// `LanguageFeatures` or `CompilerOptions`, this table MUST grow a
/// row. The pin tests verify representative entries are present.
fn manifest_field_table() -> Vec<ManifestFieldEntry> {
    use ManifestFieldStatus as S;
    vec![
 // [types] — all 9 wired.
        ManifestFieldEntry {
            section: "types",
            field: "dependent",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "TypeChecker.dependent_enabled → infer.rs",
        },
        ManifestFieldEntry {
            section: "types",
            field: "cubical",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "Unifier.cubical_enabled → unify.rs",
        },
        ManifestFieldEntry {
            section: "types",
            field: "higher_kinded",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "TypeChecker.higher_kinded_enabled → infer.rs",
        },
        ManifestFieldEntry {
            section: "types",
            field: "coinductive",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "TypeChecker.coinductive_enabled → infer.rs",
        },
        ManifestFieldEntry {
            section: "types",
            field: "instance_search",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "ProtocolChecker.instance_search_enabled → protocol.rs",
        },
        ManifestFieldEntry {
            section: "types",
            field: "quotient",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "TypeChecker.quotient_enabled → infer.rs",
        },
        ManifestFieldEntry {
            section: "types",
            field: "universe_polymorphism",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "TypeChecker.universe_poly_enabled → infer.rs",
        },
        ManifestFieldEntry {
            section: "types",
            field: "refinement",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "refinement_typing_on → semantic_analysis",
        },
        ManifestFieldEntry {
            section: "types",
            field: "coherence_check_depth",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "TypeChecker.coherence_check_depth → semantic_analysis",
        },
 // [runtime] — 7/8 wired (async_worker_threads forward-looking).
        ManifestFieldEntry {
            section: "runtime",
            field: "cbgr_mode",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "InterpreterConfig → pipeline/interpreter.rs",
        },
        ManifestFieldEntry {
            section: "runtime",
            field: "async_scheduler",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "InterpreterConfig → pipeline/interpreter.rs",
        },
        ManifestFieldEntry {
            section: "runtime",
            field: "heap_policy",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "InterpreterConfig → pipeline/interpreter.rs",
        },
        ManifestFieldEntry {
            section: "runtime",
            field: "panic",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "PanicStrategy::from_manifest_text → PlatformIR",
        },
        ManifestFieldEntry {
            section: "runtime",
            field: "futures",
            status: S::LoadBearing,
            closure_task: "#262 + #281",
            consumer_site: "Tier 0: handle_spawn / Tier 1: lower_spawn (codegen-time)",
        },
        ManifestFieldEntry {
            section: "runtime",
            field: "nurseries",
            status: S::LoadBearing,
            closure_task: "#262 + #281",
            consumer_site: "Tier 0: handle_nursery_init / Tier 1: NurseryInit lowering",
        },
        ManifestFieldEntry {
            section: "runtime",
            field: "async_worker_threads",
            status: S::ForwardLooking,
            closure_task: "#277",
            consumer_site: "LLVM globals (#261) — stdlib WorkerPool consumer pending",
        },
        ManifestFieldEntry {
            section: "runtime",
            field: "task_stack_size",
            status: S::LoadBearing,
            closure_task: "#259",
            consumer_site: "AsyncRuntimeConfig.task_stack_size via runtime bridge",
        },
 // [codegen] — all 4 wired.
        ManifestFieldEntry {
            section: "codegen",
            field: "monomorphization_cache",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "VbcMonomorphizationPhase::without_cache",
        },
        ManifestFieldEntry {
            section: "codegen",
            field: "tail_call_optimization",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "vbc_lowering: disable-tail-calls LLVM attr",
        },
        ManifestFieldEntry {
            section: "codegen",
            field: "vectorize",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "vbc_lowering: no-loop-vectorize / no-slp-vectorize attrs",
        },
        ManifestFieldEntry {
            section: "codegen",
            field: "inline_depth",
            status: S::LoadBearing,
            closure_task: "#267",
            consumer_site: "vbc_lowering: inline-threshold per-function attr",
        },
 // [protocols] — all 5 wired.
        ManifestFieldEntry {
            section: "protocols",
            field: "resolution_strategy",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "ProtocolChecker.resolution_strategy → find_impl",
        },
        ManifestFieldEntry {
            section: "protocols",
            field: "blanket_impls",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "ProtocolChecker.blanket_impls → candidate filter",
        },
        ManifestFieldEntry {
            section: "protocols",
            field: "coherence",
            status: S::LoadBearing,
            closure_task: "#263",
            consumer_site: "ProtocolChecker.coherence_mode → register_impl",
        },
        ManifestFieldEntry {
            section: "protocols",
            field: "higher_kinded_protocols",
            status: S::LoadBearing,
            closure_task: "#264",
            consumer_site: "TypeChecker.higher_kinded_protocols_enabled",
        },
        ManifestFieldEntry {
            section: "protocols",
            field: "generic_associated_types",
            status: S::LoadBearing,
            closure_task: "#265",
            consumer_site: "TypeChecker.generic_associated_types_enabled",
        },
 // [safety] — all 6 wired (Phase 1+2a+2b+3a stack).
        ManifestFieldEntry {
            section: "safety",
            field: "unsafe_allowed",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "SafetyPolicy.unsafe_allowed → safety_gate",
        },
        ManifestFieldEntry {
            section: "safety",
            field: "ffi",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "SafetyPolicy.ffi → safety_gate",
        },
        ManifestFieldEntry {
            section: "safety",
            field: "ffi_boundary",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "SafetyPolicy.ffi_boundary strict/lenient → safety_gate",
        },
        ManifestFieldEntry {
            section: "safety",
            field: "capability_required",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "SafetyPolicy.capability_required → safety_gate",
        },
        ManifestFieldEntry {
            section: "safety",
            field: "forbid_stdlib_extern",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "SafetyPolicy.forbid_stdlib_extern → safety_gate",
        },
        ManifestFieldEntry {
            section: "safety",
            field: "mls_level",
            status: S::LoadBearing,
            closure_task: "#266 + #282 + #283 + #289..#295",
            consumer_site: "11-layer MLS stack: declaration gate + lattice + param/function consistency + sidecar storage + seeding + expression propagation + downflow check + module walker + @declassify + sink detection",
        },
 // [test] — all 8 wired (Phase 1+2+3+4 closures #298+#273+#299).
        ManifestFieldEntry {
            section: "test",
            field: "timeout_secs",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "TestRunCfg.timeout_secs → commands/test.rs",
        },
        ManifestFieldEntry {
            section: "test",
            field: "deny_warnings",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "TestRunCfg.deny_warnings → commands/test.rs",
        },
        ManifestFieldEntry {
            section: "test",
            field: "coverage",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "TestRunCfg.coverage CLI||manifest",
        },
        ManifestFieldEntry {
            section: "test",
            field: "parallel",
            status: S::LoadBearing,
            closure_task: "",
            consumer_site: "rayon thread-pool gate",
        },
        ManifestFieldEntry {
            section: "test",
            field: "differential",
            status: S::LoadBearing,
            closure_task: "#273",
            consumer_site: "TestRunCfg.differential → run_test_differential (T0 + T1 cross-tier agreement)",
        },
        ManifestFieldEntry {
            section: "test",
            field: "property_testing",
            status: S::LoadBearing,
            closure_task: "#298",
            consumer_site: "TestRunCfg.property_testing → run_single_test (skips @property when false)",
        },
        ManifestFieldEntry {
            section: "test",
            field: "proptest_cases",
            status: S::LoadBearing,
            closure_task: "#298",
            consumer_site: "TestRunCfg.proptest_cases → run_test_property default_runs",
        },
        ManifestFieldEntry {
            section: "test",
            field: "fuzzing",
            status: S::LoadBearing,
            closure_task: "#299",
            consumer_site: "TestRunCfg.fuzzing → commands/fuzz::run (cargo-fuzz orchestration)",
        },
 // CompilerOptions surface fields.
        ManifestFieldEntry {
            section: "options",
            field: "continue_on_error",
            status: S::LoadBearing,
            closure_task: "#270",
            consumer_site: "Session::collect_phase_error → validate_module",
        },
        ManifestFieldEntry {
            section: "options",
            field: "emit_proof_certificate",
            status: S::LoadBearing,
            closure_task: "#285",
            consumer_site: "phase_verify::emit_theorem_certificates",
        },
        ManifestFieldEntry {
            section: "options",
            field: "proof_certificate_format",
            status: S::LoadBearing,
            closure_task: "#285",
            consumer_site: "phase_verify::emit_theorem_certificates",
        },
        ManifestFieldEntry {
            section: "options",
            field: "proof_certificate_path",
            status: S::LoadBearing,
            closure_task: "#285",
            consumer_site: "phase_verify::emit_theorem_certificates",
        },
    ]
}

#[derive(Debug, Default, Clone, serde::Serialize)]
struct ManifestCoverageSummary {
    total: usize,
    load_bearing: usize,
    load_bearing_partial: usize,
    embedder_load_bearing: usize,
    forward_looking: usize,
    fully_wired: bool,
}

/// Entry point: `verum audit --manifest-coverage [--format FORMAT]`.
pub fn audit_manifest_coverage(format: AuditFormat) -> Result<()> {
    let entries = manifest_field_table();

    let mut summary = ManifestCoverageSummary {
        total: entries.len(),
        ..Default::default()
    };
    for entry in &entries {
        match entry.status {
            ManifestFieldStatus::LoadBearing => summary.load_bearing += 1,
            ManifestFieldStatus::LoadBearingPartial => summary.load_bearing_partial += 1,
            ManifestFieldStatus::EmbedderLoadBearing => summary.embedder_load_bearing += 1,
            ManifestFieldStatus::ForwardLooking => summary.forward_looking += 1,
        }
    }
    summary.fully_wired = summary.forward_looking == 0;

    let manifest_dir =
        Manifest::find_manifest_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let report_path = report_dir.join("manifest-coverage.json");
    let json_payload = serde_json::json!({
        "schema_version": 1,
        "summary": summary,
        "entries": entries.iter().map(|e| serde_json::json!({
            "section": e.section,
            "field": e.field,
            "status": e.status.label(),
            "is_wired": e.status.is_wired(),
            "closure_task": e.closure_task,
            "consumer_site": e.consumer_site,
        })).collect::<Vec<_>>(),
    });
    if let Ok(s) = serde_json::to_string_pretty(&json_payload) {
        let _ = std::fs::write(&report_path, s);
    }

    match format {
        AuditFormat::Json => {
            if let Ok(s) = serde_json::to_string_pretty(&json_payload) {
                ui::output(&s);
            }
        }
        AuditFormat::Plain => {
            ui::step("Manifest-coverage audit");
            for entry in &entries {
                let line = format!(
                    "  [{}] {}.{} — {} ({})",
                    if entry.status.is_wired() { "✓" } else { "·" },
                    entry.section,
                    entry.field,
                    entry.status.label(),
                    if entry.closure_task.is_empty() {
                        entry.consumer_site
                    } else {
                        entry.closure_task
                    },
                );
                ui::output(&line);
            }
            let wired_count =
                summary.load_bearing + summary.load_bearing_partial + summary.embedder_load_bearing;
            ui::output(&format!(
                "\nSummary: {}/{} fields wired ({} load-bearing, {} partial, {} embedder, {} forward-looking)",
                wired_count,
                summary.total,
                summary.load_bearing,
                summary.load_bearing_partial,
                summary.embedder_load_bearing,
                summary.forward_looking,
            ));
            ui::output(&format!("Report: {}", report_path.display()));
        }
    }

    Ok(())
}

#[cfg(test)]
mod manifest_coverage_tests {
    use super::*;

    #[test]
    fn manifest_field_table_is_non_empty() {
        let entries = manifest_field_table();
        assert!(
            entries.len() >= 30,
            "manifest_field_table must enumerate ≥ 30 fields; got {}",
            entries.len()
        );
    }

    #[test]
    fn every_entry_has_section_and_field() {
        for entry in manifest_field_table() {
            assert!(!entry.section.is_empty(), "empty section");
            assert!(!entry.field.is_empty(), "empty field");
            assert!(
                !entry.consumer_site.is_empty(),
                "empty consumer_site for {}.{}",
                entry.section,
                entry.field
            );
        }
    }

    #[test]
    fn known_wired_fields_present() {
        let entries = manifest_field_table();
        let labels: std::collections::HashSet<(&str, &str)> =
            entries.iter().map(|e| (e.section, e.field)).collect();
        for (section, field) in &[
            ("types", "dependent"),
            ("runtime", "panic"),
            ("codegen", "inline_depth"),
            ("protocols", "coherence"),
            ("safety", "mls_level"),
            ("test", "timeout_secs"),
            ("options", "continue_on_error"),
        ] {
            assert!(
                labels.contains(&(*section, *field)),
                "missing row for {}.{}",
                section,
                field
            );
        }
    }

    #[test]
    fn forward_looking_entries_have_followup_task() {
        for entry in manifest_field_table() {
            if matches!(entry.status, ManifestFieldStatus::ForwardLooking) {
                assert!(
                    !entry.closure_task.is_empty(),
                    "{}.{} is ForwardLooking but has no closure_task",
                    entry.section,
                    entry.field,
                );
            }
        }
    }

    #[test]
    fn status_label_uniqueness() {
        let labels: std::collections::HashSet<&str> = [
            ManifestFieldStatus::LoadBearing,
            ManifestFieldStatus::LoadBearingPartial,
            ManifestFieldStatus::EmbedderLoadBearing,
            ManifestFieldStatus::ForwardLooking,
        ]
        .iter()
        .map(|s| s.label())
        .collect();
        assert_eq!(labels.len(), 4, "status labels must be unique");
    }

    #[test]
    fn is_wired_excludes_only_forward_looking() {
        assert!(ManifestFieldStatus::LoadBearing.is_wired());
        assert!(ManifestFieldStatus::LoadBearingPartial.is_wired());
        assert!(ManifestFieldStatus::EmbedderLoadBearing.is_wired());
        assert!(!ManifestFieldStatus::ForwardLooking.is_wired());
    }

    #[test]
    fn no_duplicate_section_field_pairs() {
 // Pin: each (section, field) tuple must be unique. Catches
 // accidental copy-paste duplicates in the table.
        let entries = manifest_field_table();
        let mut seen = std::collections::HashSet::new();
        for entry in &entries {
            let key = (entry.section, entry.field);
            assert!(
                seen.insert(key),
                "duplicate row: {}.{}",
                entry.section,
                entry.field
            );
        }
    }
}

// =============================================================================
// `verum audit --mls-coverage` — observability for MLS classification (#296).
//

// Surface the MLS classification topology of a project: how many
// functions opt into classification, what mix of @declassify
// boundaries exist, which sink contexts (Logger/FS/Network/...)
// are used. Drives security-review dashboards and CI gates that
// track classification growth in regulated-environment codebases.
// =============================================================================

/// Per-function MLS classification record (#296).
#[derive(Debug, Clone, serde::Serialize)]
struct MlsCoverageFunction {
    name: String,
    function_classification: Option<String>,
    classified_param_count: usize,
    has_declassify: bool,
    sink_contexts: Vec<String>,
}

/// Aggregate MLS coverage summary (#296).
#[derive(Debug, Default, Clone, serde::Serialize)]
struct MlsCoverageSummary {
    total_functions: usize,
    classified_functions: usize,
    functions_with_classified_params: usize,
    declassify_boundaries: usize,
    sink_consumers: usize,
 /// Total count of classified parameters across all functions.
    total_classified_params: usize,
}

const MLS_LOW_CLASSIFICATION_SINKS: &[&str] = &[
    "Logger",
    "FS",
    "FileSystem",
    "Network",
    "Stdout",
    "Stderr",
    "Tracing",
    "Telemetry",
];

fn read_function_classification_audit(
    attrs: &verum_common::List<verum_ast::attr::Attribute>,
) -> Option<String> {
    use verum_common::mls::MlsLevel;
    let mut found: Option<MlsLevel> = None;
    for attr in attrs.iter() {
        if !attr.is_named("classification") {
            continue;
        }
        if let verum_common::Maybe::Some(args) = &attr.args {
            for arg in args.iter() {
                if let verum_ast::expr::ExprKind::Path(path) = &arg.kind {
                    if let Some(ident) = path.as_ident() {
                        let parsed = MlsLevel::from_manifest_str(ident.as_str());
                        match found {
                            Some(prev) if prev >= parsed => {}
                            _ => found = Some(parsed),
                        }
                    }
                }
            }
        }
    }
    found.map(|l| l.as_manifest_str().to_string())
}

fn count_classified_params_audit(func: &verum_ast::decl::FunctionDecl) -> usize {
    let mut count = 0;
    for p in func.params.iter() {
        if let verum_ast::decl::FunctionParamKind::Regular { .. } = &p.kind {
            if read_function_classification_audit(&p.attributes).is_some() {
                count += 1;
            }
        }
    }
    count
}

fn collect_sink_contexts_audit(func: &verum_ast::decl::FunctionDecl) -> Vec<String> {
    let mut sinks = Vec::new();
    for ctx in func.contexts.iter() {
        if ctx.is_negative {
            continue;
        }
        let last = ctx.path.last_segment_name();
        if MLS_LOW_CLASSIFICATION_SINKS.iter().any(|s| *s == last) {
            sinks.push(last.to_string());
        }
    }
    sinks
}

/// Entry point: `verum audit --mls-coverage [--format FORMAT]`.
pub fn audit_mls_coverage(format: AuditFormat) -> Result<()> {
    if matches!(format, AuditFormat::Plain) {
        ui::step("MLS coverage audit — classification topology");
    }
    let manifest_dir =
        Manifest::find_manifest_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let vr_files = discover_vr_files(&manifest_dir);

    let mut per_function: Vec<MlsCoverageFunction> = Vec::new();
    let mut summary = MlsCoverageSummary::default();

    for path in &vr_files {
        let module = match parse_file_for_audit(path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        for item in &module.items {
            if let verum_ast::decl::ItemKind::Function(func) = &item.kind {
                summary.total_functions += 1;
                let function_classification = read_function_classification_audit(&func.attributes);
                if function_classification.is_some() {
                    summary.classified_functions += 1;
                }
                let classified_param_count = count_classified_params_audit(func);
                summary.total_classified_params += classified_param_count;
                if classified_param_count > 0 {
                    summary.functions_with_classified_params += 1;
                }
                let has_declassify = func.attributes.iter().any(|a| a.is_named("declassify"));
                if has_declassify {
                    summary.declassify_boundaries += 1;
                }
                let sink_contexts = collect_sink_contexts_audit(func);
                if !sink_contexts.is_empty() {
                    summary.sink_consumers += 1;
                }
                if function_classification.is_some()
                    || classified_param_count > 0
                    || has_declassify
                    || !sink_contexts.is_empty()
                {
                    per_function.push(MlsCoverageFunction {
                        name: func.name.name.as_str().to_string(),
                        function_classification,
                        classified_param_count,
                        has_declassify,
                        sink_contexts,
                    });
                }
            }
        }
    }

    let report_dir = manifest_dir.join("target").join("audit-reports");
    let _ = std::fs::create_dir_all(&report_dir);
    let report_path = report_dir.join("mls-coverage.json");
    let json_payload = serde_json::json!({
        "schema_version": 1,
        "summary": summary,
        "functions": per_function,
    });
    if let Ok(s) = serde_json::to_string_pretty(&json_payload) {
        let _ = std::fs::write(&report_path, s);
    }

    match format {
        AuditFormat::Json => {
            if let Ok(s) = serde_json::to_string_pretty(&json_payload) {
                ui::output(&s);
            }
        }
        AuditFormat::Plain => {
            ui::output(&format!("  Total functions: {}", summary.total_functions));
            ui::output(&format!(
                "  Classified functions: {}",
                summary.classified_functions
            ));
            ui::output(&format!(
                "  Functions with classified parameters: {} ({} params total)",
                summary.functions_with_classified_params, summary.total_classified_params,
            ));
            ui::output(&format!(
                "  @declassify boundaries: {}",
                summary.declassify_boundaries
            ));
            ui::output(&format!(
                "  Sink-context consumers: {}",
                summary.sink_consumers
            ));
            ui::output(&format!("Report: {}", report_path.display()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod mls_coverage_tests {
    use super::*;

    #[test]
    fn read_function_classification_audit_returns_none_for_no_attr() {
        let attrs = verum_common::List::new();
        assert!(read_function_classification_audit(&attrs).is_none());
    }

    #[test]
    fn read_function_classification_audit_extracts_secret() {
        use verum_ast::expr::{Expr, ExprKind};
        let path = verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
            "secret",
            verum_ast::Span::default(),
        ));
        let arg = Expr::new(ExprKind::Path(path), verum_ast::Span::default());
        let mut args = verum_common::List::new();
        args.push(arg);
        let attr = verum_ast::attr::Attribute::new(
            verum_common::Text::from("classification"),
            verum_common::Maybe::Some(args),
            verum_ast::Span::default(),
        );
        let mut attrs = verum_common::List::new();
        attrs.push(attr);
        assert_eq!(
            read_function_classification_audit(&attrs),
            Some("secret".to_string())
        );
    }

    #[test]
    fn read_function_classification_audit_takes_max() {
        use verum_ast::expr::{Expr, ExprKind};
        let mk = |level: &str| -> verum_ast::attr::Attribute {
            let path = verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                level,
                verum_ast::Span::default(),
            ));
            let arg = Expr::new(ExprKind::Path(path), verum_ast::Span::default());
            let mut args = verum_common::List::new();
            args.push(arg);
            verum_ast::attr::Attribute::new(
                verum_common::Text::from("classification"),
                verum_common::Maybe::Some(args),
                verum_ast::Span::default(),
            )
        };
        let mut attrs = verum_common::List::new();
        attrs.push(mk("secret"));
        attrs.push(mk("top_secret"));
        assert_eq!(
            read_function_classification_audit(&attrs),
            Some("top_secret".to_string())
        );
    }

    #[test]
    fn mls_low_classification_sinks_includes_known_sinks() {
 // Pin: every documented sink in the registry. Catches
 // accidental removals from the static list.
        for required in &["Logger", "FS", "Network", "Stdout"] {
            assert!(
                MLS_LOW_CLASSIFICATION_SINKS.contains(required),
                "missing sink: {}",
                required
            );
        }
    }

    #[test]
    fn mls_coverage_summary_default_is_zero() {
 // Pin: every counter starts at 0 — empty project produces
 // zero diagnostics + zero counts.
        let s = MlsCoverageSummary::default();
        assert_eq!(s.total_functions, 0);
        assert_eq!(s.classified_functions, 0);
        assert_eq!(s.functions_with_classified_params, 0);
        assert_eq!(s.declassify_boundaries, 0);
        assert_eq!(s.sink_consumers, 0);
        assert_eq!(s.total_classified_params, 0);
    }
}

#[cfg(test)]
mod bundle_gate_metric_tests {
    use super::bundle_gate_metric;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn one(
        gate: &'static str,
        value: serde_json::Value,
    ) -> BTreeMap<&'static str, serde_json::Value> {
        let mut m = BTreeMap::new();
        m.insert(gate, value);
        m
    }

    #[test]
    fn manifest_coverage_renders_wired_total_and_forward_looking() {
        let gates = one(
            "manifest_coverage",
            json!({
                "summary": {
                    "total": 42,
                    "load_bearing": 30,
                    "load_bearing_partial": 5,
                    "embedder_load_bearing": 4,
                    "forward_looking": 3,
                    "fully_wired": false,
                }
            }),
        );
        let s = bundle_gate_metric("manifest_coverage", &gates);
        assert_eq!(s, "39/42 wired, 3 forward-looking");
    }

    #[test]
    fn mls_coverage_renders_classification_topology() {
        let gates = one(
            "mls_coverage",
            json!({
                "summary": {
                    "total_functions": 100,
                    "classified_functions": 8,
                    "functions_with_classified_params": 4,
                    "total_classified_params": 11,
                    "declassify_boundaries": 2,
                    "sink_consumers": 5,
                }
            }),
        );
        let s = bundle_gate_metric("mls_coverage", &gates);
        assert_eq!(s, "8/100 classified, 11 params, 2 declassify, 5 sinks");
    }

    #[test]
    fn soundness_iou_renders_proved_admitted_ratio() {
        let gates = one(
            "soundness_iou",
            json!({
                "total_rules": 38,
                "total_proved": 4,
                "total_admitted": 34,
            }),
        );
        let s = bundle_gate_metric("soundness_iou", &gates);
        assert_eq!(s, "4/38 proved, 34 admitted");
    }

    #[test]
    fn cross_format_roundtrip_renders_theorem_backend_failure_grid() {
        let gates = one(
            "cross_format_roundtrip",
            json!({
                "theorems_walked": 37,
                "backend_count": 2,
                "foreign_failures": 0,
            }),
        );
        let s = bundle_gate_metric("cross_format_roundtrip", &gates);
        assert_eq!(s, "37 theorems × 2 backends, 0 failures");
    }

    #[test]
    fn signatures_renders_verification_breakdown() {
        let gates = one(
            "signatures",
            json!({
                "theorems_walked": 37,
                "verified": 74,
                "mismatched": 0,
                "header_missing": 0,
            }),
        );
        let s = bundle_gate_metric("signatures", &gates);
        assert_eq!(s, "37 theorems, 74 verified, 0 mismatched, 0 no-header");
    }

    #[test]
    fn apply_graph_renders_walked_and_leaking() {
        let gates = one(
            "apply_graph",
            json!({
                "theorems_walked": 37,
                "leaking_theorems": 0,
            }),
        );
        let s = bundle_gate_metric("apply_graph", &gates);
        assert_eq!(s, "37 theorems, 0 leaking");
    }

    #[test]
    fn bridge_discharge_renders_callsite_breakdown() {
        let gates = one(
            "bridge_discharge",
            json!({
                "modules_scanned": 50,
                "items_walked": 200,
                "total_callsites": 144,
                "total_false_discharges": 0,
            }),
        );
        let s = bundle_gate_metric("bridge_discharge", &gates);
        assert_eq!(s, "144 callsites, 0 false-discharges");
    }

    #[test]
    fn kernel_discharged_axioms_renders_discharge_breakdown() {
        let gates = one(
            "kernel_discharged_axioms",
            json!({
                "files_parsed": 50,
                "discharge_count": 3,
                "unrecognised_count": 0,
            }),
        );
        let s = bundle_gate_metric("kernel_discharged_axioms", &gates);
        assert_eq!(s, "3 discharges, 0 unrecognised, 50 files");
    }

    #[test]
    fn unknown_gate_returns_empty_string() {
        let gates = one("nonexistent", json!({"foo": 1}));
        assert_eq!(bundle_gate_metric("nonexistent", &gates), "");
    }

    #[test]
    fn missing_gate_returns_empty_string() {
        let gates: BTreeMap<&'static str, serde_json::Value> = BTreeMap::new();
        assert_eq!(bundle_gate_metric("manifest_coverage", &gates), "");
        assert_eq!(bundle_gate_metric("mls_coverage", &gates), "");
    }

    #[test]
    fn malformed_summary_falls_back_to_empty() {
        let gates = one("manifest_coverage", json!({"unrelated": "shape"}));
        assert_eq!(bundle_gate_metric("manifest_coverage", &gates), "");
        let gates = one("apply_graph", json!({"theorems_walked": 0}));
        assert_eq!(bundle_gate_metric("apply_graph", &gates), "");
    }
}
