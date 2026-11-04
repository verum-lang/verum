#![cfg(feature = "codegen")]
//! Declaration transport preserves additive implementations without changing layout.
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    module::VbcModule,
    serialize::serialize_module,
};

fn source(method: &str) -> VbcModule {
    source_in("api", method)
}

fn source_in(owner: &str, method: &str) -> VbcModule {
    let ast = Parser::new(&format!(
        "module {owner}; public type Cell<N:meta Int> is {{ public value:() }}; \
         implement<N:meta Int> Cell<N> {{ public fn {method}(&self)->Int {{ N }} }}"
    ))
    .parse_module()
    .unwrap();
    VbcCodegen::with_config(CodegenConfig::new(owner))
        .collect_declaration_interface(&[&ast])
        .unwrap()
}

#[test]
fn same_spelling_does_not_merge_different_declaring_owners() {
    let first = source_in("api", "first");
    let second = source_in("other", "second");
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    consumer.import_archive_module_types(&first);
    consumer.import_archive_module_types(&second);
    let imported = consumer.finalize_module_from_state().unwrap();
    let owners: Vec<_> = imported
        .types
        .iter()
        .filter(|ty| !ty.source_inherent_implementations.is_empty())
        .collect();
    assert_eq!(owners.len(), 2);
    for owner in owners {
        assert_eq!(owner.source_inherent_implementations.len(), 1);
        let declaration_owner = imported.strings.get(owner.origin_module.unwrap()).unwrap();
        assert_eq!(
            owner.source_inherent_implementations[0].origin_module(),
            declaration_owner
        );
    }
}

#[test]
fn repeated_owner_imports_retain_both_implementation_headers() {
    let first = source("first");
    let second = source("second");
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    consumer.import_archive_module_types(&first);
    consumer.import_archive_module_types(&second);
    consumer.import_archive_module_types(&first);
    let imported = consumer.finalize_module_from_state().unwrap();
    let wire = deserialize_module(&serialize_module(&imported).unwrap()).unwrap();
    let declarations: Vec<_> = wire
        .types
        .iter()
        .flat_map(|ty| &ty.source_inherent_implementations)
        .collect();
    assert_eq!(
        declarations.len(),
        2,
        "equal layouts cannot erase a second implementation"
    );
    for module in [&first, &second] {
        let expected = module
            .types
            .iter()
            .flat_map(|ty| &ty.source_inherent_implementations)
            .next()
            .unwrap();
        assert!(declarations.contains(&expected));
    }
}

#[test]
fn writer_and_reader_agree_on_implementation_count_budget() {
    let mut module = source("count");
    let owner = module
        .types
        .iter_mut()
        .find(|ty| !ty.source_inherent_implementations.is_empty())
        .unwrap();
    let declaration = owner.source_inherent_implementations[0].clone();
    owner.source_inherent_implementations =
        std::iter::repeat_n(declaration.clone(), 4096).collect();
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    assert_eq!(
        wire.types
            .iter()
            .map(|ty| ty.source_inherent_implementations.len())
            .sum::<usize>(),
        4096
    );
    module
        .types
        .iter_mut()
        .find(|ty| !ty.source_inherent_implementations.is_empty())
        .unwrap()
        .source_inherent_implementations
        .push(declaration);
    let error = serialize_module(&module).expect_err("writer must respect reader's budget");
    assert!(format!("{error}").contains("too many inherent implementation declarations"));
}

#[test]
fn writer_refuses_a_protocol_implementation_in_an_inherent_slot() {
    let mut module = source("count");
    let ast = Parser::new("implement P for Cell<7> {}")
        .parse_module()
        .unwrap();
    let verum_ast::ItemKind::Impl(declaration) = &ast.items[0].kind else {
        panic!("implementation")
    };
    let declaration = verum_ast::source_implementation::AssociatedImplementationSource::new(
        "api".into(),
        declaration,
    )
    .unwrap();
    module
        .types
        .iter_mut()
        .find(|ty| !ty.source_inherent_implementations.is_empty())
        .unwrap()
        .source_inherent_implementations[0] = declaration;
    let error = serialize_module(&module)
        .expect_err("writer cannot produce a carrier rejected by its reader");
    assert!(format!("{error}").contains("inherent carrier contains a protocol implementation"));
}
