//! Metadata reuse is justified by full declaration identity, never impl keys.
use std::sync::Arc;
use verum_common::{List, Maybe, OrderedMap, ResourceDiscipline, Set, Text};
use verum_types::{
    Type, TypeChecker, core_metadata::*, infer::parse_descriptor_type_string,
    protocol::CoherenceMode,
};

fn parameter() -> GenericParam {
    GenericParam {
        name: "T".into(),
        bounds: List::new(),
        default: Maybe::None,
        type_bounds: List::new(),
        pid: Maybe::Some(0),
    }
}

fn descriptor() -> ImplementationDescriptor {
    ImplementationDescriptor {
        protocol: "Inspect".into(),
        target_type: "Container".into(),
        generic_params: List::from_iter([parameter()]),
        where_clause: List::new(),
        associated_types: OrderedMap::from_iter([("Item".into(), "__generic_0".into())]),
        methods: List::new(),
        protocol_args: List::new(),
    }
}

fn metadata(implementations: List<ImplementationDescriptor>) -> CoreMetadata {
    let descriptor = TypeDescriptor {
        name: "Container".into(),
        module_path: "owner".into(),
        origin_module_path: Maybe::None,
        generic_params: List::from_iter([parameter()]),
        kind: TypeDescriptorKind::Record {
            fields: List::new(),
        },
        size: Maybe::None,
        alignment: Maybe::None,
        methods: List::new(),
        implements: List::new(),
        decl_span: Maybe::None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
    };
    let mut metadata = CoreMetadata::default();
    for key in [
        "Container",
        "owner.Container",
        "exports.First",
        "exports.Second",
    ] {
        metadata.types.insert(key.into(), descriptor.clone());
    }
    metadata.protocols.insert(
        "Inspect".into(),
        ProtocolDescriptor {
            name: "Inspect".into(),
            module_path: "owner".into(),
            origin_module_path: Maybe::None,
            generic_params: List::new(),
            super_protocols: List::new(),
            associated_types: List::from_iter([AssociatedTypeDescriptor {
                name: "Item".into(),
                bounds: List::new(),
                default: Maybe::None,
            }]),
            required_methods: List::new(),
            default_methods: List::new(),
            decl_span: Maybe::None,
        },
    );
    metadata.implementations = implementations;
    metadata
}

fn load(checker: &mut TypeChecker, name: &str) {
    checker.ensure_stdlib_type_loaded(&name.into(), &mut Default::default());
}

fn item(checker: &TypeChecker, owner: &str) -> Option<Type> {
    checker.protocol_checker.read().try_find_associated_type(
        &parse_descriptor_type_string(&format!("{owner}<Int>")),
        &"Item".into(),
    )
}

#[test]
fn exact_duplicate_generic_descriptors_are_registered_once() {
    let descriptors = (0..4096).map(|_| descriptor()).collect();
    let mut checker = TypeChecker::new_with_core(Arc::new(metadata(descriptors)));
    load(&mut checker, "Container");
    assert_eq!(checker.metrics().metadata_impl_index_entries, 4096);
    assert_eq!(checker.metrics().metadata_impl_candidates, 1);
    assert_eq!(item(&checker, "Container"), Some(Type::Int));
    assert_eq!(item(&checker, "owner.Container"), Some(Type::Int));
    assert_eq!(item(&checker, "foreign.Container"), None);
}

#[test]
fn admitted_descriptor_reuses_successful_aliases_without_rechecking_overlap() {
    let mut checker =
        TypeChecker::new_with_core(Arc::new(metadata(List::from_iter([descriptor()]))));
    checker
        .protocol_checker
        .write()
        .set_coherence_mode(CoherenceMode::Lenient);
    load(&mut checker, "Container");
    checker.protocol_checker.write().drain_coherence_warnings();
    load(&mut checker, "exports.First");
    load(&mut checker, "exports.Second");
    assert!(
        checker
            .protocol_checker
            .write()
            .drain_coherence_warnings()
            .is_empty()
    );
    assert_eq!(item(&checker, "owner.Container"), Some(Type::Int));
}

#[test]
fn rejected_alias_is_retried_without_readmitting_successful_aliases() {
    let mut checker =
        TypeChecker::new_with_core(Arc::new(metadata(List::from_iter([descriptor()]))));
    let Type::Named { path: protocol, .. } = parse_descriptor_type_string("Inspect") else {
        panic!("nominal protocol path");
    };
    // This concrete implementation conflicts with the archived generic owner.
    // Strict coherence refuses that alias; refusal must not create a receipt.
    checker
        .protocol_checker
        .write()
        .register_impl(verum_types::protocol::ProtocolImpl {
            protocol,
            protocol_args: List::new(),
            for_type: parse_descriptor_type_string("owner.Container<Int>"),
            where_clauses: List::new(),
            methods: Default::default(),
            associated_types: [("Item".into(), Type::Bool)].into_iter().collect(),
            associated_consts: Default::default(),
            specialization: Maybe::None,
            impl_crate: Maybe::None,
            span: Default::default(),
            type_param_fn_bounds: Default::default(),
        })
        .unwrap();
    load(&mut checker, "Container");
    checker
        .protocol_checker
        .write()
        .set_coherence_mode(CoherenceMode::Lenient);
    load(&mut checker, "exports.First");
    assert!(
        !checker
            .protocol_checker
            .write()
            .drain_coherence_warnings()
            .is_empty(),
        "failed alias was incorrectly cached as admitted"
    );
    load(&mut checker, "exports.Second");
    assert!(
        checker
            .protocol_checker
            .write()
            .drain_coherence_warnings()
            .is_empty(),
        "successfully retried descriptor must reuse both alias receipts"
    );
}

#[test]
fn different_associated_assignments_reach_coherence_in_source_order() {
    let mut first = descriptor();
    first.associated_types.insert("Item".into(), "Text".into());
    let mut second = first.clone();
    second.associated_types.insert("Item".into(), "Bool".into());
    let mut checker = TypeChecker::new_with_core(Arc::new(metadata(List::from_iter([
        first.clone(),
        first,
        second,
    ]))));
    checker
        .protocol_checker
        .write()
        .set_coherence_mode(CoherenceMode::Lenient);
    load(&mut checker, "Container");
    assert_eq!(checker.metrics().metadata_impl_candidates, 2);
    assert!(
        !checker
            .protocol_checker
            .write()
            .drain_coherence_warnings()
            .is_empty()
    );
    assert_eq!(item(&checker, "owner.Container"), Some(Type::Text));
}

#[test]
fn every_carried_field_participates_in_exact_descriptor_identity() {
    let original = descriptor();
    let mut variants = List::from_iter([original.clone()]);
    macro_rules! changed {
        ($value:ident, $change:expr) => {{
            let mut $value = original.clone();
            $change;
            variants.push($value);
        }};
    }
    changed!(d, d.protocol = "Other".into());
    changed!(d, d.target_type = "foreign.Container".into());
    changed!(d, d.protocol_args.push("Int".into()));
    changed!(d, d.methods.push("inspect".into()));
    changed!(d, d.where_clause.push("T: Inspect".into()));
    changed!(d, d.associated_types.insert("Item".into(), "Text".into()));
    changed!(d, d.generic_params[0].name = "U".into());
    changed!(d, d.generic_params[0].bounds.push("Inspect".into()));
    changed!(d, d.generic_params[0].default = Maybe::Some("Int".into()));
    changed!(
        d,
        d.generic_params[0]
            .type_bounds
            .push("fn(Int) -> Text".into())
    );
    changed!(d, d.generic_params[0].pid = Maybe::Some(32768));
    changed!(d, d.generic_params.push(parameter()));
    let mut declarations = Set::new();
    for value in &variants {
        assert!(
            declarations.insert(value),
            "distinct carried data was collapsed: {value:?}"
        );
    }
    assert_eq!(declarations.len(), variants.len());
    assert!(!declarations.insert(&original));

    // The actual owner index must retain same-target changes even where the
    // current archive loader does not yet enforce a carried bound.
    let same_target: List<_> = variants
        .into_iter()
        .filter(|d| d.target_type == original.target_type)
        .collect();
    let expected = same_target.len();
    let mut checker = TypeChecker::new_with_core(Arc::new(metadata(same_target)));
    load(&mut checker, "Container");
    assert_eq!(checker.metrics().metadata_impl_candidates, expected);
}

#[test]
fn installing_new_metadata_invalidates_position_receipts() {
    let initial = metadata(List::from_iter([descriptor()]));
    let mut checker = TypeChecker::new_with_core(Arc::new(initial.clone()));
    load(&mut checker, "Container");
    let mut replacement = initial;
    let mut protocol = replacement
        .protocols
        .get(&Text::from("Inspect"))
        .unwrap()
        .clone();
    protocol.name = "Measure".into();
    protocol.associated_types[0].name = "Measured".into();
    replacement.protocols.insert("Measure".into(), protocol);
    replacement.implementations[0].protocol = "Measure".into();
    replacement.implementations[0].associated_types =
        OrderedMap::from_iter([("Measured".into(), "Text".into())]);
    checker.set_core_metadata(Arc::new(replacement));
    load(&mut checker, "Container");
    assert_eq!(
        checker.protocol_checker.read().try_find_associated_type(
            &parse_descriptor_type_string("owner.Container<Int>"),
            &"Measured".into(),
        ),
        Some(Type::Text)
    );
}

#[test]
fn prepared_snapshot_reuses_index_without_sharing_checker_admissions() {
    use verum_types::{AnalysisControl, CoreMetadataSnapshot};
    let data = Arc::new(metadata((0..4096).map(|_| descriptor()).collect()));
    let snapshot =
        Arc::new(CoreMetadataSnapshot::prepare(data, &AnalysisControl::default()).unwrap());
    let mut first = TypeChecker::new_with_core_snapshot(snapshot.clone());
    let mut second = TypeChecker::new_with_core_snapshot(snapshot.clone());
    load(&mut first, "Container");
    assert_eq!(item(&second, "Container"), None);
    load(&mut second, "Container");
    assert_eq!(item(&second, "Container"), Some(Type::Int));
    assert_eq!(first.metrics().metadata_impl_index_entries, 0);
    assert_eq!(second.metrics().metadata_impl_index_entries, 0);
    assert_eq!(first.metrics().metadata_impl_candidates, 1);
    assert_eq!(second.metrics().metadata_impl_candidates, 1);
    let mut changed = descriptor();
    changed
        .associated_types
        .insert("Item".into(), "Bool".into());
    second.set_core_metadata(Arc::new(metadata(List::from_iter([changed]))));
    // Installing another artifact must discard the paired old position index.
    load(&mut second, "exports.First");
    assert_eq!(second.metrics().metadata_impl_index_entries, 1);
    let cancelled = AnalysisControl::default();
    cancelled.cancel();
    assert!(CoreMetadataSnapshot::prepare(snapshot.metadata().clone(), &cancelled).is_err());
}
