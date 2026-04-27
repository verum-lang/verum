// Remove dependencies from verum.toml and clean up lockfile entries.

use crate::config::Manifest;
use crate::error::Result;
use crate::registry::Lockfile;
use crate::ui;
use colored::Colorize;
use verum_common::{List, Set, Text};

/// Remove dependency options
pub struct RemoveOptions {
    pub name: Text,
    pub dev: bool,
    pub build: bool,
}

/// Remove dependency from project
pub fn remove(options: RemoveOptions) -> Result<()> {
    ui::step(&format!(
        "Removing dependency: {}",
        options.name.as_str().cyan()
    ));

    // Find manifest
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let mut manifest = Manifest::from_file(&manifest_path)?;

    // Remove from appropriate section
    let mut removed = false;

    if options.build {
        removed = manifest.build_dependencies.remove(&options.name).is_some();
        if removed {
            ui::info("Removed from [build-dependencies]");
        }
    } else if options.dev {
        removed = manifest.dev_dependencies.remove(&options.name).is_some();
        if removed {
            ui::info("Removed from [dev-dependencies]");
        }
    } else {
        // Try regular, dev, and build dependencies
        if manifest.dependencies.remove(&options.name).is_some() {
            ui::info("Removed from [dependencies]");
            removed = true;
        } else if manifest.dev_dependencies.remove(&options.name).is_some() {
            ui::info("Removed from [dev-dependencies]");
            removed = true;
        } else if manifest.build_dependencies.remove(&options.name).is_some() {
            ui::info("Removed from [build-dependencies]");
            removed = true;
        }
    }

    if !removed {
        ui::warn(&format!("Dependency '{}' not found", options.name));
        return Ok(());
    }

    // Save manifest
    manifest.to_file(&manifest_path)?;

    // Update lockfile
    update_lockfile(&manifest_dir, options.name.as_str())?;

    ui::success(&format!("Removed {}", options.name));

    // Show next steps
    println!();
    ui::info("Run 'verum build' to update dependencies");

    Ok(())
}

/// Update lockfile after removing dependency
fn update_lockfile(manifest_dir: &std::path::Path, removed_name: &str) -> Result<()> {
    let lockfile_path = manifest_dir.join("verum.lock");

    if !lockfile_path.exists() {
        return Ok(());
    }

    let mut lockfile = Lockfile::from_file(&lockfile_path)?;

    // Remove the dependency
    if lockfile.remove_cog(removed_name) {
        ui::info("Updated lockfile");
    }

    // Check for orphaned dependencies
    let orphans = find_orphaned_dependencies(&lockfile, removed_name);

    if !orphans.is_empty() {
        ui::info(&format!(
            "Removing {} orphaned dependencies...",
            orphans.len()
        ));

        for orphan in orphans {
            lockfile.remove_cog(orphan.as_str());
        }
    }

    // Save lockfile
    lockfile.to_file(&lockfile_path)?;

    Ok(())
}

/// Find dependencies that are no longer needed
fn find_orphaned_dependencies(lockfile: &Lockfile, removed: &str) -> List<Text> {
    let dep_graph = lockfile.dependency_graph();

    let mut needed = Set::new();
    let mut to_visit = vec![lockfile.root.clone()];

    // Mark all reachable dependencies
    while let Some(pkg) = to_visit.pop() {
        if needed.contains(&pkg) {
            continue;
        }

        needed.insert(pkg.clone());

        if let Some(deps) = dep_graph.get(&pkg) {
            for dep in deps {
                if dep != removed {
                    to_visit.push(dep.clone());
                }
            }
        }
    }

    // Find orphans
    lockfile
        .packages
        .iter()
        .filter(|p| !needed.contains(&p.name) && p.name != removed)
        .map(|p| p.name.clone())
        .collect()
}
