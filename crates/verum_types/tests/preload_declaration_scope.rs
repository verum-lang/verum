//! Preloading must follow lexical type bindings, not an archive's bare-name winner.
use std::sync::Arc;
use verum_ast::FileId;
use verum_common::{List, OrderedMap, ResourceDiscipline};
use verum_fast_parser::VerumParser;
use verum_types::{TypeChecker, core_metadata::*};

fn metadata() -> CoreMetadata {
    let mut metadata = CoreMetadata::default();
    let descriptor = TypeDescriptor {
        name: "Shadow".into(),
        module_path: "library".into(),
        origin_module_path: None,
        generic_params: List::new(),
        kind: TypeDescriptorKind::Record {
            fields: List::new(),
        },
        size: None,
        alignment: None,
        methods: List::new(),
        implements: List::new(),
        decl_span: None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
    };
    metadata.types.insert("Shadow".into(), descriptor.clone());
    metadata.types.insert("library.Shadow".into(), descriptor);
    metadata.protocols.insert(
        "Probe".into(),
        ProtocolDescriptor {
            name: "Probe".into(),
            module_path: "library".into(),
            origin_module_path: None,
            generic_params: List::new(),
            super_protocols: List::new(),
            associated_types: List::new(),
            required_methods: List::new(),
            default_methods: List::new(),
            decl_span: None,
        },
    );
    metadata.implementations.push(ImplementationDescriptor {
        protocol: "Probe".into(),
        target_type: "Shadow".into(),
        generic_params: List::new(),
        where_clause: List::new(),
        associated_types: OrderedMap::new(),
        methods: List::new(),
        protocol_args: List::new(),
    });
    metadata
}

fn preload(source: &str) -> TypeChecker {
    let module = VerumParser::new()
        .parse_module_str(source, FileId::new(1))
        .unwrap();
    let mut checker = TypeChecker::new_with_core(Arc::new(metadata()));
    checker.register_stdlib_types_for_module(&module);
    checker
}

#[test]
fn local_declarations_do_not_load_foreign_metadata_closures() {
    for source in [
        "type Shadow is (Int); fn own(value: Shadow) -> Shadow { value }",
        "fn own(value: Shadow) -> Shadow { value } type Shadow is (Int);",
        "type Shadow is { public next: &Shadow };",
        "type Shadow is protocol { fn own(&self) -> Self; }; fn use_it(value: Shadow) {}",
    ] {
        let checker = preload(source);
        assert_eq!(checker.metrics().metadata_impl_candidates, 0, "{source}");
        assert_eq!(checker.metrics().metadata_impl_index_entries, 0, "{source}");
        assert!(checker.resolved_type("Shadow").is_none(), "{source}");
    }
}

#[test]
fn generic_binders_have_item_and_nested_callable_scope() {
    for source in [
        "fn own<Shadow>(value: Shadow) -> Shadow { value }",
        "type Holder<Shadow> is { public value: Shadow };",
        "type Holder<T> is { public value: T }; implement<Shadow> Holder<Shadow> { fn own(&self, value: Shadow) -> Shadow { value } }",
        "type Holder is (); implement Holder { fn own<Shadow>(&self, value: Shadow) -> Shadow { value } }",
        "type Callback is fn<Shadow>(Shadow) -> Shadow;",
    ] {
        let checker = preload(source);
        assert_eq!(checker.metrics().metadata_impl_candidates, 0, "{source}");
        assert!(checker.resolved_type("Shadow").is_none(), "{source}");
    }
}

#[test]
fn explicit_mounts_are_resolved_by_imports_instead_of_the_bare_archive_slot() {
    for source in [
        "mount source.{Shadow}; fn own(value: Shadow) -> Shadow { value }",
        "mount source.{Other as Shadow}; fn own(value: Shadow) -> Shadow { value }",
        "mount source.Shadow; fn own(value: Shadow) -> Shadow { value }",
    ] {
        let checker = preload(source);
        assert_eq!(checker.metrics().metadata_impl_candidates, 0, "{source}");
        assert!(checker.resolved_type("Shadow").is_none(), "{source}");
    }
}

#[test]
fn explicit_external_and_implicit_library_requests_remain_loaded() {
    for source in [
        "fn external(value: Shadow) -> Shadow { value }",
        "mount source.{Shadow as Local}; fn external(value: Shadow) -> Shadow { value }",
        "fn own<Shadow>(value: Shadow) -> Shadow { value } fn external(value: Shadow) -> Shadow { value }",
        "type Shadow is (Int); fn external(value: library.Shadow) -> library.Shadow { value }",
        "type Holder<Shadow> is { public value: Shadow, public foreign: library.Shadow };",
        "type Callback is fn<Shadow>(Shadow) -> library.Shadow;",
    ] {
        let checker = preload(source);
        assert!(checker.metrics().metadata_impl_candidates > 0, "{source}");
        assert!(
            checker.resolved_type("Shadow").is_some()
                || checker.resolved_type("library.Shadow").is_some(),
            "{source}"
        );
    }
}
