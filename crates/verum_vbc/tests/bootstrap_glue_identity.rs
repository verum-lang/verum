#![cfg(feature = "codegen")]
use std::collections::HashMap;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen, context::FunctionInfo};
use verum_vbc::module::{FunctionId, VbcModule};

fn ast(text: &str) -> verum_ast::Module {
    Parser::new(text).parse_module().unwrap()
}
fn token(module: &VbcModule) -> &verum_vbc::types::TypeDescriptor {
    module
        .types
        .iter()
        .find(|ty| module.strings.get(ty.name) == Some("Token"))
        .unwrap()
}
fn producer() -> VbcModule {
    producer_at("alpha")
}
fn producer_at(owner: &str) -> VbcModule {
    let source = ast(&format!(
        "module {owner}; type Drop is protocol {{ fn drop(&mut self); }}; type Token is {{ value: Int }}; implement Drop for Token {{ fn drop(&mut self) {{}} }} implement Token {{ fn duplicate(&self) -> Token {{ Token(value: self.value) }} }}"
    ));
    let mut module = VbcCodegen::with_config(CodegenConfig::new("alpha"))
        .compile_module(&source)
        .unwrap();
    // Clone registration is a separate producer boundary. Supply a genuine
    // source body's local id to exercise the existing descriptor wire field.
    let clone = module
        .functions
        .iter()
        .find(|f| module.strings.get(f.name) == Some("Token.duplicate"))
        .unwrap()
        .id
        .0;
    module
        .types
        .iter_mut()
        .find(|ty| ty.drop_fn.is_some())
        .unwrap()
        .clone_fn = Some(clone);
    module
}
fn dependent(producer: &VbcModule) -> VbcModule {
    assert!(token(producer).drop_fn.is_some());
    let owner = token(producer)
        .origin_module
        .and_then(|id| producer.strings.get(id))
        .unwrap_or(&producer.name);
    let source = ast(&format!(
        "module dependent; mount {owner}.Token; fn inspect(t: Token) -> Int {{ 7 }}"
    ));
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("dependent"));
    cg.ctx_mut().register_function(
        format!("{owner}.Token.drop"),
        FunctionInfo {
            id: FunctionId(9000),
            param_count: 1,
            ..Default::default()
        },
    );
    cg.ctx_mut().register_function(
        format!("{owner}.Token.duplicate"),
        FunctionInfo {
            id: FunctionId(9001),
            param_count: 1,
            ..Default::default()
        },
    );
    cg.import_bootstrap_nominal_dependencies(&[&source], &[producer])
        .unwrap();
    cg.collect_unit_declarations(&[&source]).unwrap();
    cg.compile_function_bodies(&source).unwrap()
}
#[test]
fn bootstrap_glue_is_a_named_external_even_without_an_explicit_call() {
    let source = producer();
    let module = dependent(&source);
    let drop = token(&module)
        .drop_fn
        .expect("imported Drop survives bootstrap finalization");
    assert!(verum_vbc::stub_ranges::in_xmod_call_band(drop));
    let name = module
        .external_function_names
        .iter()
        .find(|(id, _)| id.0 == drop)
        .and_then(|(_, name)| module.strings.get(*name));
    assert_eq!(name, Some("alpha.Token.drop"));
    let clone = token(&module).clone_fn.expect("imported Clone");
    assert_eq!(
        module.band_reference_name(clone),
        Some("alpha.Token.duplicate")
    );
}
#[test]
fn merged_glue_resolves_the_original_owner_in_both_module_orders() {
    let source = producer();
    let dependent = dependent(&source);
    let source = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&source).unwrap(),
    )
    .unwrap();
    let dependent = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&dependent).unwrap(),
    )
    .unwrap();
    for dependency_first in [false, true] {
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("user"));
        for module in if dependency_first {
            [&dependent, &source]
        } else {
            [&source, &dependent]
        } {
            cg.import_archive_module_types(module);
        }
        let mut next = 7000;
        let mut maps = Vec::new();
        for module in [&source, &dependent] {
            let mut map = HashMap::new();
            for function in &module.functions {
                let id = FunctionId(next);
                next += 1;
                let raw = module.strings.get(function.name).unwrap();
                let name = if raw.starts_with(&format!("{}.", module.name)) {
                    raw.to_owned()
                } else {
                    format!("{}.{}", module.name, raw)
                };
                cg.ctx_mut().register_function(
                    name,
                    FunctionInfo {
                        id,
                        param_count: function.params.len(),
                        ..Default::default()
                    },
                );
                map.insert(function.id.0, id);
            }
            maps.push(map);
        }
        let order = if dependency_first { [1, 0] } else { [0, 1] };
        let modules = [&source, &dependent];
        for index in order {
            cg.merge_archive_function_bodies(modules[index], &maps[index]);
        }
        let result = cg.finalize_module_from_state().unwrap();
        let tokens = result
            .types
            .iter()
            .filter(|ty| result.strings.get(ty.name) == Some("Token"))
            .collect::<Vec<_>>();
        assert_eq!(
            tokens.len(),
            1,
            "a dependency copy must preserve the same nominal owner"
        );
        for (glue, suffix) in [
            (tokens[0].drop_fn, "Token.drop"),
            (tokens[0].clone_fn, "Token.duplicate"),
        ] {
            let glue = glue.expect("final linked glue");
            let function = result
                .functions
                .iter()
                .find(|function| function.id.0 == glue)
                .expect("local linked body");
            assert!(result.strings.get(function.name).unwrap().ends_with(suffix));
        }
        assert_ne!(tokens[0].drop_fn, tokens[0].clone_fn);
    }
}

#[test]
fn bundled_file_origin_is_preserved_in_glue_name() {
    let source = producer_at("alpha.memory");
    let dependent = dependent(&source);
    let drop = token(&dependent).drop_fn.expect("drop");
    assert_eq!(
        dependent.band_reference_name(drop),
        Some("alpha.memory.Token.drop")
    );
}

#[test]
fn actual_source_registry_carries_bundled_drop_without_a_synthetic_binding() {
    let declaration = ast(
        "module alpha.memory; type Drop is protocol { fn drop(&mut self); }; type Token is { value: Int }; implement Drop for Token { fn drop(&mut self) {} }",
    );
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("alpha"));
    let module = producer.compile_module(&declaration).unwrap();
    let source =
        ast("module dependent; mount alpha.memory.Token; fn inspect(t: Token) -> Int { 7 }");
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("dependent"));
    let registry = producer.export_functions();
    consumer.import_functions(&registry);
    consumer
        .import_bootstrap_nominal_dependencies(&[&source], &[&module])
        .unwrap();
    consumer.collect_unit_declarations(&[&source]).unwrap();
    let result = consumer.compile_function_bodies(&source).unwrap();
    let drop = token(&result)
        .drop_fn
        .expect("source registry supplied the exact glue identity");
    assert_eq!(
        result.band_reference_name(drop),
        Some("alpha.memory.Token.drop")
    );
}

#[test]
fn promoted_type_spelling_does_not_duplicate_its_declaring_file() {
    for spelling in ["Token", "memory.Token", "alpha.memory.Token"] {
        let mut source = producer_at("alpha.memory");
        let name = source.strings.intern(spelling);
        source
            .types
            .iter_mut()
            .find(|ty| ty.drop_fn.is_some())
            .unwrap()
            .name = name;
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("user"));
        cg.import_archive_module_types(&source);
        let remap = source
            .functions
            .iter()
            .enumerate()
            .map(|(i, f)| (f.id.0, FunctionId(7000 + i as u32)))
            .collect();
        cg.merge_archive_function_bodies(&source, &remap);
        let result = cg.finalize_module_from_state().unwrap();
        let drop = token(&result).drop_fn.expect("canonical source glue");
        assert!(
            result
                .functions
                .iter()
                .any(|f| f.id.0 == drop && result.strings.get(f.name) == Some("Token.drop"))
        );
    }
}
