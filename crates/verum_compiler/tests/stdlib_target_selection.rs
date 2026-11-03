//! Backing sources follow module declarations, not platform-like path names.
use std::{collections::BTreeSet, path::Path};
use verum_ast::cfg::TargetConfig;
use verum_compiler::core_compiler::StdlibModuleResolver;

fn write(root: &Path, file: &str, source: &str) {
    let path = root.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, source).unwrap();
}

fn selected(root: &Path, triple: &str) -> BTreeSet<String> {
    let mut resolver = StdlibModuleResolver::with_target(root, TargetConfig::for_target(triple));
    resolver.discover().unwrap();
    resolver
        .modules_in_order()
        .iter()
        .flat_map(|module| &module.source_files)
        .map(|path| {
            path.strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

#[test]
fn parent_declarations_select_both_files_and_complete_subtrees() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "mod.vr",
        "module core;
        @cfg(target_os = \"windows\") public module network;
        @cfg(target_os = \"linux\") public module storage;
        @cfg(target_os = \"macos\") public module standalone;",
    );
    write(
        root.path(),
        "network/mod.vr",
        "module core.network; public module worker;",
    );
    write(
        root.path(),
        "network/worker.vr",
        "module core.network.worker;",
    );
    write(
        root.path(),
        "network/deep/mod.vr",
        "module core.network.deep;",
    );
    write(root.path(), "storage.vr", "module core.storage;");
    write(root.path(), "standalone.vr", "module core.standalone;");
    assert_eq!(
        selected(root.path(), "x86_64-pc-windows-msvc"),
        BTreeSet::from([
            "mod.vr".into(),
            "network/mod.vr".into(),
            "network/worker.vr".into(),
            "network/deep/mod.vr".into(),
        ])
    );
    assert_eq!(
        selected(root.path(), "x86_64-unknown-linux-gnu"),
        BTreeSet::from(["mod.vr".into(), "storage.vr".into()])
    );
    assert_eq!(
        selected(root.path(), "aarch64-apple-darwin"),
        BTreeSet::from(["mod.vr".into(), "standalone.vr".into()])
    );
}

#[test]
fn excluded_parent_does_not_parse_unavailable_child_sources() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "mod.vr",
        "module core; @cfg(target_os = \"windows\") public module backend;",
    );
    write(
        root.path(),
        "backend/mod.vr",
        "this is deliberately invalid source",
    );
    write(
        root.path(),
        "backend.vr",
        "this is deliberately invalid source",
    );
    assert_eq!(
        selected(root.path(), "aarch64-apple-darwin"),
        BTreeSet::from(["mod.vr".into()])
    );
    let mut enabled = StdlibModuleResolver::with_target(
        root.path(),
        TargetConfig::for_target("x86_64-pc-windows-msvc"),
    );
    assert!(
        enabled
            .discover()
            .unwrap_err()
            .to_string()
            .contains("cannot parse module selection")
    );
}

#[test]
fn module_header_cfg_survives_multiline_attributes_and_nested_predicates() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "mod.vr", "module core; public module engine;");
    write(
        root.path(),
        "engine/mod.vr",
        "@arch_module(
            foundation: Foundation.ZfcTwoInacc,
            stratum: MsfsStratum.LFnd,
            lifecycle: Lifecycle.Definition
        )
        @cfg(all(any(target_os = \"linux\", target_os = \"macos\"), not(target_arch = \"x86_64\")))
        module core.engine;",
    );
    write(
        root.path(),
        "engine/worker.vr",
        "module core.engine.worker;",
    );
    assert!(selected(root.path(), "aarch64-apple-darwin").contains("engine/worker.vr"));
    assert_eq!(
        selected(root.path(), "x86_64-unknown-linux-gnu"),
        BTreeSet::from(["mod.vr".into()])
    );
}

#[test]
fn file_header_cfg_and_item_cfg_have_different_ownership() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "mod.vr",
        "module core; public module owned; public module local;",
    );
    write(
        root.path(),
        "owned.vr",
        "@cfg(target_os = \"windows\") module core.owned; fn work() {}",
    );
    write(
        root.path(),
        "local.vr",
        "module core.local; @cfg(target_os = \"windows\") mount absent.Item; fn work() {}",
    );
    assert_eq!(
        selected(root.path(), "aarch64-apple-darwin"),
        BTreeSet::from(["mod.vr".into(), "local.vr".into()])
    );
}

#[test]
fn child_cfg_does_not_disable_its_explicit_parent_header() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "mod.vr",
        "module core; @cfg(target_os = \"windows\") public module backend; fn available() {}",
    );
    write(root.path(), "backend.vr", "module core.backend;");
    assert_eq!(
        selected(root.path(), "aarch64-apple-darwin"),
        BTreeSet::from(["mod.vr".into()])
    );
}

#[test]
fn active_alternative_keeps_a_shared_backing_source() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "mod.vr",
        "module core;
        @cfg(target_os = \"windows\") public module selected;
        @cfg(target_os = \"macos\") public module selected;",
    );
    write(root.path(), "selected.vr", "module core.selected;");
    for target in ["aarch64-apple-darwin", "x86_64-pc-windows-msvc"] {
        assert!(selected(root.path(), target).contains("selected.vr"));
    }
    assert_eq!(
        selected(root.path(), "x86_64-unknown-linux-gnu"),
        BTreeSet::from(["mod.vr".into()])
    );
}

#[test]
fn source_names_do_not_imply_platform_restrictions() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "mod.vr", "module core; public module sys;");
    write(
        root.path(),
        "sys/mod.vr",
        "module core.sys; public module windows;",
    );
    write(
        root.path(),
        "sys/windows/mod.vr",
        "module core.sys.windows;",
    );
    assert!(selected(root.path(), "aarch64-apple-darwin").contains("sys/windows/mod.vr"));
}

#[test]
fn malformed_module_cfg_is_an_error_even_after_a_false_condition() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "mod.vr",
        "module core; @cfg(target_os = \"windows\") @cfg(42) public module backend;",
    );
    let mut resolver = StdlibModuleResolver::with_target(
        root.path(),
        TargetConfig::for_target("aarch64-apple-darwin"),
    );
    assert!(
        resolver
            .discover()
            .unwrap_err()
            .to_string()
            .contains("invalid @cfg")
    );
}

#[test]
fn repeated_discovery_drops_previously_selected_sources() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "mod.vr", "module core; public module backend;");
    write(root.path(), "backend.vr", "module core.backend;");
    let mut resolver = StdlibModuleResolver::with_target(
        root.path(),
        TargetConfig::for_target("aarch64-apple-darwin"),
    );
    resolver.discover().unwrap();
    assert!(
        resolver
            .get_module("core")
            .unwrap()
            .source_files
            .iter()
            .any(|path| path.ends_with("backend.vr"))
    );
    write(
        root.path(),
        "mod.vr",
        "module core; @cfg(target_os = \"windows\") public module backend;",
    );
    resolver.discover().unwrap();
    assert!(
        !resolver
            .get_module("core")
            .unwrap()
            .source_files
            .iter()
            .any(|path| path.ends_with("backend.vr"))
    );
}

#[test]
fn failed_discovery_cannot_publish_old_or_partial_selection() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "mod.vr", "module core; public module backend;");
    write(root.path(), "backend.vr", "module core.backend;");
    let mut resolver = StdlibModuleResolver::with_target(
        root.path(),
        TargetConfig::for_target("aarch64-apple-darwin"),
    );
    resolver.discover().unwrap();
    assert!(!resolver.modules_in_order().is_empty());
    write(root.path(), "backend.vr", "@cfg() module core.backend;");
    assert!(resolver.discover().is_err());
    assert!(resolver.modules_in_order().is_empty());
    assert_eq!(resolver.module_count(), 0);
}

#[test]
fn qualified_child_gate_reaches_only_its_backing_subtree() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "mod.vr",
        "module core; public module branch; @cfg(target_os = \"windows\") public module core.branch.inner;",
    );
    write(
        root.path(),
        "branch/mod.vr",
        "module core.branch; public module inner; public module sibling;",
    );
    write(
        root.path(),
        "branch/inner/mod.vr",
        "module core.branch.inner;",
    );
    write(
        root.path(),
        "branch/inner/worker.vr",
        "module core.branch.inner.worker;",
    );
    write(
        root.path(),
        "branch/sibling.vr",
        "module core.branch.sibling;",
    );
    let macos = selected(root.path(), "aarch64-apple-darwin");
    assert!(macos.contains("branch/mod.vr"));
    assert!(macos.contains("branch/sibling.vr"));
    assert!(!macos.contains("branch/inner/mod.vr"));
    assert!(!macos.contains("branch/inner/worker.vr"));
    assert!(selected(root.path(), "x86_64-pc-windows-msvc").contains("branch/inner/worker.vr"));
}

#[test]
fn a_public_child_with_the_root_name_is_not_the_root_header() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "mod.vr",
        "module core; @cfg(target_os = \"windows\") public module core; fn available() {}",
    );
    write(root.path(), "core.vr", "module core.core;");
    assert_eq!(
        selected(root.path(), "aarch64-apple-darwin"),
        BTreeSet::from(["mod.vr".into()])
    );
}

#[test]
fn logical_file_header_selects_the_file_independent_of_its_backing_name() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "mod.vr", "module core;");
    let source = "module nested { type Local is { value: Int }; }
        @cfg(target_os = \"windows\") module logical_name; fn work() {}";
    let ast = verum_fast_parser::Parser::new(source)
        .parse_module()
        .unwrap();
    assert_eq!(ast.file_header_index(), Some(1));
    assert_eq!(
        verum_vbc::codegen::VbcCodegen::resolve_full_module_path(&ast, "core").as_deref(),
        Some("core.logical_name")
    );
    write(root.path(), "physical_name.vr", source);
    assert_eq!(
        selected(root.path(), "aarch64-apple-darwin"),
        BTreeSet::from(["mod.vr".into()])
    );
    assert!(selected(root.path(), "x86_64-pc-windows-msvc").contains("physical_name.vr"));
}

#[test]
fn later_same_named_child_cannot_reclassify_the_file_header() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "mod.vr", "module core; public module parent;");
    let source = "module parent; @cfg(target_os = \"windows\") module parent; fn work() {}";
    let ast = verum_fast_parser::Parser::new(source)
        .parse_module()
        .unwrap();
    assert_eq!(ast.file_header_index(), Some(0));
    assert_eq!(
        verum_vbc::codegen::VbcCodegen::resolve_full_module_path(&ast, "core.parent").as_deref(),
        Some("core.parent")
    );
    write(root.path(), "parent/mod.vr", source);
    write(
        root.path(),
        "parent/parent.vr",
        "module core.parent.parent;",
    );
    assert_eq!(
        selected(root.path(), "aarch64-apple-darwin"),
        BTreeSet::from(["mod.vr".into(), "parent/mod.vr".into()])
    );
    assert!(selected(root.path(), "x86_64-pc-windows-msvc").contains("parent/parent.vr"));
}

#[test]
fn first_public_bodiless_module_is_a_file_header_in_both_paths() {
    let root = tempfile::tempdir().unwrap();
    let source = "@cfg(target_os = \"windows\") public module backend; fn work() {}";
    let ast = verum_fast_parser::Parser::new(source)
        .parse_module()
        .unwrap();
    assert_eq!(ast.file_header_index(), Some(0));
    assert_eq!(
        verum_vbc::codegen::VbcCodegen::resolve_full_module_path(&ast, "core").as_deref(),
        Some("core.backend")
    );
    write(root.path(), "mod.vr", source);
    assert!(selected(root.path(), "aarch64-apple-darwin").is_empty());
    assert_eq!(
        selected(root.path(), "x86_64-pc-windows-msvc"),
        BTreeSet::from(["mod.vr".into()])
    );
}
