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
        protocol: ProtocolId(7),
        methods: vec![1, 2],
        associated_types: vec![(name_sid, TypeRef::Generic(verum_vbc::types::TypeParamId(0)))],
        protocol_args_text: Vec::new(),
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
