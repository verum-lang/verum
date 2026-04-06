#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Tests for lockfile module
// Migrated from src/lockfile.rs per CLAUDE.md standards

use verum_cli::registry::lockfile::*;
use verum_cli::{List, Map, CogSource, Text};

#[test]
fn test_lockfile_creation() {
    let lockfile = Lockfile::new(Text::from("test-project"));
    assert_eq!(lockfile.root, "test-project");
    assert_eq!(lockfile.version, 1);
}

#[test]
fn test_add_remove_cog() {
    let mut lockfile = Lockfile::new(Text::from("test"));

    let package = LockedCog {
        name: Text::from("test-dep"),
        version: Text::from("1.0.0"),
        source: CogSource::Registry {
            registry: Text::from("https://packages.verum.lang"),
            version: Text::from("1.0.0"),
        },
        checksum: Text::from("abc123"),
        dependencies: Map::new(),
        features: List::new(),
        optional: false,
    };

    lockfile.add_cog(package);
    assert_eq!(lockfile.packages.len(), 1);

    assert!(lockfile.remove_cog("test-dep"));
    assert_eq!(lockfile.packages.len(), 0);
}
