// Dependency management commands

use crate::config::{Dependency, Manifest};
use crate::error::Result;
use crate::ui;
use colored::Colorize;
use verum_common::{List, Text};

/// Default registry URL for the Verum cog registry.
static DEFAULT_REGISTRY_URL: &str = "https://vcogs.io";

/// Add a dependency to the project manifest
///
/// # Arguments
/// * `name` - Package name to add
/// * `version` - Optional version constraint (defaults to "*")
/// * `dev` - Add as dev dependency (test/development only)
/// * `build` - Add as build dependency (build scripts only)
pub fn add(name: &str, version: Option<Text>, dev: bool, build: bool) -> Result<()> {
    let dep_type = if build {
        "build dependency"
    } else if dev {
        "dev dependency"
    } else {
        "dependency"
    };

    ui::step(&format!("Adding {}: {}", dep_type, name.cyan()));

    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let mut manifest = Manifest::from_file(&manifest_path)?;

    let dep = if let Some(v) = version {
        Dependency::Simple(v)
    } else {
        Dependency::Simple("*".into())
    };

    // Add to appropriate section based on flags
    // Priority: build > dev > regular
    if build {
        manifest.build_dependencies.insert(name.into(), dep);
    } else if dev {
        manifest.dev_dependencies.insert(name.into(), dep);
    } else {
        manifest.dependencies.insert(name.into(), dep);
    }

    manifest.to_file(&manifest_path)?;
    ui::success(&format!("Added {} as {}", name, dep_type));
    Ok(())
}

/// Remove a dependency from the project manifest
///
/// # Arguments
/// * `name` - Package name to remove
/// * `dev` - Remove from dev dependencies only
/// * `build` - Remove from build dependencies only
///
/// If neither `dev` nor `build` is specified, removes from all sections.
pub fn remove(name: &str, dev: bool, build: bool) -> Result<()> {
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let mut manifest = Manifest::from_file(&manifest_path)?;

    let mut removed = false;

    let name_key = Text::from(name);
    if build {
        // Remove only from build dependencies
        ui::step(&format!("Removing build dependency: {}", name.cyan()));
        if manifest.build_dependencies.remove(&name_key).is_some() {
            removed = true;
        }
    } else if dev {
        // Remove only from dev dependencies
        ui::step(&format!("Removing dev dependency: {}", name.cyan()));
        if manifest.dev_dependencies.remove(&name_key).is_some() {
            removed = true;
        }
    } else {
        // Remove from all sections
        ui::step(&format!("Removing dependency: {}", name.cyan()));
        if manifest.dependencies.remove(&name_key).is_some() {
            removed = true;
        }
        if manifest.dev_dependencies.remove(&name_key).is_some() {
            removed = true;
        }
        if manifest.build_dependencies.remove(&name_key).is_some() {
            removed = true;
        }
    }

    if removed {
        manifest.to_file(&manifest_path)?;
        ui::success(&format!("Removed {}", name));
    } else {
        ui::warn(&format!("Dependency '{}' not found", name));
    }

    Ok(())
}

pub fn update(package: Option<Text>) -> Result<()> {
    use crate::registry::{RegistryClient, resolve_version};
    use semver::Version;

    if let Some(ref pkg) = package {
        ui::step(&format!("Updating {}", pkg.as_str().cyan()));
    } else {
        ui::step("Updating all dependencies");
    }

    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let mut manifest = Manifest::from_file(&manifest_path)?;

    // Create registry client
    let client = RegistryClient::default()?;
    let mut updates = Vec::new();

    // Get dependencies to update
    let deps_to_update: Vec<(Text, Dependency)> = if let Some(ref pkg_name) = package {
        manifest
            .dependencies
            .iter()
            .filter(|(name, _)| name.as_str() == pkg_name.as_str())
            .map(|(n, d)| (n.clone(), d.clone()))
            .collect()
    } else {
        manifest
            .dependencies
            .iter()
            .map(|(n, d)| (n.clone(), d.clone()))
            .collect()
    };

    if deps_to_update.is_empty() {
        if let Some(ref pkg_name) = package {
            ui::warn(&format!("Cog '{}' not found in dependencies", pkg_name));
        } else {
            ui::info("No dependencies to update");
        }
        return Ok(());
    }

    // Check each dependency for updates
    for (name, dep) in &deps_to_update {
        let current_version = match dep {
            Dependency::Simple(v) => v.clone(),
            Dependency::Detailed { version, .. } => version.clone().unwrap_or_else(|| "*".into()),
        };

        ui::info(&format!("  Checking {}...", name));

        // Query registry for available versions via get_index
        match client.get_index(name.as_str()) {
            Ok(index_entry) => {
                // Parse available versions from index entry
                let available: Vec<Version> = index_entry
                    .versions
                    .iter()
                    .filter(|v| !v.yanked)
                    .filter_map(|v| Version::parse(v.version.as_str()).ok())
                    .collect();

                if available.is_empty() {
                    ui::warn(&format!("    No versions available for {}", name));
                    continue;
                }

                // Find latest version matching constraints
                let version_req = current_version.as_str();
                match resolve_version(version_req, &available) {
                    Ok(latest) => {
                        let latest_str: Text = latest.to_string().into();

                        // Check if update is available
                        let current_parsed = Version::parse(
                            current_version
                                .as_str()
                                .trim_start_matches(&['^', '~', '=', '>', '<', ' '][..]),
                        );

                        if let Ok(current) = current_parsed {
                            if latest > current {
                                ui::info(&format!(
                                    "    {} {} -> {} (update available)",
                                    name.as_str().cyan(),
                                    current_version,
                                    latest_str.as_str().green()
                                ));
                                updates.push((name.clone(), latest_str));
                            } else {
                                ui::info(&format!("    {} {} (up to date)", name, current_version));
                            }
                        } else {
                            // If we can't parse current version, always suggest latest
                            updates.push((name.clone(), latest_str.clone()));
                            ui::info(&format!("    {} -> {} (new version)", name, latest_str));
                        }
                    }
                    Err(e) => {
                        ui::warn(&format!("    Failed to resolve {}: {}", name, e));
                    }
                }
            }
            Err(e) => {
                // Registry unavailable - try to work offline
                ui::warn(&format!("    Registry unavailable for {}: {}", name, e));
            }
        }
    }

    // Apply updates
    if updates.is_empty() {
        ui::success("All dependencies are up to date");
    } else {
        ui::step(&format!("Applying {} updates...", updates.len()));

        for (name, new_version) in &updates {
            if let Some(dep) = manifest.dependencies.get_mut(name) {
                match dep {
                    Dependency::Simple(v) => {
                        *v = format!("^{}", new_version).into();
                    }
                    Dependency::Detailed { version, .. } => {
                        *version = Some(format!("^{}", new_version).into());
                    }
                }
                ui::info(&format!("  Updated {} to {}", name, new_version));
            }
        }

        // Save updated manifest
        manifest.to_file(&manifest_path)?;

        // Update lockfile
        update_lockfile(&manifest_dir, &manifest)?;

        ui::success(&format!("Updated {} dependencies", updates.len()));
    }

    Ok(())
}

/// Update lockfile after dependency changes
fn update_lockfile(manifest_dir: &std::path::Path, manifest: &Manifest) -> Result<()> {
    use crate::registry::types::CogSource;
    use crate::registry::{
        DependencyResolver, LockedCog, Lockfile, RegistryClient, resolve_version,
    };
    use semver::Version;
    use verum_common::Map;
    use verum_common::Set;

    let lockfile_path = Manifest::lockfile_path(&manifest_dir);
    let mut lockfile = if lockfile_path.exists() {
        Lockfile::from_file(&lockfile_path)?
    } else {
        Lockfile::new(manifest.cog.name.clone())
    };

    let client = RegistryClient::default()?;
    let mut resolver = DependencyResolver::new();

    // Add root package
    let root_version =
        Version::parse(manifest.cog.version.as_str()).unwrap_or_else(|_| Version::new(0, 0, 0));

    let root_idx = resolver.add_cog(
        manifest.cog.name.clone(),
        root_version,
        CogSource::Path {
            path: manifest_dir.to_path_buf(),
        },
        Set::new(),
    );

    // Resolve all dependencies transitively
    for (name, dep) in &manifest.dependencies {
        let version_req = match dep {
            Dependency::Simple(v) => v.clone(),
            Dependency::Detailed { version, .. } => version.clone().unwrap_or_else(|| "*".into()),
        };

        // Try to get versions from registry via get_index
        if let Ok(index_entry) = client.get_index(name.as_str()) {
            let available: Vec<Version> = index_entry
                .versions
                .iter()
                .filter(|v| !v.yanked)
                .filter_map(|v| Version::parse(v.version.as_str()).ok())
                .collect();

            if let Ok(resolved_version) = resolve_version(version_req.as_str(), &available) {
                // Add to resolver graph
                let dep_idx = resolver.add_cog(
                    name.clone(),
                    resolved_version.clone(),
                    CogSource::Registry {
                        registry: DEFAULT_REGISTRY_URL.into(),
                        version: resolved_version.to_string().into(),
                    },
                    Set::new(),
                );

                // Create edge from root to dependency
                let version_req = semver::VersionReq::parse(version_req.as_str())
                    .unwrap_or(semver::VersionReq::STAR);
                resolver.add_dependency(
                    root_idx,
                    dep_idx,
                    version_req,
                    List::new(),
                    false,
                );

                // Update lockfile
                lockfile
                    .packages
                    .retain(|p| p.name.as_str() != name.as_str());
                lockfile.packages.push(LockedCog {
                    name: name.clone(),
                    version: resolved_version.to_string().into(),
                    source: CogSource::Registry {
                        registry: DEFAULT_REGISTRY_URL.into(),
                        version: resolved_version.to_string().into(),
                    },
                    checksum: "".into(), // Filled when downloading
                    dependencies: Map::new(),
                    features: List::new(),
                    optional: false,
                });
            }
        }
    }

    // Check for conflicts and cycles
    if let Err(e) = resolver.check_conflicts() {
        ui::warn(&format!("Version conflict detected: {}", e));
    }

    if let Some(cycle) = resolver.detect_cycles() {
        ui::warn(&format!("Dependency cycle detected: {:?}", cycle));
    }

    // Save lockfile
    lockfile.to_file(&lockfile_path)?;

    Ok(())
}

pub fn list(tree: bool) -> Result<()> {
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest = Manifest::from_file(&Manifest::manifest_path(&manifest_dir))?;

    println!();
    println!("{}", "Dependencies:".bold());

    if tree {
        for name in manifest.dependencies.keys() {
            println!("  {} {}", "└─".cyan(), name);
        }
    } else {
        for (name, dep) in &manifest.dependencies {
            let version = match dep {
                Dependency::Simple(v) => v.clone(),
                Dependency::Detailed { version, .. } => {
                    version.clone().unwrap_or_else(|| "*".into())
                }
            };
            println!("  {} {}", name.as_str().cyan(), version);
        }
    }

    if !manifest.dev_dependencies.is_empty() {
        println!();
        println!("{}", "Dev Dependencies:".bold());
        for (name, dep) in &manifest.dev_dependencies {
            let version = match dep {
                Dependency::Simple(v) => v.clone(),
                Dependency::Detailed { version, .. } => {
                    version.clone().unwrap_or_else(|| "*".into())
                }
            };
            println!("  {} {}", name.as_str().cyan(), version);
        }
    }

    println!();
    Ok(())
}
