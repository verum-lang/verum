//! Context methods retain declaration-owned generic call obligations (T2401).
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn check(source: &str) -> List<Text> {
    let module = Parser::new(source).parse_module().expect("valid source");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("context_contracts");
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&module)
        .iter()
        .map(|error| format!("{error:?}").into())
        .collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(|error| format!("{error:?}").into()),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| format!("{d:?}").into()),
    );
    errors
}

fn context_call(caller: &str, accepted: bool) {
    let source =
        format!("context Jobs {{ fn run<F: fn(Int) -> Bool>(callback: F) -> Bool; }}\n{caller}");
    let errors = check(&source);
    assert_eq!(errors.is_empty(), accepted, "{source}\n{errors:#?}");
}

#[test]
fn declared_callback_and_closure_are_accepted() {
    for caller in [
        "fn probe<G: fn(Int)->Bool>(cb:G)->Bool using[Jobs] { Jobs.run(cb) }",
        "fn probe(cb:fn(Int)->Bool)->Bool using[Jobs] { Jobs.run(cb) }",
        "fn probe()->Bool using[Jobs] { Jobs.run(|x:Int| x == 1) }",
        "fn probe<G:fn(Int)->Bool>(cb:G)->Bool using[Jobs] { Jobs.run<G>(cb) }",
    ] {
        context_call(caller, true);
    }
}

#[test]
fn incompatible_callback_contracts_are_rejected() {
    for caller in [
        "fn probe<G:fn(Text)->Bool>(cb:G)->Bool using[Jobs] { Jobs.run(cb) }",
        "fn probe<G:fn(Int)->Text>(cb:G)->Bool using[Jobs] { Jobs.run(cb) }",
        "fn probe<G>(cb:G)->Bool using[Jobs] { Jobs.run(cb) }",
        "fn probe(cb:fn(Text)->Bool)->Bool using[Jobs] { Jobs.run(cb) }",
        "fn probe()->Bool using[Jobs] { Jobs.run(7) }",
        "fn probe<G:fn(Text)->Bool>(cb:G)->Bool using[Jobs] { Jobs.run<G>(cb) }",
    ] {
        context_call(caller, false);
    }
}

#[test]
fn arity_and_context_availability_are_checked() {
    context_call("fn probe()->Bool using[Jobs] { Jobs.run() }", false);
    context_call(
        "fn probe<G:fn(Int)->Bool>(cb:G)->Bool { Jobs.run(cb) }",
        false,
    );
}

#[test]
fn sibling_calls_instantiate_independently() {
    let errors = check(
        r#"
        context Jobs { fn run<A, F:fn()->A>(callback:F)->A; }
        fn probe(a:fn()->Int,b:fn()->Bool)->(Int,Bool) using[Jobs] {
            (Jobs.run(a), Jobs.run(b))
        }
    "#,
    );
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn context_owner_arguments_and_aliases_are_preserved() {
    for (callback, accepted) in [("fn(Int)->Bool", true), ("fn(Text)->Bool", false)] {
        for (using, receiver) in [("Jobs<Int>", "Jobs"), ("Jobs<Int> as worker", "worker")] {
            let source = format!(
                r#"
                context Jobs<A> {{ fn run<F:fn(A)->Bool>(cb:F)->Bool; }}
                fn probe<G:{callback}>(cb:G)->Bool using[{using}] {{ {receiver}.run(cb) }}
            "#
            );
            let errors = check(&source);
            assert_eq!(errors.is_empty(), accepted, "{source}\n{errors:#?}");
        }
    }
}

#[test]
fn context_protocol_bounds_are_proved_at_the_call() {
    for (bound, accepted) in [(":Key", true), ("", false)] {
        let source = format!(
            r#"
            type Key is protocol {{ fn key(&self)->Int; }};
            context Jobs {{ fn submit<T:Key>(value:T)->Int; }}
            fn probe<T{bound}>(value:T)->Int using[Jobs] {{ Jobs.submit(value) }}
        "#
        );
        let errors = check(&source);
        assert_eq!(errors.is_empty(), accepted, "{source}\n{errors:#?}");
    }
}

#[test]
fn method_generics_do_not_capture_the_callers_names() {
    for bounds in ["T,F:fn()->T", "F:fn()->T,T"] {
        let source = format!(
            r#"
            context Jobs {{ fn run<{bounds}>(cb:F)->T; }}
            fn probe<T,F:fn()->Int>(cb:F, other:T)->Int using[Jobs] {{ Jobs.run(cb) }}
        "#
        );
        let errors = check(&source);
        assert!(errors.is_empty(), "{source}\n{errors:#?}");
    }
}

#[test]
fn capturing_a_method_does_not_erase_its_callback_obligation() {
    for (bound, accepted) in [("fn(Int)->Bool", true), ("fn(Text)->Bool", false)] {
        context_call(
            &format!(
                r#"
            fn probe<G:{bound}>(cb:G)->Bool using[Jobs] {{
                let run = Jobs.run;
                run(cb)
            }}
        "#
            ),
            accepted,
        );
    }
}

#[test]
fn method_context_requirements_reach_the_caller() {
    for (using, accepted) in [("Jobs,Clock", true), ("Jobs", false)] {
        let source = format!(
            r#"
            context Clock {{ fn now()->Int; }}
            context Jobs {{ fn run()->Int using[Clock]; }}
            fn probe()->Int using[{using}] {{ Jobs.run() }}
        "#
        );
        let errors = check(&source);
        assert_eq!(errors.is_empty(), accepted, "{source}\n{errors:#?}");
    }
}

#[test]
fn mounted_context_keeps_its_own_signature_types() {
    use verum_ast::FileId;
    use verum_fast_parser::FastParser;
    use verum_modules::{
        ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
    };
    for (argument, accepted) in [("demo.jobs.Token", true), ("Token", false)] {
        let consumer = format!(
            r#"
            mount demo.jobs.{{Jobs as Worker}};
            type Token is {{ local:Bool }};
            fn probe<F:fn({argument})->Bool>(cb:F)->Bool using[Worker] {{ Worker.run(cb) }}
        "#
        );
        let mut registry = ModuleRegistry::new();
        for (index, (owner, source)) in [
            ("demo.jobs", "public type Token is { remote:Int }; public context Jobs { fn run<F:fn(Token)->Bool>(cb:F)->Bool; }"),
            ("demo.main", consumer.as_str()),
        ].into_iter().enumerate() {
            let file = FileId::new(index as u32);
            let ast = FastParser::new().parse_module_str(source, file).expect("valid module");
            let id = ModuleId::new(index as u32);
            let path = ModulePath::from_str(owner);
            let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), file, source.into());
            info.exports = extract_exports_from_module(&ast, id, &path).expect("exports");
            registry.register(info);
        }
        let ast = registry.get_by_path("demo.main").unwrap().ast.clone();
        let mut checker = TypeChecker::new();
        checker.register_primitives();
        checker.set_current_cog("demo");
        checker.set_current_module_path("demo.main");
        checker.set_module_registry_direct(registry);
        let mut errors: List<Text> = checker
            .prepare_module_declarations(&ast)
            .iter()
            .map(|error| format!("{error:?}").into())
            .collect();
        for item in &ast.items {
            if let Err(error) = checker.check_item(item) {
                errors.push(format!("{error:?}").into());
            }
        }
        errors.extend(
            checker
                .take_deferred_errors()
                .iter()
                .map(|e| format!("{e:?}").into()),
        );
        errors.extend(
            checker
                .diagnostics()
                .iter()
                .filter(|d| d.is_error())
                .map(|d| format!("{d:?}").into()),
        );
        assert_eq!(errors.is_empty(), accepted, "{consumer}\n{errors:#?}");
    }
}
