#![cfg(feature = "codegen")]

use std::collections::HashMap;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen, context::FunctionInfo};
use verum_vbc::module::VbcModule;
use verum_vbc::types::{TypeId, TypeParamId, TypeRef};

fn ast(text: &str) -> verum_ast::Module {
    Parser::new(text).parse_module().expect("source grammar")
}

fn producer(owner: &str, offset: u32) -> (VbcModule, HashMap<String, FunctionInfo>) {
    let source = ast(&format!(
        r#"
module {owner}.api;
type Leaf is {{ value: Int }};
type Envelope<T> is {{ item: T }};
type Secret is {{ token: Text }};
type Unused is {{ extra: Int }};
fn supply() -> Envelope<Leaf> {{ Envelope(item: Leaf(value: 7)) }}
fn accept(secret: Secret) -> Int {{ 7 }}
fn* stream() -> Leaf {{ yield Leaf(value: 9); }}
fn identity<T>(value: T) -> T {{ value }}
fn unused() -> Unused {{ Unused(extra: 0) }}
"#
    ));
    let mut cg = VbcCodegen::with_config(CodegenConfig::new(owner));
    let module = cg.compile_module(&source).expect("producer");
    let mut registry = cg.export_functions();
    // The real bootstrap owns a global function allocator; each archived
    // module has independently dense local IDs. Simulate that separation.
    for info in registry.values_mut().filter(|info| info.id.0 < 100_000) {
        info.id.0 += offset;
    }
    (module, registry)
}

fn nominal(module: &VbcModule, name: &str) -> TypeId {
    module
        .types
        .iter()
        .find(|ty| module.strings.get(ty.name) == Some(name))
        .expect(name)
        .id
}

fn consumer(
    source: &str,
    registries: &[&HashMap<String, FunctionInfo>],
) -> (VbcCodegen, verum_ast::Module) {
    let source = ast(source);
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    for i in 0..100 {
        cg.ctx_mut().intern_string_raw(&format!("other_pool_{i}"));
    }
    for registry in registries {
        cg.import_functions(registry);
    }
    (cg, source)
}

#[test]
fn mounted_and_qualified_free_returns_use_only_the_declaring_module_in_both_orders() {
    let (alpha, ar) = producer("alpha", 1000);
    let (beta, br) = producer("beta", 2000);
    for reverse in [false, true] {
        for invocation in [
            "mount alpha.api.supply as make; fn probe() -> Int { let item = make(); 7 }",
            "fn probe() -> Int { let item = alpha.api.supply(); 7 }",
        ] {
            let (mut cg, source) = consumer(&format!("module consumer; {invocation}"), &[&br, &ar]);
            let beta_before = cg.ctx_mut().functions["beta.api.supply"]
                .return_type
                .clone();
            let available = if reverse {
                [&beta, &alpha]
            } else {
                [&alpha, &beta]
            };
            let count = cg
                .import_bootstrap_nominal_dependencies(&[&source], &available)
                .unwrap();
            assert_eq!(count, 2, "only the referenced signature's nominal closure");
            let carried = cg.ctx_mut().functions["alpha.api.supply"]
                .return_type
                .clone()
                .unwrap();
            assert_eq!(
                cg.ctx_mut().functions["beta.api.supply"].return_type,
                beta_before,
                "unselected sibling metadata stays in its source pool"
            );
            cg.collect_unit_declarations(&[&source]).unwrap();
            let result = cg.compile_function_bodies(&source).unwrap();
            assert_eq!(
                carried,
                TypeRef::Instantiated {
                    base: nominal(&result, "Envelope"),
                    args: vec![TypeRef::Concrete(nominal(&result, "Leaf"))]
                }
            );
            assert!(
                result
                    .types
                    .iter()
                    .all(|ty| result.strings.get(ty.name) != Some("Unused"))
            );
            assert!(
                result
                    .types
                    .iter()
                    .filter_map(|ty| ty.origin_module.and_then(|id| result.strings.get(id)))
                    .all(|owner| owner != "beta.api")
            );
        }
    }
}

#[test]
fn parameter_and_yield_only_nominals_enter_the_same_dependency_closure() {
    let (mut alpha, mut registry) = producer("alpha", 1000);
    // AST generator compilation currently leaves yield_type absent. Model a
    // populated archive carrier without claiming to fix yield inference here.
    let stream = alpha
        .functions
        .iter_mut()
        .find(|f| {
            alpha
                .strings
                .get(f.name)
                .is_some_and(|name| name.ends_with("stream"))
        })
        .unwrap();
    stream.yield_type = Some(stream.return_type.clone());
    registry.get_mut("alpha.api.stream").unwrap().yield_type = stream.yield_type.clone();
    let (mut cg, source) = consumer(
        "module consumer; mount alpha.api.{accept, stream}; fn probe() -> Int { 7 }",
        &[&registry],
    );
    assert_eq!(
        cg.import_bootstrap_nominal_dependencies(&[&source], &[&alpha])
            .unwrap(),
        2
    );
    let carried = cg.ctx_mut().functions["alpha.api.stream"]
        .yield_type
        .clone()
        .expect("generator yield fact");
    cg.collect_unit_declarations(&[&source]).unwrap();
    let result = cg.compile_function_bodies(&source).unwrap();
    assert_eq!(carried, TypeRef::Concrete(nominal(&result, "Leaf")));
    assert_ne!(nominal(&result, "Secret"), TypeId::PTR);
    assert!(
        result
            .types
            .iter()
            .all(|ty| result.strings.get(ty.name) != Some("Envelope"))
    );
}

#[test]
fn missing_qualified_free_function_never_uses_a_same_leaf_or_bare_registry_entry() {
    let (alpha, registry) = producer("alpha", 1000);
    for text in [
        "module consumer; mount alpha.child.api.supply; fn probe() -> Int { 7 }",
        "module consumer; fn probe() -> Int { let item = supply(); 7 }",
    ] {
        let (mut cg, source) = consumer(text, &[&registry]);
        assert_eq!(
            cg.import_bootstrap_nominal_dependencies(&[&source], &[&alpha])
                .unwrap(),
            0
        );
    }
}

#[test]
fn a_declared_free_function_generic_stays_generic() {
    let (alpha, registry) = producer("alpha", 1000);
    let (mut cg, source) = consumer(
        "module consumer; mount alpha.api.identity; fn probe() -> Int { 7 }",
        &[&registry],
    );
    assert_eq!(
        cg.import_bootstrap_nominal_dependencies(&[&source], &[&alpha])
            .unwrap(),
        0
    );
    assert_eq!(
        cg.ctx_mut().functions["alpha.api.identity"].return_type,
        Some(TypeRef::Generic(TypeParamId(0)))
    );
}

#[test]
fn unknown_user_type_in_selected_free_signature_is_rejected() {
    let (mut alpha, registry) = producer("alpha", 1000);
    alpha
        .functions
        .iter_mut()
        .find(|f| {
            alpha
                .strings
                .get(f.name)
                .is_some_and(|name| name.ends_with("supply"))
        })
        .unwrap()
        .return_type = TypeRef::Concrete(TypeId(999_999));
    let (mut cg, source) = consumer(
        "module consumer; mount alpha.api.supply; fn probe() -> Int { 7 }",
        &[&registry],
    );
    let error = cg
        .import_bootstrap_nominal_dependencies(&[&source], &[&alpha])
        .expect_err("foreign ID must not pass through");
    assert!(
        error.to_string().contains("unknown source TypeId 999999"),
        "{error}"
    );
}

#[test]
fn bootstrap_signature_updates_follow_live_alias_ids_across_repeated_imports() {
    let (alpha, ar) = producer("alpha", 1000);
    let (beta, br) = producer("beta", 2000);
    let target = ar["alpha.api.supply"].clone();
    let foreign = br["beta.api.supply"].clone();
    for reverse in [false, true] {
        let registries = if reverse { [&ar, &br] } else { [&br, &ar] };
        let available = if reverse {
            [&alpha, &beta]
        } else {
            [&beta, &alpha]
        };
        let (mut cg, source) = consumer(
            "module consumer; mount alpha.api.supply as make; fn probe() -> Int { 7 }",
            &registries,
        );
        for alias in ["supply", "supply#0", "make", "consumer.make"] {
            cg.ctx_mut()
                .register_function_authoritative(alias.into(), target.clone());
        }
        cg.ctx_mut()
            .register_function_authoritative("rebound".into(), target.clone());
        cg.ctx_mut()
            .register_function_authoritative("rebound".into(), foreign.clone());
        let scoped_before = cg.ctx_mut().scoped_functions.clone();
        cg.import_bootstrap_nominal_dependencies(&[&source], &available)
            .unwrap();
        let expected = cg.ctx_mut().functions["alpha.api.supply"]
            .return_type
            .clone();
        assert_ne!(
            expected, target.return_type,
            "must cross source and consumer pools"
        );
        for alias in ["supply", "supply#0", "make", "consumer.make"] {
            assert_eq!(
                cg.ctx_mut().functions[alias].return_type,
                expected,
                "{alias}"
            );
            assert_eq!(cg.ctx_mut().functions[alias].yield_type, None);
        }
        assert_eq!(
            cg.ctx_mut().functions["rebound"].return_type,
            foreign.return_type
        );
        assert_eq!(
            cg.ctx_mut().functions["beta.api.supply"].return_type,
            foreign.return_type
        );
        assert!(
            !cg.ctx_mut()
                .archive_fn_param_types
                .contains_key(&foreign.id.0)
        );
        assert!(cg.ctx_mut().archive_fn_param_types[&target.id.0].is_empty());

        // A later import observes the current alias ownership, including an
        // alias added after the first pass and one rebound away from its ID.
        cg.ctx_mut()
            .register_function_authoritative("late".into(), target.clone());
        cg.ctx_mut()
            .register_function_authoritative("make".into(), foreign.clone());
        cg.ctx_mut()
            .register_function_authoritative("rebound".into(), target.clone());
        cg.import_bootstrap_nominal_dependencies(&[&source], &available)
            .unwrap();
        for alias in ["late", "rebound", "consumer.make", "alpha.api.supply"] {
            assert_eq!(
                cg.ctx_mut().functions[alias].return_type,
                expected,
                "{alias}"
            );
        }
        assert_eq!(
            cg.ctx_mut().functions["make"].return_type,
            foreign.return_type
        );
        assert_eq!(
            cg.ctx_mut().functions["make#0"].return_type,
            foreign.return_type
        );
        assert_eq!(cg.ctx_mut().scoped_functions.len(), scoped_before.len());
        for (key, before) in scoped_before {
            let after = &cg.ctx_mut().scoped_functions[&key];
            assert_eq!(after.id, before.id);
            assert_eq!(after.return_type, before.return_type);
            assert_eq!(after.yield_type, before.yield_type);
        }
    }
}

#[test]
fn shared_consumer_id_keeps_ordered_signature_and_parameter_cache_last_wins() {
    let (mut alpha, ar) = producer("alpha", 1000);
    let (beta, br) = producer("beta", 2000);
    let generic = ar["alpha.api.identity"].clone();
    let mut concrete = br["beta.api.supply"].clone();
    concrete.id = generic.id;
    // As in the existing yield-carrier import pin, populate the optional
    // archive fact explicitly: generator yield inference is outside this unit.
    let descriptor = alpha
        .functions
        .iter_mut()
        .find(|function| alpha.strings.get(function.name) == Some("alpha.api.identity"))
        .unwrap();
    descriptor.yield_type = Some(descriptor.return_type.clone());
    for reverse in [false, true] {
        let available = if reverse {
            [&beta, &alpha]
        } else {
            [&alpha, &beta]
        };
        let (mut cg, source) = consumer(
            "module consumer; mount alpha.api.identity; mount beta.api.supply; fn probe() -> Int { 7 }",
            &[&ar, &br],
        );
        // Two exact source roots deliberately resolve to one consumer ID.
        // Preserve the old ordered-site contract even for this alias state.
        cg.ctx_mut()
            .register_function_authoritative("alpha.api.identity".into(), generic.clone());
        cg.ctx_mut()
            .register_function_authoritative("beta.api.supply".into(), concrete.clone());
        cg.ctx_mut()
            .register_function_authoritative("shared".into(), generic.clone());
        cg.import_bootstrap_nominal_dependencies(&[&source], &available)
            .unwrap();
        let result = cg.ctx_mut().functions["shared"]
            .return_type
            .clone()
            .unwrap();
        let yield_type = cg.ctx_mut().functions["shared"].yield_type.clone();
        if reverse {
            assert_eq!(result, TypeRef::Generic(TypeParamId(0)));
            assert_eq!(yield_type, Some(result.clone()));
            assert_eq!(
                cg.ctx_mut().archive_fn_param_types[&generic.id.0].as_slice(),
                &[result.clone()]
            );
            assert_eq!(
                cg.ctx_mut().archive_fn_parameter_generics[&generic.id.0].as_slice(),
                &[Some(TypeParamId(0))]
            );
        } else {
            assert!(matches!(result, TypeRef::Instantiated { .. }));
            assert_eq!(
                yield_type, None,
                "later non-generator clears the earlier yield"
            );
            assert!(cg.ctx_mut().archive_fn_param_types[&generic.id.0].is_empty());
            assert!(cg.ctx_mut().archive_fn_parameter_generics[&generic.id.0].is_empty());
        }
        for alias in [
            "alpha.api.identity",
            "beta.api.supply",
            "shared",
            "shared#1",
        ] {
            assert_eq!(
                cg.ctx_mut().functions[alias].return_type,
                Some(result.clone()),
                "{alias}"
            );
            assert_eq!(
                cg.ctx_mut().functions[alias].yield_type,
                yield_type,
                "{alias}"
            );
        }
    }
}
