use verum_common::{List, ResourceDiscipline, Set, Text};
use verum_modules::{ExportKind, ModuleId, ModuleInfo};
use verum_types::{core_metadata::*, module_projection::project_modules};

fn ty(name: &str, entry: &str, origin: Option<&str>, public: bool) -> TypeDescriptor {
    TypeDescriptor {
        name: name.into(),
        module_path: entry.into(),
        origin_module_path: origin.map(Text::from),
        generic_params: List::new(),
        kind: TypeDescriptorKind::Record {
            fields: List::new(),
        },
        size: None,
        alignment: None,
        methods: List::new(),
        implements: List::new(),
        decl_span: None,
        is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
        is_public: public,
    }
}
fn function(name: &str, entry: &str, origin: Option<&str>, public: bool) -> FunctionDescriptor {
    FunctionDescriptor {
        name: name.into(),
        module_path: entry.into(),
        origin_module_path: origin.map(Text::from),
        generic_params: List::new(),
        params: List::new(),
        return_type: "Int".into(),
        contexts: List::new(),
        is_async: false,
        is_unsafe: false,
        intrinsic_id: None,
        parent_type: None,
        impl_generic_names: List::new(),
        is_const: false,
        decl_span: None,
        is_public: public,
        explicit_type_param_ids: None,
    }
}
fn project(metadata: &CoreMetadata, scope: Option<&Set<Text>>) -> List<ModuleInfo> {
    let mut id = 10;
    project_modules(metadata, scope, |_| {
        id += 1;
        ModuleId::new(id)
    })
    .unwrap()
}
fn module<'a>(modules: &'a [ModuleInfo], path: &str) -> &'a ModuleInfo {
    modules
        .iter()
        .find(|m| m.path.to_string() == path)
        .unwrap_or_else(|| panic!("missing {path}"))
}

#[test]
fn descriptor_identity_ignores_lookup_aliases_and_keeps_exact_source_owner() {
    let mut metadata = CoreMetadata::default();
    let descriptor = ty("Packet", "lib", Some("lib.source"), true);
    for key in ["Packet", "lib.Packet", "lib.source.Packet"] {
        metadata.types.insert(key.into(), descriptor.clone());
    }
    metadata
        .types
        .insert("Other".into(), ty("Packet", "lib", Some("lib.other"), true));
    metadata.functions.insert(
        "lib.source.answer".into(),
        function("answer", "lib", Some("lib.source"), true),
    );
    metadata.functions.insert(
        "answer".into(),
        metadata
            .functions
            .get(&Text::from("lib.source.answer"))
            .unwrap()
            .clone(),
    );
    let modules = project(&metadata, None);
    let source = module(&modules, "lib.source");
    assert_eq!(source.exports.len(), 2);
    assert_eq!(
        source.exports.get(&"Packet".into()).unwrap().source_module,
        source.id
    );
    assert!(
        modules.iter().all(|m| m.path.to_string() != "lib"),
        "archive entry must not grant exports"
    );
    assert_ne!(module(&modules, "lib.other").id, source.id);
}

#[test]
fn entry_owned_declarations_and_evidenced_umbrella_reexports_remain_available() {
    let mut metadata = CoreMetadata::default();
    metadata
        .types
        .insert("Direct".into(), ty("Direct", "lib", None, true));
    metadata.types.insert(
        "Packet".into(),
        ty("Packet", "lib", Some("lib.source"), true),
    );
    metadata.module_reexports.insert(
        "lib".into(),
        List::from_iter([("Packet".into(), "Packet".into(), "lib.source".into())]),
    );
    let modules = project(&metadata, None);
    let umbrella = module(&modules, "lib");
    assert!(umbrella.exports.get(&"Direct".into()).is_some());
    let alias = umbrella.exports.get(&"Packet".into()).unwrap();
    assert_eq!(alias.source_module, module(&modules, "lib.source").id);
    assert_eq!(alias.original_name.as_deref(), Some("Packet"));
}

#[test]
fn qualified_free_functions_do_not_turn_methods_into_module_functions() {
    let mut metadata = CoreMetadata::default();
    for name in [
        "owner.answer",
        "owner::legacy",
        "Thing.method",
        "owner.Thing.method",
        "owner.child.function",
        "work$closure$0",
    ] {
        metadata
            .functions
            .insert(name.into(), function(name, "owner", None, true));
    }
    let mut method = function("hidden_method", "owner", None, true);
    method.parent_type = Some("Thing".into());
    metadata
        .functions
        .insert("Thing.hidden_method".into(), method);
    let modules = project(&metadata, None);
    let exports = &module(&modules, "owner").exports;
    assert_eq!(exports.len(), 2);
    assert!(exports.get(&"answer".into()).is_some());
    assert!(exports.get(&"legacy".into()).is_some());
    assert!(exports.get(&"method".into()).is_none());
}

#[test]
fn private_sum_constructors_do_not_conflict_with_public_source_exports() {
    let mut metadata = CoreMetadata::default();
    for (name, public) in [("VerifierError", true), ("BackendReply", false)] {
        let mut descriptor = ty(name, "bundle", Some("owner"), public);
        descriptor.kind = TypeDescriptorKind::Variant {
            cases: List::from_iter([VariantCase {
                name: "Arity".into(),
                payload: Some(VariantPayload::Tuple(List::from_iter(["Int".into()]))),
            }]),
        };
        metadata.types.insert(name.into(), descriptor);
    }
    let modules = project(&metadata, None);
    let exports = &module(&modules, "owner").exports;
    assert_eq!(exports.len(), 2);
    assert_eq!(
        exports.get(&"Arity".into()).unwrap().kind,
        ExportKind::Function
    );
    assert!(exports.get(&"BackendReply".into()).is_none());
    assert!(
        modules
            .iter()
            .all(|module| module.path.to_string() != "bundle")
    );
}

#[test]
fn context_source_declaration_owns_its_lowered_type_surface() {
    let mut metadata = CoreMetadata::default();
    metadata
        .types
        .insert("Timing".into(), ty("Timing", "owner", None, true));
    let mut context = verum_ast::ContextDecl::synthetic();
    context.name = verum_ast::Ident::new("Timing", verum_ast::Span::dummy());
    metadata.context_decl_nodes.insert("Timing".into(), context);
    metadata
        .context_declaring_modules
        .insert("Timing".into(), "owner".into());
    let modules = project(&metadata, None);
    let exports = &module(&modules, "owner").exports;
    assert_eq!(exports.len(), 1);
    assert_eq!(
        exports.get(&"Timing".into()).unwrap().kind,
        ExportKind::Context
    );
}

#[test]
fn private_declarations_and_protocols_cannot_be_elevated_by_reexports() {
    let mut metadata = CoreMetadata::default();
    let mut protocol = ty("SecretProtocol", "owner", None, false);
    protocol.kind = TypeDescriptorKind::Protocol {
        super_protocols: List::new(),
        associated_types: List::new(),
        required_methods: List::new(),
        default_methods: List::new(),
    };
    metadata.types.insert("SecretProtocol".into(), protocol);
    metadata
        .functions
        .insert("hidden".into(), function("hidden", "owner", None, false));
    metadata.module_reexports.insert(
        "facade".into(),
        List::from_iter([
            (
                "PublicProtocol".into(),
                "SecretProtocol".into(),
                "owner".into(),
            ),
            ("visible".into(), "hidden".into(), "owner".into()),
        ]),
    );
    let modules = project(&metadata, None);
    let owner = module(&modules, "owner");
    assert!(owner.exports.public_exports().next().is_none());
    assert!(owner.exports.get(&"hidden".into()).is_none());
    assert!(module(&modules, "facade").exports.is_empty());
}

#[test]
fn renamed_reexport_chains_keep_the_original_declaration_and_ignore_cycles() {
    let mut metadata = CoreMetadata::default();
    metadata
        .functions
        .insert("answer".into(), function("answer", "origin", None, true));
    for i in 0..40 {
        let (source, original) = if i == 0 {
            ("origin".into(), "answer".into())
        } else {
            (format!("hop{}", i - 1), format!("alias{}", i - 1))
        };
        metadata.module_reexports.insert(
            format!("hop{i}").into(),
            List::from_iter([(format!("alias{i}").into(), original.into(), source.into())]),
        );
    }
    metadata.module_reexports.insert(
        "cycle_a".into(),
        List::from_iter([("x".into(), "x".into(), "cycle_b".into())]),
    );
    metadata.module_reexports.insert(
        "cycle_b".into(),
        List::from_iter([("x".into(), "x".into(), "cycle_a".into())]),
    );
    let modules = project(&metadata, None);
    let alias = module(&modules, "hop39")
        .exports
        .get(&"alias39".into())
        .unwrap();
    assert_eq!(alias.original_name.as_deref(), Some("answer"));
    assert_eq!(alias.source_module, module(&modules, "origin").id);
    assert!(module(&modules, "cycle_a").exports.is_empty());
}

#[test]
fn scope_follows_explicit_dependencies_without_admitting_unrelated_descendants() {
    let mut metadata = CoreMetadata::default();
    metadata
        .functions
        .insert("answer".into(), function("answer", "source", None, true));
    metadata.functions.insert(
        "secret".into(),
        function("secret", "facade.unrelated", None, true),
    );
    metadata.module_reexports.insert(
        "facade".into(),
        List::from_iter([("answer".into(), "answer".into(), "source".into())]),
    );
    let scope = Set::from_iter([Text::from("facade")]);
    let modules = project(&metadata, Some(&scope));
    assert!(
        module(&modules, "facade")
            .exports
            .get(&"answer".into())
            .is_some()
    );
    assert_eq!(modules.len(), 2);
    let other = project(&metadata, None);
    assert_eq!(
        other.len(),
        3,
        "a scoped projection cannot poison later complete results"
    );
}

#[test]
fn ambiguous_reexports_refuse_instead_of_choosing_a_same_spelled_declaration() {
    let mut metadata = CoreMetadata::default();
    for owner in ["a", "b"] {
        metadata.functions.insert(
            format!("{owner}.answer").into(),
            function("answer", owner, None, true),
        );
    }
    metadata.module_reexports.insert(
        "facade".into(),
        List::from_iter([
            ("answer".into(), "answer".into(), "a".into()),
            ("answer".into(), "answer".into(), "b".into()),
        ]),
    );
    let mut id = 0;
    assert!(
        project_modules(&metadata, None, |_| {
            id += 1;
            ModuleId::new(id)
        })
        .is_err()
    );
}
