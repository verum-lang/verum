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
// Tests for enterprise module
// Migrated from src/enterprise.rs per CLAUDE.md standards

use verum_cli::Text;
use verum_cli::registry::enterprise::*;

#[test]
fn test_default_config() {
    let config = EnterpriseConfig::default();
    assert!(!config.offline);
    assert!(config.proxy.is_none());
}

#[test]
fn test_cog_access_control() {
    let mut config = EnterpriseConfig::default();
    config
        .access_control
        .deny_list
        .push(Text::from("bad-package"));

    let client = EnterpriseClient::new(config).unwrap();

    assert!(!client.is_cog_allowed("bad-package"));
    assert!(client.is_cog_allowed("good-package"));
}

#[test]
fn test_mirror_selection() {
    let mut config = EnterpriseConfig::default();
    config.mirrors.push(MirrorConfig {
        name: Text::from("Corporate Mirror"),
        url: Text::from("https://mirror.corp.com"),
        priority: 1,
        packages: None,
    });

    let client = EnterpriseClient::new(config).unwrap();
    let mirror = client.get_mirror_url("some-package");

    assert_eq!(mirror, Some(Text::from("https://mirror.corp.com")));
}

#[test]
fn test_sbom_generator() {
    let generator = SbomGenerator::new(SbomFormat::Spdx);
    assert!(matches!(generator.format, SbomFormat::Spdx));
}
