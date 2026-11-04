use verum_ast::cfg::{CfgEvaluator, SourceModuleSelection};
use verum_fast_parser::Parser;
fn selected(source: &str, triple: &str) -> Result<SourceModuleSelection, verum_ast::Span> {
    let ast = Parser::new(source).parse_module().unwrap();
    CfgEvaluator::for_target(triple).source_module_selection(&ast, "fixture.root")
}
#[test]
fn header_cfg_is_file_selection_and_child_cfg_is_not() {
    let source = "@cfg(target_os = \"windows\") module fixture.root; @cfg(target_os = \"linux\") public module socket;";
    let windows = selected(source, "x86_64-pc-windows-msvc").unwrap();
    assert!(windows.include);
    assert!(!windows.includes_child("socket"));
    let linux = selected(source, "x86_64-unknown-linux-gnu").unwrap();
    assert!(!linux.include);
    assert!(linux.includes_child("socket"));
}
#[test]
fn item_cfg_never_conditions_the_file() {
    for item in ["mount missing.Packet;", "fn value()->Int{1}"] {
        let source = format!("module fixture.root; @cfg(target_os = \"windows\") {item}");
        assert!(selected(&source, "aarch64-apple-darwin").unwrap().include);
    }
}
#[test]
fn inactive_header_cannot_hide_malformed_header_or_child_predicates() {
    for source in [
        "@cfg(target_os = \"windows\") @cfg(42) module fixture.root;",
        "@cfg(target_os = \"windows\") module fixture.root; @cfg(42) public module child;",
    ] {
        assert!(selected(source, "aarch64-apple-darwin").is_err());
    }
}
#[test]
fn repeated_child_alternatives_and_inherited_restrictions_keep_their_meaning() {
    let selected=selected("module fixture.root; @cfg(target_os = \"windows\") public module io; @cfg(target_os = \"linux\") public module io; @cfg(target_os = \"windows\") public module io.private;","x86_64-unknown-linux-gnu").unwrap();
    assert!(selected.includes_child("io"));
    assert!(!selected.includes_child("io.private.deep"));
    let child = selected.for_child("io");
    assert!(child.include);
    assert!(!child.includes_child("private"));
    let mut lower = SourceModuleSelection::unrestricted();
    lower.intersect(child);
    assert!(!lower.includes_child("private.deep"));
}
#[test]
fn actual_winsock_header_selects_windows_without_losing_declarations() {
    let core = std::env::var_os("VERUM_TEST_CFG_CORE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../core"));
    let ast = Parser::new(&std::fs::read_to_string(core.join("sys/windows/winsock2.vr")).unwrap())
        .parse_module()
        .unwrap();
    let owner = "core.sys.windows.winsock2";
    assert!(
        CfgEvaluator::for_target("x86_64-pc-windows-msvc")
            .source_module_selection(&ast, owner)
            .unwrap()
            .include
    );
    for target in ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"] {
        assert!(
            !CfgEvaluator::for_target(target)
                .source_module_selection(&ast, owner)
                .unwrap()
                .include
        );
    }
    let count = ast
        .items
        .iter()
        .filter(|item| matches!(item.kind, verum_ast::ItemKind::Function(_)))
        .count();
    assert!(
        count > 30,
        "the complete actual Windows declarations remain present"
    );
    // The OSError mount follows the file selection; it is not independently disabled.
    for item in &ast.items {
        if matches!(item.kind, verum_ast::ItemKind::Mount(_)) {
            assert!(CfgEvaluator::for_target("aarch64-apple-darwin").should_include_item(item));
        }
    }
}
#[test]
fn malformed_module_metadata_is_refused_after_an_inactive_condition() {
    let mut ast = Parser::new("@cfg(target_os = \"windows\") @cfg(42) module fixture.root;")
        .parse_module()
        .unwrap();
    let header = ast.file_header_index().unwrap();
    ast.attributes = ast.items[header].attributes.clone();
    ast.items[header].attributes.clear();
    assert!(
        CfgEvaluator::for_target("aarch64-apple-darwin")
            .source_module_selection(&ast, "fixture.root")
            .is_err()
    );
}

#[test]
fn inactive_parent_cannot_reenable_an_active_child() {
    let parent = selected(
        "@cfg(target_os = \"windows\") module fixture.root; public module available;",
        "aarch64-apple-darwin",
    )
    .unwrap();
    assert!(!parent.include);
    assert!(parent.includes_child("available"));
    let child = parent.for_child("available");
    assert!(!child.include);
    assert!(!child.for_child("nested").include);
}
