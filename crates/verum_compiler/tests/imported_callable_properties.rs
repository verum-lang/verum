//! Namespace spelling preserves the selected declaration's callable contract.
use std::sync::Arc;
use verum_common::{List, Text};
use verum_compiler::{
    CompilationPipeline, CompilerOptions, CoreConfig, Session, archive_ctx_loader::ArchiveCtxCache,
};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn contract(provider: &str, consumer: &str) -> Result<(), Text> {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    std::fs::create_dir(&core).unwrap();
    std::fs::write(
        core.join("mod.vr"),
        "module core; public module raw; public module consumer;",
    )
    .unwrap();
    std::fs::write(core.join("raw.vr"), format!("module core.raw;\n{provider}")).unwrap();
    std::fs::write(
        core.join("consumer.vr"),
        format!("module core.consumer;\n{consumer}"),
    )
    .unwrap();
    let output = root.path().join("stdlib.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
        .compile_core()
        .map_err(|error| Text::from(format!("{error}\n{}", session.format_diagnostics())))?;
    let archive = verum_vbc::archive::read_archive_from_file(&output).unwrap();
    let app = Parser::new("module app; mount core.consumer.probe; fn main() -> Int { probe() }")
        .parse_module()
        .unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app"));
    ArchiveCtxCache::new(&archive)
        .apply_lazy_with_types(&mut codegen, &app, &[])
        .unwrap();
    codegen.collect_unit_declarations(&[&app]).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&[&app], ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let restored = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, restored] {
        let entry = module.find_function_by_name("app.main").unwrap();
        let result = Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap();
        assert_eq!(result.try_as_i64(), Some(73));
    }
    Ok(())
}

const RAW: &str = "public type Raw is { public code: Int }; implement Raw { public pure fn from_code(code: Int) -> Raw { Raw { code: code } } }";

fn check_static_case(forward: bool, nested: bool, qualified: bool) {
    let (imports, source_type) = if qualified {
        ("mount core.raw as raw;", "raw.Raw")
    } else {
        ("mount core.raw.Raw;", "Raw")
    };
    let body = if nested {
        format!("Classified.from_raw({source_type}.from_code(code))")
    } else {
        format!("let value = {source_type}.from_code(code); Classified.from_raw(value)")
    };
    let wrapper = format!("public pure fn from_code(code: Int) -> Classified {{ {body} }}");
    let worker = format!(
        "public pure fn from_raw(value: {source_type}) -> Classified {{ if value.code == 1 {{ Classified.Known }} else {{ Classified.Unknown }} }}"
    );
    let methods = if forward {
        format!("{wrapper}\n{worker}")
    } else {
        format!("{worker}\n{wrapper}")
    };
    let consumer = format!(
        "{imports} public type Classified is Known | Unknown; implement Classified {{ {methods} }} public fn probe() -> Int {{ if Classified.from_code(1) is Classified.Known {{ 73 }} else {{ 0 }} }}"
    );
    contract(RAW, &consumer).unwrap_or_else(|error| panic!("{error}"));
}

macro_rules! static_case {
    ($name:ident, $forward:expr, $nested:expr, $qualified:expr) => {
        #[test]
        fn $name() {
            check_static_case($forward, $nested, $qualified);
        }
    };
}
static_case!(direct_nested_imported, false, true, false);
static_case!(direct_sequential_imported, false, false, false);
static_case!(forward_nested_imported, true, true, false);
static_case!(forward_sequential_imported, true, false, false);
static_case!(direct_nested_qualified, false, true, true);
static_case!(direct_sequential_qualified, false, false, true);
static_case!(forward_nested_qualified, true, true, true);
static_case!(forward_sequential_qualified, true, false, true);

#[test]
fn same_owner_static_call_control() {
    contract(
        "",
        &format!("{RAW} public pure fn probe() -> Int {{ Raw.from_code(73).code }}"),
    )
    .unwrap();
}

#[test]
fn impure_imported_static_call_is_rejected() {
    let provider = "public type Source is (); extern \"C\" { fn touch(); } implement Source { public fn value() -> Int { unsafe { touch(); } 73 } }";
    for (imports, callee) in [
        ("mount core.raw.Source;", "Source.value()"),
        ("mount core.raw as raw;", "raw.Source.value()"),
    ] {
        let error = contract(
            provider,
            &format!("{imports} public pure fn probe() -> Int {{ {callee} }}"),
        )
        .expect_err("FFI call was treated as pure");
        assert!(error.contains("E503"), "{error}");
        assert!(
            !error.contains("E100"),
            "call must resolve before purity refusal: {error}"
        );
    }
}

#[test]
fn unknown_imported_callback_is_rejected() {
    let provider = "public type Source is (); implement Source { public fn value(callback: fn(Int) -> Int) -> Int { callback(73) } }";
    for (imports, callee) in [
        ("mount core.raw.Source;", "Source.value(callback)"),
        ("mount core.raw as raw;", "raw.Source.value(callback)"),
    ] {
        let error = contract(provider, &format!("{imports} public pure fn unknown(callback: fn(Int) -> Int) -> Int {{ {callee} }} public fn probe() -> Int {{ 73 }}")).expect_err("unknown callback was treated as pure");
        assert!(error.contains("E503"), "{error}");
        assert!(
            !error.contains("E100"),
            "call must resolve before purity refusal: {error}"
        );
    }
}

#[test]
fn namespace_free_calls_keep_the_selected_bound() {
    for call in ["raw.identity(73)", "core.raw.identity(73)"] {
        contract(
            "public pure fn identity(value: Int) -> Int { value }",
            &format!("mount core.raw; public pure fn probe() -> Int {{ {call} }}"),
        )
        .unwrap();
    }
}

#[test]
fn inline_calls_preserve_the_selected_bound() {
    for (provider, accepted) in [
        ("public pure fn value() -> Int { 73 }", true),
        (
            "extern \"C\" { fn touch(); } public fn value() -> Int { unsafe { touch(); } 73 }",
            false,
        ),
    ] {
        let source = format!(
            "public module provider {{ {provider} }} pure fn probe() -> Int {{ provider.value() }}"
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        let mut checker = verum_types::TypeChecker::new();
        checker.register_primitives();
        let mut errors: List<_> = checker.prepare_module_declarations(&ast);
        errors.extend(
            ast.items
                .iter()
                .filter_map(|item| checker.check_item(item).err()),
        );
        errors.extend(checker.take_deferred_errors());
        let messages: List<Text> = errors
            .iter()
            .map(|error| format!("{error:?}").into())
            .chain(
                checker
                    .diagnostics()
                    .iter()
                    .filter(|diagnostic| diagnostic.is_error())
                    .map(|diagnostic| diagnostic.message().into()),
            )
            .collect();
        if accepted {
            assert!(messages.is_empty(), "{source}: {messages:?}");
        } else {
            assert!(
                messages
                    .iter()
                    .any(|message| message.contains("ImpurePureFunction")),
                "{source}: {messages:?}"
            );
        }
    }
}

#[test]
fn calculus_callback_keeps_its_declared_signature_across_archive_execution() {
    let calculus = include_str!("../../../core/math/calculus.vr");
    let module = Parser::new(calculus)
        .parse_module()
        .expect("actual calculus grammar");
    let span = module
        .items
        .iter()
        .find_map(|item| match &item.kind {
            verum_ast::ItemKind::Function(function) if function.name.name == "diff_central" => {
                Some(item.span)
            }
            _ => None,
        })
        .expect("calculus differentiation function");
    // Item spans begin at the function keyword. Include its source visibility.
    let start = calculus[..span.start as usize]
        .rfind('\n')
        .map_or(0, |newline| newline + 1);
    let function = &calculus[start..span.end as usize];
    let accepted = "mount core.raw.diff_central;
        pure fn square(x: Float) -> Float { x * x }
        public fn probe() -> Int {
            let factor: Float = 3.0;
            let scaled: fn(Float) -> Float = |x| factor * x * x;
            let named = diff_central(square, 3.0, 1.0);
            let captured = diff_central(scaled, 3.0, 1.0);
            if named == 6.0 && captured == 18.0 { 73 } else { 0 }
        }";
    contract(function, accepted).expect("named and capturing callbacks preserve their signatures");
    for (callback, expected) in [
        (
            "pure fn wrong(x:Bool)->Float { 1.0 }",
            "expected 'Bool', found 'Float'",
        ),
        (
            "pure fn wrong(x:Float)->Bool { true }",
            "expected 'Float', found 'Bool'",
        ),
        (
            "pure fn wrong(x:Float, y:Float)->Float { x+y }",
            "expected 'fn(Float) -> Float', found 'fn(Float, Float) -> Float'",
        ),
    ] {
        let consumer = format!(
            "mount core.raw.diff_central; {callback}
            public fn probe()->Int {{ if diff_central(wrong,3.0,1.0)==6.0 {{73}} else {{0}} }}"
        );
        let error = contract(function, &consumer)
            .expect_err("contradictory callback passed its declared bound");
        assert!(
            error.contains("E400") && error.contains(expected),
            "{error}"
        );
        assert!(
            !error.contains("type not found"),
            "wrong rejection: {error}"
        );
    }
}
