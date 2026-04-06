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
// Tests for sat_resolver module
// Migrated from src/sat_resolver.rs per CLAUDE.md standards

use verum_cli::registry::sat_resolver::*;
use verum_cli::{DependencySpec, List, Map, CogMetadata, Text, TierArtifacts};

#[test]
fn test_sat_resolver_creation() {
    let resolver = SatResolver::new();
    assert_eq!(resolver.clauses.len(), 0);
}

#[test]
fn test_simple_dependency() {
    let mut resolver = SatResolver::new();

    // Add packages
    let mut pkg_a = CogMetadata {
        name: Text::from("pkg_a"),
        version: Text::from("1.0.0"),
        description: None,
        authors: List::new(),
        license: None,
        repository: None,
        homepage: None,
        keywords: List::new(),
        categories: List::new(),
        readme: None,
        dependencies: Map::new(),
        features: Map::new(),
        artifacts: TierArtifacts::default(),
        proofs: None,
        cbgr_profiles: None,
        signature: None,
        ipfs_hash: None,
        checksum: Text::from("abc"),
        published_at: 0,
    };

    let pkg_b = CogMetadata {
        name: Text::from("pkg_b"),
        version: Text::from("1.0.0"),
        description: None,
        authors: List::new(),
        license: None,
        repository: None,
        homepage: None,
        keywords: List::new(),
        categories: List::new(),
        readme: None,
        dependencies: Map::new(),
        features: Map::new(),
        artifacts: TierArtifacts::default(),
        proofs: None,
        cbgr_profiles: None,
        signature: None,
        ipfs_hash: None,
        checksum: Text::from("def"),
        published_at: 0,
    };

    // pkg_a depends on pkg_b
    pkg_a.dependencies.insert(
        Text::from("pkg_b"),
        DependencySpec::Simple(Text::from("1.0.0")),
    );

    resolver.add_metadata(pkg_a);
    resolver.add_metadata(pkg_b);

    // Add constraints
    let var_a = CogVar::new("pkg_a", Version::parse("1.0.0").unwrap());
    let version_req = VersionReq::parse("1.0.0").unwrap();

    resolver.add_dependency_constraint(&var_a, "pkg_b", &version_req);
    resolver.add_root_constraint(&var_a);

    // Solve
    let result = resolver.solve().unwrap();
    assert_eq!(result.selected.len(), 2);
    assert!(result.conflicts.is_empty());
}
