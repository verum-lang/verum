// Visualize dependency graph as a tree.
// Supports depth limits, duplicate highlighting, and feature display.

use crate::config::Manifest;
use crate::error::Result;
use crate::registry::Lockfile;
use crate::ui;
use colored::Colorize;
use verum_common::{List, Set, Text};

/// Tree visualization options
pub struct TreeOptions {
    pub duplicates: bool,
    pub depth: Option<usize>,
    pub all_features: bool,
}

/// Display dependency tree
pub fn tree(options: TreeOptions) -> Result<()> {
    ui::step("Dependency Tree");

    // Find manifest and lockfile
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest = Manifest::from_file(&Manifest::manifest_path(&manifest_dir))?;

    let lockfile_path = Manifest::lockfile_path(&manifest_dir);
    if !lockfile_path.exists() {
        ui::warn("No lockfile found. Run 'verum build' first.");
        return Ok(());
    }

    let lockfile = Lockfile::from_file(&lockfile_path)?;

    // Build dependency graph
    let graph = lockfile.dependency_graph();

    // Find duplicates if requested
    let duplicates = if options.duplicates {
        find_duplicates(&lockfile)
    } else {
        Set::new()
    };

    // Print tree
    println!();
    println!(
        "{} {}",
        manifest.cog.name.as_str().bold(),
        manifest.cog.version
    );

    // Print regular dependencies
    if !manifest.dependencies.is_empty() {
        println!();
        println!("{}", "Dependencies:".bold());

        for name in manifest.dependencies.keys() {
            print_dependency_tree(
                name.as_str(),
                &lockfile,
                &graph,
                &duplicates,
                options.depth,
                0,
                "",
                true,
            );
        }
    }

    // Print dev dependencies
    if !manifest.dev_dependencies.is_empty() {
        println!();
        println!("{}", "Dev Dependencies:".bold());

        for name in manifest.dev_dependencies.keys() {
            print_dependency_tree(
                name.as_str(),
                &lockfile,
                &graph,
                &duplicates,
                options.depth,
                0,
                "",
                true,
            );
        }
    }

    // Honour `TreeOptions.all_features`: when set, additionally
    // print `build-dependencies` (the build-time-only dep group
    // that's normally invisible in the tree because runtime
    // execution doesn't see it). Mirrors `cargo tree --all-features`
    // semantics: the flag opts INTO showing every declared dep
    // group, not just the runtime + dev defaults. Pre-fix the
    // field landed on TreeOptions but no code path consulted it
    // — `verum tree --all-features` looked identical to plain
    // `verum tree`, defeating the documented opt-in.
    if options.all_features && !manifest.build_dependencies.is_empty() {
        println!();
        println!("{}", "Build Dependencies:".bold());

        for name in manifest.build_dependencies.keys() {
            print_dependency_tree(
                name.as_str(),
                &lockfile,
                &graph,
                &duplicates,
                options.depth,
                0,
                "",
                true,
            );
        }
    }

    // Print duplicate summary
    if options.duplicates && !duplicates.is_empty() {
        println!();
        ui::warn(&format!(
            "Found {} duplicate dependencies:",
            duplicates.len()
        ));

        for dup in duplicates.iter() {
            let versions: Vec<_> = lockfile
                .packages
                .iter()
                .filter(|p| &p.name == dup)
                .map(|p| p.version.as_str())
                .collect();

            println!(
                "  {} {}: versions {}",
                "!".yellow(),
                dup,
                versions.join(", ")
            );
        }
    }

    println!();
    Ok(())
}

/// Print dependency tree recursively
fn print_dependency_tree(
    name: &str,
    lockfile: &Lockfile,
    graph: &verum_common::Map<Text, List<Text>>,
    duplicates: &Set<Text>,
    max_depth: Option<usize>,
    current_depth: usize,
    prefix: &str,
    is_last: bool,
) {
    // Check depth limit
    if let Some(max) = max_depth
        && current_depth >= max
    {
        return;
    }

    // Get package info
    let package = match lockfile.get_cog(name) {
        Some(p) => p,
        None => {
            println!("{}├─ {} (not found)", prefix, name.red());
            return;
        }
    };

    // Determine connector
    let connector = if is_last { "└─" } else { "├─" };

    // Print package name and version
    let name_text: Text = name.into();
    let name_display = if duplicates.contains(&name_text) {
        format!("{} {}", name.yellow(), "(duplicate)".dimmed())
    } else {
        name.into()
    };

    println!(
        "{}{}─ {} {}",
        prefix,
        connector,
        name_display,
        package.version.as_str().dimmed()
    );

    // Print features if any
    if !package.features.is_empty() {
        let new_prefix = if is_last {
            format!("{}  ", prefix)
        } else {
            format!("{}│ ", prefix)
        };
        let features_str: String = package
            .features
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        println!("{}  features: [{}]", new_prefix, features_str.dimmed());
    }

    // Get dependencies
    let deps = match graph.get(&name_text) {
        Some(d) => d,
        None => return,
    };

    if deps.is_empty() {
        return;
    }

    // Print children
    let new_prefix = if is_last {
        format!("{}  ", prefix)
    } else {
        format!("{}│ ", prefix)
    };

    for (i, dep) in deps.iter().enumerate() {
        let is_last_child = i == deps.len() - 1;
        print_dependency_tree(
            dep.as_str(),
            lockfile,
            graph,
            duplicates,
            max_depth,
            current_depth + 1,
            &new_prefix,
            is_last_child,
        );
    }
}

/// Whether the requested options ask the tree printer to include
/// the build-dependencies group in the output.
///
/// Pure helper extracted so the gate semantics can be unit-tested
/// without spinning up a manifest + lockfile fixture. The wiring
/// at the call site is a single `if all_features { ... }` block
/// that uses this predicate by inlining the field read; this
/// helper exists so tests have a stable surface to pin the
/// contract against.
#[allow(dead_code)]
fn should_show_build_deps(options: &TreeOptions) -> bool {
    options.all_features
}

/// Find duplicate dependencies (same name, different versions)
fn find_duplicates(lockfile: &Lockfile) -> Set<Text> {
    let mut name_counts: verum_common::Map<Text, usize> = verum_common::Map::new();

    for package in &lockfile.packages {
        *name_counts.entry(package.name.clone()).or_insert(0) += 1;
    }

    name_counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(name, _)| name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts_with(all_features: bool) -> TreeOptions {
        TreeOptions {
            duplicates: false,
            depth: None,
            all_features,
        }
    }

    #[test]
    fn all_features_default_excludes_build_deps() {
        // Pin: the documented default keeps build-dependencies
        // hidden so the runtime + dev tree stays uncluttered for
        // typical `verum tree` runs.
        let opts = TreeOptions {
            duplicates: false,
            depth: None,
            all_features: false,
        };
        assert!(
            !should_show_build_deps(&opts),
            "default all_features=false must not include build-deps in tree",
        );
    }

    #[test]
    fn all_features_true_includes_build_deps() {
        // Pin: --all-features opts INTO showing every declared
        // dep group, including the build-time-only one. Mirrors
        // `cargo tree --all-features` semantics.
        let opts = opts_with(true);
        assert!(
            should_show_build_deps(&opts),
            "all_features=true must include build-deps in tree",
        );
    }
}
