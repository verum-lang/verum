//! T1721: converting a record cannot grant access to hidden declared fields.
//! Source declarations and decoded metadata are independent authority inputs.

// The checker metadata API currently requires std::sync::Arc.
use std::sync::Arc;
use verum_ast::{FileId, ItemKind, Visibility};
use verum_common::{List, OrderedMap, ResourceDiscipline, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::core_metadata::{
    CoreMetadata, FieldDescriptor, GenericParam, ImplementationDescriptor, ProtocolDescriptor,
    TypeDescriptor, TypeDescriptorKind,
};
use verum_types::{TypeChecker, TypeError};

const SECRET_OWNER: &str = "core.structural_secret";
const PUBLIC_OWNER: &str = "core.structural_public";
const SECRET: &str = "public type Packet is { body: Int, public marker: Int };";
const PUBLIC: &str = "public type Packet is { public body: Int, public marker: Int };";

#[derive(Clone, Copy, Debug)]
enum Input {
    Source,
    LazyMetadata,
    EagerMetadata,
}

#[derive(Debug, Default)]
struct Diagnostics {
    messages: List<Text>,
    private_fields: List<(Text, Text)>,
    invalid_casts: usize,
    mismatches: List<(Text, Text)>,
}

impl Diagnostics {
    fn add(&mut self, error: &TypeError) {
        if let TypeError::Mismatch { actual, expected, .. } = error {
            self.mismatches.push((actual.clone(), expected.clone()));
        }
        if matches!(error, TypeError::InvalidCast { .. }) {
            self.invalid_casts += 1;
        }
        if let TypeError::VisibilityError {
            name, module_path, ..
        } = error
        {
            self.private_fields
                .push((name.clone(), module_path.clone()));
        }
        self.messages.push(format!("{error:?}").into());
    }
}

fn metadata(reverse: bool) -> CoreMetadata {
    let mut metadata = CoreMetadata::default();
    let mut owners = [SECRET_OWNER, PUBLIC_OWNER];
    if reverse {
        owners.reverse();
    }
    for owner in owners {
        let descriptor = TypeDescriptor {
            name: "Packet".into(),
            module_path: owner.into(),
            origin_module_path: None,
            generic_params: List::new(),
            kind: TypeDescriptorKind::Record {
                fields: ["body", "marker"]
                    .into_iter()
                    .map(|name| {
                        let visibility = if owner == SECRET_OWNER && name == "body" {
                            Visibility::Private
                        } else {
                            Visibility::Public
                        };
                        FieldDescriptor {
                            name: name.into(),
                            ty: "Int".into(),
                            is_public: visibility == Visibility::Public,
                            declared_visibility: Some(visibility),
                        }
                    })
                    .collect(),
            },
            size: Some(16),
            alignment: Some(8),
            methods: List::new(),
            implements: List::new(),
            decl_span: None,
            is_public: true,
            is_transparent_wrapper: false,
            resource_discipline: ResourceDiscipline::Unrestricted,
        };
        let key: Text = format!("{owner}.Packet").into();
        metadata.type_declaration_order.push(key.clone());
        metadata.types.insert(key, descriptor.clone());
        // Vary the bare-slot winner without changing either exact declaration.
        metadata.types.insert("Packet".into(), descriptor);
    }
    serde_json::from_slice(&serde_json::to_vec(&metadata).expect("encode fixture metadata"))
        .expect("decode fixture metadata")
}

fn with_collection_metadata(mut metadata: CoreMetadata) -> CoreMetadata {
    let storage = TypeDescriptor {
        name: "Storage".into(),
        module_path: "core.structural_collection".into(),
        origin_module_path: None,
        generic_params: [GenericParam {
            name: "T".into(),
            bounds: List::new(),
            default: None,
            type_bounds: List::new(),
            pid: Some(0),
        }].into_iter().collect(),
        kind: TypeDescriptorKind::Record {
            fields: [FieldDescriptor {
                name: "item".into(),
                ty: "__generic_0".into(),
                is_public: true,
                declared_visibility: Some(Visibility::Public),
            }].into_iter().collect(),
        },
        size: None,
        alignment: None,
        methods: List::new(),
        implements: ["ArrayCoercible".into()].into_iter().collect(),
        decl_span: None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
    };
    metadata.type_declaration_order.push("core.structural_collection.Storage".into());
    metadata.types.insert("core.structural_collection.Storage".into(), storage.clone());
    metadata.types.insert("Storage".into(), storage);
    metadata.protocols.insert("ArrayCoercible".into(), ProtocolDescriptor {
        name: "ArrayCoercible".into(),
        module_path: "core.structural_collection".into(),
        origin_module_path: None,
        generic_params: List::new(),
        super_protocols: List::new(),
        associated_types: List::new(),
        required_methods: List::new(),
        default_methods: List::new(),
        decl_span: None,
    });
    // Use an explicit declaring target in the metadata carrier, preserving the
    // exact owner used by this mounted collection. The public constructor runs
    // the production marker scan; no unifier registration hook is used.
    metadata.implementations.push(ImplementationDescriptor {
        protocol: "ArrayCoercible".into(),
        target_type: "core.structural_collection.Storage".into(),
        generic_params: List::new(),
        where_clause: List::new(),
        associated_types: OrderedMap::new(),
        methods: List::new(),
        protocol_args: List::new(),
    });
    serde_json::from_slice(&serde_json::to_vec(&metadata).expect("encode collection metadata"))
        .expect("decode collection metadata")
}

fn check(input: Input, reverse: bool, body: &str) -> Diagnostics {
    check_fixture(input, reverse, body, false)
}

fn check_fixture(input: Input, reverse: bool, body: &str, collection: bool) -> Diagnostics {
    let collection_mount = if collection {
        "mount core.structural_collection.{Storage};"
    } else {
        ""
    };
    let consumer = format!(
        "mount {SECRET_OWNER}.{{Packet as Secret}}; mount {PUBLIC_OWNER}.{{Packet as Open}}; type Carrier<T> is {{ public item: T }}; type Identity<T> is T; {collection_mount} {body}"
    );
    let mut checker = if collection {
        let descriptors = with_collection_metadata(if matches!(input, Input::Source) {
            CoreMetadata::default()
        } else {
            metadata(reverse)
        });
        let descriptors = Arc::new(descriptors);
        let mut checker = TypeChecker::new_with_core(descriptors.clone());
        if matches!(input, Input::EagerMetadata) {
            checker.load_stdlib_from_metadata(&descriptors);
        }
        checker
    } else {
        match input {
            Input::Source => TypeChecker::new(),
            Input::LazyMetadata => TypeChecker::new_with_core(Arc::new(metadata(reverse))),
            Input::EagerMetadata => TypeChecker::new_with_core_eager(Arc::new(metadata(reverse))),
        }
    };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    let mut registry = ModuleRegistry::new();
    let mut owners = [(SECRET_OWNER, SECRET), (PUBLIC_OWNER, PUBLIC)];
    if reverse {
        owners.reverse();
    }
    for (index, (owner, source)) in owners.into_iter().enumerate() {
        let source = if matches!(input, Input::Source) {
            source
        } else {
            ""
        };
        let file = FileId::new(index as u32);
        let id = ModuleId::new(index as u32);
        let module = FastParser::new()
            .parse_module_str(source, file)
            .expect("owner grammar");
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(id, path.clone(), module.clone(), file, source.into());
        info.exports = extract_exports_from_module(&module, id, &path).expect("owner exports");
        registry.register(info);
    }
    if collection {
        let path = ModulePath::from_str("core.structural_collection");
        let file = FileId::new(3);
        let module = FastParser::new().parse_module_str("", file).expect("empty collection module");
        registry.register(ModuleInfo::new(ModuleId::new(3), path, module, file, "".into()));
    }
    let module = FastParser::new()
        .parse_module_str(&consumer, FileId::new(2))
        .expect("consumer grammar");
    let id = ModuleId::new(2);
    let path = ModulePath::from_str("consumer");
    let mut info = ModuleInfo::new(
        id,
        path.clone(),
        module.clone(),
        FileId::new(2),
        consumer.into(),
    );
    info.exports = extract_exports_from_module(&module, id, &path).expect("consumer exports");
    registry.register(info);
    checker.set_module_registry_direct(registry.clone());
    checker.register_stdlib_types_for_module(&module);
    let mut errors = Diagnostics::default();
    for item in &module.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "consumer", &registry) {
                errors.add(&error);
            }
        }
    }
    check_items(&mut checker, &module, &mut errors);
    errors
}

fn check_items(checker: &mut TypeChecker, module: &verum_ast::Module, errors: &mut Diagnostics) {
    for item in &module.items {
        let result = match &item.kind {
            ItemKind::Type(declaration) => checker.register_type_declaration(declaration),
            ItemKind::Function(function) => checker.register_function_signature(function),
            ItemKind::Impl(declaration) => checker.register_impl_block(declaration),
            _ => continue,
        };
        if let Err(error) = result {
            errors.add(&error);
        }
    }
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.add(&error);
        }
    }
    for error in checker.take_deferred_errors() {
        errors.add(&error);
    }
    for error in checker.diagnostic_sources() {
        errors.add(error);
    }
}

fn assert_private(input: Input, body: &str) {
    for reverse in [false, true] {
        let errors = check(input, reverse, body);
        assert!(
            errors
                .private_fields
                .iter()
                .any(|(field, owner)| field.as_str() == "body" && owner.as_str() == SECRET_OWNER),
            "{input:?}, reverse={reverse}: must refuse the exact inaccessible declaration: {body}\n{errors:?}"
        );
    }
}

fn assert_allowed(input: Input, body: &str) {
    for reverse in [false, true] {
        let errors = check(input, reverse, body);
        assert!(
            errors.messages.is_empty(),
            "{input:?}, reverse={reverse}: {body}\n{errors:?}"
        );
    }
}

// The existing cast checker refuses nominal/structural casts for both public
// and private records. Retain that boundary; this task does not add cast syntax.
fn assert_invalid_cast(input: Input, body: &str) {
    for reverse in [false, true] {
        let errors = check(input, reverse, body);
        assert!(
            errors.invalid_casts > 0,
            "{input:?}, reverse={reverse}: {body}\n{errors:?}"
        );
    }
}

macro_rules! conversion_controls {
    ($module:ident, $input:expr) => {
        mod $module {
            use super::*;

            #[test]
            fn owned_annotation_cannot_erase_private_owner() {
                assert_private($input, "fn probe(value: Secret) -> Int { let fields: { body: Int, marker: Int } = value; fields.body }");
            }

            #[test]
            fn borrowed_annotation_cannot_erase_private_owner() {
                assert_private($input, "fn probe(value: &Secret) -> Int { let fields: &{ body: Int, marker: Int } = value; fields.body }");
            }

            #[test]
            fn owned_argument_cannot_erase_private_owner() {
                assert_private($input, "fn read(value: { body: Int, marker: Int }) -> Int { value.body } fn probe(value: Secret) -> Int { read(value) }");
            }

            #[test]
            fn borrowed_argument_cannot_erase_private_owner() {
                assert_private($input, "fn read(value: &{ body: Int, marker: Int }) -> Int { value.body } fn probe(value: &Secret) -> Int { read(value) }");
            }

            #[test]
            fn return_cannot_erase_private_owner() {
                assert_private($input, "fn probe(value: Secret) -> ({ body: Int, marker: Int }) { value }");
            }

            #[test]
            fn variable_cannot_forge_a_private_nominal_record() {
                assert_private($input, "fn probe(value: { body: Int, marker: Int }) -> Secret { let forged: Secret = value; forged }");
            }

            #[test]
            fn argument_cannot_forge_a_private_nominal_record() {
                assert_private($input, "fn accept(value: Secret) {} fn probe(value: { body: Int, marker: Int }) { accept(value); }");
            }

            #[test]
            fn return_cannot_forge_a_private_nominal_record() {
                assert_private($input, "fn probe(value: { body: Int, marker: Int }) -> Secret { value }");
            }

            #[test]
            fn a_tuple_cannot_erase_its_private_record_element() {
                assert_private($input, "fn probe(value: (Secret, Int)) -> ({ body: Int, marker: Int }, Int) { value }");
            }

            #[test]
            fn a_declared_container_cannot_erase_its_private_type_argument() {
                assert_private($input, "fn probe(value: Carrier<Secret>) -> Carrier<{ body: Int, marker: Int }> { value }");
            }

            #[test]
            fn an_outer_record_cannot_erase_a_private_nested_record() {
                assert_private($input, "fn probe(value: { nested: Secret }) -> ({ nested: { body: Int, marker: Int } }) { value }");
            }

            #[test]
            fn nested_structural_values_cannot_forge_private_records() {
                for body in [
                    "fn probe(value: ({ body: Int, marker: Int }, Int)) -> (Secret, Int) { value }",
                    "fn probe(value: Carrier<{ body: Int, marker: Int }>) -> Carrier<Secret> { value }",
                    "fn probe(value: { nested: { body: Int, marker: Int } }) -> ({ nested: Secret }) { value }",
                ] {
                    assert_private($input, body);
                }
            }

            #[test]
            fn tuple_assignment_cannot_erase_private_owner() {
                assert_private($input, "fn probe(value: Secret, initial: ({ body: Int, marker: Int }, Int)) -> Int { let mut pair = initial; pair.0 = value; pair.0.body }");
            }

            #[test]
            fn array_elements_cannot_erase_private_owner() {
                assert_private($input, "fn probe(value: [Secret; 2]) -> [{ body: Int, marker: Int }; 2] { value }");
            }

            #[test]
            fn callable_results_cannot_erase_private_owner() {
                assert_private($input, "fn probe(value: fn() -> Secret) -> fn() -> ({ body: Int, marker: Int }) { value }");
            }

            #[test]
            fn callable_results_cannot_forge_private_owner() {
                assert_private($input, "fn probe(value: fn() -> ({ body: Int, marker: Int })) -> fn() -> Secret { value }");
            }

            #[test]
            fn callable_parameters_cannot_expose_private_owner() {
                assert_private($input, "fn probe(value: fn({ body: Int, marker: Int }) -> Int) -> fn(Secret) -> Int { value }");
            }

            #[test]
            fn callable_parameters_cannot_forge_private_owner() {
                assert_private($input, "fn probe(value: fn(Secret) -> Int) -> fn({ body: Int, marker: Int }) -> Int { value }");
            }

            #[test]
            fn public_callable_conversions_and_private_identity_remain_usable() {
                for body in [
                    "fn probe(value: fn() -> Open) -> fn() -> ({ body: Int, marker: Int }) { value }",
                    "fn probe(value: fn() -> ({ body: Int, marker: Int })) -> fn() -> Open { value }",
                    "fn probe(value: fn({ body: Int, marker: Int }) -> Int) -> fn(Open) -> Int { value }",
                    "fn probe(value: fn(Open) -> Int) -> fn({ body: Int, marker: Int }) -> Int { value }",
                    "fn probe(value: fn(Secret) -> Secret) -> fn(Secret) -> Secret { value }",
                ] {
                    assert_allowed($input, body);
                }
            }

            #[test]
            fn explicit_private_to_structural_cast_remains_unsupported() {
                assert_invalid_cast($input, "fn probe(value: Secret) -> Int { let fields = value as { body: Int, marker: Int }; fields.body }");
            }

            #[test]
            fn explicit_structural_to_private_cast_remains_unsupported() {
                assert_invalid_cast($input, "fn probe(value: { body: Int, marker: Int }) -> Secret { value as Secret }");
            }

            #[test]
            fn assignment_cannot_erase_private_owner() {
                assert_private($input, "fn probe(value: Secret, initial: { body: Int, marker: Int }) -> Int { let mut fields = initial; fields = value; fields.body }");
            }

            #[test]
            fn field_assignment_cannot_erase_private_owner() {
                assert_private($input, "type Holder is { public fields: { body: Int, marker: Int } }; fn probe(value: Secret, target: &mut Holder) { target.fields = value; }");
            }

            #[test]
            fn assignment_cannot_forge_a_private_nominal_record() {
                assert_private($input, "fn probe(initial: Secret, value: { body: Int, marker: Int }) -> Secret { let mut result = initial; result = value; result }");
            }

            #[test]
            fn different_nominal_owners_are_not_interchangeable() {
                for body in [
                    "fn probe(value: Secret) -> Open { value }",
                    "fn probe(value: Open) -> Secret { value }",
                ] {
                    for reverse in [false, true] {
                        let errors = check($input, reverse, body);
                        assert!(!errors.messages.is_empty(), "different nominal owner accepted: {body}");
                    }
                }
            }

            #[test]
            fn public_structural_conversions_remain_usable() {
                for body in [
                    "fn probe(value: Open) -> Int { let fields: { body: Int, marker: Int } = value; fields.body }",
                    "fn read(value: { body: Int, marker: Int }) -> Int { value.body } fn probe(value: Open) -> Int { read(value) }",
                    "fn probe(value: Open) -> ({ body: Int, marker: Int }) { value }",
                    "fn probe(value: { body: Int, marker: Int }) -> Open { value }",
                ] {
                    assert_allowed($input, body);
                }
            }

            #[test]
            fn public_borrowed_structural_conversions_remain_usable() {
                for body in [
                    "fn probe(value: &Open) -> Int { let fields: &{ body: Int, marker: Int } = value; fields.body }",
                    "fn read(value: &{ body: Int, marker: Int }) -> Int { value.body } fn probe(value: &Open) -> Int { read(value) }",
                ] {
                    assert_allowed($input, body);
                }
            }

            #[test]
            fn public_nested_structural_conversions_remain_usable() {
                for body in [
                    "fn probe(value: (Open, Int)) -> ({ body: Int, marker: Int }, Int) { value }",
                    "fn probe(value: Carrier<Open>) -> Carrier<{ body: Int, marker: Int }> { value }",
                    "fn probe(value: { nested: Open }) -> ({ nested: { body: Int, marker: Int } }) { value }",
                ] {
                    assert_allowed($input, body);
                }
            }

            #[test]
            fn public_assignment_and_nested_inverse_conversions_remain_usable() {
                for body in [
                    "fn probe(value: Open, initial: { body: Int, marker: Int }) -> Int { let mut fields = initial; fields = value; fields.body }",
                    "type Holder is { public fields: { body: Int, marker: Int } }; fn probe(value: Open, target: &mut Holder) { target.fields = value; }",
                    "fn probe(value: Open, initial: ({ body: Int, marker: Int }, Int)) -> Int { let mut pair = initial; pair.0 = value; pair.0.body }",
                    "fn probe(initial: Open, value: { body: Int, marker: Int }) -> Open { let mut result = initial; result = value; result }",
                    "fn probe(value: ({ body: Int, marker: Int }, Int)) -> (Open, Int) { value }",
                    "fn probe(value: Carrier<{ body: Int, marker: Int }>) -> Carrier<Open> { value }",
                    "fn probe(value: { nested: { body: Int, marker: Int } }) -> ({ nested: Open }) { value }",
                    "fn probe(value: [Open; 2]) -> [{ body: Int, marker: Int }; 2] { value }",
                ] {
                    assert_allowed($input, body);
                }
            }

            #[test]
            fn explicit_public_structural_casts_remain_unsupported() {
                for body in [
                    "fn probe(value: Open) -> Int { let fields = value as { body: Int, marker: Int }; fields.body }",
                    "fn probe(value: { body: Int, marker: Int }) -> Open { value as Open }",
                ] {
                    assert_invalid_cast($input, body);
                }
            }

            #[test]
            fn original_private_nominal_and_alias_transfers_remain_usable() {
                for body in [
                    "fn probe(value: Secret) -> Secret { value }",
                    "fn accept(value: &Secret) {} fn probe(value: &Secret) { accept(value); }",
                    "fn probe(value: Identity<Secret>) -> Secret { value }",
                    "fn probe(value: Secret) -> Identity<Secret> { value }",
                    "fn probe(value: Carrier<Secret>) -> Carrier<Secret> { value }",
                ] {
                    assert_allowed($input, body);
                }
            }
        }
    };
}

conversion_controls!(source, Input::Source);
conversion_controls!(lazy_metadata, Input::LazyMetadata);
conversion_controls!(eager_metadata, Input::EagerMetadata);

#[test]
fn declaring_module_can_convert_its_private_record() {
    let source = "type Packet is { body: Int, public marker: Int }; fn expose(value: Packet) -> ({ body: Int, marker: Int }) { value } fn construct(value: { body: Int, marker: Int }) -> Packet { value }";
    let module = FastParser::new()
        .parse_module_str(source, FileId::new(0))
        .expect("local grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("owner");
    let mut errors = Diagnostics::default();
    check_items(&mut checker, &module, &mut errors);
    assert!(
        errors.messages.is_empty(),
        "same-owner conversion: {errors:?}"
    );
}

// The guard leaves differing nominal heads to existing nominal compatibility.
// Require that compatibility to refuse the actual declaring identities, rather
// than accepting any unrelated diagnostic as evidence of owner separation.
fn assert_nominal_mismatch(input: Input, body: &str, from: &str, to: &str) {
    for reverse in [false, true] {
        let errors = check(input, reverse, body);
        assert!(
            errors.mismatches.iter().any(|(actual, expected)| {
                actual.as_str() == from && expected.as_str() == to
            }),
            "{input:?}, reverse={reverse}: expected nominal mismatch {from} -> {to}: {body}\n{errors:?}"
        );
    }
}

// The collection declaration and its marker travel through decoded metadata;
// the private/public element declaration still follows the selected Input path.
fn assert_private_collection(input: Input, body: &str) {
    for reverse in [false, true] {
        let errors = check_fixture(input, reverse, body, true);
        assert!(errors.private_fields.iter().any(|(field, owner)| {
            field.as_str() == "body" && owner.as_str() == SECRET_OWNER
        }), "{input:?}, reverse={reverse}: collection must refuse exact private field: {body}\n{errors:?}");
    }
}

fn assert_allowed_collection(input: Input, body: &str) {
    for reverse in [false, true] {
        let errors = check_fixture(input, reverse, body, true);
        assert!(errors.messages.is_empty(), "{input:?}, reverse={reverse}: {body}\n{errors:?}");
    }
}

macro_rules! mixed_conversion_controls {
    ($module:ident, $input:expr) => {
        mod $module {
            use super::*;

            #[test]
            fn qualified_aliases_cannot_expose_an_unrelated_private_record() {
                assert_nominal_mismatch(
                    $input,
                    "type SecretAlias is core.structural_secret.Packet; type OpenAlias is core.structural_public.Packet; fn probe(value: SecretAlias) -> OpenAlias { value }",
                    "core.structural_secret.Packet",
                    "core.structural_public.Packet",
                );
            }

            #[test]
            fn qualified_aliases_cannot_forge_an_unrelated_private_record() {
                assert_nominal_mismatch(
                    $input,
                    "type SecretAlias is core.structural_secret.Packet; type OpenAlias is core.structural_public.Packet; fn probe(value: OpenAlias) -> SecretAlias { value }",
                    "core.structural_public.Packet",
                    "core.structural_secret.Packet",
                );
            }

            #[test]
            fn a_local_homonym_cannot_expose_an_imported_private_record() {
                assert_nominal_mismatch(
                    $input,
                    "type Packet is { public body: Int, public marker: Int }; fn probe(value: Secret) -> Packet { value }",
                    "core.structural_secret.Packet",
                    "consumer.Packet",
                );
            }

            #[test]
            fn a_local_homonym_cannot_forge_an_imported_private_record() {
                assert_nominal_mismatch(
                    $input,
                    "type Packet is { public body: Int, public marker: Int }; fn probe(value: Packet) -> Secret { value }",
                    "consumer.Packet",
                    "core.structural_secret.Packet",
                );
            }

            #[test]
            fn qualified_aliases_preserve_original_private_owner_transfers() {
                for body in [
                    "type SecretAlias is core.structural_secret.Packet; fn probe(value: SecretAlias) -> Secret { value }",
                    "type SecretAlias is core.structural_secret.Packet; fn probe(value: Secret) -> SecretAlias { value }",
                ] {
                    assert_allowed($input, body);
                }
            }

            #[test]
            fn array_to_slice_cannot_expose_private_elements() {
                assert_private($input, "fn probe(value: &[Secret; 2]) -> &[{ body: Int, marker: Int }] { value }");
            }

            #[test]
            fn array_to_slice_cannot_forge_private_elements() {
                assert_private($input, "fn probe(value: &[{ body: Int, marker: Int }; 2]) -> &[Secret] { value }");
            }

            #[test]
            fn slice_to_array_cannot_expose_private_elements() {
                assert_private($input, "fn probe(value: &[Secret]) -> &[{ body: Int, marker: Int }; 2] { value }");
            }

            #[test]
            fn slice_to_array_cannot_forge_private_elements() {
                assert_private($input, "fn probe(value: &[{ body: Int, marker: Int }]) -> &[Secret; 2] { value }");
            }

            #[test]
            fn public_array_slice_element_conversions_remain_usable() {
                for body in [
                    "fn probe(value: &[Open; 2]) -> &[{ body: Int, marker: Int }] { value }",
                    "fn probe(value: &[{ body: Int, marker: Int }; 2]) -> &[Open] { value }",
                    "fn probe(value: &[Open]) -> &[{ body: Int, marker: Int }; 2] { value }",
                    "fn probe(value: &[{ body: Int, marker: Int }]) -> &[Open; 2] { value }",
                ] {
                    assert_allowed($input, body);
                }
            }

            #[test]
            fn array_slice_conversions_preserve_original_private_elements() {
                for body in [
                    "fn probe(value: &[Secret; 2]) -> &[Secret] { value }",
                    "fn probe(value: &[Secret]) -> &[Secret; 2] { value }",
                ] {
                    assert_allowed($input, body);
                }
            }

            #[test]
            fn registered_collection_to_slice_cannot_expose_private_elements() {
                assert_private_collection($input, "fn probe(value: &Storage<Secret>) -> &[{ body: Int, marker: Int }] { value }");
            }

            #[test]
            fn registered_collection_to_slice_cannot_forge_private_elements() {
                assert_private_collection($input, "fn probe(value: &Storage<{ body: Int, marker: Int }>) -> &[Secret] { value }");
            }

            #[test]
            fn slice_to_registered_collection_cannot_expose_private_elements() {
                assert_private_collection($input, "fn probe(value: &[Secret]) -> &Storage<{ body: Int, marker: Int }> { value }");
            }

            #[test]
            fn slice_to_registered_collection_cannot_forge_private_elements() {
                assert_private_collection($input, "fn probe(value: &[{ body: Int, marker: Int }]) -> &Storage<Secret> { value }");
            }

            #[test]
            fn public_collection_slice_element_conversions_remain_usable() {
                for body in [
                    "fn probe(value: &Storage<Open>) -> &[{ body: Int, marker: Int }] { value }",
                    "fn probe(value: &Storage<{ body: Int, marker: Int }>) -> &[Open] { value }",
                    "fn probe(value: &[Open]) -> &Storage<{ body: Int, marker: Int }> { value }",
                    "fn probe(value: &[{ body: Int, marker: Int }]) -> &Storage<Open> { value }",
                ] {
                    assert_allowed_collection($input, body);
                }
            }

            #[test]
            fn collection_slice_conversions_preserve_original_private_elements() {
                for body in [
                    "fn probe(value: &Storage<Secret>) -> &[Secret] { value }",
                    "fn probe(value: &[Secret]) -> &Storage<Secret> { value }",
                ] {
                    assert_allowed_collection($input, body);
                }
            }

            #[test]
            fn registered_collection_to_array_cannot_expose_private_elements() {
                assert_private_collection($input, "fn probe(value: &Storage<Secret>) -> &[{ body: Int, marker: Int }; 2] { value }");
            }

            #[test]
            fn registered_collection_to_array_cannot_forge_private_elements() {
                assert_private_collection($input, "fn probe(value: &Storage<{ body: Int, marker: Int }>) -> &[Secret; 2] { value }");
            }

            #[test]
            fn array_to_registered_collection_cannot_expose_private_elements() {
                assert_private_collection($input, "fn probe(value: &[Secret; 2]) -> &Storage<{ body: Int, marker: Int }> { value }");
            }

            #[test]
            fn array_to_registered_collection_cannot_forge_private_elements() {
                assert_private_collection($input, "fn probe(value: &[{ body: Int, marker: Int }; 2]) -> &Storage<Secret> { value }");
            }

            #[test]
            fn public_collection_array_element_conversions_remain_usable() {
                for body in [
                    "fn probe(value: &Storage<Open>) -> &[{ body: Int, marker: Int }; 2] { value }",
                    "fn probe(value: &Storage<{ body: Int, marker: Int }>) -> &[Open; 2] { value }",
                    "fn probe(value: &[Open; 2]) -> &Storage<{ body: Int, marker: Int }> { value }",
                    "fn probe(value: &[{ body: Int, marker: Int }; 2]) -> &Storage<Open> { value }",
                ] {
                    assert_allowed_collection($input, body);
                }
            }

            #[test]
            fn collection_array_conversions_preserve_original_private_elements() {
                for body in [
                    "fn probe(value: &Storage<Secret>) -> &[Secret; 2] { value }",
                    "fn probe(value: &[Secret; 2]) -> &Storage<Secret> { value }",
                ] {
                    assert_allowed_collection($input, body);
                }
            }
        }
    };
}

mixed_conversion_controls!(source_mixed, Input::Source);
mixed_conversion_controls!(lazy_metadata_mixed, Input::LazyMetadata);
mixed_conversion_controls!(eager_metadata_mixed, Input::EagerMetadata);
