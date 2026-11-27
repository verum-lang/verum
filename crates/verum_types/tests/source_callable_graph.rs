//! Declaration preparation must bound shared source-call graphs without
//! borrowing another declaration's synchrony or overlooking a reachable call.
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use verum_ast::{FileId, ItemKind};
use verum_common::Text;
use verum_fast_parser::{Parser, VerumParser};
use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
use verum_types::{Type, TypeChecker, computational_properties::PropertyRequirement};

fn checker(source: &str) -> TypeChecker {
    let module = Parser::new(source)
        .parse_module()
        .expect("production grammar");
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    let errors = checker.prepare_module_declarations(&module);
    assert!(errors.is_empty(), "{errors:?}");
    checker
}

fn bound(checker: &TypeChecker, name: &str) -> PropertyRequirement {
    let scheme = checker.ctx_env_lookup(name).expect("prepared declaration");
    let Type::Function {
        property_requirement,
        ..
    } = &scheme.ty
    else {
        panic!("expected callable, found {:?}", scheme.ty);
    };
    property_requirement.clone()
}

fn diamond(levels: usize) -> Text {
    let mut source = Text::from("fn leaf0()->Int {1}\n");
    for level in 1..=levels {
        let previous = level - 1;
        source.push_str(&format!(
            "fn leaf{level}()->Int {{leaf{previous}()+leaf{previous}()}}\n"
        ));
    }
    source
}

#[test]
fn shared_diamond_retains_a_synchronous_contract() {
    let checked = checker(&diamond(5));
    assert!(!bound(&checked, "leaf5").may_be_async());
}

#[test]
fn a_closed_recursive_component_excludes_suspension() {
    for source in [
        "fn first()->Int {first()}",
        "fn first()->Int {second()} fn second()->Int {first()}",
        "fn first()->Int {leaf()+second()} fn second()->Int {first()} fn leaf()->Int {1}",
    ] {
        assert_eq!(bound(&checker(source), "first"), PropertyRequirement::source_callable(false), "{source}");
    }
}

#[test]
fn a_recursive_component_checks_its_exits_and_contracts() {
    for source in [
        "fn first()->Int {second()} fn second()->Int {first()+leaf()} async fn leaf()->Int {1}",
        "fn first()->Int {leaf()+second()} fn second()->Int {first()} async fn leaf()->Int {1}",
        "fn first()->Int {second()} fn second()->Int {first()+missing()}",
        "fn first()->Int {second()} fn second()->Int requires missing_guard() {first()}",
        "fn first()->Int {second()} fn second()->Int ensures missing_guard() {first()}",
        "fn first()->Int {second()} fn second()->Int requires ready() {first()} async fn ready()->Bool {true}",
    ] {
        assert!(bound(&checker(source), "first").may_be_async(), "{source}");
    }
}

#[test]
fn a_declared_callback_bounds_calls_through_its_parameter() {
    for source in [
        "fn invoke(callback:fn(Int)->Int)->Int {callback(4)}",
        "fn invoke<F:fn(Int)->Int>(callback:F)->Int {callback(4)}",
        "fn invoke<F>(callback:F)->Int where F:fn(Int)->Int {callback(4)}",
    ] {
        assert!(!bound(&checker(source), "invoke").may_be_async(), "{source}");
    }
    for source in [
        "fn invoke(callback:async fn(Int)->Int)->Int {callback(4)}",
        "fn invoke<F:async fn(Int)->Int>(callback:F)->Int {callback(4)}",
        "type Pending is async fn(Int)->Int;
         fn invoke(callback:Pending)->Int where Pending:fn(Int)->Int {callback(4)}",
        "fn invoke(callback:fn(Int)->Int, pending:async fn(Int)->Int)->Int {
             let callback = pending; callback(4)
         }",
        "fn invoke(callback:fn(Int)->Int, pending:async fn(Int)->Int)->Int {
             callback(4) + pending(4)
         }",
    ] {
        assert!(bound(&checker(source), "invoke").may_be_async(), "{source}");
    }
}

#[test]
fn lexical_variant_construction_keeps_the_selected_payload_call() {
    let source = "type Answer is Good(Int) | Bad(Int);
        fn mapped<F:fn(Int)->Int>(value:Answer, f:F)->Answer {
            match value { Good(n) => Good(f(n)), Bad(e) => Bad(e) }
        }
        fn pending(value:Answer, f:async fn(Int)->Int)->Answer {
            match value { Good(n) => Good(f(n)), Bad(e) => Bad(e) }
        }
        fn shadow(Good:async fn(Int)->Answer)->Answer { Good(3) }";
    let prepared = checker(source);
    assert!(!bound(&prepared, "mapped").may_be_async());
    assert!(bound(&prepared, "pending").may_be_async());
    assert!(bound(&prepared, "shadow").may_be_async());
}

#[test]
fn async_leaves_and_unknown_callees_remain_unproved() {
    for source in [
        "async fn leaf()->Int {1} fn root()->Int {leaf()}",
        "fn root()->Int {missing()}",
        "fn leaf()->Int {1} fn root(leaf:async fn()->Int)->Int {leaf()}",
    ] {
        assert!(bound(&checker(source), "root").may_be_async(), "{source}");
    }
}

fn registry(providers: &[(&str, &str)]) -> ModuleRegistry {
    let mut registry = ModuleRegistry::new();
    for (owner, source) in providers {
        let id = registry.allocate_id();
        let file = FileId::new(id.as_u32());
        let module = VerumParser::new()
            .parse_module_str(source, file)
            .expect("source grammar");
        let path = ModulePath::from_str(owner);
        let exports = extract_exports_from_module(&module, id, &path).expect("source exports");
        let mut info = ModuleInfo::new(id, path, module, file, (*source).into());
        info.exports = exports;
        registry.register(info);
    }
    registry
}

#[test]
fn same_leaf_names_keep_their_declaring_owner() {
    let source = "fn accept()->Int {providers.ready.leaf()}
        fn refuse()->Int {providers.ready.leaf()+providers.pending.leaf()}";
    let registry = registry(&[
        ("providers.ready", "public fn leaf()->Int {1}"),
        ("providers.pending", "public async fn leaf()->Int {1}"),
        ("cog", source),
    ]);
    let module = registry.get_by_path("cog").unwrap();
    let mut checked = TypeChecker::with_minimal_context();
    checked.register_primitives();
    checked.set_module_registry_direct(registry);
    checked.set_current_module_path("cog");
    let errors = checked.prepare_module_declarations(&module.ast);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(!bound(&checked, "accept").may_be_async());
    assert!(bound(&checked, "refuse").may_be_async());
}

#[test]
fn a_cycle_across_modules_preserves_the_exact_owner_of_an_exit() {
    let source = "fn entry(n:Int)->Int {providers.left.step(n)}";
    for (exit, synchronous) in [("public fn exit()->Int {0}", true),
                               ("public async fn exit()->Int {0}", false)] {
        let registry = registry(&[
            ("providers.left", "public fn step(n:Int)->Int {providers.right.step(n)}"),
            ("providers.right", "public fn step(n:Int)->Int {
                if n > 0 { providers.left.step(n-1) } else { providers.end.exit() }
            }"),
            ("providers.end", exit),
            ("cog", source),
        ]);
        let module = registry.get_by_path("cog").unwrap();
        let mut checked = TypeChecker::with_minimal_context();
        checked.register_primitives();
        checked.set_module_registry_direct(registry);
        checked.set_current_module_path("cog");
        let errors = checked.prepare_module_declarations(&module.ast);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(!bound(&checked, "entry").may_be_async(), synchronous, "{exit}");
    }
}

#[test]
fn generated_same_span_declarations_cannot_share_a_proof() {
    let source = "type Ready is (); type Pending is ();
        implement Ready {fn leaf()->Int {1}}
        implement Pending {async fn leaf()->Int {1}}
        fn accept()->Int {Ready.leaf()+Ready.leaf()}
        fn refuse()->Int {Ready.leaf()+Pending.leaf()}";
    let mut module = Parser::new(source).parse_module().unwrap();
    let generated = verum_ast::Span::new(0, 0, FileId::new(7));
    for item in &mut module.items {
        if let ItemKind::Impl(implementation) = &mut item.kind {
            for item in &mut implementation.items {
                if let verum_ast::decl::ImplItemKind::Function(function) = &mut item.kind {
                    function.span = generated;
                    function.name.span = generated;
                }
            }
        }
    }
    let mut checked = TypeChecker::with_minimal_context();
    checked.register_primitives();
    let errors = checked.prepare_module_declarations(&module);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(!bound(&checked, "accept").may_be_async());
    assert!(bound(&checked, "refuse").may_be_async());
}

fn chain_bound(levels: usize) -> PropertyRequirement {
    let mut source = Text::from("fn leaf0()->Int {1}\n");
    for level in 1..=levels {
        source.push_str(&format!("fn leaf{level}()->Int {{leaf{}()}}\n", level - 1));
    }
    let registry = registry(&[("cog", &source)]);
    let module = registry.get_by_path("cog").unwrap();
    let mut checked = TypeChecker::with_minimal_context();
    checked.register_primitives();
    checked.set_module_registry_direct(registry);
    checked.set_current_module_path("cog");
    let ItemKind::Function(function) = &module.ast.items.last().unwrap().kind else {
        unreachable!()
    };
    checked.register_function_signature(function).unwrap();
    bound(&checked, &format!("leaf{levels}"))
}

#[test]
fn long_acyclic_graph_uses_the_ordinary_thread_stack() {
    assert!(!chain_bound(256).may_be_async());
}

#[test]
fn excessive_source_call_work_remains_unproved() {
    assert!(chain_bound(4097).may_be_async());
}

#[test]
fn shared_diamond_work_is_bounded() {
    const CHILD: &str = "VERUM_CALLABLE_DIAMOND_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let checked = checker(&diamond(20));
        assert!(!bound(&checked, "leaf20").may_be_async());
        return;
    }
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "shared_diamond_work_is_bounded",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let timed_out = loop {
        if child.try_wait().unwrap().is_some() {
            break false;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            break true;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = child.wait_with_output().unwrap();
    assert!(
        !timed_out && output.status.success(),
        "shared source-call work did not finish: timeout={timed_out}, status={}\n{}\n{}",
        output.status,
        Text::from_utf8_lossy(&output.stdout),
        Text::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn typed_method_chains_use_the_implementation_selected_by_the_consumer() {
    let provider = r#"
        public type Ready is { public number: Int };
        implement Ready {
            public fn leaf(&self)->Int { self.number }
            public fn middle(&self)->Int { self.leaf() }
            public fn read(&self)->Int { self.middle() }
        }
        public type Open is { public callback: async fn(Int)->Int };
        implement Open {
            public fn read(&self)->Int { (self.callback)(1) }
        }
        public type Pending is { public number: Int };
        implement Pending {
            public async fn leaf(&self)->Int { self.number }
            public fn middle(&self)->Int { self.leaf() }
            public fn read(&self)->Int { self.middle() }
        }
        public type Measure is protocol {
            fn read(&self)->Int;
            fn unit(&self)->Int;
        };
        public type Measured is { public ready: Ready };
        implement Measure for Measured {
            fn read(&self)->Int { self.ready.read() }
            fn unit(&self)->Int { 1 }
        }
        public type Deferred is { public open: Open };
        implement Measure for Deferred {
            fn read(&self)->Int { self.open.read() }
            fn unit(&self)->Int { 1 }
        }
        public type Alternative is protocol { fn read(&self)->Int; };
        public type Ambiguous is { public ready: Ready, public open: Open };
        implement Measure for Ambiguous {
            fn read(&self)->Int { self.ready.read() }
            fn unit(&self)->Int { 1 }
        }
        implement Alternative for Ambiguous {
            fn read(&self)->Int { self.open.read() }
        }
        public type Guarded is { public ready: Ready };
        implement Guarded {
            public unsafe fn leaf(&self)->Int { self.ready.read() }
            public fn read(&self)->Int { unsafe { self.leaf() } }
        }
        public type Indirect is { public open: Open };
        implement Indirect {
            public unsafe fn leaf(&self)->Int { self.open.read() }
            public fn read(&self)->Int { unsafe { self.leaf() } }
        }
    "#;
    for (receiver, accepts) in [
        ("Ready", true), ("Pending", false), ("Open", false),
        ("Measured", true), ("Deferred", false), ("Measure", false),
        ("Guarded", true), ("Indirect", false), ("Ambiguous", false),
    ] {
        let consumer = format!("mount provider.{receiver};
            fn call(input:{receiver})->Int {{
                let read:fn({receiver})->Int = |value| value.read();
                read(input)
            }}
            fn infer_first(input:{receiver})->Int {{
                let inferred = |value:{receiver}| value.read();
                let read:fn({receiver})->Int = inferred;
                read(input)
            }}");
        let graph = registry(&[("provider", provider), ("consumer", &consumer)]);
        let module = graph.get_by_path("consumer").unwrap();
        let mut checked = TypeChecker::with_minimal_context();
        checked.register_builtins();
        checked.set_current_module_path("consumer");
        checked.set_module_registry_direct(graph);
        checked.register_stdlib_types_for_module(&module.ast);
        let mut errors = checked.prepare_module_declarations(&module.ast);
        checked.with_module_declarations(&module.ast, |checked| {
            for item in &module.ast.items {
                if let Err(error) = checked.check_item(item) { errors.push(error); }
            }
        });
        errors.extend(checked.take_deferred_errors());
        let errors: verum_common::List<_> = errors.iter().chain(checked.diagnostic_sources()).collect();
        if accepts {
            assert!(errors.is_empty(), "{receiver}: {errors:?}");
            let site = "value.read()";
            for (offset, _) in consumer.match_indices(site) {
                let span = verum_ast::Span::new(offset as u32, (offset + site.len()) as u32, module.file_id);
                assert!(matches!(checked.type_registry.lookup_call_target(span),
                    Some(Type::Function { property_requirement, .. }) if !property_requirement.may_be_async()));
            }
        } else {
            let unresolved = errors.iter().any(|error| matches!(error, verum_types::TypeError::Mismatch {
                expected, actual, span,
            } if expected.contains("excludes {Async}") && actual.contains("unrestricted")
                && &consumer[span.start as usize..span.end as usize] == "|value| value.read()"));
            assert!(unresolved || (receiver == "Ambiguous" && errors.iter().any(|error|
                matches!(error, verum_types::TypeError::AmbiguousMethod { .. }))), "{receiver}: {errors:?}");
        }
    }
}
