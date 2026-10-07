//! Explicit source slots select carried IDs, including non-type holes and legacy absence.
use std::sync::Arc;
use verum_common::{List, Set, Text};
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, core_metadata::*};

fn descriptor(owner: &str, id: u16, slots: Option<List<Option<u16>>>) -> FunctionDescriptor {
    FunctionDescriptor {
        name: "choose".into(),
        module_path: owner.into(),
        origin_module_path: None,
        generic_params: List::from_iter([GenericParam {
            name: "T".into(),
            bounds: List::new(),
            default: None,
            type_bounds: List::new(),
            pid: Some(id),
        }]),
        params: List::from_iter([ParamDescriptor {
            name: "value".into(),
            ty: format!("__generic_{id}").into(),
            declared_ty: "T".into(),
            has_default: false,
            default_literal: None,
        }]),
        return_type: format!("__generic_{id}").into(),
        contexts: List::new(),
        is_async: false,
        is_unsafe: false,
        intrinsic_id: None,
        parent_type: None,
        impl_generic_names: List::new(),
        is_const: false,
        decl_span: None,
        is_public: true,
        explicit_type_param_ids: slots,
    }
}
fn errors(metadata: CoreMetadata, source: &str, eager: bool) -> List<Text> {
    let metadata: CoreMetadata =
        serde_json::from_slice(&serde_json::to_vec(&metadata).unwrap()).unwrap();
    // Archive imports supply module identities independently of source ASTs.
    // Empty declarations deliberately force the checker to use the metadata.
    let mut registry = verum_modules::ModuleRegistry::new();
    let owners: Set<Text> = metadata
        .functions
        .values()
        .map(|fd| fd.module_path.clone())
        .collect();
    for (index, owner) in owners.iter().enumerate() {
        registry.register(verum_modules::ModuleInfo::new(
            verum_modules::ModuleId::new(index as u32),
            verum_modules::ModulePath::from_str(owner),
            Parser::new("").parse_module().unwrap(),
            verum_ast::FileId::new(index as u32),
            Text::new(),
        ));
    }
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(Arc::new(metadata))
    } else {
        TypeChecker::new_with_core(Arc::new(metadata))
    };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry);
    let ast = Parser::new(source).parse_module().expect("source grammar");
    checker.register_stdlib_types_for_module(&ast);
    let mut errors: List<Text> = ast
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|e| format!("{e:?}").into())
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| format!("{e:?}").into()),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| format!("{e:?}").into()),
    );
    errors
}
fn metadata(id: u16, slots: Option<List<Option<u16>>>) -> CoreMetadata {
    let mut metadata = CoreMetadata::default();
    metadata.functions.insert(
        "core.alpha.choose".into(),
        descriptor("core.alpha", id, slots),
    );
    metadata
}
#[test]
fn imported_slots_bind_carried_id_after_non_type_hole() {
    for eager in [false, true] {
        for id in [0, 7, 0x8000] {
            let metadata = metadata(id, Some(List::from_iter([None, Some(id)])));
            let good = "fn probe()->Bool { core.alpha.choose<[2; 3], Bool>(true) }";
            assert!(
                errors(metadata.clone(), good, eager).is_empty(),
                "id={id} eager={eager}: {:?}",
                errors(metadata.clone(), good, eager)
            );
            let valid_array = "fn probe()->[Byte; 3] { core.alpha.choose<[2; 3], [Byte; 3]>([1 as Byte, 2 as Byte, 3 as Byte]) }";
            assert!(
                errors(metadata.clone(), valid_array, eager).is_empty(),
                "{valid_array}: {:?}",
                errors(metadata.clone(), valid_array, eager)
            );
            let bad = "fn probe()->Bool { core.alpha.choose<[2; 3], [Byte; 3]>(true) }";
            assert!(
                !errors(metadata, bad, eager).is_empty(),
                "array type must constrain imported value, id={id}"
            );
        }
    }
}
#[test]
fn absent_or_invalid_roster_does_not_guess_bracket_role() {
    for slots in [None, Some(List::from_iter([Some(9)]))] {
        let metadata = metadata(7, slots);
        let source = "fn probe()->Bool { core.alpha.choose<[Byte; 3]>(true) }";
        let errors = errors(metadata, source, false);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("declaration-owned slot metadata")),
            "{errors:?}"
        );
    }
}
#[test]
fn missing_json_field_remains_unknown() {
    let fd = descriptor("core.alpha", 7, Some(List::from_iter([Some(7)])));
    let mut json = serde_json::to_value(fd).unwrap();
    json.as_object_mut()
        .unwrap()
        .remove("explicit_type_param_ids");
    let fd: FunctionDescriptor = serde_json::from_value(json).unwrap();
    assert!(fd.explicit_type_param_ids.is_none());
}

#[test]
fn sibling_and_renamed_imports_keep_their_own_slot_rosters() {
    for eager in [false, true] {
        for reverse in [false, true] {
            let alpha = descriptor("core.alpha", 7, Some(List::from_iter([None, Some(7)])));
            let beta = descriptor("core.beta", 0x8000, Some(List::from_iter([Some(0x8000)])));
            let mut metadata = CoreMetadata::default();
            let entries = if reverse {
                [("core.beta.choose", beta), ("core.alpha.choose", alpha)]
            } else {
                [("core.alpha.choose", alpha), ("core.beta.choose", beta)]
            };
            for (key, fd) in entries {
                metadata.functions.insert(key.into(), fd);
            }
            for (prefix, call) in [
                ("", "core.alpha.choose<[2; 3], [Byte; 3]>(true)"),
                (
                    "mount core.alpha.{choose};",
                    "choose<[2; 3], [Byte; 3]>(true)",
                ),
                (
                    "mount core.alpha.{choose as selected};",
                    "selected<[2; 3], [Byte; 3]>(true)",
                ),
                ("", "core.beta.choose<[Byte; 3]>(true)"),
            ] {
                let bad = format!("{prefix} fn probe()->Bool {{ {call} }}");
                let good = bad.replace("[Byte; 3]", "Bool");
                assert!(
                    errors(metadata.clone(), &good, eager).is_empty(),
                    "{good}: {:?}",
                    errors(metadata.clone(), &good, eager)
                );
                assert!(
                    !errors(metadata.clone(), &bad, eager).is_empty(),
                    "wrong array value accepted: {bad}"
                );
            }
        }
    }
}

#[test]
fn duplicate_carried_ids_do_not_prove_slot_identity() {
    let mut metadata = metadata(7, Some(List::from_iter([Some(7)])));
    let fd = metadata
        .functions
        .get_mut(&Text::from("core.alpha.choose"))
        .unwrap();
    fd.generic_params.push(GenericParam {
        name: "U".into(),
        bounds: List::new(),
        default: None,
        type_bounds: List::new(),
        pid: Some(7),
    });
    let problems = errors(
        metadata,
        "fn probe()->Bool {core.alpha.choose<[Byte; 3]>(true)}",
        false,
    );
    assert!(
        problems
            .iter()
            .any(|e| e.contains("declaration-owned slot metadata")),
        "{problems:?}"
    );
}
