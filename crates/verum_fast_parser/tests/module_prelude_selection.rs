//! File attributes and retained module metadata share the prelude policy.
use verum_ast::prelude::{implicit_prelude_path, inject_implicit_prelude_mount};
use verum_fast_parser::Parser;

#[test]
fn source_header_can_disable_implicit_prelude_injection() {
    let mut module = Parser::new("@no_implicit_prelude module fixture.root; fn answer()->Int{42}")
        .parse_module()
        .unwrap();
    assert!(module.has_no_implicit_prelude());
    assert_eq!(implicit_prelude_path(&module), None);
    let original = module.clone();
    inject_implicit_prelude_mount(&mut module);
    assert_eq!(module, original);
}

#[test]
fn retained_module_attributes_keep_the_same_policy() {
    let mut module = Parser::new("@no_implicit_prelude module fixture.root;")
        .parse_module()
        .unwrap();
    let header = module.file_header_index().unwrap();
    module.attributes = std::mem::take(&mut module.items[header].attributes);
    assert!(module.has_no_implicit_prelude());
    assert_eq!(implicit_prelude_path(&module), None);
}

#[test]
fn a_child_attribute_does_not_disable_the_parent_prelude() {
    let mut module = Parser::new("module fixture.root; @no_implicit_prelude public module child;")
        .parse_module()
        .unwrap();
    assert!(!module.has_no_implicit_prelude());
    assert_eq!(implicit_prelude_path(&module), Some("core.prelude"));
    inject_implicit_prelude_mount(&mut module);
    let once = module.clone();
    inject_implicit_prelude_mount(&mut module);
    assert_eq!(module, once);
}
