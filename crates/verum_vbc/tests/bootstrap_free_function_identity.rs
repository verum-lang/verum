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
