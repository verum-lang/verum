use verum_common::{FileId, Maybe};
use verum_fast_parser::FastParser;
use verum_kernel::arch::Lifecycle;
use verum_kernel::arch_parse::{ArchParseError, declared_arch_shape_of_module};

fn parse(source: &str) -> verum_ast::Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(0))
        .expect("valid source")
}

#[test]
fn only_the_module_boundary_supplies_file_metadata() {
    for source in [
        "// @arch_module(lifecycle = Lifecycle.Definition)\nmodule core.example;",
        "module core.example; const label: Text = \"@arch_module\";",
        "module core.example; @arch_module(lifecycle = Lifecycle.Definition) fn helper() {}",
        "module core.example; @arch_module(lifecycle = Lifecycle.Definition) module child {}",
    ] {
        assert!(
            declared_arch_shape_of_module(&parse(source))
                .unwrap()
                .is_none(),
            "{source}"
        );
    }
}

#[test]
fn definition_declaration_does_not_invent_capabilities_or_proof_witnesses() {
    let shape = declared_arch_shape_of_module(&parse(
        "@arch_module(lifecycle = Lifecycle.Definition) module core.example;",
    ))
    .unwrap()
    .unwrap();
    assert!(matches!(shape.lifecycle, Lifecycle::Definition));
    assert_eq!(shape.required_capability_constraint(), Maybe::None);
    assert_eq!(shape.exposed_capability_constraint(), Maybe::None);
    assert!(shape.preserves.is_empty());
    assert!(shape.cve_closure.constructive.is_none());
    assert!(shape.cve_closure.verifiable_strategy.is_none());
    assert!(shape.cve_closure.executable.is_none());
}

#[test]
fn invalid_or_ambiguous_module_metadata_is_rejected() {
    for source in [
        "@arch_module(lifecycle = 42) module core.example;",
        "@arch_module(unknown_field = true) module core.example;",
        "@arch_module(lifecycle = Lifecycle.Definition, lifecycle = Lifecycle.Definition) module core.example;",
        "@arch_module(lifecycle = Lifecycle.Definition) @arch_module(lifecycle = Lifecycle.Definition) module core.example;",
    ] {
        assert!(
            declared_arch_shape_of_module(&parse(source)).is_err(),
            "{source}"
        );
    }
}

#[test]
fn module_and_header_attributes_cannot_silently_override_each_other() {
    let mut module = parse("@arch_module(lifecycle = Lifecycle.Definition) module core.example;");
    let index = module.file_header_index().unwrap();
    module.attributes = module.items[index].attributes.clone();
    assert!(matches!(
        declared_arch_shape_of_module(&module),
        Err(ArchParseError::DuplicateDeclaration { .. })
    ));
}
