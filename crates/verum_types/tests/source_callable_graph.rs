//! Declaration preparation must bound shared source-call graphs without
//! borrowing another declaration's synchrony or proving recursive cycles.
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
fn recursive_cycles_remain_unproved() {
    for source in [
        "fn first()->Int {first()}",
        "fn first()->Int {second()} fn second()->Int {first()}",
        "fn first()->Int {leaf()+second()} fn second()->Int {first()} fn leaf()->Int {1}",
    ] {
        assert!(bound(&checker(source), "first").may_be_async(), "{source}");
    }
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
