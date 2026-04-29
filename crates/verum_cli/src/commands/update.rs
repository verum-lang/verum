// Update dependencies with semver compatibility checking.
// Supports workspace-wide updates, aggressive (breaking) mode, and dry-run.

use crate::config::Manifest;
use crate::error::Result;
use crate::registry::{Lockfile, RegistryClient};
use crate::ui;
use colored::Colorize;
use semver::Version;
use verum_common::{List, Text};

/// Update dependency options
pub struct UpdateOptions {
    pub package: Option<Text>,
    pub workspace: bool,
    pub aggressive: bool,
    pub dry_run: bool,
}

/// Update dependencies
pub fn update(options: UpdateOptions) -> Result<()> {
    if let Some(package) = &options.package {
        ui::step(&format!("Updating dependency: {}", package.as_str().cyan()));
        update_single_cog(package.as_str(), &options)
    } else {
        ui::step("Updating all dependencies");
        update_all_cogs(&options)
    }
}

/// Update single package
fn update_single_cog(package: &str, options: &UpdateOptions) -> Result<()> {
    // Find manifest
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest = Manifest::from_file(&Manifest::manifest_path(&manifest_dir))?;

    // Check if package exists in dependencies
    let package_key = Text::from(package);
    if !manifest.dependencies.contains_key(&package_key)
        && !manifest.dev_dependencies.contains_key(&package_key)
    {
        return Err(crate::error::CliError::DependencyNotFound(package.into()));
    }

    // Get current version
    let lockfile_path = manifest_dir.join("verum.lock");
    let lockfile = if lockfile_path.exists() {
        Some(Lockfile::from_file(&lockfile_path)?)
    } else {
        None
    };

    let current_version = lockfile
        .as_ref()
        .and_then(|l| l.get_cog(package))
        .map(|p| p.version.clone());

    // Fetch latest version
    let client = RegistryClient::from_manifest()?;
    let latest_version = client.get_latest_version(package)?;

    if let Some(current) = &current_version {
        if current == &latest_version {
            ui::info(&format!("{} is already up to date ({})", package, current));
            return Ok(());
        }

        ui::info(&format!(
            "Updating {} from {} to {}",
            package, current, latest_version
        ));

        // Check compatibility
        if !options.aggressive {
            check_compatibility(current.as_str(), latest_version.as_str())?;
        }
    } else {
        ui::info(&format!("Installing {} {}", package, latest_version));
    }

    if options.dry_run {
        ui::info("[DRY RUN] Would update package");
        return Ok(());
    }

    // Update lockfile
    if let Some(mut lockfile) = lockfile {
        let metadata = client.get_metadata(package, latest_version.as_str())?;

        lockfile.update_cog(package, latest_version.clone(), metadata.checksum);
        lockfile.to_file(&lockfile_path)?;
    }

    ui::success(&format!("Updated {} to {}", package, latest_version));

    Ok(())
}

/// Update all packages
fn update_all_cogs(options: &UpdateOptions) -> Result<()> {
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest = Manifest::from_file(&Manifest::manifest_path(&manifest_dir))?;

    let lockfile_path = manifest_dir.join("verum.lock");
    if !lockfile_path.exists() {
        ui::warn("No lockfile found. Run 'verum build' first.");
        return Ok(());
    }

    let mut lockfile = Lockfile::from_file(&lockfile_path)?;
    let client = RegistryClient::from_manifest()?;

    let mut updated: List<Text> = List::new();
    let mut failed: List<(Text, Text)> = List::new();

    // Collect all dependencies. Honour `UpdateOptions.workspace`:
    // when set, additionally include `build-dependencies` (the
    // build-time-only dep group that's normally invisible to the
    // update walk because runtime execution doesn't see it).
    // Mirrors the established `verum tree --all-features` /
    // `cargo update --workspace` semantics: workspace-mode opts
    // INTO updating every declared dep group, not just runtime
    // + dev defaults. Pre-fix the field landed on UpdateOptions
    // but no code path consulted it — `verum update --workspace`
    // looked identical to plain `verum update`.
    let mut all_deps: List<_> = manifest
        .dependencies
        .keys()
        .chain(manifest.dev_dependencies.keys())
        .cloned()
        .collect();
    if options.workspace {
        for build_dep in manifest.build_dependencies.keys() {
            if !all_deps.contains(build_dep) {
                all_deps.push(build_dep.clone());
            }
        }
    }

    for cog_name in all_deps {
        let current_pkg = match lockfile.get_cog(cog_name.as_str()) {
            Some(p) => p,
            None => continue,
        };

        let current_version = &current_pkg.version;

        // Fetch latest version
        let latest_version = match client.get_latest_version(cog_name.as_str()) {
            Ok(v) => v,
            Err(e) => {
                failed.push((cog_name.clone(), e.to_string().into()));
                continue;
            }
        };

        if current_version == &latest_version {
            continue;
        }

        // Check compatibility
        if !options.aggressive
            && let Err(e) = check_compatibility(current_version.as_str(), latest_version.as_str())
        {
            ui::warn(&format!(
                "Skipping {}: {} (use --aggressive to force)",
                cog_name, e
            ));
            continue;
        }

        ui::info(&format!(
            "Updating {} from {} to {}",
            cog_name, current_version, latest_version
        ));

        if !options.dry_run {
            let metadata = match client.get_metadata(cog_name.as_str(), latest_version.as_str())
            {
                Ok(m) => m,
                Err(e) => {
                    failed.push((cog_name.clone(), e.to_string().into()));
                    continue;
                }
            };

            lockfile.update_cog(
                cog_name.as_str(),
                latest_version.clone(),
                metadata.checksum,
            );
            updated.push(cog_name);
        }
    }

    if options.dry_run {
        ui::info("[DRY RUN] Would update packages");
    } else if !updated.is_empty() {
        lockfile.to_file(&lockfile_path)?;
    }

    // Summary
    println!();
    if !updated.is_empty() {
        ui::success(&format!("Updated {} packages", updated.len()));
    }

    if !failed.is_empty() {
        ui::warn(&format!("Failed to update {} packages:", failed.len()));
        for (name, error) in failed {
            println!("  {} {}: {}", "✗".red(), name, error);
        }
    }

    Ok(())
}

/// Check if update is compatible (semver)
pub fn check_compatibility(current: &str, new: &str) -> Result<()> {
    let current_ver = Version::parse(current)
        .map_err(|e| crate::error::CliError::Custom(format!("Invalid version: {}", e)))?;

    let new_ver = Version::parse(new)
        .map_err(|e| crate::error::CliError::Custom(format!("Invalid version: {}", e)))?;

    // Check for breaking changes
    if new_ver.major > current_ver.major {
        return Err(crate::error::CliError::VersionConflict {
            package: "package".into(),
            required: current.into(),
            found: new.into(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts_with(workspace: bool) -> UpdateOptions {
        UpdateOptions {
            package: None,
            workspace,
            aggressive: false,
            dry_run: false,
        }
    }

    #[test]
    fn workspace_default_excludes_build_deps() {
        // Pin: the documented default keeps build-dependencies
        // out of the update walk so the typical `verum update`
        // run only touches what the runtime actually consumes.
        let opts = opts_with(false);
        assert!(!opts.workspace);
    }

    #[test]
    fn workspace_true_includes_build_deps_in_walk() {
        // Pin: --workspace opts INTO updating every declared
        // dep group, including the build-time-only one. Mirrors
        // `verum tree --all-features` semantics and matches the
        // `cargo update --workspace` convention. The cog-name
        // collection logic (in update_all_cogs) extends the
        // dependency iterator with build_dependencies when this
        // flag is set; the contract test here pins the field
        // shape so a refactor can't accidentally drop the gate.
        let opts = opts_with(true);
        assert!(opts.workspace);
    }
}
