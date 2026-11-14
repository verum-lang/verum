//! T1715: named variant payload policy belongs to the exact sum declaration.
//! These are parsed-source checker controls. Archive/metadata producer and
//! consumer controls remain separate; this file makes no runtime claim.

use verum_ast::{
    FileId, ItemKind, Visibility,
    decl::{TypeDeclBody, VariantData},
};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::{TypeChecker, TypeError};

const ALPHA: &str = r#"
    public type Packet is Empty | Data { body: Int, public stamp: Int };
"#;
const BETA: &str = r#"
    public type Packet is Empty | Data { public body: Bool, public stamp: Int };
"#;
const GENERIC_ALPHA: &str = r#"
    public type Packet<T> is Empty | Data { body: T, public stamp: Int };
"#;
const GENERIC_BETA: &str = r#"
    public type Packet<T> is Empty | Data { public body: T, public stamp: Int };
"#;
const OWNERS: [&str; 2] = ["core.payload_alpha", "core.payload_beta"];
const MOUNTS: &str =
    "mount core.payload_alpha.{Packet as A}; mount core.payload_beta.{Packet as B};";

fn module(source: &str, id: u32, owner: &str) -> ModuleInfo {
    let file = FileId::new(id);
    let ast = FastParser::new()
        .parse_module_str(source, file)
        .expect("fixture grammar");
    let path = ModulePath::from_str(owner);
    let id = ModuleId::new(id);
    let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), file, source.into());
    info.exports = extract_exports_from_module(&ast, id, &path).expect("fixture exports");
    info
}

#[derive(Debug)]
struct CheckResult {
    messages: List<Text>,
    private_body: bool,
}

impl CheckResult {
    fn record(&mut self, error: &TypeError) {
        self.messages.push(format!("{error:?}").into());
        self.private_body |= matches!(error,
            TypeError::VisibilityError { name, module_path, .. }
                if name.as_str() == "body" && module_path.as_str() == OWNERS[0]
        );
    }
}

fn errors(consumer: &str, generic: bool, reverse: bool) -> CheckResult {
    let mut registry = ModuleRegistry::new();
    let mut declarations = if generic {
        [(OWNERS[0], GENERIC_ALPHA), (OWNERS[1], GENERIC_BETA)]
    } else {
        [(OWNERS[0], ALPHA), (OWNERS[1], BETA)]
    };
    if reverse {
        declarations.reverse();
    }
    for (id, (owner, source)) in declarations.into_iter().enumerate() {
        registry.register(module(source, id as u32, owner));
    }
    let consumer = module(consumer, 2, "consumer");
    let ast = consumer.ast.clone();
    registry.register(consumer);
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry.clone());
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = CheckResult {
        messages: List::new(),
        private_body: false,
    };
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "consumer", &registry) {
                errors.record(&error);
            }
        }
    }
    for item in &ast.items {
        let result = match &item.kind {
            ItemKind::Type(declaration) => checker.register_type_declaration(declaration),
            ItemKind::Function(declaration) => checker.register_function_signature(declaration),
            _ => continue,
        };
        if let Err(error) = result {
            errors.record(&error);
        }
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.record(&error);
        }
    }
    for error in checker.take_deferred_errors() {
        errors.record(&error);
    }
    for error in checker.diagnostic_sources() {
        errors.record(error);
    }
    errors
}

fn accepted(body: &str, generic: bool) {
    let source: Text = format!("{MOUNTS} {body}").into();
    for reverse in [false, true] {
        let errors = errors(source.as_str(), generic, reverse);
        assert!(errors.messages.is_empty(), "reverse={reverse}: {errors:?}");
    }
}

fn private(body: &str, generic: bool) {
    let source: Text = format!("{MOUNTS} {body}").into();
    for reverse in [false, true] {
        let errors = errors(source.as_str(), generic, reverse);
        assert!(
            errors.private_body,
            "reverse={reverse}: expected declaring-owner private body refusal, got {errors:?}"
        );
    }
}

#[test]
fn parser_retains_private_and_public_variant_field_policy() {
    let info = module(ALPHA, 0, OWNERS[0]);
    let declaration = info
        .ast
        .items
        .iter()
        .find_map(|item| match &item.kind {
            ItemKind::Type(declaration) => Some(declaration),
            _ => None,
        })
        .expect("sum declaration");
    let TypeDeclBody::Variant(variants) = &declaration.body else {
        panic!("sum body");
    };
    let variant = variants
        .iter()
        .find(|variant| variant.name.name.as_str() == "Data")
        .expect("Data variant");
    let Some(VariantData::Record(fields)) = &variant.data else {
        panic!("named payload");
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name.name.as_str(), "body");
    assert_eq!(fields[0].visibility, Visibility::Private);
    assert_eq!(fields[1].visibility, Visibility::Public);
}

#[test]
fn qualified_private_payload_literal_is_refused() {
    accepted("fn probe() -> B { B.Data { body: true, stamp: 1 } }", false);
    private("fn probe() -> A { A.Data { body: 7, stamp: 1 } }", false);
}

#[test]
fn bare_private_payload_literal_is_refused() {
    // Mount one sum per consumer so constructor spelling is unambiguous.
    for reverse in [false, true] {
        let good = errors(
            "mount core.payload_beta.Packet; fn probe() -> Packet { Data { body: true, stamp: 1 } }",
            false,
            reverse,
        );
        assert!(
            good.messages.is_empty(),
            "public bare constructor: {good:?}"
        );
        let bad = errors(
            "mount core.payload_alpha.Packet; fn probe() -> Packet { Data { body: 7, stamp: 1 } }",
            false,
            reverse,
        );
        assert!(
            bad.private_body,
            "private bare constructor must retain parent: {bad:?}"
        );
    }
}

#[test]
fn positional_private_payload_constructor_is_refused() {
    accepted("fn probe() -> B { B.Data(true, 1) }", false);
    private("fn probe() -> A { A.Data(7, 1) }", false);
}

#[test]
fn qualified_private_payload_pattern_is_refused() {
    accepted(
        "fn probe(value: B) -> Bool { match value { B.Data { body, .. } => body, B.Empty => false } }",
        false,
    );
    private(
        "fn probe(value: A) -> Int { match value { A.Data { body, .. } => body, A.Empty => 0 } }",
        false,
    );
}

#[test]
fn bare_private_payload_pattern_is_refused() {
    accepted(
        "fn probe(value: B) -> Bool { match value { Data { body, .. } => body, Empty => false } }",
        false,
    );
    private(
        "fn probe(value: A) -> Int { match value { Data { body, .. } => body, Empty => 0 } }",
        false,
    );
}

#[test]
fn reference_private_payload_pattern_is_refused() {
    accepted(
        "fn probe(value: &B) -> Bool { match value { B.Data { body, .. } => *body, B.Empty => false } }",
        false,
    );
    private(
        "fn probe(value: &A) -> Int { match value { A.Data { body, .. } => *body, A.Empty => 0 } }",
        false,
    );
}

#[test]
fn explicitly_ignored_private_payload_field_is_refused() {
    accepted(
        "fn probe(value: B) -> Int { match value { B.Data { body: _, .. } => 1, B.Empty => 0 } }",
        false,
    );
    private(
        "fn probe(value: A) -> Int { match value { A.Data { body: _, .. } => 1, A.Empty => 0 } }",
        false,
    );
}

#[test]
fn public_payload_projection_can_omit_private_fields_with_rest() {
    accepted(
        "fn probe(value: A) -> Int { match value { A.Data { stamp, .. } => stamp, A.Empty => 0 } }",
        false,
    );
}

#[test]
fn owner_local_private_payload_construction_and_pattern_remain_valid() {
    let source: Text = format!("{ALPHA} fn probe() -> Int {{ let value = Packet.Data {{ body: 7, stamp: 1 }}; match value {{ Packet.Data {{ body, .. }} => body, Packet.Empty => 0 }} }}").into();
    for reverse in [false, true] {
        let errors = errors(source.as_str(), false, reverse);
        assert!(errors.messages.is_empty(), "owner-local source: {errors:?}");
    }
}

#[test]
fn aliased_generic_payload_pattern_keeps_declaring_owner() {
    accepted(
        "type Alias<T> is B<T>; fn probe(value: Alias<Bool>) -> Bool { match value { B.Data { body, .. } => body, B.Empty => false } }",
        true,
    );
    private(
        "type Alias<T> is A<T>; fn probe(value: Alias<Int>) -> Int { match value { A.Data { body, .. } => body, A.Empty => 0 } }",
        true,
    );
}

#[test]
fn generic_public_payload_field_preserves_instantiated_type() {
    accepted(
        "fn probe(value: B<Int>) -> Int { match value { B.Data { body, .. } => body, B.Empty => 0 } }",
        true,
    );
    accepted(
        "fn probe(value: B<Bool>) -> Bool { match value { B.Data { body, .. } => body, B.Empty => false } }",
        true,
    );
}

#[test]
fn bare_constructor_values_keep_private_payload_policy() {
    for reverse in [false, true] {
        let good = errors("mount core.payload_beta.Packet; fn probe() -> Packet { let make = Data; make(true, 1) }", false, reverse);
        assert!(good.messages.is_empty(), "public constructor value: {good:?}");
        let bad = errors("mount core.payload_alpha.Packet; fn probe() -> Packet { let make = Data; make(7, 1) }", false, reverse);
        assert!(bad.private_body, "private constructor value: {bad:?}");
    }
}

#[test]
fn qualified_constructor_values_keep_private_payload_policy() {
    accepted("fn probe() -> B { let make = B.Data; make(true, 1) }", false);
    private("fn probe() -> A { let make = A.Data; make(7, 1) }", false);
}

#[test]
fn local_record_and_function_bindings_outrank_same_spelled_cases() {
    accepted("type Data is { body: Bool, stamp: Int }; fn probe() -> Data { Data { body: true, stamp: 1 } }", false);
    accepted("fn Data(value: Int) -> A { A.Empty } fn probe() -> A { let make = Data; make(7) }", false);
}

fn metadata_errors(consumer: &str, reverse: bool, eager: bool, missing_policy: bool) -> CheckResult {
    use std::sync::Arc;
    use verum_common::ResourceDiscipline;
    use verum_types::core_metadata::{CoreMetadata, FieldDescriptor, TypeDescriptor, TypeDescriptorKind,
        VariantCase, VariantPayload, CORE_METADATA_VERSION, encode_metadata, decode_metadata};
    let mut metadata = CoreMetadata { version: CORE_METADATA_VERSION, ..Default::default() };
    let mut owners = [(OWNERS[0], "Int"), (OWNERS[1], "Bool")];
    if reverse { owners.reverse(); }
    for (owner, payload) in owners {
        let descriptor = TypeDescriptor {
            source_declaration: None, name: "Packet".into(), module_path: owner.into(),
            origin_module_path: None, generic_params: List::new(),
            kind: TypeDescriptorKind::Variant { cases: [
                VariantCase { name: "Empty".into(), payload: None },
                VariantCase { name: "Data".into(), payload: Some(VariantPayload::Record(
                    [("body", payload), ("stamp", "Int")].into_iter().map(|(name, ty)| {
                        let public = owner != OWNERS[0] || name != "body";
                        FieldDescriptor { name: name.into(), ty: ty.into(), is_public: public,
                            declared_visibility: if missing_policy && name == "body" { None }
                                else { Some(if public { Visibility::Public } else { Visibility::Private }) } }
                    }).collect())) },
            ].into_iter().collect() },
            size: None, alignment: None, methods: List::new(), implements: List::new(),
            decl_span: None, is_public: true, is_transparent_wrapper: false,
            resource_discipline: ResourceDiscipline::Unrestricted,
        };
        let key: Text = format!("{owner}.Packet").into();
        metadata.types.insert(key.clone(), descriptor.clone());
        metadata.type_declaration_order.push(key);
        metadata.types.insert("Packet".into(), descriptor);
    }
    let encoded = encode_metadata(&metadata).expect("current metadata frame");
    let metadata = Arc::new(decode_metadata(&encoded).expect("decoded field policies"));
    let mut checker = if eager { TypeChecker::new_with_core_eager(metadata) }
        else { TypeChecker::new_with_core(metadata) };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    let mut registry = ModuleRegistry::new();
    for (id, owner) in OWNERS.into_iter().enumerate() {
        registry.register(module("", id as u32, owner));
    }
    let consumer = module(consumer, 2, "consumer");
    let ast = consumer.ast.clone();
    registry.register(consumer);
    checker.set_module_registry_direct(registry.clone());
    checker.register_stdlib_types_for_module(&ast);
    let mut result = CheckResult { messages: List::new(), private_body: false };
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "consumer", &registry) { result.record(&error); }
        }
    }
    for item in &ast.items {
        let registered = match &item.kind {
            ItemKind::Type(declaration) => checker.register_type_declaration(declaration),
            ItemKind::Function(declaration) => checker.register_function_signature(declaration),
            _ => continue,
        };
        if let Err(error) = registered { result.record(&error); }
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) { result.record(&error); }
    }
    for error in checker.take_deferred_errors() { result.record(&error); }
    for error in checker.diagnostic_sources() { result.record(error); }
    result
}

#[test]
fn decoded_metadata_preserves_constructor_and_pattern_field_policy() {
    for eager in [false, true] {
        for reverse in [false, true] {
            for body in [
                "fn probe() -> B { B.Data { body: true, stamp: 1 } }",
                "fn probe() -> B { B.Data(true, 1) }",
                "fn probe(value: B) -> Bool { match value { B.Data { body, .. } => body, B.Empty => false } }",
                "fn probe(value: A) -> Int { match value { A.Data { stamp, .. } => stamp, A.Empty => 0 } }",
            ] {
                let good = metadata_errors(&format!("{MOUNTS} {body}"), reverse, eager, false);
                assert!(good.messages.is_empty(), "eager={eager} reverse={reverse} {body}: {good:?}");
            }
            for body in [
                "fn probe() -> A { A.Data { body: 7, stamp: 1 } }",
                "fn probe() -> A { A.Data(7, 1) }",
                "fn probe(value: A) -> Int { match value { A.Data { body, .. } => body, A.Empty => 0 } }",
            ] {
                let bad = metadata_errors(&format!("{MOUNTS} {body}"), reverse, eager, false);
                assert!(bad.private_body, "eager={eager} reverse={reverse} {body}: {bad:?}");
            }
        }
    }
}

#[test]
fn missing_decoded_payload_policy_does_not_authorize_public_construction() {
    for eager in [false, true] {
        for reverse in [false, true] {
            let bad = metadata_errors(&format!("{MOUNTS} fn probe() -> B {{ B.Data {{ body: true, stamp: 1 }} }}"), reverse, eager, true);
            assert!(bad.messages.iter().any(|message| message.contains("VisibilityError")
                && message.contains("unknown archived visibility") && message.contains(OWNERS[1])
                && message.contains("body")), "eager={eager} reverse={reverse}: {bad:?}");
        }
    }
}
