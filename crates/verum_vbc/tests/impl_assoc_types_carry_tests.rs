//! Pillar-3 increment 1 (ARRAY-ITER-CONCRETIZE-1) — the VBC carry of
//! impl-block associated-type bindings (`type Item = &T;`).
//!
//! Pins the writer chain: AST impl item → `collect_declarations`'
//! protocol-impl attach (`ProtocolImpl.associated_types`) → the
//! minor-4 wire format round-trip.  Pre-carry the bindings were
//! dropped at the VBC boundary, so the archive-driven typechecker
//! could never resolve `::Item<ListIter<T>>` projections and every
//! iterator-closure field access failed E103.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

/// The load-bearing case (self-contained mirror of stdlib's
/// `implement<T> Iterator for ListIter<T> { type Item = &T; ... }`):
/// the binding must land on the attached `ProtocolImpl`, resolved
/// against the parent's TypeParamId space.
#[test]
fn protocol_impl_carries_assoc_type_binding() {
    let source = r#"
module probe.assoc;

public type Holder<T> is { v: T };

public type Pick is protocol {
    type Item;
    fn pick(&self) -> Int;
};

implement<T> Pick for Holder<T> {
    type Item = &T;

    public fn pick(&self) -> Int {
        7
    }
}
"#;
    let mut parser = Parser::new(source);
    let module_ast = parser
        .parse_module()
        .unwrap_or_else(|e| panic!("parse failed: {:?}", e));
    // Diagnostic: confirm the parser routed `type Item = &T;` into
    // ImplItemKind::Type within the impl's items.
    for it in module_ast.items.iter() {
        if let verum_ast::ItemKind::Impl(id) = &it.kind {
            let kinds: Vec<&str> = id
                .items
                .iter()
                .map(|i| match &i.kind {
                    verum_ast::decl::ImplItemKind::Function(_) => "Function",
                    verum_ast::decl::ImplItemKind::Type { .. } => "Type",
                    _ => "Other",
                })
                .collect();
            eprintln!("[pin-diag] impl items: {:?}", kinds);
        }
    }
    let config = CodegenConfig::new("probe_assoc.vr").with_validation();
    let mut codegen = VbcCodegen::with_config(config);
    codegen
        .compile_module_with_mounts(&module_ast, "probe_assoc.vr", ".")
        .unwrap_or_else(|e| panic!("compile failed: {:?}", e));
    let module = codegen
        .finalize_module()
        .unwrap_or_else(|e| panic!("finalize failed: {:?}", e));

    // Diagnostic: enumerate EVERY Holder-named descriptor (id slots,
    // protocol counts, assoc counts, structural richness).
    for t in module.types.iter() {
        if module.strings.get(t.name).map(|n| n == "Holder").unwrap_or(false) {
            eprintln!(
                "[pin-diag] Holder id={} fields={} variants={} protocols={} assoc_per_proto={:?}",
                t.id.0,
                t.fields.len(),
                t.variants.len(),
                t.protocols.len(),
                t.protocols
                    .iter()
                    .map(|p| p.associated_types.len())
                    .collect::<Vec<_>>()
            );
        }
    }

    let holder = module
        .types
        .iter()
        .find(|t| module.strings.get(t.name).map(|n| n == "Holder").unwrap_or(false))
        .expect("Holder TypeDescriptor present");

    assert!(
        !holder.protocols.is_empty(),
        "Holder should carry the Pick protocol impl"
    );

    let carried: Vec<(String, String)> = holder
        .protocols
        .iter()
        .flat_map(|pi| pi.associated_types.iter())
        .filter_map(|(name_sid, tref)| {
            module
                .strings
                .get(*name_sid)
                .map(|n| (n.to_string(), format!("{:?}", tref)))
        })
        .collect();

    assert!(
        carried.iter().any(|(n, _)| n == "Item"),
        "Holder's Pick impl must carry the `type Item = &T;` binding; \
         carried bindings: {:?}; protocols: {}",
        carried,
        holder.protocols.len()
    );
}

/// Wire-format round-trip: minor-4 archives must preserve the carried
/// bindings through serialize → deserialize.
#[test]
fn assoc_bindings_roundtrip_through_wire_format() {
    use verum_vbc::module::VbcModule;
    use verum_vbc::types::{
        ProtocolId, ProtocolImpl, TypeDescriptor, TypeId, TypeRef,
    };

    let mut m = VbcModule::new("assoc_rt".to_string());
    let name_sid = m.intern_string("Item");
    let type_name = m.intern_string("Probe");
    let mut td = TypeDescriptor::default();
    td.id = TypeId(4242);
    td.name = type_name;
    td.protocols.push(ProtocolImpl {
        source_implementation: None,
        protocol: ProtocolId(7),
        methods: vec![1, 2],
        associated_types: vec![(name_sid, TypeRef::Generic(verum_vbc::types::TypeParamId(0)))],
        protocol_args_text: Vec::new(),
            type_param_fn_bounds: Vec::new(),
    });
    m.types.push(td);

    let bytes = verum_vbc::serialize::serialize_module(&m).expect("serialize");
    let back = verum_vbc::deserialize::deserialize_module(&bytes).expect("deserialize");
    let td_back = back
        .types
        .iter()
        .find(|t| t.id == TypeId(4242))
        .expect("descriptor round-trips");
    assert_eq!(td_back.protocols.len(), 1);
    let pi = &td_back.protocols[0];
    assert_eq!(pi.methods, vec![1, 2]);
    assert_eq!(pi.associated_types.len(), 1, "binding must survive the wire");
    assert!(matches!(
        pi.associated_types[0].1,
        TypeRef::Generic(verum_vbc::types::TypeParamId(0))
    ));
}

fn compile_function_bound_impl() -> verum_vbc::module::VbcModule {
    let source = r#"
module probe.bounds;

public type Select<I, F> is { source: I, predicate: F };
public type Predicate is protocol {};
implement<I, F: fn(&I.Item) -> Bool> Predicate for Select<I, F> {}
"#;
    let ast = Parser::new(source).parse_module().expect("parse");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("bounds.vr").with_validation());
    codegen
        .compile_module_with_mounts(&ast, "bounds.vr", ".")
        .expect("compile");
    codegen.finalize_module().expect("finalize")
}

fn assert_function_bound(module: &verum_vbc::module::VbcModule) {
    let ty = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Select"))
        .unwrap();
    let bounds = &ty.protocols[0].type_param_fn_bounds;
    assert_eq!(bounds.len(), 1);
    assert_eq!(module.get_string(bounds[0].0), Some("F"));
    assert_eq!(
        module.get_string(bounds[0].1),
        Some("fn(&::Item<I>) -> Bool")
    );
}

#[test]
fn implementation_function_bound_survives_finalize_wire_and_link() {
    use verum_vbc::{
        deserialize::deserialize_module, linker::VbcLinker, module::VbcModule,
        serialize::serialize_module,
    };

    assert!(Parser::new("fn(&::Item<I>) -> Bool").parse_type().is_err());
    let module = compile_function_bound_impl();
    assert_function_bound(&module);
    let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    assert_function_bound(&module);
    // Put a different name at the parameter's exact byte offset in the
    // preceding archive entry. Both operands must resolve in their owner.
    let param = module
        .types
        .iter()
        .flat_map(|ty| &ty.protocols)
        .flat_map(|implementation| &implementation.type_param_fn_bounds)
        .next()
        .unwrap()
        .0;
    let mut prefix = VbcModule::new("x".repeat(param.0 as usize - 4));
    assert_eq!(prefix.intern_string("wrong_parameter"), param);
    let mut archive = verum_vbc::archive::ArchiveBuilder::new();
    archive.add_module("a.decoy", &prefix, &[]).unwrap();
    archive.add_module("b.owner", &module, &[]).unwrap();
    let mut linker = VbcLinker::new("aarch64-apple-darwin");
    linker.add_archive(&archive.finish()).unwrap();
    assert_function_bound(&linker.finalize());
}

#[test]
fn wire_and_linker_reject_a_dangling_implementation_bound_operand() {
    use verum_vbc::{
        linker::{LinkError, VbcLinker},
        types::StringId,
    };

    let template = compile_function_bound_impl();
    for damage_name in [true, false] {
        let mut module = template.clone();
        let ty = module
            .types
            .iter_mut()
            .find(|ty| !ty.protocols.is_empty())
            .unwrap();
        let pair = &mut ty.protocols[0].type_param_fn_bounds[0];
        if damage_name {
            pair.0 = StringId(u32::MAX);
        } else {
            pair.1 = StringId(u32::MAX);
        }
        let error = verum_vbc::serialize::serialize_module(&module).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unresolved implementation function-bound string")
        );
        let mut linker = VbcLinker::new("aarch64-apple-darwin");
        assert!(matches!(
            linker.add_user_module(module),
            Err(LinkError::DanglingReference {
                kind: "string",
                source_id: u32::MAX
            })
        ));
    }
}

#[test]
fn in_range_function_bound_operands_must_have_the_declared_grammar() {
    use verum_vbc::linker::{LinkError, VbcLinker};
    let template = compile_function_bound_impl();
    for (parameter, bound) in [
        ("core.term.raw", "fn(Int) -> Bool"),
        ("F", "SomeOrdinaryType"),
        ("F", "fn(&::Item<I, J>) -> Bool"),
        ("F", "fn(Int) -> Bool trailing"),
        ("F", "\x1b[?1003h"),
    ] {
        let mut module = template.clone();
        let pair = (module.intern_string(parameter), module.intern_string(bound));
        let ty = module
            .types
            .iter_mut()
            .find(|ty| !ty.protocols.is_empty())
            .unwrap();
        ty.protocols[0].type_param_fn_bounds[0] = pair;
        assert!(
            verum_vbc::serialize::serialize_module(&module)
                .unwrap_err()
                .to_string()
                .contains("invalid implementation function-bound declaration"),
            "{parameter:?} {bound:?}"
        );
        assert!(matches!(
            VbcLinker::new("aarch64-apple-darwin").add_user_module(module),
            Err(LinkError::InvalidImplementationBound)
        ));
    }
}

#[test]
fn reader_rejects_a_callable_spelling_replaced_in_the_wire_string_pool() {
    let module = compile_function_bound_impl();
    let mut bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let bound = b"fn(&::Item<I>) -> Bool";
    let at = bytes.windows(bound.len()).position(|s| s == bound).unwrap();
    bytes[at..at + bound.len()].fill(b'X');
    let error = verum_vbc::deserialize::deserialize_module(&bytes).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("invalid implementation function-bound declaration")
    );
}
