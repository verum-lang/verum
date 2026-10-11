//! T1212: mounted static methods keep their descriptor's exact declaring owner.
//!
//! Unlike the source-only homonym controls, both metadata declarations here
//! carry a real `new` signature, including its parent and nominal return. Empty
//! registry ASTs force the archive-consumer path; JSON decoding exercises the
//! metadata data model, not a VBC producer or an ordinary CLI bake.

use std::sync::Arc;
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Maybe, ResourceDiscipline, Set, Text};
use verum_fast_parser::{FastParser, Parser};
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::{Type, TypeChecker, core_metadata::*};

const ALPHA: &str = "core.alpha.Builder";
const BETA: &str = "core.beta.Builder";

fn declaration(owner: &str, public: bool) -> TypeDescriptor {
    TypeDescriptor {
        name: "Builder".into(),
        module_path: "core.bundle".into(),
        origin_module_path: Maybe::Some(owner.into()),
        generic_params: List::new(),
        kind: TypeDescriptorKind::Record {
            // Identical shapes must still remain nominally distinct.
            fields: List::from_iter([FieldDescriptor {
                name: "value".into(),
                ty: "Int".into(),
                is_public: true,
                declared_visibility: Maybe::Some(verum_ast::Visibility::Public),
            }]),
        },
        size: Maybe::Some(8),
        alignment: Maybe::Some(8),
        methods: List::from_iter(["new".into(), "read".into()]),
        implements: List::new(),
        decl_span: Maybe::None,
        is_public: public,
        is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
    }
}

fn method(owner: &str, name: &str, public: bool) -> FunctionDescriptor {
    FunctionDescriptor {
        name: format!("Builder.{name}").into(),
        module_path: "core.bundle".into(),
        origin_module_path: Maybe::Some(owner.into()),
        generic_params: List::new(),
        params: if name == "new" {
            List::new()
        } else {
            List::from_iter([ParamDescriptor {
                name: "self".into(),
                ty: "&Builder".into(),
                declared_ty: "&Builder".into(),
                has_default: false,
                default_literal: Maybe::None,
            }])
        },
        return_type: if name == "new" {
            "Builder"
        } else if owner == "core.beta" {
            "Bool"
        } else {
            "Int"
        }
        .into(),
        contexts: List::new(),
        is_async: false,
        is_unsafe: false,
        intrinsic_id: Maybe::None,
        parent_type: Maybe::Some(format!("{owner}.Builder").into()),
        impl_generic_names: List::new(),
        is_const: false,
        decl_span: Maybe::None,
        is_public: public,
        explicit_type_param_ids: Maybe::None,
    }
}

fn metadata(reverse: bool, beta_public: bool) -> CoreMetadata {
    let mut metadata = CoreMetadata::default();
    let owners = if reverse {
        ["core.beta", "core.alpha"]
    } else {
        ["core.alpha", "core.beta"]
    };
    for owner in owners {
        let public = owner == "core.alpha" || beta_public;
        let descriptor = declaration(owner, public);
        let key: Text = format!("{owner}.Builder").into();
        metadata.types.insert(key.clone(), descriptor);
        metadata.type_declaration_order.push(key);
        for name in ["new", "read"] {
            metadata.functions.insert(
                format!("{owner}.Builder.{name}").into(),
                method(owner, name, public),
            );
        }
    }
    // The global leaf belongs to the foreign declaration in both load orders.
    metadata
        .types
        .insert("Builder".into(), declaration("core.beta", beta_public));
    for name in ["new", "read"] {
        metadata.functions.insert(
            format!("Builder.{name}").into(),
            method("core.beta", name, beta_public),
        );
    }
    metadata
}

fn checker(
    metadata: CoreMetadata,
    eager: bool,
    decoded: bool,
    sources: &[(&str, &str)],
) -> TypeChecker {
    let metadata = if decoded {
        serde_json::from_slice(&serde_json::to_vec(&metadata).unwrap()).unwrap()
    } else {
        metadata
    };
    let mut registry = ModuleRegistry::new();
    let mut owners: Set<Text> = metadata
        .types
        .values()
        .map(|descriptor| {
            descriptor
                .origin_module_path
                .as_ref()
                .unwrap_or(&descriptor.module_path)
                .clone()
        })
        .collect();
    owners.insert("core.missing".into());
    for (owner, _) in sources {
        owners.insert((*owner).into());
    }
    let mut owners: List<Text> = owners.into_iter().collect();
    owners.sort();
    for (index, owner) in owners.iter().enumerate() {
        let source = sources
            .iter()
            .find(|(candidate, _)| *candidate == owner.as_str())
            .map(|(_, source)| *source)
            .unwrap_or("");
        let file_id = FileId::new(index as u32);
        let ast = FastParser::new()
            .parse_module_str(source, file_id)
            .expect("declaration grammar");
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), file_id, source.into());
        info.exports = extract_exports_from_module(&ast, id, &path).expect("declaration exports");
        registry.register(info);
    }
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(Arc::new(metadata))
    } else {
        TypeChecker::new_with_core(Arc::new(metadata))
    };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry);
    checker
}

fn check_source(checker: &mut TypeChecker, source: &str) -> List<Text> {
    let module = FastParser::new()
        .parse_module_str(source, FileId::new(100))
        .expect("consumer grammar");
    checker.register_stdlib_types_for_module(&module);
    let mut errors = List::new();
    for item in &module.items {
        if matches!(item.kind, ItemKind::Mount(_)) {
            if let Err(error) = checker.check_item(item) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
    for item in &module.items {
        if let ItemKind::Function(function) = &item.kind {
            if let Err(error) = checker.register_function_signature(function) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
    for item in &module.items {
        if !matches!(item.kind, ItemKind::Mount(_)) {
            if let Err(error) = checker.check_item(item) {
                errors.push(format!("{error:?}").into());
            }
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

fn expression_type(checker: &mut TypeChecker, source: &str) -> Result<Type, Text> {
    let expression = Parser::new(source)
        .parse_expr()
        .expect("expression grammar");
    checker
        .synth_expr(&expression)
        .map(|result| result.ty)
        .map_err(|error| format!("{error:?}").into())
}

fn assert_owner(checker: &mut TypeChecker, expression: &str, owner: &str) {
    let ty = expression_type(checker, expression)
        .unwrap_or_else(|error| panic!("{expression}: {error}"));
    let Type::Named { path, args } = ty else {
        panic!("{expression}: not a nominal type: {ty:?}")
    };
    assert!(args.is_empty());
    let actual: Text = path
        .segments
        .iter()
        .map(|segment| match segment {
            verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
            _ => panic!("unexpected owner path: {path:?}"),
        })
        .collect::<List<_>>()
        .join(".")
        .into();
    assert_eq!(actual.as_str(), owner, "{expression}");
}

fn mounted_owner(mount: &str, expression: &str) {
    for decoded in [false, true] {
        for eager in [false, true] {
            for reverse in [false, true] {
                let mut checker = checker(metadata(reverse, false), eager, decoded, &[]);
                let errors = check_source(&mut checker, mount);
                assert!(
                    errors.is_empty(),
                    "{mount} decoded={decoded} eager={eager} reverse={reverse}: {errors:?}"
                );
                assert_owner(&mut checker, expression, ALPHA);
            }
        }
    }
}

#[test]
fn dotted_mount_selects_exact_metadata_constructor() {
    mounted_owner("mount core.alpha.Builder;", "Builder.new()");
}

#[test]
fn braced_mount_selects_exact_metadata_constructor() {
    mounted_owner("mount core.alpha.{Builder};", "Builder.new()");
}

#[test]
fn renamed_mounts_select_exact_metadata_constructor() {
    for mount in [
        "mount core.alpha.Builder as Chosen;",
        "mount core.alpha.{Builder as Chosen};",
    ] {
        mounted_owner(mount, "Chosen.new()");
    }
}

#[test]
fn alternating_metadata_owners_do_not_share_static_cache_entries() {
    for decoded in [false, true] {
        for eager in [false, true] {
            for reverse in [false, true] {
                let mut checker = checker(metadata(reverse, true), eager, decoded, &[]);
                let mounts = if reverse {
                    "mount core.beta.Builder as B; mount core.alpha.Builder as A;"
                } else {
                    "mount core.alpha.Builder as A; mount core.beta.Builder as B;"
                };
                assert!(check_source(&mut checker, mounts).is_empty());
                let order = if reverse {
                    [("B.new()", BETA), ("A.new()", ALPHA)]
                } else {
                    [("A.new()", ALPHA), ("B.new()", BETA)]
                };
                for _ in 0..3 {
                    for (expression, owner) in order {
                        assert_owner(&mut checker, expression, owner);
                    }
                }
            }
        }
    }
}

#[test]
fn inferred_local_keeps_the_mounted_owner_through_instance_method_lookup() {
    for decoded in [false, true] {
        let mut checker = checker(metadata(false, false), false, decoded, &[]);
        let errors = check_source(
            &mut checker,
            "mount core.alpha.Builder; fn probe() -> Int { let value = Builder.new(); value.read() }",
        );
        assert!(errors.is_empty(), "{errors:?}");
    }
}

#[test]
fn source_mount_keeps_its_constructor_against_real_metadata_method_homonym() {
    let source = "public type Builder is { value: Int }; implement Builder { public fn new() -> Builder { Builder { value: 37 } } public fn read(&self) -> Int { self.value } }";
    for eager in [false, true] {
        for decoded in [false, true] {
            let mut checker = checker(
                metadata(false, false),
                eager,
                decoded,
                &[("local.owned", source)],
            );
            let errors = check_source(&mut checker, "mount local.owned.Builder;");
            assert!(errors.is_empty(), "{errors:?}");
            assert_owner(&mut checker, "Builder.new()", "local.owned.Builder");
        }
    }
}

#[test]
fn missing_exact_constructor_cannot_use_another_owners_bare_method() {
    for decoded in [false, true] {
        for eager in [false, true] {
            let mut metadata = metadata(false, true);
            metadata.functions.remove(&"core.alpha.Builder.new".into());
            let mut checker = checker(metadata, eager, decoded, &[]);
            assert!(check_source(&mut checker, "mount core.alpha.Builder;").is_empty());
            assert!(
                expression_type(&mut checker, "Builder.new()").is_err(),
                "foreign constructor filled a qualified hole"
            );
        }
    }
}

#[test]
fn missing_and_private_owners_remain_refused_despite_public_bare_constructor() {
    for owner in ["core.missing", "core.beta"] {
        for decoded in [false, true] {
            let mut metadata = metadata(false, false);
            metadata
                .functions
                .insert("Builder.new".into(), method("core.alpha", "new", true));
            let mut checker = checker(metadata, false, decoded, &[]);
            let errors = check_source(&mut checker, &format!("mount {owner}.Builder;"));
            let result = expression_type(&mut checker, "Builder.new()");
            assert!(
                !errors.is_empty() || result.is_err(),
                "{owner}: private/missing mount and call accepted: {result:?}"
            );
        }
    }
}

#[test]
fn same_shaped_metadata_owners_remain_nominally_distinct() {
    for decoded in [false, true] {
        let mut checker = checker(metadata(false, true), false, decoded, &[]);
        let errors = check_source(
            &mut checker,
            "mount core.alpha.{Builder as A}; mount core.beta.{Builder as B}; fn wrong() -> A { B.new() }",
        );
        assert!(
            errors.iter().any(|error| error.contains("Mismatch")),
            "{errors:?}"
        );
    }
}

#[test]
fn changing_the_public_bare_winner_does_not_change_either_declared_owner() {
    for reverse in [false, true] {
        for eager in [false, true] {
            let mut metadata = metadata(reverse, true);
            metadata
                .types
                .insert("Builder".into(), declaration("core.alpha", true));
            for name in ["new", "read"] {
                metadata.functions.insert(
                    format!("Builder.{name}").into(),
                    method("core.alpha", name, true),
                );
            }
            let mut checker = checker(metadata, eager, true, &[]);
            let errors = check_source(
                &mut checker,
                "mount core.alpha.Builder as A; mount core.beta.Builder as B;",
            );
            assert!(errors.is_empty(), "{errors:?}");
            assert_owner(&mut checker, "B.new()", BETA);
            assert_owner(&mut checker, "A.new()", ALPHA);
            assert_owner(&mut checker, "B.new()", BETA);
        }
    }
}

#[test]
fn matching_legacy_bare_method_keeps_its_declaring_owner() {
    for decoded in [false, true] {
        for origin_is_present in [false, true] {
            let mut metadata = CoreMetadata::default();
            let mut descriptor = declaration("core.alpha", true);
            descriptor.module_path = "core.alpha".into();
            if !origin_is_present {
                descriptor.origin_module_path = Maybe::None;
            }
            metadata.types.insert(ALPHA.into(), descriptor.clone());
            metadata.types.insert("Builder".into(), descriptor);
            metadata.type_declaration_order.push(ALPHA.into());
            let mut constructor = method("core.alpha", "new", true);
            constructor.module_path = "core.alpha".into();
            constructor.parent_type = Maybe::Some("Builder".into());
            if !origin_is_present {
                constructor.origin_module_path = Maybe::None;
            }
            metadata.functions.insert("Builder.new".into(), constructor);
            let mut checker = checker(metadata, false, decoded, &[]);
            assert!(check_source(&mut checker, "mount core.alpha.Builder;").is_empty());
            assert_owner(&mut checker, "Builder.new()", ALPHA);
            assert_owner(&mut checker, "Builder.new()", ALPHA);
        }
    }
}

#[test]
fn method_owner_is_independent_of_its_return_type() {
    for decoded in [false, true] {
        let mut metadata = metadata(false, true);
        metadata
            .functions
            .get_mut(&"core.alpha.Builder.new".into())
            .unwrap()
            .return_type = "Int".into();
        metadata
            .functions
            .get_mut(&"core.beta.Builder.new".into())
            .unwrap()
            .return_type = "Bool".into();
        metadata
            .functions
            .get_mut(&"Builder.new".into())
            .unwrap()
            .return_type = "Bool".into();
        let mut checker = checker(metadata, true, decoded, &[]);
        assert!(
            check_source(
                &mut checker,
                "mount core.alpha.Builder as A; mount core.beta.Builder as B;"
            )
            .is_empty()
        );
        assert_eq!(expression_type(&mut checker, "A.new()").unwrap(), Type::Int);
        assert_eq!(
            expression_type(&mut checker, "B.new()").unwrap(),
            Type::Bool
        );
        assert_eq!(expression_type(&mut checker, "A.new()").unwrap(), Type::Int);
    }
}

#[test]
fn exact_function_index_cannot_replace_its_declared_parent() {
    for decoded in [false, true] {
        let mut metadata = metadata(false, true);
        metadata.functions.insert(
            "core.alpha.Builder.new".into(),
            method("core.beta", "new", true),
        );
        let mut checker = checker(metadata, false, decoded, &[]);
        assert!(check_source(&mut checker, "mount core.alpha.Builder;").is_empty());
        assert!(
            expression_type(&mut checker, "Builder.new()").is_err(),
            "a foreign parent was accepted under an exact function index"
        );
    }
}

#[test]
fn later_method_chain_uses_the_declared_return_owner() {
    for decoded in [false, true] {
        let mut metadata = metadata(false, true);
        metadata
            .functions
            .get_mut(&"core.alpha.Builder.new".into())
            .unwrap()
            .return_type = BETA.into();
        let mut checker = checker(metadata, false, decoded, &[]);
        assert!(check_source(&mut checker, "mount core.alpha.Builder;").is_empty());
        assert_eq!(
            expression_type(&mut checker, "Builder.new().read()").unwrap(),
            Type::Bool
        );
    }
}
