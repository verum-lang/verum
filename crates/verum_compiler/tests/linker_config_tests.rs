//! Integration tests for linker configuration from Verum.toml
//!
//! These tests verify that linker configuration is correctly parsed from
//! Verum.toml and applied to the compilation pipeline.

use verum_compiler::{LinkerTomlConfig, ProjectConfig};
use verum_compiler::phases::linking::{OutputKind, LTOConfig};
use std::path::PathBuf;

/// Test complete Verum.toml parsing with all sections
#[test]
fn test_complete_verum_toml() {
    let toml_str = r#"
[cog]
name = "my_app"
version = "1.0.0"
authors = ["Test Author"]
description = "A test application"

[linker]
output = "executable"
lto = "thin"
use_lld = true
pic = true
strip = false
debug_info = true
static_link = false
entry_point = "main"
libraries = ["pthread", "m", "dl"]
extra_flags = ["-Wl,--as-needed"]

[linker.linux]
libraries = ["rt"]
extra_flags = ["-Wl,--hash-style=gnu"]

[linker.macos]
extra_flags = ["-framework", "CoreFoundation"]

[profile.release.linker]
lto = "full"
strip = true
debug_info = false
"#;

    let config: ProjectConfig = toml::from_str(toml_str).unwrap();

    // Check package section
    assert_eq!(config.cog.name, "my_app");
    assert_eq!(config.cog.version, "1.0.0");
    assert_eq!(config.cog.authors, vec!["Test Author"]);

    // Check base linker section
    assert_eq!(config.linker.output, "executable");
    assert_eq!(config.linker.lto, "thin");
    assert!(config.linker.use_lld);
    assert!(config.linker.pic);
    assert!(!config.linker.strip);
    assert!(config.linker.debug_info);

    // Check platform-specific settings
    let linux = config.linker.linux.as_ref().unwrap();
    assert_eq!(linux.libraries, vec!["rt"]);

    let macos = config.linker.macos.as_ref().unwrap();
    assert_eq!(macos.extra_flags, vec!["-framework", "CoreFoundation"]);

    // Check profile override
    let release_config = config.linker_config_for_profile("release");
    assert_eq!(release_config.lto, "full");
    assert!(release_config.strip);
    assert!(!release_config.debug_info);
}

/// Test conversion to LinkingConfig
#[test]
fn test_linker_config_conversion() {
    let toml_str = r#"
[linker]
output = "shared"
lto = "full"
use_lld = true
pic = true
exports = ["api_init", "api_process", "api_cleanup"]
"#;

    let config: LinkerTomlConfig = toml::from_str(toml_str).unwrap();
    let linking_config = config
        .to_linking_config(PathBuf::from("libtest.so"))
        .unwrap();

    assert_eq!(linking_config.output_kind, OutputKind::SharedLibrary);
    assert_eq!(linking_config.lto, LTOConfig::Full);
    assert!(linking_config.use_llvm_linker);
    assert!(linking_config.pic);
    assert_eq!(linking_config.exported_symbols.len(), 3);
    assert!(linking_config.entry_point.is_none()); // No entry point for shared libs
}

/// Test profile-based configuration merging
#[test]
fn test_profile_merging() {
    let toml_str = r#"
[linker]
output = "executable"
lto = "none"
libraries = ["base_lib"]

[profile.dev.linker]
debug_info = true
libraries = ["debug_lib"]

[profile.release.linker]
lto = "full"
strip = true
libraries = ["release_lib"]
"#;

    let config: ProjectConfig = toml::from_str(toml_str).unwrap();

    // Dev profile should have debug settings
    let dev = config.linker_config_for_profile("dev");
    assert_eq!(dev.lto, "none"); // Inherited from base
    assert!(dev.debug_info);
    assert!(dev.libraries.contains(&"base_lib".to_string()));
    assert!(dev.libraries.contains(&"debug_lib".to_string()));

    // Release profile should have optimized settings
    let release = config.linker_config_for_profile("release");
    assert_eq!(release.lto, "full"); // Overridden
    assert!(release.strip);
    assert!(release.libraries.contains(&"base_lib".to_string()));
    assert!(release.libraries.contains(&"release_lib".to_string()));
}

/// Test LTO configuration options
#[test]
fn test_lto_options() {
    // Test all valid LTO modes
    let modes = [
        ("none", LTOConfig::None),
        ("off", LTOConfig::None),
        ("thin", LTOConfig::Thin),
        ("thinlto", LTOConfig::Thin),
        ("full", LTOConfig::Full),
        ("lto", LTOConfig::Full),
    ];

    for (input, expected) in modes {
        let toml_str = format!(r#"
[linker]
lto = "{}"
"#, input);

        let config: LinkerTomlConfig = toml::from_str(&toml_str).unwrap();
        let linking_config = config
            .to_linking_config(PathBuf::from("test"))
            .unwrap();

        assert_eq!(linking_config.lto, expected, "Failed for input: {}", input);
    }
}

/// Test output type options
#[test]
fn test_output_types() {
    let types = [
        ("executable", OutputKind::Executable),
        ("exe", OutputKind::Executable),
        ("bin", OutputKind::Executable),
        ("shared", OutputKind::SharedLibrary),
        ("dylib", OutputKind::SharedLibrary),
        ("so", OutputKind::SharedLibrary),
        ("static", OutputKind::StaticLibrary),
        ("lib", OutputKind::StaticLibrary),
        ("object", OutputKind::ObjectFile),
        ("obj", OutputKind::ObjectFile),
    ];

    for (input, expected) in types {
        let toml_str = format!(r#"
[linker]
output = "{}"
"#, input);

        let config: LinkerTomlConfig = toml::from_str(&toml_str).unwrap();
        let linking_config = config
            .to_linking_config(PathBuf::from("test"))
            .unwrap();

        assert_eq!(linking_config.output_kind, expected, "Failed for input: {}", input);
    }
}

/// Test static library configuration
#[test]
fn test_static_library() {
    let toml_str = r#"
[linker]
output = "static"
lto = "none"
"#;

    let config: LinkerTomlConfig = toml::from_str(&toml_str).unwrap();
    let linking_config = config
        .to_linking_config(PathBuf::from("libtest.a"))
        .unwrap();

    assert_eq!(linking_config.output_kind, OutputKind::StaticLibrary);
    assert!(linking_config.entry_point.is_none());
}

/// Test extra linker flags
#[test]
fn test_extra_flags() {
    let toml_str = r#"
[linker]
extra_flags = ["-Wl,--gc-sections", "-Wl,-dead_strip", "-fuse-ld=lld"]
"#;

    let config: LinkerTomlConfig = toml::from_str(&toml_str).unwrap();
    let linking_config = config
        .to_linking_config(PathBuf::from("test"))
        .unwrap();

    assert_eq!(linking_config.extra_flags.len(), 3);
}

/// Test target triple configuration
#[test]
fn test_target_triple() {
    let toml_str = r#"
[linker]
target = "x86_64-unknown-linux-gnu"
"#;

    let config: LinkerTomlConfig = toml::from_str(&toml_str).unwrap();
    let linking_config = config
        .to_linking_config(PathBuf::from("test"))
        .unwrap();

    assert!(linking_config.target_triple.is_some());
    assert_eq!(
        linking_config.target_triple.as_ref().unwrap().as_str(),
        "x86_64-unknown-linux-gnu"
    );
}

/// Test native target (should be None)
#[test]
fn test_native_target() {
    let toml_str = r#"
[linker]
target = "native"
"#;

    let config: LinkerTomlConfig = toml::from_str(&toml_str).unwrap();
    let linking_config = config
        .to_linking_config(PathBuf::from("test"))
        .unwrap();

    assert!(linking_config.target_triple.is_none());
}

/// Test default configuration fallback
#[test]
fn test_default_config() {
    let config = LinkerTomlConfig::default();
    let linking_config = config
        .to_linking_config(PathBuf::from("a.out"))
        .unwrap();

    assert_eq!(linking_config.output_kind, OutputKind::Executable);
    assert_eq!(linking_config.lto, LTOConfig::Thin);
    assert!(linking_config.pic);
    assert!(!linking_config.strip);
    assert!(linking_config.debug_info);
    assert!(linking_config.entry_point.is_some());
}

/// Test unknown profile falls back to base config
#[test]
fn test_unknown_profile_fallback() {
    let toml_str = r#"
[linker]
output = "executable"
lto = "none"
libraries = ["base_lib"]
"#;

    let config: ProjectConfig = toml::from_str(&toml_str).unwrap();

    // Unknown profile should fall back to base linker settings
    let unknown = config.linker_config_for_profile("unknown");
    assert_eq!(unknown.lto, "none");
    assert!(unknown.libraries.contains(&"base_lib".to_string()));

    // Debug and release without specific config should also use base
    let dev = config.linker_config_for_profile("dev");
    assert_eq!(dev.lto, "none");

    let release = config.linker_config_for_profile("release");
    assert_eq!(release.lto, "none");
}
