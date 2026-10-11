//! Prepared T1713 consumer controls; not executed by the preparing agent.
//! Source fields are parsed, and the same declaration policy supplies an
//! explicitly synthetic CoreMetadata -> JSON -> consumer boundary. This file
//! does not invoke the VBC archive producer or establish ordinary CLI behavior.
//!
//! lower_field_visibility normalizes In scopes in the declaring source module;
//! archive_metadata::field_declared_visibility reconstructs absolute names.
//! Source cases below retain the original self/super/cog policy. Metadata cases
//! mirror that normalized contract and leave the source owner module empty.
//! Leading-dot restriction syntax remains outside this file (T1716).

use std::sync::Arc; // TypeChecker's public metadata constructor requires Arc.
use verum_ast::{FileId, ItemKind, Module, PathSegment, Span, Visibility, decl::TypeDeclBody};
use verum_common::{List, ResourceDiscipline, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;
use verum_types::core_metadata::{
    CoreMetadata, FieldDescriptor, TypeDescriptor, TypeDescriptorKind,
};

const OWNER: &str = "core.field_scope.defining";
const BUNDLE: &str = "archive.bundle";

#[derive(Clone, Copy, Debug)]
enum PolicyEvidence {
    Declared,
    LegacyNull,
    LegacyOmitted,
}

#[derive(Clone, Copy, Debug)]
enum Input {
    Source,
    Metadata { eager: bool, policy: PolicyEvidence },
}

fn declared_inputs() -> [Input; 3] {
    [
        Input::Source,
        Input::Metadata {
            eager: false,
            policy: PolicyEvidence::Declared,
        },
        Input::Metadata {
            eager: true,
            policy: PolicyEvidence::Declared,
        },
    ]
}

fn parse(source: &str, file: u32) -> Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(file))
        .expect("fixture follows production grammar")
}

fn declaration_source(modifier: &str) -> Text {
    format!("public type Packet is {{ {modifier} body: Int }};").into()
}

fn parsed_field(modifier: &str) -> verum_ast::decl::RecordField {
    let ast = parse(declaration_source(modifier).as_str(), 0);
    let ItemKind::Type(declaration) = &ast.items[0].kind else {
        panic!("fixture must declare Packet");
    };
    let TypeDeclBody::Record(fields) = &declaration.body else {
        panic!("fixture must declare a record");
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name.name.as_str(), "body");
    fields[0].clone()
}

fn decoded_metadata(modifier: &str, evidence: PolicyEvidence) -> CoreMetadata {
    let field = parsed_field(modifier);
    let declared_visibility = match evidence {
        PolicyEvidence::Declared => Some(
            field
                .visibility
                .resolve_declared_scope(OWNER)
                .expect("declaring scope is valid"),
        ),
        PolicyEvidence::LegacyNull | PolicyEvidence::LegacyOmitted => None,
    };
    let descriptor = TypeDescriptor {
        name: "Packet".into(),
        // Bundle location deliberately differs from the declaration owner.
        module_path: BUNDLE.into(),
        origin_module_path: Some(OWNER.into()),
        generic_params: List::new(),
        kind: TypeDescriptorKind::Record {
            fields: [FieldDescriptor {
                name: "body".into(),
                ty: "Int".into(),
                is_public: match evidence {
                    PolicyEvidence::Declared => field.visibility.is_public(),
                    // Old public evidence must never replace the missing policy.
                    PolicyEvidence::LegacyNull | PolicyEvidence::LegacyOmitted => true,
                },
                declared_visibility: declared_visibility.clone(),
            }]
            .into_iter()
            .collect(),
        },
        size: Some(8),
        alignment: Some(8),
        methods: List::new(),
        implements: List::new(),
        decl_span: None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
    };
    let key: Text = format!("{OWNER}.Packet").into();
    let mut metadata = CoreMetadata::default();
    metadata.types.insert(key.clone(), descriptor.clone());
    metadata.types.insert("Packet".into(), descriptor);
    metadata.type_declaration_order.push(key.clone());
    let mut json = serde_json::to_value(&metadata).expect("metadata encode");
    if matches!(evidence, PolicyEvidence::LegacyOmitted) {
        for descriptor in json
            .get_mut("types")
            .and_then(serde_json::Value::as_object_mut)
            .expect("metadata types object")
            .values_mut()
        {
            let fields = descriptor
                .get_mut("kind")
                .and_then(|kind| kind.get_mut("Record"))
                .and_then(|record| record.get_mut("fields"))
                .and_then(serde_json::Value::as_array_mut)
                .expect("record field wire shape");
            for field in fields {
                assert!(
                    field
                        .as_object_mut()
                        .expect("field object")
                        .remove("declared_visibility")
                        .is_some()
                );
            }
        }
    }
    let decoded: CoreMetadata = serde_json::from_value(json).expect("metadata decode");
    let TypeDescriptorKind::Record { fields } = &decoded.types.get(&key).expect("exact decoded owner").kind else {
        panic!("decoded descriptor lost record kind");
    };
    assert_eq!(fields[0].declared_visibility, declared_visibility);
    if !matches!(evidence, PolicyEvidence::Declared) {
        assert!(
            fields[0].is_public,
            "legacy refusal needs conflicting public bit"
        );
        assert!(fields[0].declared_visibility.is_none());
    }
    decoded
}

fn errors(modifier: &str, input: Input, caller: &str, field: &str) -> List<Text> {
    let owner_source = match input {
        Input::Source => declaration_source(modifier),
        Input::Metadata { .. } => Text::new(),
    };
    let consumer: Text = if caller == OWNER {
        match input {
            Input::Source => {
                format!("{owner_source} fn probe(value: &Packet) -> Int {{ value.{field} }}").into()
            }
            // Same-owner archive access still needs metadata authority. It
            // must not acquire a parsed Packet declaration as a fallback.
            Input::Metadata { .. } => {
                format!("fn probe(value: &{OWNER}.Packet) -> Int {{ value.{field} }}").into()
            }
        }
    } else {
        format!("mount {OWNER}.{{Packet}}; fn probe(value: &Packet) -> Int {{ value.{field} }}")
            .into()
    };
    let mut registry = ModuleRegistry::new();
    for (index, (module_owner, source)) in [(OWNER, &owner_source), (caller, &consumer)]
        .into_iter()
        .enumerate()
    {
        if index == 0 && caller == OWNER {
            continue;
        }
        let id = ModuleId::new(index as u32);
        let file = FileId::new(index as u32);
        let ast = parse(source.as_str(), index as u32);
        let path = ModulePath::from_str(module_owner);
        let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), file, source.clone());
        info.exports = extract_exports_from_module(&ast, id, &path).expect("fixture exports");
        registry.register(info);
    }
    let mut checker = match input {
        Input::Source => TypeChecker::new(),
        Input::Metadata { eager, policy } => {
            let metadata = Arc::new(decoded_metadata(modifier, policy));
            if eager {
                TypeChecker::new_with_core_eager(metadata)
            } else {
                TypeChecker::new_with_core(metadata)
            }
        }
    };
    checker.register_primitives();
    checker.set_current_module_path(caller);
    checker.set_module_registry_direct(registry.clone());
    let ast = parse(consumer.as_str(), 1);
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, caller, &registry) {
                errors.push(format!("{error:?}").into());
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
            errors.push(format!("{error:?}").into());
        }
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors
}

fn assert_refused(errors: &List<Text>, input: Input, caller: &str) {
    assert!(
        errors.iter().any(|error| {
            error.contains("VisibilityError")
                && error.contains("name: Text { inner: \"body\" }")
                && error.contains("module_path: Text { inner: \"core.field_scope.defining\" }")
        }),
        "{input:?}, caller={caller}: exact declaring field must be refused: {errors:?}"
    );
}

fn assert_policy(modifier: &str, allowed: &[&str], refused: &[&str]) {
    for input in declared_inputs() {
        for caller in allowed {
            let errors = errors(modifier, input, caller, "body");
            assert!(
                errors.is_empty(),
                "{modifier}, {input:?}, caller={caller}: {errors:?}"
            );
        }
        for caller in refused {
            assert_refused(&errors(modifier, input, caller, "body"), input, caller);
        }
    }
}

fn assert_declared_scope(modifier: &str, expected: &str) {
    let parsed = parsed_field(modifier).visibility;
    let resolved = parsed
        .resolve_declared_scope(OWNER)
        .expect("valid declaring scope");
    let Visibility::PublicIn(path) = &resolved else {
        panic!("expected path-restricted visibility");
    };
    assert_eq!(path.span, Span::dummy());
    let names: List<Text> = path
        .segments
        .iter()
        .map(|segment| {
            let PathSegment::Name(ident) = segment else {
                panic!("archive contract requires absolute name segments");
            };
            assert_eq!(ident.span, Span::dummy());
            ident.name.clone()
        })
        .collect();
    assert_eq!(names.join("."), expected);
    assert_eq!(
        resolved.resolve_declared_scope("other.consumer").unwrap(),
        resolved,
        "an archived absolute policy cannot be rebound in the consumer"
    );
}

#[test]
fn explicit_public_fields_allow_every_caller_but_not_an_undeclared_member() {
    // Public has no legitimate refused caller scope. Its negative keeps member
    // lookup meaningful rather than inventing a visibility restriction.
    assert_policy(
        "public",
        &[OWNER, "core.other.reader", "other.reader", BUNDLE],
        &[],
    );
    for input in declared_inputs() {
        let errors = errors("public", input, "other.reader", "absent_field");
        assert!(
            errors.iter().any(|error| error.contains("absent_field")),
            "{input:?}: missing exact member must remain refused: {errors:?}"
        );
    }
}

#[test]
fn explicit_private_fields_allow_only_the_declaring_module() {
    assert_policy(
        "private",
        &[OWNER],
        &[
            "core.field_scope",
            "core.field_scope.defining.child",
            "core.other",
            "other.reader",
        ],
    );
}

#[test]
fn archived_cog_visibility_uses_declaring_cog_instead_of_bundle_cog() {
    assert_policy(
        "public(cog)",
        &[OWNER, "core", "core.other.reader"],
        &["other.reader", "archive", BUNDLE],
    );
}

#[test]
fn archived_super_visibility_allows_owner_and_immediate_parent_only() {
    // The field guard preserves owner-local access before VisibilityChecker;
    // the latter alone grants PublicSuper only to the immediate parent.
    assert_policy(
        "public(super)",
        &[OWNER, "core.field_scope"],
        &[
            "core",
            "core.field_scope.sibling",
            "core.field_scope.defining.child",
            "archive",
        ],
    );
}

#[test]
fn explicit_internal_fields_do_not_grant_cog_wide_access() {
    assert_policy(
        "internal",
        &[OWNER],
        &[
            "core.field_scope",
            "core.field_scope.defining.child",
            "core.other",
            "other.reader",
        ],
    );
}

#[test]
fn explicit_protected_fields_do_not_grant_descendant_access() {
    assert_policy(
        "protected",
        &[OWNER],
        &[
            "core.field_scope",
            "core.field_scope.defining.child",
            "core.other",
            "other.reader",
        ],
    );
}

#[test]
fn self_restricted_field_scope_belongs_to_the_declaration() {
    let modifier = "public(in self.trusted)";
    assert_declared_scope(modifier, "core.field_scope.defining.trusted");
    assert_policy(
        modifier,
        &[
            OWNER,
            "core.field_scope.defining.trusted",
            "core.field_scope.defining.trusted.reader",
        ],
        &[
            "core.field_scope.defining.trustedness",
            "core.field_scope.trusted",
            "other.trusted",
        ],
    );
}

#[test]
fn super_restricted_field_scope_belongs_to_the_declaration() {
    let modifier = "public(in super.trusted)";
    assert_declared_scope(modifier, "core.field_scope.trusted");
    assert_policy(
        modifier,
        &[
            OWNER,
            "core.field_scope.trusted",
            "core.field_scope.trusted.reader",
        ],
        &[
            "core.field_scope",
            "core.field_scope.trustedness",
            "archive.trusted",
            "other.trusted",
        ],
    );
}

#[test]
fn cog_restricted_field_scope_belongs_to_the_declaration() {
    let modifier = "public(in cog.trusted)";
    assert_declared_scope(modifier, "core.trusted");
    assert_policy(
        modifier,
        &[OWNER, "core.trusted", "core.trusted.reader"],
        &[
            "core",
            "core.trustedness",
            "archive.trusted",
            "other.trusted",
        ],
    );
}

#[test]
fn absolute_restricted_scope_can_name_a_different_cog_without_rebinding() {
    let modifier = "public(in other.allowed)";
    assert_declared_scope(modifier, "other.allowed");
    assert_policy(
        modifier,
        &[OWNER, "other.allowed", "other.allowed.reader"],
        &["other", "other.allowedness", "core.allowed", BUNDLE],
    );
}

fn assert_legacy_refused(policy: PolicyEvidence) {
    for eager in [false, true] {
        let input = Input::Metadata { eager, policy };
        for caller in ["core.field_scope.reader", "other.reader", BUNDLE] {
            let errors = errors("public", input, caller, "body");
            assert_refused(&errors, input, caller);
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("unknown archived visibility")),
                "{input:?}, caller={caller}: missing evidence must be named: {errors:?}"
            );
        }
    }
}

#[test]
fn null_archived_policy_is_unknown_even_when_legacy_public_bit_is_true() {
    assert_legacy_refused(PolicyEvidence::LegacyNull);
}

#[test]
fn omitted_archived_policy_is_unknown_even_when_legacy_public_bit_is_true() {
    assert_legacy_refused(PolicyEvidence::LegacyOmitted);
}
