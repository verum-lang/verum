//! Receiver tests use the same protocol declarations as real library code.
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenError, ItemFailurePolicy, VbcCodegen},
};

pub fn declarations() -> verum_ast::Module {
    let mut protocols = Parser::new(include_str!("../../../../core/base/protocols.vr"))
        .parse_module().expect("canonical protocol grammar");
    protocols.items.retain(|item| {
        matches!(&item.kind, verum_ast::ItemKind::Module(decl) if decl.items.is_none())
            || item.kind.type_or_protocol_name().is_some_and(|name|
                matches!(name.as_str(), "Deref" | "DerefMut"))
    });
    assert_eq!(protocols.items.len(), 3, "header and two actual protocol declarations");
    protocols
}

pub fn compile(config: CodegenConfig, source: &verum_ast::Module) -> Result<VbcModule, CodegenError> {
    let protocols = declarations();
    let mut maybe = Parser::new(include_str!("../../../../core/base/maybe.vr"))
        .parse_module().expect("canonical Maybe grammar");
    maybe.items.retain(|item| {
        matches!(&item.kind, verum_ast::ItemKind::Module(decl) if decl.items.is_none())
            || item.kind.type_or_protocol_name().is_some_and(|name| name.as_str() == "Maybe")
    });
    let units: List<_> = [&protocols, &maybe, source].into_iter().collect();
    let mut compiler = VbcCodegen::with_config(config);
    compiler.collect_unit_declarations(&units)?;
    compiler.resolve_pending_imports();
    if let Err(error) = compiler.compile_unit_items(&units, ItemFailurePolicy::Strict) {
        for (name, function) in compiler.export_functions() {
            if name.ends_with(".deref") || name.ends_with(".deref_mut") {
                eprintln!("selected declaration {name}: {:?}", function.callable_signature);
            }
        }
        return Err(error);
    }
    compiler.finalize_module_from_state()
}
