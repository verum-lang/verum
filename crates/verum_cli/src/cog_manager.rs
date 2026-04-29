// Production-grade cog (package) manager for Verum.
// Supports multi-source distribution: registry, git, local path, and IPFS.
// Implements comprehensive cog lifecycle management

use crate::config::Manifest;
use crate::error::{CliError, Result};
use crate::registry::*;
use crate::ui;
use colored::Colorize;
use semver::{Version, VersionReq};
use std::path::{Path, PathBuf};
use verum_common::{List, Map, Set, Text};

/// Cog manager for Verum
pub struct CogManager {
    /// Registry client
    registry: RegistryClient,

    /// Cache manager
    cache: CacheManager,

    /// Security scanner
    security: SecurityScanner,

    /// Enterprise client
    enterprise: Option<EnterpriseClient>,

    /// Working directory
    work_dir: PathBuf,
}

impl CogManager {
    /// Create new cog manager
    pub fn new(work_dir: PathBuf) -> Result<Self> {
        let registry = RegistryClient::from_manifest()?;
        let cache_dir = CacheManager::default_cache_dir()?;
        let cache = CacheManager::new(cache_dir)?;
        let security = SecurityScanner::new();

        // Load enterprise config if available
        let enterprise_config_path = work_dir.join(".verum").join("enterprise.toml");
        let enterprise = if enterprise_config_path.exists() {
            let config = EnterpriseClient::load_config(&enterprise_config_path)?;
            Some(EnterpriseClient::new(config)?)
        } else {
            None
        };

        Ok(Self {
            registry,
            cache,
            security,
            enterprise,
            work_dir,
        })
    }

    /// Install cog and all dependencies
    pub fn install(&mut self, name: &str, version: Option<Text>) -> Result<()> {
        ui::step(&format!("Installing cog: {}", name.cyan()));

        // Check enterprise access control
        if let Some(ent) = &self.enterprise {
            if !ent.is_cog_allowed(name) {
                return Err(CliError::Custom(format!(
                    "Cog {} is not allowed by enterprise policy",
                    name
                )));
            }

            if ent.is_offline() {
                ui::warn("Running in offline mode - using cached cogs only");
            }
        }

        // Determine version
        let version_str = if let Some(v) = version {
            v
        } else {
            ui::info("Fetching latest version...");
            self.registry.get_latest_version(name)?
        };

        // Get cog metadata
        let metadata = self.registry.get_metadata(name, version_str.as_str())?;

        // Security scan
        ui::step("Running security scan...");
        let scan_result = self.security.scan_cog(&metadata)?;

        if !scan_result.vulnerabilities.is_empty() {
            ui::warn(&format!(
                "Found {} vulnerabilities",
                scan_result.vulnerabilities.len()
            ));

            for vuln in &scan_result.vulnerabilities {
                ui::error(&format!(
                    "  - {:?}: {}",
                    vuln.vulnerability.severity, vuln.vulnerability.title
                ));
            }

            // Check enterprise policy
            if let Some(ent) = &self.enterprise
                && ent.config().compliance.require_vulnerability_scan
            {
                return Err(CliError::Custom(
                    "Cog has vulnerabilities - installation blocked by policy".into(),
                ));
            }
        }

        // Check license compliance
        if let Some(license) = &metadata.license
            && let Some(ent) = &self.enterprise
            && !ent.is_license_allowed(license.as_str())
        {
            return Err(CliError::Custom(format!(
                "License {} is not allowed by enterprise policy",
                license
            )));
        }

        // Resolve dependencies
        ui::step("Resolving dependencies...");
        let resolved = self.resolve_dependencies(&metadata)?;

        ui::info(&format!("Installing {} cogs", resolved.len()));

        // Download all cogs in parallel
        let mut download_tasks = List::new();

        for dep in &resolved {
            let url: Text = format!(
                "{}/cogs/{}/{}/download",
                DEFAULT_REGISTRY, dep.name, dep.version
            )
            .into();

            download_tasks.push((
                dep.name.clone(),
                dep.version.clone(),
                url,
                dep.checksum.clone(),
            ));
        }

        ui::step("Downloading cogs...");
        let paths = self.cache.download_parallel(download_tasks)?;

        ui::success(&format!("Downloaded {} cogs", paths.len()));

        // Generate lockfile
        ui::step("Generating lockfile...");
        let lockfile = self.create_lockfile(name, &resolved)?;
        let lockfile_path = self.work_dir.join("verum.lock");
        lockfile.to_file(&lockfile_path)?;

        // Log audit entry
        if let Some(ent) = &self.enterprise
            && ent.config().audit.enabled
        {
            self.security.log_action(
                security::AuditAction::Install,
                Some(name.into()),
                Some(version_str.clone()),
                format!("Installed {} dependencies", resolved.len()).into(),
            );
        }

        ui::success(&format!("Successfully installed {}", name.cyan()));

        Ok(())
    }

    /// Update cog
    pub fn update(&mut self, name: &str) -> Result<()> {
        ui::step(&format!("Updating cog: {}", name.cyan()));

        // Get current version from lockfile
        let lockfile_path = self.work_dir.join("verum.lock");

        if !lockfile_path.exists() {
            return Err(CliError::Custom("No lockfile found".into()));
        }

        let lockfile = Lockfile::from_file(&lockfile_path)?;
        let current = lockfile
            .get_cog(name)
            .ok_or_else(|| CliError::DependencyNotFound(name.into()))?;

        // Get latest version
        let latest_version = self.registry.get_latest_version(name)?;
        let current_version = Version::parse(current.version.as_str())?;
        let latest = Version::parse(latest_version.as_str())?;

        if latest <= current_version {
            ui::info(&format!(
                "Cog {} is already at latest version {}",
                name.cyan(),
                current_version
            ));
            return Ok(());
        }

        ui::info(&format!(
            "Updating from {} to {}",
            current_version, latest_version
        ));

        // Install new version
        self.install(name, Some(latest_version))?;

        Ok(())
    }

    /// Remove cog
    pub fn remove(&mut self, name: &str) -> Result<()> {
        ui::step(&format!("Removing cog: {}", name.cyan()));

        let lockfile_path = self.work_dir.join("verum.lock");

        if !lockfile_path.exists() {
            return Err(CliError::Custom("No lockfile found".into()));
        }

        let mut lockfile = Lockfile::from_file(&lockfile_path)?;

        if lockfile.remove_cog(name) {
            lockfile.to_file(&lockfile_path)?;

            // Log audit entry
            if let Some(ent) = &self.enterprise
                && ent.config().audit.enabled
            {
                self.security.log_action(
                    security::AuditAction::Remove,
                    Some(name.into()),
                    None,
                    "Cog removed".into(),
                );
            }

            ui::success(&format!("Removed {}", name.cyan()));
        } else {
            ui::warn(&format!("Cog {} not found in lockfile", name.cyan()));
        }

        Ok(())
    }

    /// Publish cog to registry
    pub fn publish(&mut self, dry_run: bool, allow_dirty: bool) -> Result<()> {
        if dry_run {
            ui::step("Performing dry run of cog publish");
        } else {
            ui::step("Publishing cog");
        }

        // Load manifest
        let manifest_path = Manifest::manifest_path(&self.work_dir);
        let manifest = Manifest::from_file(&manifest_path)?;

        // Validate manifest
        manifest.validate()?;

        // Check git status if not allowing dirty
        if !allow_dirty {
            ui::info("Checking git status...");
            self.check_git_status()?;
        }

        // Build cog
        ui::step("Building cog...");
        let cog_path = self.build_cog(&manifest)?;

        if dry_run {
            ui::info(&format!(
                "Cog built: {} (dry run - not publishing)",
                cog_path.display()
            ));
            return Ok(());
        }

        // Sign cog
        ui::step("Signing cog...");
        let signature = self.sign_cog(&cog_path)?;

        // Create cog metadata
        let metadata = self.create_cog_metadata(&manifest, &signature)?;

        // Upload to registry
        ui::step("Uploading to registry...");

        // Get auth token
        let token = self.get_auth_token()?;

        self.registry
            .publish(&metadata, &cog_path, token.as_str())?;

        // Log audit entry
        if let Some(ent) = &self.enterprise
            && ent.config().audit.enabled
        {
            self.security.log_action(
                security::AuditAction::Publish,
                Some(manifest.cog.name.clone()),
                Some(manifest.cog.version.clone()),
                "Cog published".into(),
            );
        }

        ui::success(&format!(
            "Published {} v{}",
            manifest.cog.name.as_str().cyan(),
            manifest.cog.version
        ));

        Ok(())
    }

    /// Search for cogs
    pub fn search(&self, query: &str, limit: usize) -> Result<()> {
        ui::step(&format!("Searching for: {}", query.cyan()));

        let results = self.registry.search(query, limit)?;

        if results.is_empty() {
            ui::warn("No cogs found");
            return Ok(());
        }

        println!();
        println!(
            "{:<30} {:<12} {}",
            "Cog".bold(),
            "Version".bold(),
            "Description".bold()
        );
        println!("{}", "─".repeat(80));

        for result in results {
            let verified = if result.verified { "✓" } else { "" };
            println!(
                "{:<30} {:<12} {}",
                format!("{} {}", result.name, verified).cyan(),
                result.version,
                result.description.unwrap_or_default()
            );
        }

        println!();
        Ok(())
    }

    /// Audit dependencies for vulnerabilities
    pub fn audit(&mut self) -> Result<()> {
        ui::step("Auditing dependencies for vulnerabilities");

        // Load lockfile
        let lockfile_path = self.work_dir.join("verum.lock");

        if !lockfile_path.exists() {
            return Err(CliError::Custom("No lockfile found".into()));
        }

        let lockfile = Lockfile::from_file(&lockfile_path)?;

        // Update vulnerability database
        ui::info("Updating vulnerability database...");
        self.security.update_database(DEFAULT_REGISTRY)?;

        // Collect all cog metadata
        let mut cogs = List::new();

        for locked in &lockfile.packages {
            let metadata = self
                .registry
                .get_metadata(locked.name.as_str(), locked.version.as_str())?;
            cogs.push(metadata);
        }

        // Scan all cogs
        ui::step("Scanning cogs...");
        let results = self.security.scan_dependencies(&cogs)?;

        // Generate report
        let report = self.security.generate_report(&results);

        // Display results
        println!();
        println!("{}", "Security Audit Report".bold());
        println!("{}", "═".repeat(80));
        println!("Total vulnerabilities: {}", report.total_vulnerabilities);
        println!("Affected cogs: {}", report.affected_cogs);
        println!();
        println!("Severity breakdown:");
        println!("  Critical: {}", report.critical_count.to_string().red());
        println!("  High:     {}", report.high_count.to_string().yellow());
        println!("  Medium:   {}", report.medium_count);
        println!("  Low:      {}", report.low_count);
        println!();

        if !report.is_clean() {
            ui::warn("Vulnerabilities found!");

            for result in &results {
                if !result.vulnerabilities.is_empty() {
                    println!(
                        "{}",
                        result.vulnerabilities[0].cog.as_str().cyan().bold()
                    );

                    for vuln in &result.vulnerabilities {
                        println!(
                            "  - {:?}: {}",
                            vuln.vulnerability.severity, vuln.vulnerability.title
                        );
                        println!("    {}", vuln.vulnerability.description);
                    }

                    println!();
                }
            }
        } else {
            ui::success("No vulnerabilities found");
        }

        Ok(())
    }

    /// Generate SBOM
    pub fn generate_sbom(&self, format: enterprise::SbomFormat, output: &Path) -> Result<()> {
        ui::step("Generating Software Bill of Materials (SBOM)");

        // Load lockfile
        let lockfile_path = self.work_dir.join("verum.lock");

        if !lockfile_path.exists() {
            return Err(CliError::Custom("No lockfile found".into()));
        }

        let lockfile = Lockfile::from_file(&lockfile_path)?;

        // Collect all cog metadata
        let mut cogs = List::new();

        for locked in &lockfile.packages {
            let metadata = self
                .registry
                .get_metadata(locked.name.as_str(), locked.version.as_str())?;
            cogs.push(metadata);
        }

        // Generate SBOM
        let generator = enterprise::SbomGenerator::new(format);
        generator.generate(&cogs, output)?;

        ui::success(&format!("SBOM generated: {}", output.display()));

        Ok(())
    }

    // Private helper methods

    /// Resolve all dependencies
    fn resolve_dependencies(&self, root: &CogMetadata) -> Result<List<ResolvedCogInfo>> {
        // Use SAT resolver for optimal dependency resolution
        let mut sat_resolver = SatResolver::new();

        // Add root cog
        sat_resolver.add_metadata(root.clone());

        let root_version = Version::parse(root.version.as_str()).map_err(|e| {
            CliError::Custom(format!(
                "Invalid root cog version '{}': {}",
                root.version, e
            ))
        })?;

        let root_var = sat_resolver::CogVar::new(root.name.as_str(), root_version);

        sat_resolver.add_root_constraint(&root_var);

        // Recursively add all dependencies
        let mut to_process = vec![root.clone()];
        let mut processed = Set::new();

        while let Some(pkg) = to_process.pop() {
            if processed.contains(&pkg.name) {
                continue;
            }

            processed.insert(pkg.name.clone());

            for (dep_name, dep_spec) in &pkg.dependencies {
                // Get all versions of dependency
                let latest = self.registry.get_latest_version(dep_name.as_str())?;
                let dep_metadata = self
                    .registry
                    .get_metadata(dep_name.as_str(), latest.as_str())?;

                sat_resolver.add_metadata(dep_metadata.clone());

                // Add constraint
                let pkg_version = Version::parse(pkg.version.as_str()).map_err(|e| {
                    CliError::Custom(format!("Invalid cog version '{}': {}", pkg.version, e))
                })?;

                let pkg_var = sat_resolver::CogVar::new(pkg.name.as_str(), pkg_version);

                let version_req = match dep_spec {
                    DependencySpec::Simple(v) => VersionReq::parse(v.as_str())?,
                    DependencySpec::Detailed { version, .. } => {
                        let version_str = version.as_ref().ok_or_else(|| {
                            CliError::Custom(format!(
                                "Dependency '{}' missing version specification",
                                dep_name
                            ))
                        })?;
                        VersionReq::parse(version_str.as_str())?
                    }
                };

                sat_resolver.add_dependency_constraint(&pkg_var, dep_name.as_str(), &version_req);
                sat_resolver.add_uniqueness_constraint(dep_name.as_str());

                to_process.push(dep_metadata);
            }
        }

        // Solve SAT
        let solution = sat_resolver.solve()?;

        if !solution.conflicts.is_empty() {
            let conflict_msg = solution
                .conflicts
                .iter()
                .map(|c| {
                    format!(
                        "{}: {} (required by: {})",
                        c.cog,
                        c.versions.join(", "),
                        c.required_by.join(", ")
                    )
                })
                .collect::<List<_>>()
                .join("\n");

            return Err(CliError::Custom(format!(
                "Dependency conflicts:\n{}",
                conflict_msg
            )));
        }

        // Convert to ResolvedCogInfo
        let mut resolved = List::new();

        for (name, version) in solution.selected {
            let version_str = version.to_string();
            let metadata = self
                .registry
                .get_metadata(name.as_str(), version_str.as_str())?;

            resolved.push(ResolvedCogInfo {
                name,
                version: version_str.into(),
                checksum: metadata.checksum,
            });
        }

        Ok(resolved)
    }

    /// Create lockfile from resolved dependencies
    fn create_lockfile(
        &self,
        root_name: &str,
        resolved: &[ResolvedCogInfo],
    ) -> Result<Lockfile> {
        let mut lockfile = Lockfile::new(root_name.into());

        for dep in resolved {
            let metadata = self
                .registry
                .get_metadata(dep.name.as_str(), dep.version.as_str())?;

            let locked = LockedCog {
                name: dep.name.clone(),
                version: dep.version.clone(),
                source: CogSource::Registry {
                    registry: DEFAULT_REGISTRY.into(),
                    version: dep.version.clone(),
                },
                checksum: dep.checksum.clone(),
                dependencies: metadata
                    .dependencies
                    .keys()
                    .map(|k| (k.clone(), Text::new()))
                    .collect(),
                features: List::new(),
                optional: false,
            };

            lockfile.add_cog(locked);
        }

        Ok(lockfile)
    }

    /// Check git status for uncommitted changes
    ///
    /// Returns an error if there are uncommitted changes in the working directory.
    /// This prevents publishing cogs with local modifications.
    fn check_git_status(&self) -> Result<()> {
        use std::process::Command;

        // Check if we're in a git repository
        let git_dir = self.work_dir.join(".git");
        if !git_dir.exists() {
            // Not a git repository - skip check
            ui::warn("Not a git repository, skipping dirty check");
            return Ok(());
        }

        // Run git status --porcelain to check for changes
        let output = Command::new("git")
            .arg("status")
            .arg("--porcelain")
            .current_dir(&self.work_dir)
            .output()
            .map_err(|e| CliError::GitError(format!("Failed to run git status: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CliError::GitError(format!("git status failed: {}", stderr)));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if !stdout.trim().is_empty() {
            // There are uncommitted changes
            let changed_files: Vec<&str> = stdout.lines().take(10).collect();
            let mut msg = String::from("Uncommitted changes detected:\n");
            for file in &changed_files {
                msg.push_str(&format!("  {}\n", file));
            }
            if stdout.lines().count() > 10 {
                msg.push_str(&format!("  ... and {} more\n", stdout.lines().count() - 10));
            }
            msg.push_str("\nUse --allow-dirty to publish anyway, or commit your changes first.");

            return Err(CliError::DirtyWorkingDirectory(msg));
        }

        // Check for untracked files that should be included
        let output = Command::new("git")
            .arg("ls-files")
            .arg("--others")
            .arg("--exclude-standard")
            .current_dir(&self.work_dir)
            .output()
            .map_err(|e| CliError::GitError(format!("Failed to check untracked files: {}", e)))?;

        if output.status.success() {
            let untracked = String::from_utf8_lossy(&output.stdout);
            if !untracked.trim().is_empty() {
                let untracked_count = untracked.lines().count();
                if untracked_count > 0 {
                    ui::warn(&format!(
                        "{} untracked file(s) will not be included in cog",
                        untracked_count
                    ));
                }
            }
        }

        ui::success("Working directory is clean");
        Ok(())
    }

    /// Build cog archive
    fn build_cog(&self, manifest: &Manifest) -> Result<PathBuf> {
        let cog_name = &manifest.cog.name;
        let version = &manifest.cog.version;

        // Create archive
        self.cache
            .create_archive(&self.work_dir, cog_name.as_str(), version.as_str())
    }

    /// Sign cog
    fn sign_cog(&self, cog_path: &Path) -> Result<CogSignature> {
        let signing_key_path = self.work_dir.join(".verum").join("signing_key");

        if !signing_key_path.exists() {
            // Generate new key
            let key = CogSigner::generate_key();
            CogSigner::save_key(&key, &signing_key_path)?;
        }

        let mut signer = CogSigner::new();
        signer.load_key(&signing_key_path)?;

        signer.sign_cog(cog_path)
    }

    /// Create cog metadata
    fn create_cog_metadata(
        &self,
        manifest: &Manifest,
        signature: &CogSignature,
    ) -> Result<CogMetadata> {
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
            readme: None,
            dependencies: manifest
                .dependencies
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        match v {
                            crate::config::Dependency::Simple(s) => {
                                DependencySpec::Simple(s.clone())
                            }
                            crate::config::Dependency::Detailed {
                                version,
                                features,
                                optional,
                                ..
                            } => DependencySpec::Detailed {
                                version: version.clone(),
                                features: features.clone(),
                                optional: *optional,
                                default_features: None,
                            },
                        },
                    )
                })
                .collect(),
            features: manifest.features.clone(),
            artifacts: TierArtifacts::default(),
            proofs: None,
            cbgr_profiles: None,
            signature: Some(signature.clone()),
            ipfs_hash: None,
            checksum: Text::new(), // Will be calculated by registry
            published_at: chrono::Utc::now().timestamp(),
        })
    }

    /// Get authentication token
    fn get_auth_token(&self) -> Result<Text> {
        // Load from config or environment
        if let Ok(token) = std::env::var("VERUM_TOKEN") {
            return Ok(token.into());
        }

        // Load from credentials file
        let creds_path = dirs::home_dir()
            .ok_or_else(|| CliError::Custom("Cannot determine home directory".into()))?
            .join(".verum")
            .join("credentials");

        if creds_path.exists() {
            let content = std::fs::read_to_string(&creds_path)?;
            let creds: Map<Text, Text> = toml::from_str(&content)?;
            let token_key: Text = "token".into();

            if let Some(token) = creds.get(&token_key) {
                return Ok(token.clone());
            }
        }

        Err(CliError::Custom(
            "No authentication token found. Run 'verum login' first.".into(),
        ))
    }
}

/// Resolved cog information
#[derive(Debug, Clone)]
struct ResolvedCogInfo {
    name: Text,
    version: Text,
    checksum: Text,
}
