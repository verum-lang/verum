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
    let manifest = Manifest::from_file(&manifest_dir.join("Verum.toml"))?;

    let lockfile_path = manifest_dir.join("Verum.lock");
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
