use std::{fs, sync::Arc};
use verum_compiler::api::compile_to_vbc;
use verum_compiler::precompile::{PrecompileConfig, precompile_stdlib};
use verum_vbc::{VbcModule, archive::read_archive_from_file, interpreter::Interpreter};

fn program(root: &str, expr: &str) -> String {
    match root {
        "function" => format!("public fn main() -> Int {{ {expr} }}"),
        "const" => format!("public const VALUE: Int = {expr}; public fn main() -> Int {{ VALUE }}"),
        "impl" => format!(
            "public type Holder is {{ public value: Int }}; implement Holder {{ public fn answer() -> Int {{ {expr} }} }} public fn main() -> Int {{ Holder.answer() }}"
        ),
        _ => panic!(),
    }
}
fn execute(module: VbcModule, name: &str) -> i64 {
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    let mut observations = Vec::new();
    for m in [module, wire] {
        let id = m.find_function_by_name(name).unwrap();
        let mut vm = Interpreter::new(Arc::new(m));
        vm.run_global_ctors().unwrap();
        let value = vm.execute_function(id).unwrap();
        observations.push((value.try_as_i64(), format!("{value:?}")));
    }
    assert!(
        observations.iter().all(|(integer, _)| *integer == Some(42)),
        "actual source/wire {name}: {observations:?}"
    );
    42
}
fn produce(source: &str) -> Result<VbcModule, String> {
    let temp = tempfile::tempdir().unwrap();
    let core = temp.path().join("core");
    fs::create_dir(&core).unwrap();
    fs::write(core.join("mod.vr"), source).unwrap();
    let cfg = PrecompileConfig {
        stdlib_path: core,
        output_path: temp.path().join("runtime.vbca"),
        target_triple: None,
        verbose: false,
    };
    let result = precompile_stdlib(&cfg).map_err(|e| format!("{e:#}"));
    match result {
        Ok(result) => {
            assert!(result.errors.is_empty());
            Ok(read_archive_from_file(&cfg.output_path)
                .unwrap()
                .load_module("core")
                .unwrap())
        }
        Err(e) => {
            assert!(!cfg.output_path.exists());
            Err(e)
        }
    }
}
#[test]
fn ordinary_plain_controls() {
    for root in ["function", "const", "impl"] {
        let src = program(root, "42");
        execute(compile_to_vbc(&src).unwrap(), "main");
    }
}
#[test]
fn producer_plain_controls() {
    for root in ["function", "const", "impl"] {
        let src = program(root, "42");
        execute(produce(&src).unwrap(), "core.main");
    }
}
#[test]
fn ordinary_top_const_value() {
    execute(
        compile_to_vbc(&program("const", "@const { 6 * 7 }")).unwrap(),
        "main",
    );
}
#[test]
fn ordinary_impl_value() {
    execute(
        compile_to_vbc(&program("impl", "@const { 6 * 7 }")).unwrap(),
        "main",
    );
}
#[test]
fn ordinary_top_const_failure() {
    assert!(
        compile_to_vbc(&program("const", "@const { 1 / (1 - 1) }")).is_err(),
        "bad top-level const accepted"
    );
}
#[test]
fn ordinary_impl_failure() {
    assert!(
        compile_to_vbc(&program("impl", "@const { 1 / (1 - 1) }")).is_err(),
        "bad impl const accepted"
    );
}
#[test]
fn producer_top_const_value() {
    execute(
        produce(&program("const", "@const { 6 * 7 }")).unwrap(),
        "core.main",
    );
}
#[test]
fn producer_impl_value() {
    execute(
        produce(&program("impl", "@const { 6 * 7 }")).unwrap(),
        "core.main",
    );
}
#[test]
fn producer_top_const_failure() {
    assert!(
        produce(&program("const", "@const { 1 / (1 - 1) }")).is_err(),
        "bad top-level const published"
    );
}
#[test]
fn producer_impl_failure() {
    assert!(
        produce(&program("impl", "@const { 1 / (1 - 1) }")).is_err(),
        "bad impl const published"
    );
}
#[test]
fn producer_top_function_failure_control() {
    assert!(
        produce(&program("function", "@const { 1 / (1 - 1) }")).is_err(),
        "existing function-root gate must reject"
    );
}

#[test]
fn ordinary_function_const_value() {
    execute(
        compile_to_vbc(&program("function", "@const { 6 * 7 }")).unwrap(),
        "main",
    );
}
#[test]
fn ordinary_function_const_failure() {
    assert!(
        compile_to_vbc(&program("function", "@const { 1 / (1 - 1) }")).is_err(),
        "bad function const accepted"
    );
}
#[test]
fn producer_function_const_value() {
    execute(
        produce(&program("function", "@const { 6 * 7 }")).unwrap(),
        "core.main",
    );
}

fn both(source: &str) {
    execute(compile_to_vbc(source).unwrap(), "main");
    execute(produce(source).unwrap(), "core.main");
}
#[test]
fn nested_values_preserve_runtime_carriers() {
    for expr in [
        "(@const { (20, 22) }).0 + (@const { (20, 22) }).1",
        "(@const { [40, 2] })[0] + (@const { [40, 2] })[1]",
        "if (@const { true }) { 42 } else { 0 }",
        "if (@const { 1.5 }) == 1.5 { 42 } else { 0 }",
        "if (@const { 'x' }) == 'x' { 42 } else { 0 }",
    ] {
        both(&program("function", expr));
    }
}
#[test]
fn module_constant_and_meta_locals_are_owned() {
    both(
        "const factor: Int = 6; meta fn calculate() -> Int { factor * 7 } public fn main() -> Int { @const { calculate() } }",
    );
    both(
        "const value: Int = 99; public fn main() -> Int { let value = 0; @const { let value = 42; value } }",
    );
    both(
        "const value: Int = 42; public fn main() -> Int { let answer = @const { value }; let value = 0; answer }",
    );
}
#[test]
fn runtime_shadow_cannot_fall_through_to_module_constant() {
    for source in [
        "const value: Int = 42; public fn main() -> Int { let value = 0; @const { value } }",
        "const value: Int = 42; fn f(value: Int) -> Int { @const { value } } public fn main() -> Int { f(0) }",
    ] {
        assert!(
            compile_to_vbc(source).is_err(),
            "runtime capture accepted: {source}"
        );
        assert!(
            produce(source).is_err(),
            "runtime capture published: {source}"
        );
    }
}
#[test]
fn quoted_syntax_is_not_executed_or_spliced_as_a_runtime_value() {
    for root in ["function", "const", "impl"] {
        let source = program(root, "@const { quote { 1 / 0 } }");
        let ordinary = compile_to_vbc(&source).unwrap_err().to_string();
        assert!(
            ordinary.contains("TokenStream") && ordinary.contains("Int"),
            "{ordinary}"
        );
        let producer = produce(&source).unwrap_err();
        assert!(
            producer.contains("TokenStream") && producer.contains("Int"),
            "{producer}"
        );
        assert!(!ordinary.contains("Division by zero"));
        assert!(!producer.contains("Division by zero"));
    }
}
#[test]
fn well_typed_syntax_value_has_no_implicit_runtime_splice() {
    let source = "public fn main() -> TokenStream { @const { quote { 1 / 0 } } }";
    for error in [
        compile_to_vbc(source).unwrap_err().to_string(),
        produce(source).unwrap_err(),
    ] {
        assert!(error.contains("syntax value"), "{error}");
        assert!(!error.contains("Division by zero"), "{error}");
    }
}

#[test]
fn cyclic_module_constants_refuse_without_recursing_forever() {
    let source = "const a: Int = b; const b: Int = a; public fn main() -> Int { @const { a } }";
    assert!(compile_to_vbc(source).is_err());
    assert!(produce(source).is_err());
}

#[test]
fn evaluation_occurs_once_even_when_the_const_is_referenced_before_declaration() {
    const CHILD: &str = "VERUM_CONST_MATERIALIZATION_ONCE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "evaluation_occurs_once_even_when_the_const_is_referenced_before_declaration",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env_remove("VERUM_META_ENGINE")
            .env_remove("VERUM_BAKE_VALIDATION")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{:?}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("CONST_EVALUATED_ONCE"));
        return;
    }
    let declaration = "const token: Text = @const { gensym(\"once\") };";
    let function =
        "public fn main() -> Int { if (@const { token }) == (@const { token }) { 42 } else { 0 } }";
    for source in [
        format!("{declaration} {function}"),
        format!("{function} {declaration}"),
    ] {
        let before = verum_compiler::meta::builtins::code_gen::gensym_counter();
        both(&source);
        assert_eq!(
            verum_compiler::meta::builtins::code_gen::gensym_counter() - before,
            2,
            "one ordinary and one producer evaluation"
        );
    }
    println!("CONST_EVALUATED_ONCE");
}

#[test]
fn division_failure_comes_from_evaluation_in_every_source_root() {
    for root in ["function", "const", "impl"] {
        let source = program(root, "@const { 1 / (1 - 1) }");
        for error in [
            compile_to_vbc(&source).unwrap_err().to_string(),
            produce(&source).unwrap_err(),
        ] {
            assert!(
                error.contains("M603") && error.contains("Division by zero"),
                "{root}: {error}"
            );
        }
    }
}

#[test]
fn declared_meta_function_takes_precedence_over_builtin_spelling() {
    both("meta fn abs(value: Int) -> Int { 42 } public fn main() -> Int { @const { abs(7) } }");
}

#[test]
fn staged_local_type_obligations_are_checked_before_erasure() {
    for root in ["function", "const", "impl"] {
        let source = program(root, "@const { let invalid: Bool = 42; 42 }");
        assert!(
            compile_to_vbc(&source).is_err(),
            "erased ill-typed local in ordinary {root}"
        );
        assert!(
            produce(&source).is_err(),
            "erased ill-typed local in producer {root}"
        );
    }
}

#[test]
fn generic_callable_constant_cannot_resolve_as_a_builtin() {
    let source =
        "const abs<T>: fn(Int) -> Int = |value| 13; public fn main() -> Int { @const { abs(7) } }";
    for error in [
        compile_to_vbc(source).unwrap_err().to_string(),
        produce(source).unwrap_err(),
    ] {
        assert!(
            error.contains("callable `abs`") && error.contains("binding"),
            "{error}"
        );
    }
}

#[test]
fn inherited_value_parameter_wrong_kind_is_refused_before_materialization() {
    let source = "type Holder<N: meta Int> is { value: Int }; const N: Int = 42; implement Holder<N> { fn probe() -> Int { @const { N } } } public fn main() -> Int { 0 }";
    for error in [
        compile_to_vbc(source).unwrap_err().to_string(),
        produce(source).unwrap_err(),
    ] {
        assert!(
            error.contains("wrong kind") && error.contains("Holder.N"),
            "{error}"
        );
    }
}
