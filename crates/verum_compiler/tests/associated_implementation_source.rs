//! Parsed-source carrier boundaries; these tests do not establish implementation conformance.
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use verum_ast::decl::{ImplDecl, ImplItemKind, ItemKind};
use verum_ast::source_implementation::AssociatedImplementationSource as Carrier;
use verum_ast::{Type, TypeKind};
use verum_common::Heap;
use verum_fast_parser::Parser;

const OWNER: &str = "demo.family";
const SOURCE: &str = r#"
implement<T> Family for Holder<T> {
    type Item<T> = (T, Int);
    async fn inspect(callback: async fn(Int) -> Int, value: Int) -> Int using [Log]
        requires value >= 0
        ensures result >= 0
    { callback(value) }
}
"#;

fn implementation(source: &str) -> ImplDecl {
    let mut parsed = Parser::new(source)
        .parse_module()
        .expect("actual Verum source grammar");
    assert_eq!(parsed.items.len(), 1);
    let ItemKind::Impl(declaration) = parsed.items.pop().unwrap().kind else {
        panic!("fixture must declare exactly one implementation")
    };
    declaration
}

fn carrier() -> Carrier {
    Carrier::new(OWNER.into(), &implementation(SOURCE)).expect("representable source declaration")
}

fn decode_declaration(declaration: &ImplDecl) -> Result<Carrier, serde_json::Error> {
    let mut envelope = serde_json::to_value(carrier()).unwrap();
    envelope["declaration"] =
        serde_json::Value::String(serde_json::to_string(declaration).unwrap());
    serde_json::from_value(envelope)
}

fn hash(value: &Carrier) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn owner_and_same_spelled_outer_and_local_telescope_survive() {
    let original = implementation(SOURCE);
    let source = Carrier::new(OWNER.into(), &original).unwrap();
    assert_eq!(source.origin_module(), OWNER);
    let retained = source.declaration().unwrap();
    assert_eq!(retained.generics, original.generics);
    assert_eq!(retained.kind, original.kind);
    assert_eq!(retained.generic_where_clause, original.generic_where_clause);
    assert_eq!(retained.meta_where_clause, original.meta_where_clause);
    assert_eq!(retained.specialize_attr, original.specialize_attr);
    assert_eq!(retained.items[0], original.items[0]);
    let ImplItemKind::Type { type_params, .. } = &retained.items[0].kind else {
        panic!()
    };
    assert_eq!(retained.generics.len(), 1);
    assert_eq!(type_params.len(), 1);
    // Both are spelled T, but their declaration spans and lexical lists remain distinct.
    let outer_name = match &retained.generics[0].kind {
        verum_ast::ty::GenericParamKind::Type { name, .. } => name,
        _ => panic!("outer type binder"),
    };
    let local_name = match &type_params[0].kind {
        verum_ast::ty::GenericParamKind::Type { name, .. } => name,
        _ => panic!("local type binder"),
    };
    assert_eq!(outer_name.name, local_name.name);
    assert_ne!(outer_name.span, local_name.span);
    let other_owner = Carrier::new("other.family".into(), &original).unwrap();
    assert_ne!(
        source, other_owner,
        "owner participates in declaration identity"
    );
}

#[test]
fn method_contracts_survive_without_executable_bodies() {
    let original = implementation(SOURCE);
    let retained = Carrier::new(OWNER.into(), &original)
        .unwrap()
        .declaration()
        .unwrap();
    let ImplItemKind::Function(original_method) = &original.items[1].kind else {
        panic!()
    };
    let ImplItemKind::Function(retained_method) = &retained.items[1].kind else {
        panic!()
    };
    assert!(original_method.body.is_some());
    assert!(original_method.is_async);
    assert!(!original_method.contexts.is_empty());
    assert!(!original_method.requires.is_empty());
    assert!(!original_method.ensures.is_empty());
    let mut expected = original_method.clone();
    expected.body = None;
    assert_eq!(
        retained_method, &expected,
        "all signature fields remain authoritative"
    );
    assert_eq!(retained.items[1].visibility, original.items[1].visibility);
    assert_eq!(retained.items[1].attributes, original.items[1].attributes);
}

#[test]
fn excluded_large_body_does_not_consume_the_signature_budget() {
    let source = format!(
        "implement Family for Holder {{ type Item<T> = T; fn body() -> Text {{ \"{}\" }} }}",
        "x".repeat(1024 * 1024 + 1),
    );
    let original = implementation(&source);
    let source = Carrier::new(OWNER.into(), &original).expect("the executable body is excluded");
    assert!(serde_json::to_string(&source).unwrap().len() < 16 * 1024);
    let retained = source.declaration().unwrap();
    let ImplItemKind::Function(function) = &retained.items[1].kind else {
        panic!()
    };
    assert!(function.body.is_none());
}

#[test]
fn decoded_carrier_refuses_an_executable_method_body() {
    let error = decode_declaration(&implementation(SOURCE)).unwrap_err();
    assert!(error.to_string().contains("method body"), "{error}");
}

#[test]
fn associated_constants_and_proof_obligations_are_retained_exactly() {
    for source in [
        "implement Family for Holder { type Item<T> = T; const SIZE: Int = 1; }",
        "implement Family for Holder { type Item<T> = T; proof law by trivial; }",
    ] {
        let declaration = implementation(source);
        assert_eq!(
            declaration.items.len(),
            2,
            "parser must retain the declared item"
        );
        let retained = Carrier::new(OWNER.into(), &declaration).unwrap().declaration().unwrap();
        assert_eq!(retained, declaration, "a carrier must not erase the obligation");
        assert_eq!(decode_declaration(&declaration).unwrap().declaration().unwrap(), declaration);
    }
}

#[test]
fn payload_and_owner_byte_limits_reject_oversized_input() {
    let mut envelope = serde_json::to_value(carrier()).unwrap();
    envelope["declaration"] = serde_json::Value::String(" ".repeat(1024 * 1024 + 1));
    let error = serde_json::from_value::<Carrier>(envelope).unwrap_err();
    assert!(error.to_string().contains("byte budget"), "{error}");
    for owner in [String::new(), "x".repeat(1024 * 1024 + 1)] {
        assert!(Carrier::new(owner.clone().into(), &implementation(SOURCE)).is_err());
        let mut envelope = serde_json::to_value(carrier()).unwrap();
        envelope["origin_module"] = serde_json::Value::String(owner);
        assert!(serde_json::from_value::<Carrier>(envelope).is_err());
    }
}

#[test]
fn nested_type_reader_and_writer_keep_their_recursion_limits() {
    let mut declaration = implementation("implement Family for Holder { type Item<T> = T; }");
    let ImplItemKind::Type { ty, .. } = &mut declaration.items[0].kind else {
        panic!()
    };
    for _ in 0..160 {
        *ty = Type::new(
            TypeKind::Reference {
                mutable: false,
                inner: Heap::new(ty.clone()),
            },
            ty.span,
        );
    }
    let error = Carrier::new(OWNER.into(), &declaration).unwrap_err();
    assert!(error.contains("budget"), "{error}");
    let error = decode_declaration(&declaration).unwrap_err();
    assert!(error.to_string().contains("recursion limit"), "{error}");
}

#[test]
fn decode_normalizes_json_spelling_without_changing_identity_or_hash() {
    let original = carrier();
    let mut envelope = serde_json::to_value(&original).unwrap();
    // Value map key ordering and whitespace differ from typed-struct serialization.
    let declaration = serde_json::to_value(original.declaration().unwrap()).unwrap();
    envelope["declaration"] =
        serde_json::Value::String(serde_json::to_string_pretty(&declaration).unwrap());
    let restored: Carrier = serde_json::from_value(envelope).unwrap();
    assert_eq!(original, restored);
    assert_eq!(hash(&original), hash(&restored));
    assert_eq!(
        serde_json::to_vec(&original).unwrap(),
        serde_json::to_vec(&restored).unwrap()
    );
    assert_eq!(
        original.declaration().unwrap(),
        restored.declaration().unwrap()
    );
}
