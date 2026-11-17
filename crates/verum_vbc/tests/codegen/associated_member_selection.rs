//! Exact source heads, not a nominal owner's unconditional assignment table.
use super::*;
use crate::{codegen::CodegenConfig, deserialize::deserialize_module, serialize::serialize_module};
use verum_common::well_known_types::WellKnownProtocol;
use verum_fast_parser::Parser;

const PROTOCOL: &str =
    "module core.base.iterator; public type IntoIterator is protocol { type Item; };";

fn query(compiler: &VbcCodegen, argument: TypeRef) -> Option<TypeRef> {
    let owner = compiler.type_name_to_id["model.Carrier"];
    compiler.declared_protocol_member(
        &TypeRef::instantiated(owner, [argument]),
        WellKnownProtocol::IntoIterator,
        "Item",
    )
}

fn source_and_wire(sources: &[&str], mut check: impl FnMut(&mut VbcCodegen)) {
    let parsed: List<_> = std::iter::once(PROTOCOL)
        .chain(sources.iter().copied())
        .map(|source| Parser::new(source).parse_module().unwrap())
        .collect();
    let files: List<_> = parsed.iter().collect();
    let mut source = VbcCodegen::with_config(CodegenConfig::new("model"));
    let module = source.collect_declaration_interface(&files).unwrap();
    source.ctx.current_source_module = Some("consumer".into());
    check(&mut source);
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    let mut imported = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    imported.import_archive_module_types(&wire);
    imported.check_source_declaration_error().unwrap();
    check(&mut imported);
}

#[test]
fn disjoint_concrete_heads_select_different_items_in_both_orders() {
    let integer = "implement IntoIterator for Carrier<Int> { type Item = Int; }";
    let boolean = "implement IntoIterator for Carrier<Bool> { type Item = Bool; }";
    for (first, second) in [(integer, boolean), (boolean, integer)] {
        source_and_wire(
            &[&format!(
                r#"
            module model; mount core.base.iterator.IntoIterator;
            public type Carrier<T> is {{ value: T }};
            {first} {second}
        "#
            )],
            |compiler| {
                for expected in [TypeId::INT, TypeId::BOOL] {
                    assert_eq!(
                        query(compiler, TypeRef::Concrete(expected)),
                        Some(TypeRef::Concrete(expected))
                    );
                }
                assert_eq!(query(compiler, TypeRef::Concrete(TypeId::TEXT)), None);
            },
        );
    }
}

#[test]
fn generic_equation_transports_actual_arguments_and_keeps_caller_binders_distinct() {
    source_and_wire(
        &[r#"
        module model; mount core.base.iterator.IntoIterator;
        public type Carrier<T> is { value: T };
        implement<U> IntoIterator for Carrier<U> { type Item = (U, Bool); }
    "#],
        |compiler| {
            // This unrelated caller substitution must not enter the retained impl.
            compiler.current_source_substitution.insert(
                "U".into(),
                super::super::ImplReceiverWitness {
                    ty: TypeRef::Concrete(TypeId::TEXT),
                    semantic: true,
                },
            );
            for actual in [
                TypeRef::Concrete(TypeId::INT),
                TypeRef::Generic(crate::types::TypeParamId(5)),
            ] {
                compiler
                    .ctx
                    .current_generic_param_ids
                    .insert("Caller".into(), 5);
                assert_eq!(
                    query(compiler, actual.clone()),
                    Some(TypeRef::Tuple(vec![
                        actual,
                        TypeRef::Concrete(TypeId::BOOL)
                    ]))
                );
            }
        },
    );
}

#[test]
fn body_names_belong_to_the_implementation_owner_through_wire() {
    source_and_wire(
        &[
            "module fixture.payload; public type Token is { value: Int };",
            "module fixture.consumer; public type Token is { value: Bool };",
            r#"module model; mount core.base.iterator.IntoIterator; mount fixture.payload.Token as Local;
            public type Carrier<T> is { value: T };
            implement IntoIterator for Carrier<Int> { type Item = Local; }"#,
        ],
        |compiler| {
            compiler.ctx.current_source_module = Some("fixture.consumer".into());
            let expected = compiler.type_name_to_id["fixture.payload.Token"];
            assert_ne!(expected, compiler.type_name_to_id["fixture.consumer.Token"]);
            assert_eq!(
                query(compiler, TypeRef::Concrete(TypeId::INT)),
                Some(TypeRef::Concrete(expected))
            );
        },
    );
}

#[test]
fn overlapping_equal_or_conflicting_equations_cannot_choose_by_order() {
    for item in ["Int", "Bool"] {
        let generic = "implement<T> IntoIterator for Carrier<T> { type Item = T; }";
        let concrete = format!("implement IntoIterator for Carrier<Int> {{ type Item = {item}; }}");
        for (first, second) in [(generic, concrete.as_str()), (concrete.as_str(), generic)] {
            source_and_wire(
                &[&format!(
                    r#"
                module model; mount core.base.iterator.IntoIterator;
                public type Carrier<T> is {{ value: T }};
                {first} {second}
            "#
                )],
                |compiler| assert_eq!(query(compiler, TypeRef::Concrete(TypeId::INT)), None),
            );
        }
    }
}

#[test]
fn missing_or_contradictory_source_cannot_borrow_the_unconditional_table() {
    for missing in [false, true] {
        source_and_wire(
            &[r#"
            module model; mount core.base.iterator.IntoIterator;
            public type Carrier<T> is { value: T };
            implement<T> IntoIterator for Carrier<T> { type Item = T; }
        "#],
            |compiler| {
                assert_eq!(
                    query(compiler, TypeRef::Concrete(TypeId::INT)),
                    Some(TypeRef::Concrete(TypeId::INT))
                );
                let owner = compiler.type_name_to_id["model.Carrier"];
                let descriptor = compiler.types.iter_mut().find(|ty| ty.id == owner).unwrap();
                assert!(!descriptor.protocols.is_empty());
                for row in &mut descriptor.protocols {
                    if missing {
                        row.source_implementation = None;
                    } else {
                        assert!(!row.associated_types.is_empty());
                        row.associated_types[0].1 = TypeRef::Concrete(TypeId::BOOL);
                    }
                }
                assert_eq!(query(compiler, TypeRef::Concrete(TypeId::INT)), None);
            },
        );
    }
}

#[test]
fn a_registered_conditional_head_is_not_proof_of_its_requirements() {
    source_and_wire(
        &[r#"
        module model; mount core.base.iterator.IntoIterator;
        public type Carrier<T> is { value: T };
        public type Required is protocol {};
        implement<T: Required> IntoIterator for Carrier<T> { type Item = T; }
    "#],
        |compiler| assert_eq!(query(compiler, TypeRef::Concrete(TypeId::INT)), None),
    );
}

#[test]
fn repeated_interface_rows_keep_one_source_identity() {
    source_and_wire(
        &[r#"
        module model; mount core.base.iterator.IntoIterator;
        public type Carrier<T> is { value: T };
        implement IntoIterator for Carrier<Int> { type Item = Int; }
    "#],
        |compiler| {
            let owner = compiler.type_name_to_id["model.Carrier"];
            let descriptor = compiler.types.iter_mut().find(|ty| ty.id == owner).unwrap();
            descriptor.protocols.extend(descriptor.protocols.clone());
            assert_eq!(
                query(compiler, TypeRef::Concrete(TypeId::INT)),
                Some(TypeRef::Concrete(TypeId::INT))
            );
        },
    );
}

#[test]
fn repeated_source_does_not_hide_a_contradictory_carried_equation() {
    for corrupt_first in [false, true] {
        source_and_wire(
            &[r#"
            module model; mount core.base.iterator.IntoIterator;
            public type Carrier<T> is { value: T };
            implement<T> IntoIterator for Carrier<T> { type Item = T; }
        "#],
            |compiler| {
                assert_eq!(
                    query(compiler, TypeRef::Concrete(TypeId::INT)),
                    Some(TypeRef::Concrete(TypeId::INT))
                );
                let owner = compiler.type_name_to_id["model.Carrier"];
                let descriptor = compiler.types.iter_mut().find(|ty| ty.id == owner).unwrap();
                let original = descriptor.protocols.first().unwrap().clone();
                assert!(original.source_implementation.is_some());
                assert_eq!(original.associated_types.len(), 1);
                let mut corrupt = original.clone();
                corrupt.associated_types[0].1 = TypeRef::Concrete(TypeId::BOOL);
                descriptor.protocols.clear();
                descriptor.protocols.extend(if corrupt_first {
                    [corrupt, original]
                } else {
                    [original, corrupt]
                });
                assert_eq!(
                    query(compiler, TypeRef::Concrete(TypeId::INT)),
                    None,
                    "same source identity cannot suppress contradictory imported evidence"
                );
            },
        );
    }
}

#[test]
fn actual_advance_by_body_keeps_the_range_element_declaration() {
    check_actual_advance_by_body(false);
}

#[test]
fn default_body_does_not_select_the_consumers_callable_homonym() {
    check_actual_advance_by_body(true);
}

fn compile_lexical_default_helper(present: bool) -> super::super::CodegenResult<crate::VbcModule> {
    let mut owner = Parser::new(
        r#"
        module fixture.owner;
        public type Reader is protocol {
            fn sample(&self) -> Int { private_value() }
        };
    "#,
    )
    .parse_module()
    .unwrap();
    if present {
        owner.items.extend(
            Parser::new("fn private_value() -> Int { 17 }")
                .parse_module()
                .unwrap()
                .items,
        );
    }
    let consumer = Parser::new(
        r#"
        module fixture.consumer; mount fixture.owner.Reader;
        type Thing is { marker: Int };
        implement Reader for Thing {}
        fn private_value() -> Int { 91 }
        fn probe() -> Int { let value = Thing { marker: 0 }; value.sample() }
    "#,
    )
    .parse_module()
    .unwrap();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("fixture.consumer"));
    let files = [&owner, &consumer];
    compiler.collect_unit_declarations(&files)?;
    compiler.compile_unit_items(&files, crate::codegen::ItemFailurePolicy::Strict)?;
    compiler.finalize_module_from_state()
}

#[test]
fn a_default_calls_its_private_source_helper_through_wire() {
    let module = compile_lexical_default_helper(true).unwrap();
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, wire] {
        crate::validate::validate_module(&module).unwrap();
        let entry = module
            .find_function_by_name("fixture.consumer.probe")
            .unwrap();
        let result = crate::interpreter::Interpreter::new(module.into())
            .execute_function(entry)
            .unwrap();
        assert_eq!(result.try_as_i64(), Some(17));
    }
}

#[test]
fn a_consumer_helper_cannot_fill_a_missing_default_dependency() {
    let error = compile_lexical_default_helper(false).unwrap_err();
    assert!(
        matches!(error.kind,
        super::super::CodegenErrorKind::UndefinedFunction(ref name) if name == "private_value"),
        "missing lexical dependency must be refused at its call: {error:?}"
    );
}

fn check_actual_advance_by_body(shadow: bool) {
    use crate::codegen::ItemFailurePolicy;
    use verum_ast::{
        ItemKind, TypeDeclBody,
        decl::{ImplItemKind, ImplKind, ProtocolItemKind},
    };
    // Supply the actual constructor declarations and the exact predicate used
    // by advance_by. The consumer deliberately does not mount these helpers.
    let [result, maybe] = [
        ("result.vr", "Result", None),
        ("maybe.vr", "Maybe", Some("is_none")),
    ]
    .map(|(file, name, method)| {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../core/base")
                .join(file),
        )
        .unwrap();
        let mut module = Parser::new(&source).parse_module().unwrap();
        module.items.retain_mut(|item| match &mut item.kind {
            ItemKind::Module(declaration) => declaration.items.is_none(),
            ItemKind::Type(declaration) => declaration.name.as_str() == name,
            ItemKind::Impl(implementation)
                if method.is_some()
                    && matches!(&implementation.kind, ImplKind::Inherent(ty)
                        if verum_ast::pretty::format_type(ty).as_str() == format!("{name}<T>").as_str()) =>
            {
                implementation.items.retain(|item| {
                    matches!(&item.kind,
                        ImplItemKind::Function(function) if Some(function.name.as_str()) == method)
                });
                assert_eq!(implementation.items.len(), 1);
                true
            }
            _ => false,
        });
        assert_eq!(
            module
                .items
                .iter()
                .filter(|item| matches!(item.kind, ItemKind::Type(_)))
                .count(),
            1
        );
        module
    });
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/base/iterator.vr"
    ))
    .unwrap();
    let mut iterator = Parser::new(&source).parse_module().unwrap();
    iterator.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Type(declaration) => matches!(
            declaration.name.as_str(),
            "Iterator" | "IntoIterator" | "Range"
        ),
        ItemKind::Function(function) => function.name.as_str() == "range",
        ItemKind::Impl(implementation) => {
            let ImplKind::Protocol {
                protocol, for_type, ..
            } = &implementation.kind
            else {
                return false;
            };
            let receiver = verum_ast::pretty::format_type(for_type);
            if protocol.to_string() == "IntoIterator"
                && matches!(receiver.as_str(), "Range<Int>" | "Range<Int8>")
            {
                return true;
            }
            if protocol.to_string() == "Iterator" && receiver.as_str() == "Range<Int>" {
                implementation.items.retain(|item| match &item.kind {
                    ImplItemKind::Type { name, .. } => name.as_str() == "Item",
                    ImplItemKind::Function(function) => function.name.as_str() == "next",
                    _ => false,
                });
                assert_eq!(implementation.items.len(), 2);
                return true;
            }
            false
        }
        _ => false,
    });
    iterator.items.extend(
        Parser::new("mount core.base.result.Result; mount core.base.maybe.Maybe;")
            .parse_module()
            .unwrap()
            .items,
    );
    let protocol = iterator
        .items
        .iter_mut()
        .find_map(|item| match &mut item.kind {
            ItemKind::Type(declaration) if declaration.name.as_str() == "Iterator" => {
                Some(declaration)
            }
            _ => None,
        })
        .unwrap();
    let TypeDeclBody::Protocol(body) = &mut protocol.body else {
        unreachable!()
    };
    let original = body
        .items
        .iter()
        .find_map(|item| match &item.kind {
            ProtocolItemKind::Function { decl, .. } if decl.name.as_str() == "advance_by" => {
                Some(item.kind.clone())
            }
            _ => None,
        })
        .unwrap();
    body.items.retain(|item| match &item.kind {
        ProtocolItemKind::Type { name, .. } => name.as_str() == "Item",
        ProtocolItemKind::Function { decl, .. } => {
            matches!(decl.name.as_str(), "next" | "advance_by")
        }
        _ => false,
    });
    assert!(
        body.items.iter().any(|item| item.kind == original),
        "the actual default signature and body remain unchanged"
    );
    let mut client = Parser::new(
        r#"
        module consumer; mount core.base.iterator.Iterator;
        type Empty is { marker: Int };
        implement Iterator for Empty {
            type Item = Int;
            fn next(&mut self) -> Maybe<Int> { Maybe.None }
        }
        fn probe() -> Int {
            let mut values = Empty { marker: 0 };
            match values.advance_by(3) {
                Result.Err(index) => index,
                Result.Ok(_) => -1,
            }
        }
    "#,
    )
    .parse_module()
    .unwrap();
    if shadow {
        client.items.extend(
            Parser::new("fn range(start: Int, end: Int) -> Bool { false }")
                .parse_module()
                .unwrap()
                .items,
        );
    }
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    let files = [&result, &maybe, &iterator, &client];
    compiler.collect_unit_declarations(&files).unwrap();
    compiler
        .compile_unit_items(&files, ItemFailurePolicy::Strict)
        .unwrap();
    let module = compiler.finalize_module_from_state().unwrap();
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, wire] {
        crate::validate::validate_module(&module).unwrap();
        let entry = module.find_function_by_name("consumer.probe").unwrap();
        let result = crate::interpreter::Interpreter::new(module.into())
            .execute_function(entry)
            .unwrap();
        assert_eq!(result.try_as_i64(), Some(0));
    }
}
