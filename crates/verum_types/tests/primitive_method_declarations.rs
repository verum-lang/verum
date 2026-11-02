//! Primitive value types may be built in while their stdlib methods are declarations.
use std::sync::Arc;
use verum_ast::{
    ItemKind,
    decl::{ImplItemKind, ImplKind},
    ty::TypeKind,
};
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

fn declare_metadata_method(meta: &mut CoreMetadata, receiver: &str, name: &str, result: &str) {
    let key = Text::from(receiver);
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
            methods: List::from_single(name.into()),
            implements: List::new(),
            decl_span: None,
            is_public: true,
            is_transparent_wrapper: false,
            resource_discipline: ResourceDiscipline::Unrestricted,
        },
    );
    meta.type_declaration_order.push(key.clone());
    let method = Text::from(format!("{receiver}.{name}"));
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
            return_type: result.into(),
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
}

#[test]
fn metadata_supplies_the_method_signature_in_both_loading_modes() {
    // Distinguishable return types make any substituted compiler signature observable.
    let mut meta = CoreMetadata::default();
    declare_metadata_method(&mut meta, "UInt64", "to_be_bytes", "Bool");
    declare_metadata_method(&mut meta, "Float", "floor", "Bool");
    let meta = Arc::new(meta);
    for mut checker in [
        TypeChecker::new_with_core(meta.clone()),
        TypeChecker::new_with_core_eager(meta.clone()),
    ] {
        checker.register_primitives();
        let errors = check(
            &mut checker,
            "fn declared(x: UInt64) -> Bool { x.to_be_bytes() }
             fn declared_float(x: Float) -> Bool { x.floor() }
             fn qualified_float(x: Float) -> Bool { Float.floor(x) }",
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

#[test]
fn absent_scalar_methods_fail_in_source_and_both_metadata_modes() {
    let meta = Arc::new(CoreMetadata::default());
    for mut checker in [
        TypeChecker::new(),
        TypeChecker::new_with_core(meta.clone()),
        TypeChecker::new_with_core_eager(meta.clone()),
    ] {
        checker.register_primitives();
        for (source, method) in [
            ("fn f(x: Float) -> Float { x.floor() }", "floor"),
            ("fn g(x: Int) -> Int { x.rotate_left(1) }", "rotate_left"),
            ("fn h(x: Float) -> Float { Float.floor(x) }", "floor"),
            ("fn i(x: Bool) -> Int { x.epoch() }", "epoch"),
        ] {
            let errors = check(&mut checker, source);
            assert!(
                errors.iter().any(|msg| msg.contains(method)),
                "{source}: {errors:?}"
            );
        }
    }
}

#[test]
fn declared_scalar_and_reference_operations_keep_their_receiver_types() {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let core = Parser::new(include_str!("../../../core/base/primitives.vr"))
        .parse_module()
        .unwrap();
    for mut item in core.items {
        if let ItemKind::Impl(decl) = &mut item.kind
            && matches!(&decl.kind, ImplKind::Inherent(ty) if matches!(ty.kind, TypeKind::Float | TypeKind::Int))
        {
            decl.items
                .retain(|i| matches!(&i.kind, ImplItemKind::Function(f) if matches!(f.name.name.as_str(), "floor" | "min" | "max_value")));
            checker.register_impl_block(decl).unwrap();
        }
    }
    let source = "implement Float { fn rounded(self) -> Float { self.floor() } }
                  fn apply(x: Float) -> Float { x.floor() }
                  fn explicit(x: Float) -> Float { Float.floor(x) }
                  fn minimum(x: Int, y: Int) -> Int { Int.min(x, y) }
                  fn upper_bound() -> Int { Int.max_value() }
                  fn epoch_of_ref(x: &Int) -> Int { x.epoch() }
                  fn epoch_of_caps(x: &Int) -> Int { let caps = x.epoch_caps(); caps.epoch() }
                  fn readable(x: &Int) -> Bool { let caps = x.epoch_caps(); caps.can_read() }";
    let errors = check(&mut checker, source);
    assert!(errors.is_empty(), "{errors:?}");
    for invalid in [
        "fn wrong_return(x: Float) -> Bool { x.floor() }",
        "fn wrong_receiver(x: Bool) -> Float { Float.floor(x) }",
        "fn no_receiver() -> Float { Float.floor() }",
        "fn wrong_argument(x: Int, y: Bool) -> Int { Int.min(x, y) }",
        "fn shadowed(Float: Bool, value: Float) -> Float { Float.floor(value) }",
    ] {
        let errors = check(&mut checker, invalid);
        assert!(!errors.is_empty(), "{invalid}");
    }
}
