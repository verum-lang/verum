use std::sync::Arc;
use verum_common::Text;
use verum_compiler::{
    api::compile_to_vbc,
    meta::{ConstValue, MetaContext, MetaError, MetaRegistry, const_lowering::ConstLowering},
};
use verum_fast_parser::Parser;
use verum_vbc::{codegen::VbcCodegen, interpreter::Interpreter};

#[test]
fn exact_source_owners_do_not_share_same_spelling_constant_or_function() {
    for reverse in [false, true] {
        let mut units = [
            ("fixture.alpha", Parser::new("const factor: Int = 6; meta fn calc(n: Int) -> Int { factor * n } public fn main() -> Int { @const { calc(7) } }").parse_module().unwrap(), 42),
            ("fixture.beta", Parser::new("const factor: Int = 2; meta fn calc(n: Int) -> Int { factor + n } public fn main() -> Int { @const { calc(11) } }").parse_module().unwrap(), 13),
        ];
        if reverse {
            units.reverse();
        }
        let mut lowering = ConstLowering::new(MetaContext::new());
        for (owner, module, _) in &units {
            lowering.register(owner, module).unwrap();
        }
        for (owner, module, expected) in &mut units {
            lowering.lower(owner, module).unwrap();
            let vbc = VbcCodegen::new().compile_module(module).unwrap();
            let wire = verum_vbc::deserialize::deserialize_module(
                &verum_vbc::serialize::serialize_module(&vbc).unwrap(),
            )
            .unwrap();
            for module in [vbc, wire] {
                let entry = module.find_function_by_name("main").unwrap();
                let mut vm = Interpreter::new(Arc::new(module));
                vm.run_global_ctors().unwrap();
                assert_eq!(
                    vm.execute_function(entry).unwrap().try_as_i64(),
                    Some(*expected)
                );
            }
        }
    }
}
#[test]
fn sibling_constant_is_not_an_implicit_import() {
    let foreign = Parser::new("const value: Int = 42;")
        .parse_module()
        .unwrap();
    let mut caller = Parser::new("fn main() -> Int { @const { value } }")
        .parse_module()
        .unwrap();
    let mut lowering = ConstLowering::new(MetaContext::new());
    lowering.register("fixture.foreign", &foreign).unwrap();
    lowering.register("fixture.caller", &caller).unwrap();
    assert!(lowering.lower("fixture.caller", &mut caller).is_err());
}
#[test]
fn local_items_do_not_capture_a_module_constant_through_runtime_shadow() {
    for source in [
        "const x: Int = 42; fn f(x: Int) -> Int { const y: Int = @const { x }; y }",
        "const x: Int = 42; fn f() -> Int { const x: Int = 1; @const { x } }",
    ] {
        let mut module = Parser::new(source).parse_module().unwrap();
        let mut lowering = ConstLowering::new(MetaContext::new());
        lowering.register("fixture.local", &module).unwrap();
        assert!(
            lowering.lower("fixture.local", &mut module).is_err(),
            "local scope was captured: {source}"
        );
    }
}
#[test]
fn quote_and_generic_declarations_are_deferred_but_backend_is_fail_closed() {
    let mut staged = Parser::new("meta fn syntax() -> TokenStream { quote { @const { 1 / 0 } } } fn generic<T>() -> Int { @const { 1 / 0 } }").parse_module().unwrap();
    let original = staged.clone();
    let mut lowering = ConstLowering::new(MetaContext::new());
    lowering.register("fixture.stage", &staged).unwrap();
    lowering.lower("fixture.stage", &mut staged).unwrap();
    assert_eq!(staged, original);
    let module = Parser::new("fn main() -> Int { @const { 42 } }")
        .parse_module()
        .unwrap();
    let error = VbcCodegen::new()
        .compile_module(&module)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unmaterialized @const"), "{error}");
}
#[test]
fn failed_meta_call_restores_lexical_owner_and_bindings() {
    let module = Parser::new("meta fn fail(n: Int) -> Int { let temporary = 1; n / 0 }")
        .parse_module()
        .unwrap();
    let verum_ast::ItemKind::Function(f) = &module.items[0].kind else {
        panic!()
    };
    let mut registry = MetaRegistry::new();
    registry
        .register_meta_function(&Text::from("callee.owner"), f)
        .unwrap();
    let function = registry
        .get_user_meta_fn(&Text::from("callee.owner"), &Text::from("fail"))
        .unwrap();
    let mut context = MetaContext::new().with_current_module(Text::from("caller.owner"));
    context.bind(Text::from("n"), ConstValue::Int(42));
    assert!(matches!(
        context.execute_user_meta_fn(&function, vec![ConstValue::Int(7)]),
        Err(MetaError::DivisionByZero)
    ));
    assert_eq!(context.current_module.as_str(), "caller.owner");
    assert_eq!(context.get(&Text::from("n")), Some(ConstValue::Int(42)));
    assert_eq!(context.get(&Text::from("temporary")), None);
}
#[test]
fn nested_meta_block_restores_lexical_shadow() {
    for expr in ["@const { let value = 42; { let value = 0; }; value }"] {
        let module = compile_to_vbc(&format!("public fn main() -> Int {{ {expr} }}")).unwrap();
        let entry = module.find_function_by_name("main").unwrap();
        let mut vm = Interpreter::new(Arc::new(module));
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(42));
    }
}

#[test]
fn shadowed_callable_does_not_resolve_as_module_function_or_builtin() {
    for source in [
        "meta fn helper() -> Int { 42 } fn f(helper: fn() -> Int) -> Int { @const { helper() } }",
        "fn f(len: fn(Int) -> Int) -> Int { @const { len([1, 2]) } }",
        "meta fn helper() -> Int { 42 } fn f() -> Int { @const { let helper = 1; helper() } }",
    ] {
        let mut module = Parser::new(source).parse_module().unwrap();
        let mut lowering = ConstLowering::new(MetaContext::new());
        lowering.register("fixture.call", &module).unwrap();
        let error = lowering
            .lower("fixture.call", &mut module)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("shadowed by a runtime declaration or value binding"),
            "{error}"
        );
    }
}
#[test]
fn unsupported_numeric_carriers_do_not_lose_their_type() {
    for expr in [
        "@const { 42u8 }",
        "@const { 1.5f32 }",
        "@const { 42 as Int8 }",
    ] {
        assert!(
            compile_to_vbc(&format!("fn main() {{ {expr} }}")).is_err(),
            "{expr}"
        );
    }
}

#[test]
fn context_provider_alias_is_a_runtime_binding_barrier() {
    for body in [
        "provide Db as source = 0; @const { source }",
        "provide Db as source = 0 in { @const { source } }",
    ] {
        let mut module = Parser::new(&format!(
            "const source: Int = 42; fn f() -> Int {{ {body} }}"
        ))
        .parse_module()
        .unwrap();
        let mut lowering = ConstLowering::new(MetaContext::new());
        lowering.register("fixture.provider", &module).unwrap();
        let error = lowering
            .lower("fixture.provider", &mut module)
            .unwrap_err()
            .to_string();
        assert!(error.contains("runtime binding `source`"), "{error}");
    }
}

#[test]
fn source_error_preserves_the_const_span_and_owner() {
    let source = "module fixture.diagnostic;\nfn main() -> Int {\n @const { 1 / 0 }\n}";
    let mut module = Parser::new(source).parse_module().unwrap();
    let mut lowering = ConstLowering::new(MetaContext::new());
    lowering.register("fixture.diagnostic", &module).unwrap();
    let error = lowering
        .lower("fixture.diagnostic", &mut module)
        .unwrap_err();
    assert_eq!(error.owner.as_str(), "fixture.diagnostic");
    assert!(source[error.span.start as usize..error.span.end as usize].contains("@const"));
    let error = compile_to_vbc(source).unwrap_err();
    assert_eq!(error.diagnostics.len(), 1);
    assert_eq!(error.diagnostics[0].primary_labels()[0].span.line, 3);
}

#[test]
fn unsigned_value_without_declared_width_is_refused() {
    assert!(
        verum_compiler::meta::const_lowering::materialize_runtime_value(
            ConstValue::UInt(42),
            verum_ast::Span::dummy(),
        )
        .is_err()
    );
}

#[test]
fn meta_match_bindings_and_boolean_short_circuit_preserve_lexical_values() {
    for expression in [
        "@const { let value = 42; match 0 { value if false => 0, _ => value } }",
        "@const { let value = 42; match 0 { value => value }; value }",
        "@const { if false && (1 / 0 == 0) { 0 } else { 42 } }",
        "@const { if true || (1 / 0 == 0) { 42 } else { 0 } }",
    ] {
        let module =
            compile_to_vbc(&format!("public fn main() -> Int {{ {expression} }}")).unwrap();
        let wire = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&module).unwrap(),
        )
        .unwrap();
        for module in [module, wire] {
            let entry = module.find_function_by_name("main").unwrap();
            let mut vm = Interpreter::new(Arc::new(module));
            assert_eq!(
                vm.execute_function(entry).unwrap().try_as_i64(),
                Some(42),
                "{expression}"
            );
        }
    }
}

#[test]
fn unrepresented_meta_control_flow_is_refused_before_value_materialization() {
    for expression in [
        "@const { return 0; 42 }",
        "@const { loop { break 0; }; 42 }",
        "@const { while false { }; 42 }",
        "@const { for element in [1] { }; 42 }",
    ] {
        let mut module = Parser::new(&format!("fn main() -> Int {{ {expression} }}"))
            .parse_module()
            .unwrap();
        let mut lowering = ConstLowering::new(MetaContext::new());
        lowering.register("fixture.control", &module).unwrap();
        let error = lowering.lower("fixture.control", &mut module).unwrap_err();
        assert!(
            error.to_string().contains("control flow"),
            "{expression}: {error}"
        );
    }
}

#[test]
fn shorthand_record_pattern_does_not_capture_a_module_constant() {
    for body in [
        "let Sample { value } = input; @const { value }",
        "match input { Sample { value } => @const { value } }",
    ] {
        let source = format!(
            "type Sample is {{ value: Int }}; const value: Int = 7; fn probe(input: Sample) -> Int {{ {body} }}"
        );
        let mut module = Parser::new(&source).parse_module().unwrap();
        let mut lowering = ConstLowering::new(MetaContext::new());
        lowering.register("fixture.pattern", &module).unwrap();
        let error = lowering.lower("fixture.pattern", &mut module).unwrap_err();
        assert!(
            error.to_string().contains("runtime binding `value`"),
            "{source}: {error}"
        );
    }
}

#[test]
fn declared_runtime_function_cannot_fall_through_to_a_meta_builtin() {
    let source = "fn abs(value: Int) -> Int { 123 } public fn main() -> Int { @const { abs(-7) } }";
    let mut module = Parser::new(source).parse_module().unwrap();
    let mut lowering = ConstLowering::new(MetaContext::new());
    lowering.register("fixture.runtime", &module).unwrap();
    let error = lowering.lower("fixture.runtime", &mut module).unwrap_err();
    assert!(error.to_string().contains("runtime"), "{error}");
}

#[test]
fn public_multifile_frontend_preserves_file_owners_through_wire_execution() {
    use verum_compiler::api::{CommonPipelineConfig, SourceFile, run_common_pipeline};
    use verum_vbc::codegen::ItemFailurePolicy;
    for absolute in [false, true] {
        for reverse in [false, true] {
            let prefix = if absolute {
                "/source/project/"
            } else {
                "project/"
            };
            let mut sources = vec![
                SourceFile::new(
                    format!("{prefix}alpha.vr"),
                    "const value: Int = 6; meta fn calc(n: Int) -> Int { value * n } public fn answer() -> Int { @const { calc(7) } }",
                ),
                SourceFile::new(
                    format!("{prefix}beta.vr"),
                    "const value: Int = 2; meta fn calc(n: Int) -> Int { value + n } public fn answer() -> Int { @const { calc(11) } }",
                ),
            ];
            if reverse {
                sources.reverse();
            }
            let result = run_common_pipeline(&sources, &CommonPipelineConfig::minimal()).unwrap();
            assert!(!result.has_errors());
            let owned: Vec<_> = result
                .ast_modules
                .iter()
                .map(|module| (result.source_owners[&module.file_id].0.as_str(), module))
                .collect();
            let mut codegen = VbcCodegen::new();
            codegen.collect_owned_unit_declarations(&owned).unwrap();
            codegen
                .compile_owned_unit_items(&owned, ItemFailurePolicy::Strict)
                .unwrap();
            let module = codegen.finalize_module().unwrap();
            let wire = verum_vbc::deserialize::deserialize_module(
                &verum_vbc::serialize::serialize_module(&module).unwrap(),
            )
            .unwrap();
            for module in [module, wire] {
                for (owner, expected) in [("cog.alpha", 42), ("cog.beta", 13)] {
                    let entry = module
                        .find_function_by_name(&format!("{owner}.answer"))
                        .unwrap();
                    let mut vm = Interpreter::new(Arc::new(module.clone()));
                    vm.run_global_ctors().unwrap();
                    assert_eq!(
                        vm.execute_function(entry).unwrap().try_as_i64(),
                        Some(expected),
                        "{owner}, absolute={absolute}, reverse={reverse}"
                    );
                }
            }
        }
    }
}

#[test]
fn logical_guard_binding_cannot_capture_a_module_constant() {
    let source = "const selected: Bool = true; fn probe(input: Bool) -> Int { if input is selected && @const { selected } { 1 } else { 0 } }";
    let mut module = Parser::new(source).parse_module().unwrap();
    let mut lowering = ConstLowering::new(MetaContext::new());
    lowering.register("fixture.guard", &module).unwrap();
    let error = lowering.lower("fixture.guard", &mut module).unwrap_err();
    assert!(
        error.to_string().contains("runtime binding `selected`"),
        "{error}"
    );
}

#[test]
fn selected_multifile_source_excludes_inactive_descendant_const_before_evaluation() {
    use verum_compiler::api::{CommonPipelineConfig, SourceFile, run_common_pipeline};
    for reverse in [false, true] {
        let mut sources = vec![
            SourceFile::new(
                "project/mod.vr",
                "module cog; @cfg(feature=\"missing_const_fixture\") public module hidden; public fn main() -> Int { @const { 42 } }",
            ),
            SourceFile::new(
                "project/hidden.vr",
                "module cog.hidden; meta fn fail() -> Int { 1 / 0 } public const value: Int = @const { fail() };",
            ),
        ];
        if reverse {
            sources.reverse();
        }
        let result = run_common_pipeline(&sources, &CommonPipelineConfig::minimal()).unwrap();
        assert_eq!(result.ast_modules.len(), 1);
        assert_eq!(
            result.source_owners[&result.ast_modules[0].file_id]
                .0
                .as_str(),
            "cog"
        );
        let vbc = VbcCodegen::new()
            .compile_module(&result.ast_modules[0])
            .unwrap();
        let entry = vbc.find_function_by_name("cog.main").unwrap();
        let mut vm = Interpreter::new(Arc::new(vbc));
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(42));
    }
}
