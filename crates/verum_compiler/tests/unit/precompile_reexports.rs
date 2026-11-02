use super::{bind_source_declarations, reexport_declaring_module, scan_module_reexports};
use verum_common::{List, ResourceDiscipline, Text};
use verum_types::core_metadata::{
    CoreMetadata, FunctionDescriptor, TypeDescriptor, TypeDescriptorKind,
};

fn ty(entry: &str, origin: Option<&str>) -> TypeDescriptor {
    TypeDescriptor {
        source_declaration: None,
        name: "Contract".into(),
        module_path: entry.into(),
        origin_module_path: origin.map(Text::from),
        generic_params: List::new(),
        kind: TypeDescriptorKind::Protocol {
            super_protocols: List::new(),
            associated_types: List::new(),
            required_methods: List::new(),
            default_methods: List::new(),
        },
        size: None,
        alignment: None,
        methods: List::new(),
        implements: List::new(),
        decl_span: None,
        is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
        is_public: true,
    }
}

#[test]
fn qualified_declaration_wins_over_colliding_bare_archive_entry() {
    let mut metadata = CoreMetadata::default();
    metadata.types.insert("Contract".into(), ty("core", None));
    metadata.types.insert(
        "core.api.protocols.Contract".into(),
        ty("core.api", Some("core.api.protocols")),
    );
    assert_eq!(
        reexport_declaring_module(&metadata, "Contract", "core.api.protocols"),
        Some("core.api.protocols".into())
    );
    assert_eq!(
        reexport_declaring_module(&metadata, "Contract", "core.other"),
        None
    );
    assert_eq!(
        reexport_declaring_module(&metadata, "Contract", "core.api.protocols.absent"),
        None
    );
    assert_eq!(
        reexport_declaring_module(&metadata, "Absent", "core.api.protocols"),
        None
    );
}

#[test]
fn legacy_bare_index_requires_exact_declared_owner() {
    let mut metadata = CoreMetadata::default();
    metadata
        .types
        .insert("Contract".into(), ty("archive", Some("core.api.protocols")));
    assert_eq!(
        reexport_declaring_module(&metadata, "Contract", "core.api.protocols"),
        Some("core.api.protocols".into())
    );
    for path in ["archive", "core", "core.api", "core.api.protocols.child"] {
        assert_eq!(
            reexport_declaring_module(&metadata, "Contract", path),
            None,
            "{path}"
        );
    }
}

#[test]
fn source_reexport_alias_chain_retains_the_declaring_file() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("api")).unwrap();
    std::fs::write(
        root.path().join("mod.vr"),
        "public mount .api.{Alias as PublicContract};",
    )
    .unwrap();
    std::fs::write(
        root.path().join("api/mod.vr"),
        "public mount .protocols.{Contract as Alias};",
    )
    .unwrap();
    let mut metadata = CoreMetadata::default();
    metadata.types.insert("Contract".into(), ty("core", None));
    metadata.types.insert(
        "core.api.protocols.Contract".into(),
        ty("core.api", Some("core.api.protocols")),
    );
    scan_module_reexports(&mut metadata, root.path(), false).unwrap();
    let exports = metadata.module_reexports.get(&Text::from("core")).unwrap();
    assert_eq!(
        exports.as_slice(),
        &[(
            "PublicContract".into(),
            "Contract".into(),
            "core.api.protocols".into()
        )]
    );
}

#[test]
fn inline_globs_use_exact_public_surfaces_and_keep_declaration_owners() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("mod.vr"),
        "public module prelude { public mount super.api.*; }",
    )
    .unwrap();
    std::fs::write(
        root.path().join("api.vr"),
        "public type Contract is protocol {}; type Secret is protocol {};",
    )
    .unwrap();
    std::fs::create_dir(root.path().join("api")).unwrap();
    std::fs::write(
        root.path().join("api/deep.vr"),
        "public type Hidden is protocol {};",
    )
    .unwrap();
    let mut metadata = CoreMetadata::default();
    for (name, owner) in [
        ("Contract", "core.api"),
        ("Secret", "core.api"),
        ("Hidden", "core.api.deep"),
    ] {
        let mut descriptor = ty("core", Some(owner));
        descriptor.name = name.into();
        metadata.types.insert(name.into(), descriptor);
    }
    scan_module_reexports(&mut metadata, root.path(), false).unwrap();
    for module in ["core", "core.prelude"] {
        let exports = metadata.module_reexports.get(&Text::from(module)).unwrap();
        assert_eq!(
            exports.as_slice(),
            &[("Contract".into(), "Contract".into(), "core.api".into())]
        );
    }
}

#[test]
fn named_reexport_chains_are_not_truncated_by_a_magic_depth() {
    let root = tempfile::tempdir().unwrap();
    for i in 0..40 {
        let target = if i == 39 {
            "api".to_owned()
        } else {
            format!("m{}", i + 1)
        };
        std::fs::write(
            root.path().join(format!("m{i}.vr")),
            format!("public mount core.{target}.Contract;"),
        )
        .unwrap();
    }
    std::fs::write(
        root.path().join("api.vr"),
        "public type Contract is protocol {};",
    )
    .unwrap();
    let mut metadata = CoreMetadata::default();
    metadata
        .types
        .insert("Contract".into(), ty("core", Some("core.api")));
    scan_module_reexports(&mut metadata, root.path(), false).unwrap();
    assert_eq!(
        metadata
            .module_reexports
            .get(&Text::from("core.m0"))
            .unwrap()
            .as_slice(),
        &[("Contract".into(), "Contract".into(), "core.api".into())]
    );
}

#[test]
fn source_binding_repairs_shell_provenance_without_guessing_between_homonyms() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("one.vr"), "type Contract is protocol {};").unwrap();
    let mut metadata = CoreMetadata::default();
    metadata
        .types
        .insert("Contract".into(), ty("core.importer", None));
    bind_source_declarations(&mut metadata, root.path(), &[], false);
    let descriptor = metadata.types.get(&Text::from("Contract")).unwrap();
    assert_eq!(descriptor.origin_module_path, Some("core.one".into()));
    assert!(
        !descriptor.is_public,
        "an imported shell must not publish a private declaration"
    );

    std::fs::write(
        root.path().join("two.vr"),
        "public type Contract is protocol {};",
    )
    .unwrap();
    metadata.types.insert("Contract".into(), ty("core", None));
    metadata
        .types
        .insert("core.two.Contract".into(), ty("core", Some("core.two")));
    bind_source_declarations(&mut metadata, root.path(), &[], false);
    assert_eq!(
        metadata
            .types
            .get(&Text::from("Contract"))
            .unwrap()
            .origin_module_path,
        None
    );
    let exact = metadata
        .types
        .get(&Text::from("core.two.Contract"))
        .unwrap();
    assert_eq!(exact.origin_module_path, Some("core.two".into()));
    assert!(exact.is_public);
}

#[test]
fn qualified_function_shells_bind_only_to_top_level_declarations_of_the_same_kind() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("functions.vr"),
        "public fn Contract() -> Int { 42 }",
    )
    .unwrap();
    std::fs::write(
        root.path().join("types.vr"),
        "type Contract is protocol {};",
    )
    .unwrap();
    let mut metadata = CoreMetadata::default();
    metadata.types.insert("Contract".into(), ty("core", None));
    let function = FunctionDescriptor {
        source_signature: None,
        name: "core.Contract".into(),
        module_path: "core".into(),
        origin_module_path: None,
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
        is_public: false,
        explicit_type_param_ids: None,
    };
    metadata
        .functions
        .insert("core.Contract".into(), function.clone());
    let mut method = function;
    method.name = "Container.Contract".into();
    metadata
        .functions
        .insert("Container.Contract".into(), method);
    bind_source_declarations(&mut metadata, root.path(), &[], false);
    let function = &metadata
        .functions
        .get(&Text::from("core.Contract"))
        .unwrap();
    assert_eq!(function.origin_module_path, Some("core.functions".into()));
    assert!(function.is_public);
    assert_eq!(
        metadata
            .types
            .get(&Text::from("Contract"))
            .unwrap()
            .origin_module_path,
        Some("core.types".into())
    );
    assert_eq!(
        metadata
            .functions
            .get(&Text::from("Container.Contract"))
            .unwrap()
            .origin_module_path,
        None
    );
    // An explicit source re-export identifies the owner when the same free
    // function name exists in several modules. A nearest-path guess cannot.
    std::fs::write(
        root.path().join("other.vr"),
        "public fn Contract() -> Bool { true }",
    )
    .unwrap();
    let copied = metadata
        .functions
        .get_mut(&Text::from("core.Contract"))
        .unwrap();
    copied.origin_module_path = None;
    metadata.module_reexports.insert(
        "core".into(),
        [(
            "Contract".into(),
            "Contract".into(),
            "core.functions".into(),
        )]
        .into_iter()
        .collect(),
    );
    bind_source_declarations(&mut metadata, root.path(), &[], false);
    assert_eq!(
        metadata
            .functions
            .get(&Text::from("core.Contract"))
            .unwrap()
            .origin_module_path,
        Some("core.functions".into())
    );
}

#[test]
#[ignore = "requires VERUM_TEST_LIBRARY_METADATA and VERUM_TEST_METADATA_OUTPUT artifact paths"]
fn refresh_release_reexports_with_production_scanner() {
    let input = std::env::var_os("VERUM_TEST_LIBRARY_METADATA").expect("input release metadata");
    let output = std::env::var_os("VERUM_TEST_METADATA_OUTPUT").expect("isolated output metadata");
    assert_ne!(input, output);
    let mut metadata: CoreMetadata = crate::embedded_stdlib_metadata::decode_metadata(&std::fs::read(input).unwrap()).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core");
    scan_module_reexports(&mut metadata, &root, false).unwrap();
    assert!(
        metadata
            .module_reexports
            .get(&Text::from("core"))
            .unwrap()
            .iter()
            .any(|(name, original, owner)| name == "Display"
                && original == "Display"
                && owner == "core.base.protocols")
    );
    let exports = metadata
        .module_reexports
        .get(&Text::from("core.base"))
        .unwrap();
    let display = exports
        .iter()
        .find(|(name, _, _)| name == "Display")
        .unwrap();
    assert_eq!(
        display,
        &(
            "Display".into(),
            "Display".into(),
            "core.base.protocols".into()
        )
    );
    let mut id = 0;
    verum_types::module_projection::project_modules(&metadata, None, |_| {
        id += 1;
        verum_modules::ModuleId::new(id)
    })
    .expect("the complete emitted library must have consistent public exports");
    metadata.version = verum_types::core_metadata::CORE_METADATA_VERSION;
    std::fs::write(output, crate::embedded_stdlib_metadata::encode_metadata(&metadata).unwrap()).unwrap();
}
