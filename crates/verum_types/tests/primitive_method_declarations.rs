//! Primitive value types may be built in while their stdlib methods are declarations.
use std::sync::Arc;
use verum_ast::ItemKind;
use verum_common::{List, ResourceDiscipline, Text};
use verum_fast_parser::Parser;
use verum_types::{Type, TypeChecker, core_metadata::*, protocol::ProtocolChecker};

fn check(checker: &mut TypeChecker, source: &str) -> Vec<String> {
    let ast = Parser::new(source).parse_module().unwrap();
    for item in &ast.items {
        match &item.kind {
            ItemKind::Impl(decl) => checker.register_impl_block(decl).unwrap(),
            ItemKind::Function(decl) => checker.register_function_signature(decl).unwrap(),
            _ => panic!("unexpected test declaration"),
        }
    }
    let mut errors = Vec::new();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error.to_string());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(ToString::to_string),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| format!("{d:?}")),
    );
    errors
}

#[test]
fn methods_require_a_declaration() {
    let protocol = ProtocolChecker::new();
    assert!(protocol.method_registry().is_empty());
    assert!(protocol.lookup_method(&Type::Float, "floor").is_none());

    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let errors = check(
        &mut checker,
        "fn absent(x: UInt64) { let bytes = x.to_be_bytes(); }",
    );
    assert!(
        errors.iter().any(|msg| msg.contains("to_be_bytes")),
        "{errors:?}"
    );

    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let errors = check(
        &mut checker,
        "implement UInt64 { fn to_be_bytes(self) -> Bool { true } }
         fn declared(x: UInt64) -> Bool { x.to_be_bytes() }",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn metadata_supplies_the_method_signature_in_both_loading_modes() {
    // A distinguishable return type makes any substituted compiler signature observable.
    let mut meta = CoreMetadata::default();
    let key = Text::from("UInt64");
    meta.types.insert(
        key.clone(),
        TypeDescriptor {
            source_declaration: None,
            name: key.clone(),
            module_path: "".into(),
            origin_module_path: None,
            generic_params: List::new(),
            kind: TypeDescriptorKind::Record {
                fields: List::new(),
            },
            size: None,
            alignment: None,
            methods: List::from_single("to_be_bytes".into()),
            implements: List::new(),
            decl_span: None,
            is_public: true,
            is_transparent_wrapper: false,
            resource_discipline: ResourceDiscipline::Unrestricted,
        },
    );
    meta.type_declaration_order.push(key.clone());
    let method = Text::from("UInt64.to_be_bytes");
    meta.functions.insert(
        method.clone(),
        FunctionDescriptor {
            source_signature: None,
            name: method,
            module_path: "core.base".into(),
            origin_module_path: Some("core.base.primitives".into()),
            generic_params: List::new(),
            params: List::from_single(ParamDescriptor {
                name: "self".into(),
                ty: key.clone(),
                declared_ty: key.clone(),
                has_default: false,
                default_literal: None,
            }),
            return_type: "Bool".into(),
            contexts: List::new(),
            is_async: false,
            is_unsafe: false,
            intrinsic_id: None,
            parent_type: Some(key),
            impl_generic_names: List::new(),
            is_const: false,
            decl_span: None,
            is_public: true,
            explicit_type_param_ids: None,
        },
    );
    let meta = Arc::new(meta);
    for mut checker in [
        TypeChecker::new_with_core(meta.clone()),
        TypeChecker::new_with_core_eager(meta.clone()),
    ] {
        checker.register_primitives();
        let errors = check(
            &mut checker,
            "fn declared(x: UInt64) -> Bool { x.to_be_bytes() }",
        );
        assert!(errors.is_empty(), "{errors:?}");
        let errors = check(
            &mut checker,
            "fn missing(x: UInt64) { let bytes = x.to_le_bytes(); }",
        );
        assert!(
            errors.iter().any(|error| error.contains("to_le_bytes")),
            "{errors:?}"
        );
    }
}
