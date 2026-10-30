use super::*;
use crate::{
    AnalysisControl, CoreMetadataSnapshot, core_metadata::*, metadata_snapshot::MetadataNameIndex,
};
use std::sync::Arc;

fn function(key: &str) -> FunctionDescriptor {
    let (owner, name) = key.rsplit_once('.').unwrap_or(("", key));
    FunctionDescriptor {
        name: name.into(),
        module_path: owner.into(),
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
        is_public: true,
        explicit_type_param_ids: None,
    }
}

fn metadata(keys: &[&str]) -> CoreMetadata {
    let mut data = CoreMetadata::default();
    for key in keys {
        data.functions.insert((*key).into(), function(key));
        data.types.insert(
            (*key).into(),
            TypeDescriptor {
                name: (*key).into(),
                module_path: "bundle".into(),
                origin_module_path: None,
                generic_params: List::new(),
                kind: TypeDescriptorKind::Alias {
                    target: "Int".into(),
                },
                size: None,
                alignment: None,
                methods: List::new(),
                implements: List::new(),
                decl_span: None,
                is_public: false,
                is_transparent_wrapper: false,
                resource_discipline: verum_common::ResourceDiscipline::Unrestricted,
            },
        );
    }
    data
}

#[test]
fn name_index_matches_scan_for_dotted_names_boundaries_and_aliases() {
    let mut data = metadata(&[
        "value",
        ".value",
        "",
        "a",
        "a.",
        "a.value",
        "a.b.value",
        "a.bb.value",
        "a.b.nested.value",
        "a.B.value",
        "ab.b.value",
        "a._b.value",
        "a.λ.value",
        "a.b.項",
        "a.b..value",
        "a.b.valueExtra",
        "a.b.Item",
        "b.Item",
    ]);
    // Many distinct owners and matching leaf aliases must remain distinct.
    for n in 0..128 {
        let key = format!("a.module_{n}.value");
        data.functions.insert(key.clone().into(), function(&key));
    }
    let index = MetadataNameIndex::prepare(&data, None).unwrap();
    for prefix in ["", "a", "a.b", "ab", "a._b", "a.λ", "absent"] {
        for item in [
            "",
            "value",
            "nested.value",
            "b.value",
            ".value",
            "Item",
            "項",
            "absent",
        ] {
            let indexed: List<_> = index.function_keys_between(prefix, item).collect();
            let reference = data.function_keys_between(prefix, item);
            assert_eq!(
                indexed.as_slice(),
                reference.as_slice(),
                "{prefix} / {item}"
            );
            let suffix = format!(".{item}");
            let expected: List<_> = data
                .types
                .keys()
                .filter(|key| key.as_str().ends_with(&suffix))
                .collect();
            assert_eq!(
                index.type_keys_ending_with(item).collect::<List<_>>(),
                expected
            );
        }
    }
    let checker = TypeChecker::new_with_core(Arc::new(data));
    assert!(
        checker
            .mounted_type_name_is_contested("Item", "a.b")
            .unwrap()
    );
    assert!(
        !checker
            .mounted_type_name_is_contested("Item", "missing")
            .unwrap()
    );
}

#[test]
fn equal_length_replacement_does_not_reuse_old_name_buckets() {
    let original = Arc::new(metadata(&["pkg.child.old"]));
    let snapshot =
        Arc::new(CoreMetadataSnapshot::prepare(original, &AnalysisControl::default()).unwrap());
    let mut first = TypeChecker::new_with_core_snapshot(snapshot.clone());
    let mut second = TypeChecker::new_with_core_snapshot(snapshot);
    assert!(Arc::ptr_eq(
        &first.metadata_name_index().unwrap(),
        &second.metadata_name_index().unwrap()
    ));
    assert!(
        first
            .resolve_value_via_metadata_reexports("pkg", "old")
            .is_some()
    );
    second.set_core_metadata(Arc::new(metadata(&["pkg.child.new"])));
    assert!(
        second
            .resolve_value_via_metadata_reexports("pkg", "new")
            .is_some()
    );
    assert!(
        second
            .resolve_value_via_metadata_reexports("pkg", "old")
            .is_none()
    );
    assert!(
        first
            .resolve_value_via_metadata_reexports("pkg", "old")
            .is_some()
    );
    assert!(
        first
            .resolve_value_via_metadata_reexports("pkg", "new")
            .is_none()
    );
}

#[test]
fn onehop_keeps_shape_order_and_visibility_refusal() {
    let mut data = metadata(&[
        "pkg.alpha.pick",
        "pkg.beta.pick",
        "pkg.gamma.pick",
        "pkg.Type.pick",
    ]);
    data.functions
        .get_mut(&Text::from("pkg.alpha.pick"))
        .unwrap()
        .return_type = "__opaque_1".into();
    data.functions
        .get_mut(&Text::from("pkg.beta.pick"))
        .unwrap()
        .is_public = false;
    // The earliest fully typed candidate wins BEFORE visibility checking.
    // A later public homonym must not bypass that refusal.
    let mut checker = TypeChecker::new_with_core(Arc::new(data.clone()));
    assert!(
        checker
            .resolve_value_via_metadata_reexports("pkg", "pick")
            .is_none()
    );
    data.functions
        .get_mut(&Text::from("pkg.beta.pick"))
        .unwrap()
        .is_public = true;
    checker.set_core_metadata(Arc::new(data));
    let (_, owner) = checker
        .resolve_value_via_metadata_reexports("pkg", "pick")
        .unwrap();
    assert_eq!(owner.as_str(), "pkg.beta.pick");
    let mut checker = TypeChecker::new_with_core(Arc::new(metadata(&[
        "pkg.Type.pick",
        "pkg.child.deep.pick",
    ])));
    assert!(
        checker
            .resolve_value_via_metadata_reexports("pkg", "pick")
            .is_none()
    );
    assert!(
        checker
            .resolve_value_via_metadata_reexports("pkg", "deep.pick")
            .is_some()
    );
}

#[test]
fn shared_names_do_not_share_generic_scheme_variables() {
    let mut data = metadata(&["pkg.child.identity"]);
    let fd = data
        .functions
        .get_mut(&Text::from("pkg.child.identity"))
        .unwrap();
    fd.generic_params.push(GenericParam {
        name: "T".into(),
        bounds: List::new(),
        default: None,
        type_bounds: List::new(),
        pid: Some(0),
    });
    fd.params.push(ParamDescriptor {
        name: "value".into(),
        ty: "__generic_0".into(),
        declared_ty: "T".into(),
        has_default: false,
        default_literal: None,
    });
    fd.return_type = "__generic_0".into();
    fd.explicit_type_param_ids = Some(List::from_iter([Some(0)]));
    let snapshot = Arc::new(
        CoreMetadataSnapshot::prepare(Arc::new(data), &AnalysisControl::default()).unwrap(),
    );
    for _ in 0..2 {
        let mut checker = TypeChecker::new_with_core_snapshot(snapshot.clone());
        let (scheme, _) = checker
            .resolve_value_via_metadata_reexports("pkg", "identity")
            .unwrap();
        let Type::Function {
            params,
            return_type,
            ..
        } = &scheme.ty
        else {
            panic!("function scheme")
        };
        assert_eq!(scheme.vars.len(), 1, "{scheme:?}");
        assert_eq!(params.as_slice(), &[Type::Var(scheme.vars[0])]);
        assert_eq!(return_type.as_ref(), &Type::Var(scheme.vars[0]));
        checker.ctx.env.insert_value("identity", scheme);
        let source = verum_fast_parser::Parser::new("fn check() -> Int { identity(42) }")
            .parse_module()
            .unwrap();
        checker.check_item(&source.items[0]).unwrap();
    }
}

#[test]
fn cancelled_index_never_becomes_a_missing_import_verdict() {
    let mut checker = TypeChecker::new_with_core(Arc::new(metadata(&["pkg.child.value"])));
    let control = AnalysisControl::default();
    control.cancel();
    checker.set_analysis_control(control);
    assert!(matches!(
        checker.metadata_name_index(),
        Err(TypeError::AnalysisCancelled)
    ));
    assert!(checker.metadata_names.get().is_none());
    let source = verum_fast_parser::Parser::new("mount pkg.value;")
        .parse_module()
        .unwrap();
    assert!(matches!(
        checker
            .prepare_module_imports(&source, "consumer")
            .as_slice(),
        [TypeError::AnalysisCancelled]
    ));
}
