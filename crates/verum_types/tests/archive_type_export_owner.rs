//! T1732: an archive export table must not erase a record's declaring owner.
//!
//! The archive-shaped registry deliberately has a real parsed export table and
//! an empty AST, matching the ordinary archive loader. JSON decoding below is a
//! CoreMetadata consumer control, not a VBC producer or fresh-bake claim.

use std::sync::Arc;
use verum_ast::{FileId, ItemKind, Visibility};
use verum_common::{List, ResourceDiscipline, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::core_metadata::{
    CoreMetadata, FieldDescriptor, FunctionDescriptor, GenericParam, TypeDescriptor,
    TypeDescriptorKind,
};
use verum_types::{TypeChecker, TypeError};

const ALPHA: &str = "core.export_alpha";
const BETA: &str = "core.export_beta";

#[derive(Clone, Copy, Debug)]
enum Surface {
    Source,
    MetadataOnly,
    ArchiveExports,
}

enum Bundle<'a> {
    Declaration(&'a str),
    Reexport(&'a str),
    RenamedReexport(&'a str),
}

enum Observation {
    Visibility {
        name: Text,
        visibility: Text,
        owner: Text,
    },
    Missing(Text),
    Other(Text),
}

impl std::fmt::Debug for Observation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Visibility {
                name,
                visibility,
                owner,
            } => formatter
                .debug_struct("Visibility")
                .field("name", name)
                .field("visibility", visibility)
                .field("owner", owner)
                .finish(),
            Self::Missing(detail) => formatter.debug_tuple("Missing").field(detail).finish(),
            Self::Other(detail) => formatter.debug_tuple("Other").field(detail).finish(),
        }
    }
}

fn observe(error: &TypeError) -> Observation {
    match error {
        TypeError::VisibilityError {
            name,
            visibility,
            module_path,
            ..
        } => Observation::Visibility {
            name: name.clone(),
            visibility: visibility.clone(),
            owner: module_path.clone(),
        },
        TypeError::TypeNotFound { .. } | TypeError::ImportItemNotFound { .. } => {
            Observation::Missing(format!("{error:?}").into())
        }
        _ => Observation::Other(format!("{error:?}").into()),
    }
}

fn declaration(owner: &str, generic: bool, policy: Option<Visibility>) -> TypeDescriptor {
    TypeDescriptor {
        name: "Packet".into(),
        module_path: "core.bundle".into(),
        origin_module_path: Some(owner.into()),
        generic_params: if generic {
            List::from_iter([GenericParam {
                name: "T".into(),
                bounds: List::new(),
                default: None,
                type_bounds: List::new(),
                pid: Some(0),
            }])
        } else {
            List::new()
        },
        kind: TypeDescriptorKind::Record {
            fields: List::from_iter([FieldDescriptor {
                name: "value".into(),
                ty: if generic {
                    "T"
                } else if owner == ALPHA {
                    "Int"
                } else {
                    "Bool"
                }
                .into(),
                is_public: policy == Some(Visibility::Public),
                declared_visibility: policy,
            }]),
        },
        size: Some(8),
        alignment: Some(8),
        methods: List::new(),
        implements: List::new(),
        decl_span: None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
    }
}

fn producer(owner: &str, generic: bool) -> FunctionDescriptor {
    FunctionDescriptor {
        name: "produce".into(),
        module_path: "core.bundle".into(),
        origin_module_path: Some(owner.into()),
        generic_params: List::new(),
        params: List::new(),
        return_type: format!("{owner}.Packet{}", if generic { "<Int>" } else { "" }).into(),
        contexts: List::new(),
        is_async: false,
        is_unsafe: false,
        intrinsic_id: None,
        parent_type: None,
        impl_generic_names: List::new(),
        is_const: false,
        decl_span: None,
        is_public: true,
        explicit_type_param_ids: None,
    }
}

fn check(
    source: &str,
    surface: Surface,
    eager: bool,
    decoded: bool,
    reverse: bool,
    generic: bool,
    policy: Option<Visibility>,
    missing: bool,
) -> List<Observation> {
    check_with_bundle(
        source, surface, eager, decoded, reverse, generic, policy, missing, None,
    )
}

fn check_with_bundle(
    source: &str,
    surface: Surface,
    eager: bool,
    decoded: bool,
    reverse: bool,
    generic: bool,
    policy: Option<Visibility>,
    missing: bool,
    bundle: Option<Bundle<'_>>,
) -> List<Observation> {
    let mut metadata = CoreMetadata::default();
    let export_name = if matches!(&bundle, Some(Bundle::RenamedReexport(_))) {
        "Forwarded"
    } else {
        "Packet"
    };
    let owners = if reverse {
        [BETA, ALPHA]
    } else {
        [ALPHA, BETA]
    };
    for owner in owners {
        if missing && owner == ALPHA {
            continue;
        }
        let descriptor = declaration(
            owner,
            generic,
            if owner == ALPHA {
                policy.clone()
            } else {
                Some(Visibility::Public)
            },
        );
        let key: Text = format!("{owner}.Packet").into();
        metadata.type_declaration_order.push(key.clone());
        metadata.types.insert(key, descriptor.clone());
        metadata.types.insert("Packet".into(), descriptor);
        metadata
            .functions
            .insert(format!("{owner}.produce").into(), producer(owner, generic));
        let mut bare = producer(owner, generic);
        bare.name = "produce_bare".into();
        bare.return_type = if generic { "Packet<Int>" } else { "Packet" }.into();
        metadata
            .functions
            .insert(format!("{owner}.produce_bare").into(), bare);
    }
    if let Some(bundle) = bundle {
        let owner = match bundle {
            Bundle::Declaration(owner) => owner,
            Bundle::Reexport(owner) | Bundle::RenamedReexport(owner) => {
                metadata.module_reexports.insert(
                    ALPHA.into(),
                    List::from_iter([(export_name.into(), "Packet".into(), owner.into())]),
                );
                owner
            }
        };
        let key = Text::from("core.Packet");
        metadata.type_declaration_order.push(key.clone());
        metadata
            .types
            .insert(key, declaration(owner, generic, policy.clone()));
    }
    let mut checker = if matches!(surface, Surface::Source) {
        TypeChecker::new()
    } else {
        let metadata = if decoded {
            serde_json::from_slice(&serde_json::to_vec(&metadata).expect("metadata encode"))
                .expect("metadata decode")
        } else {
            metadata
        };
        if eager {
            TypeChecker::new_with_core_eager(Arc::new(metadata))
        } else {
            TypeChecker::new_with_core(Arc::new(metadata))
        }
    };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    let mut registry = ModuleRegistry::new();
    for (index, owner) in owners.into_iter().enumerate() {
        let field_type = if generic {
            "T"
        } else if owner == ALPHA {
            "Int"
        } else {
            "Bool"
        };
        let visibility = if owner == ALPHA && policy == Some(Visibility::Private) {
            "private"
        } else {
            "public"
        };
        let declared_name = if owner == ALPHA { export_name } else { "Packet" };
        let declaration: Text = format!(
            "public type {declared_name}{} is {{ {visibility} value: {field_type} }};",
            if generic { "<T>" } else { "" },
        )
        .into();
        let file = FileId::new(index as u32);
        let parsed = FastParser::new()
            .parse_module_str(&declaration, file)
            .expect("declaration grammar");
        let empty = FastParser::new()
            .parse_module_str("", file)
            .expect("empty archive AST");
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let ast = if matches!(surface, Surface::Source) {
            parsed.clone()
        } else {
            empty.clone()
        };
        let mut info = ModuleInfo::new(id, path.clone(), ast, file, "".into());
        info.exports = extract_exports_from_module(
            if matches!(surface, Surface::MetadataOnly) {
                &empty
            } else {
                &parsed
            },
            id,
            &path,
        )
        .expect("actual parsed export table");
        registry.register(info);
    }
    let module = FastParser::new()
        .parse_module_str(source, FileId::new(100))
        .expect("consumer grammar");
    checker.set_module_registry_direct(registry.clone());
    checker.register_stdlib_types_for_module(&module);
    let mut errors = List::new();
    for item in &module.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "consumer", &registry) {
                errors.push(observe(&error));
            }
        }
    }
    for item in &module.items {
        if let ItemKind::Function(function) = &item.kind {
            if let Err(error) = checker.register_function_signature(function) {
                errors.push(observe(&error));
            }
        }
    }
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(observe(&error));
        }
    }
    errors.extend(checker.take_deferred_errors().iter().map(observe));
    errors.extend(checker.diagnostic_sources().iter().map(observe));
    errors
}

fn each_metadata(mut control: impl FnMut(bool, bool, bool)) {
    for eager in [false, true] {
        for decoded in [false, true] {
            for reverse in [false, true] {
                control(eager, decoded, reverse);
            }
        }
    }
}

fn public_control(source: &str, surface: Surface, generic: bool) {
    each_metadata(|eager, decoded, reverse| {
        let errors = check(
            source,
            surface,
            eager,
            decoded,
            reverse,
            generic,
            Some(Visibility::Public),
            false,
        );
        assert!(
            errors.is_empty(),
            "{surface:?} eager={eager} decoded={decoded} reverse={reverse}: {errors:?}"
        );
    });
}

const DIRECT: &str =
    "mount core.export_alpha.{Packet}; fn read(packet: &Packet) -> Int { packet.value }";

#[test]
fn source_declaration_parameter_keeps_public_policy() {
    public_control(DIRECT, Surface::Source, false);
}

#[test]
fn metadata_only_parameter_keeps_public_policy() {
    public_control(DIRECT, Surface::MetadataOnly, false);
}

#[test]
fn archive_export_parameter_keeps_public_policy() {
    public_control(DIRECT, Surface::ArchiveExports, false);
}

#[test]
fn archive_export_renames_keep_both_owners_and_field_types() {
    public_control(
        "mount core.export_alpha.{Packet as A}; mount core.export_beta.{Packet as B};
         fn left(packet: &A) -> Int { packet.value }
         fn right(packet: &B) -> Bool { packet.value }",
        Surface::ArchiveExports,
        false,
    );
}

#[test]
fn archive_export_inferred_return_keeps_declared_owner() {
    public_control(
        "mount core.export_alpha.{Packet, produce}; fn read() -> Int { let packet = produce(); packet.value }",
        Surface::ArchiveExports,
        false,
    );
}

#[test]
fn archive_export_bare_return_keeps_mounted_declaration() {
    public_control(
        "mount core.export_alpha.{Packet, produce_bare}; fn read() -> Int { let packet = produce_bare(); packet.value }",
        Surface::ArchiveExports,
        false,
    );
}

#[test]
fn archive_export_generic_parameter_keeps_arguments() {
    public_control(
        "mount core.export_alpha.{Packet as A}; mount core.export_beta.{Packet as B};
         fn left(packet: &A<Int>) -> Int { packet.value }
         fn right(packet: &B<Bool>) -> Bool { packet.value }",
        Surface::ArchiveExports,
        true,
    );
}

fn refused_policy(policy: Option<Visibility>, expected: &str) {
    each_metadata(|eager, decoded, reverse| {
        let errors = check(
            DIRECT,
            Surface::ArchiveExports,
            eager,
            decoded,
            reverse,
            false,
            policy.clone(),
            false,
        );
        assert!(!errors.is_empty(), "field policy was bypassed");
        for error in &errors {
            assert!(
                matches!(error, Observation::Visibility { name, visibility, owner }
                if name.as_str() == "value" && visibility.as_str().contains(expected)
                    && owner.as_str() == ALPHA),
                "wrong refusal eager={eager} decoded={decoded} reverse={reverse}: {error:?}"
            );
        }
    });
}

#[test]
fn archive_export_private_field_uses_actual_owner() {
    refused_policy(Some(Visibility::Private), "Private");
}

#[test]
fn archive_export_unknown_policy_stays_unknown() {
    refused_policy(None, "unknown archived");
}

#[test]
fn missing_archive_owner_cannot_borrow_foreign_public_field() {
    each_metadata(|eager, decoded, reverse| {
        let errors = check(
            DIRECT,
            Surface::ArchiveExports,
            eager,
            decoded,
            reverse,
            false,
            Some(Visibility::Public),
            true,
        );
        assert!(
            !errors.is_empty(),
            "absent declaration borrowed a foreign field policy"
        );
        assert!(errors.iter().any(|error| matches!(error,
                Observation::Visibility { visibility, .. } if visibility.as_str().contains("unknown declaration")
        ) || matches!(error, Observation::Missing(_))), "{errors:?}");
    });
}

#[test]
fn archive_bundle_key_retains_its_actual_declaring_owner() {
    each_metadata(|eager, decoded, reverse| {
        for generic in [false, true] {
            let source = if generic {
                "mount core.export_alpha.{Packet}; fn read(packet: &Packet<Int>) -> Int { packet.value }"
            } else {
                DIRECT
            };
            let errors = check_with_bundle(
                source,
                Surface::ArchiveExports,
                eager,
                decoded,
                reverse,
                generic,
                Some(Visibility::Public),
                true,
                Some(Bundle::Declaration(ALPHA)),
            );
            assert!(
                errors.is_empty(),
                "true owner eager={eager} decoded={decoded} reverse={reverse} generic={generic}: {errors:?}"
            );
        }
    });
}

#[test]
fn foreign_archive_bundle_key_cannot_supply_missing_owner() {
    each_metadata(|eager, decoded, reverse| {
        // Match the foreign field's Bool type so a type mismatch cannot hide
        // accidental selection of the wrong declaring module.
        for generic in [false, true] {
            let source = if generic {
                "mount core.export_alpha.{Packet}; fn read(packet: &Packet<Bool>) -> Bool { packet.value }"
            } else {
                "mount core.export_alpha.{Packet}; fn read(packet: &Packet) -> Bool { packet.value }"
            };
            let errors = check_with_bundle(
                source,
                Surface::ArchiveExports,
                eager,
                decoded,
                reverse,
                generic,
                Some(Visibility::Public),
                true,
                Some(Bundle::Declaration(BETA)),
            );
            assert!(
                !errors.is_empty(),
                "foreign bundle supplied absent owner: eager={eager} decoded={decoded} reverse={reverse} generic={generic}"
            );
            assert!(errors.iter().any(|error| matches!(error,
                Observation::Visibility { visibility, .. }
                    if visibility.as_str().contains("unknown declaration")
            ) || matches!(error, Observation::Missing(_))), "{errors:?}");
        }
    });
}

fn bundle_refused_policy(policy: Option<Visibility>, expected: &str) {
    each_metadata(|eager, decoded, reverse| {
        let errors = check_with_bundle(
            DIRECT,
            Surface::ArchiveExports,
            eager,
            decoded,
            reverse,
            false,
            policy.clone(),
            true,
            Some(Bundle::Declaration(ALPHA)),
        );
        assert!(!errors.is_empty(), "bundle field policy was bypassed");
        for error in &errors {
            assert!(
                matches!(error, Observation::Visibility { name, visibility, owner }
                    if name.as_str() == "value" && visibility.as_str().contains(expected)
                        && owner.as_str() == ALPHA),
                "wrong bundle refusal eager={eager} decoded={decoded} reverse={reverse}: {error:?}"
            );
        }
    });
}

#[test]
fn archive_bundle_private_field_keeps_its_policy() {
    bundle_refused_policy(Some(Visibility::Private), "Private");
}

#[test]
fn archive_bundle_unknown_field_keeps_its_policy() {
    bundle_refused_policy(None, "unknown archived");
}

#[test]
fn explicit_archive_reexport_keeps_its_declared_target() {
    each_metadata(|eager, decoded, reverse| {
        let errors = check_with_bundle(
            "mount core.export_alpha.{Packet}; fn read(packet: &Packet) -> Bool { packet.value }",
            Surface::ArchiveExports,
            eager,
            decoded,
            reverse,
            false,
            Some(Visibility::Public),
            true,
            Some(Bundle::Reexport(BETA)),
        );
        assert!(
            errors.is_empty(),
            "explicit reexport eager={eager} decoded={decoded} reverse={reverse}: {errors:?}"
        );
    });
}

#[test]
fn renamed_archive_reexport_keeps_its_original_declared_name() {
    each_metadata(|eager, decoded, reverse| {
        let errors = check_with_bundle(
            "mount core.export_alpha.{Forwarded as Local}; fn read(packet: &Local) -> Bool { packet.value }",
            Surface::ArchiveExports,
            eager,
            decoded,
            reverse,
            false,
            Some(Visibility::Public),
            true,
            Some(Bundle::RenamedReexport(BETA)),
        );
        assert!(
            errors.is_empty(),
            "renamed reexport eager={eager} decoded={decoded} reverse={reverse}: {errors:?}"
        );
    });
}
