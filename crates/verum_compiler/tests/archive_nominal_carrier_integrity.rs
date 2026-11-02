//! Archive publication cannot combine a stale semantic declaration with a new ABI.
use verum_common::List;
use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::Parser;
use verum_vbc::{
    archive::{ArchiveBuilder, read_archive, write_archive},
    codegen::{CodegenConfig, VbcCodegen},
    module::VbcModule,
};

fn declaration(level: u32) -> VbcModule {
    let source = format!(
        "module core.carrier; public type Carrier<A> is {{ public value: Type({level}), public extra: A }};"
    );
    let ast = Parser::new(&source).parse_module().unwrap();
    let mut module = VbcCodegen::with_config(CodegenConfig::new("core.carrier"))
        .compile_module(&ast)
        .unwrap();
    let owner = module.intern_string("core.carrier");
    for descriptor in &mut module.types {
        if descriptor.source_declaration.is_some() {
            descriptor.origin_module = Some(owner);
        }
    }
    module
}

fn archive_metadata(first: &VbcModule, second: &VbcModule) -> Result<(), String> {
    let mut builder = ArchiveBuilder::new();
    builder.add_module("core.a_dependency", first, &[]).unwrap();
    builder.add_module("core.z_owner", second, &[]).unwrap();
    let mut bytes = List::new();
    write_archive(&builder.finish(), &mut bytes).unwrap();
    let archive = read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
    archive_to_core_metadata(&archive)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[test]
fn same_owner_conflicts_are_rejected_in_both_archive_orders() {
    let first = declaration(1);
    let second = declaration(2);
    for (left, right) in [(&first, &second), (&second, &first)] {
        let error = archive_metadata(left, right).unwrap_err();
        assert!(
            error.contains("conflicting semantic type declarations for core.carrier.Carrier"),
            "{error}"
        );
    }
}

#[test]
fn exact_same_owner_replay_is_idempotent() {
    let module = declaration(1);
    archive_metadata(&module, &module).unwrap();
}

#[test]
fn abi_only_claim_cannot_erase_a_semantic_declaration_in_either_order() {
    let semantic = declaration(1);
    let mut abi_only = semantic.clone();
    for descriptor in &mut abi_only.types {
        descriptor.source_declaration = None;
    }
    for (left, right) in [(&semantic, &abi_only), (&abi_only, &semantic)] {
        let error = archive_metadata(left, right).unwrap_err();
        assert!(
            error.contains("semantic and ABI-only declarations disagree for core.carrier.Carrier"),
            "{error}"
        );
    }
}

#[test]
fn archive_copies_cannot_supply_missing_or_different_lexical_scopes() {
    let first = declaration(1);
    assert!(first.declaration_type_bindings.get(&"core.carrier".into()).is_some());
    let mut missing = first.clone();
    missing.declaration_type_bindings.clear();
    let mut changed = first.clone();
    changed.declaration_type_bindings.get_mut(&"core.carrier".into()).unwrap()
        .insert("Alias".into(), Some("foreign.Token".into()));
    for second in [missing, changed] {
        for (left, right) in [(&first, &second), (&second, &first)] {
            let error = archive_metadata(left, right).unwrap_err();
            assert!(error.contains("conflicting declaration type bindings for core.carrier"), "{error}");
        }
    }
}

#[test]
fn nested_nominal_names_use_the_scope_entered_by_the_consumer() {
    let mut first = declaration(1);
    first.types.iter_mut().find(|ty| ty.source_declaration.is_some()).unwrap().name =
        first.strings.intern("nested.Carrier");
    let bindings = first.declaration_type_bindings.remove(&"core.carrier".into()).unwrap();
    first.declaration_type_bindings.insert("core.carrier.nested".into(), bindings);
    let mut second = first.clone();
    second.declaration_type_bindings.clear();
    let error = archive_metadata(&first, &second).unwrap_err();
    assert!(error.contains("conflicting declaration type bindings for core.carrier.nested"), "{error}");
}
