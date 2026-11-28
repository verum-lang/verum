//! Nominal Future implementations retain their declared Output through imports and projections.
//! These parsed-source controls check both provider registration orders. They do not
//! execute a scheduler or prescribe a nominal carrier for keyword spawn.
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;

// The operator protocol has a canonical owner; the library handle is a
// separate nominal declaration and must explicitly implement that protocol.
const FUTURE: &str = r#"
    public type Poll<T> is Ready(T) | Pending;
    public type Context is ();
    public type Future is protocol {
        type Output;
        fn poll(&mut self, cx: &mut Context) -> Poll<Self.Output>;
    };
"#;

const TASKS: &str = r#"
    mount core.async.future.{Future, Poll, Context};
    public type Result<T, E> is Ok(T) | Err(E);
    public type JoinError is Cancelled | Panicked;
    public type JoinHandle<T> is { public value: T };
    implement<T> Future for JoinHandle<T> {
        type Output = Result<T, JoinError>;
        fn poll(&mut self, _cx: &mut Context) -> Poll<Result<T, JoinError>> {
            Poll.Pending
        }
    }
    @intrinsic("async_spawn")
    public fn spawn_with_env<F: Future>(future: F) -> JoinHandle<F.Output>;
"#;

const FOREIGN: &str = r#"
    public type JoinHandle<T> is { public value: T };
"#;

const MOUNTS: &str = r#"
    mount core.async.future.Future;
    mount demo.tasks.{Result, JoinError, JoinHandle};
"#;

fn check(body: &str, reverse: bool) -> (TypeChecker, List<Text>) {
    let consumer = format!("{MOUNTS}\n{body}");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("demo");
    checker.set_current_module_path("demo.main");
    let mut registry = ModuleRegistry::new();
    let mut providers = [
        ("core.async.future", FUTURE),
        ("demo.tasks", TASKS),
        ("demo.foreign", FOREIGN),
    ];
    let consumer_file = FileId::new(providers.len() as u32);
    let consumer_id = ModuleId::new(providers.len() as u32);
    if reverse {
        providers.reverse();
    }
    for (index, (owner, source)) in providers.iter().enumerate() {
        let file = FileId::new(index as u32);
        let ast = FastParser::new()
            .parse_module_str(source, file)
            .expect("provider source grammar");
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let mut module = ModuleInfo::new(id, path.clone(), ast.clone(), file, (*source).into());
        module.exports = extract_exports_from_module(&ast, id, &path).expect("provider exports");
        registry.register(module);
    }
    let ast = FastParser::new()
        .parse_module_str(&consumer, consumer_file)
        .expect("consumer source grammar");
    let id = consumer_id;
    let path = ModulePath::from_str("demo.main");
    let mut module = ModuleInfo::new(
        id,
        path.clone(),
        ast.clone(),
        consumer_file,
        consumer.into(),
    );
    module.exports = extract_exports_from_module(&ast, id, &path).expect("consumer exports");
    registry.register(module);
    checker.set_module_registry_direct(registry.clone());
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "demo.main", &registry) {
                errors.push(format!("import: {error:?}").into());
            }
        }
    }
    for item in &ast.items {
        let result = match &item.kind {
            ItemKind::Type(decl) => checker.register_type_declaration(decl),
            ItemKind::Impl(decl) => checker.register_impl_block(decl),
            ItemKind::Function(decl) => checker.register_function_signature(decl),
            _ => continue,
        };
        if let Err(error) = result {
            errors.push(format!("declaration: {error:?}").into());
        }
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("body: {error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("deferred: {e:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| Text::from(format!("diagnostic: {e:?}"))),
    );
    (checker, errors)
}

fn accepts(source: &str) {
    let mut failures = List::new();
    for reverse in [false, true] {
        let (_, errors) = check(source, reverse);
        if !errors.is_empty() {
            failures.push(Text::from(format!("reverse={reverse}: {errors:?}")));
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

fn refuses_type_mismatch(source: &str) {
    for reverse in [false, true] {
        let (_, errors) = check(source, reverse);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("Mismatch") || e.contains("E400")),
            "reverse={reverse}: expected a type refusal, got {errors:?}"
        );
        assert!(
            !errors
                .iter()
                .any(|e| e.starts_with("import:") || e.starts_with("declaration:")),
            "reverse={reverse}: refusal must come from the tested expression: {errors:?}"
        );
    }
}

#[test]
fn declared_handle_await_preserves_the_outer_task_result() {
    accepts(
        r#"
        async fn probe(handle: JoinHandle<Int>) -> Result<Int, JoinError> {
            handle.await
        }
    "#,
    );
}

#[test]
fn record_projection_await_preserves_the_same_associated_output() {
    accepts(
        r#"
        type Held is { handle: JoinHandle<Int> };
        async fn probe(held: Held) -> Result<Int, JoinError> { held.handle.await }
    "#,
    );
}

#[test]
fn renamed_handle_import_uses_the_declared_future_implementation() {
    accepts(
        r#"
        mount demo.tasks.{JoinHandle as Ticket};
        async fn probe(handle: Ticket<Int>) -> Result<Int, JoinError> { handle.await }
    "#,
    );
}

#[test]
fn a_nested_child_result_keeps_both_result_layers() {
    accepts(
        r#"
        async fn probe(handle: JoinHandle<Result<Int, Text>>) -> Result<Result<Int, Text>, JoinError> {
            handle.await
        }
    "#,
    );
}

#[test]
fn a_foreign_same_leaf_handle_stays_an_ordinary_value() {
    accepts(
        r#"
        mount demo.foreign.{JoinHandle as ForeignHandle};
        async fn probe(handle: ForeignHandle<Int>) -> ForeignHandle<Int> { handle.await }
    "#,
    );
}

#[test]
fn a_foreign_same_leaf_handle_does_not_unwrap_by_spelling() {
    refuses_type_mismatch(
        r#"
        mount demo.foreign.{JoinHandle as ForeignHandle};
        async fn probe(handle: ForeignHandle<Int>) -> Int { handle.await }
    "#,
    );
}

#[test]
fn an_actual_handle_cannot_drop_the_outer_result_layer() {
    refuses_type_mismatch(
        r#"
        async fn probe(handle: JoinHandle<Int>) -> Int { handle.await }
    "#,
    );
}

#[test]
fn eager_async_calls_continue_to_return_their_plain_value() {
    accepts(
        r#"
        async fn child() -> Int { 41 }
        fn main() {
            let before_await: Int = child();
            let after_await: Int = child().await;
        }
    "#,
    );
}

#[test]
fn fixed_array_element_preserves_the_declared_future_output() {
    accepts(
        r#"
        async fn probe(handles: [JoinHandle<Int>; 1]) -> Result<Int, JoinError> {
            handles[0].await
        }
    "#,
    );
}

#[test]
fn imported_spawn_signature_preserves_its_future_output_projection() {
    accepts(
        r#"
        mount demo.tasks.{spawn_with_env as submit};
        async fn probe(input: JoinHandle<Int>) -> Result<Result<Int, JoinError>, JoinError> {
            submit(input).await
        }
    "#,
    );
}

#[test]
fn await_uses_the_selected_future_output_under_caller_premises() {
    let declarations = r#"
        mount core.async.future.{Poll, Context};
        type Envelope<T, F: Future<Output=T>> is { public inner: F };
        implement<T, F: Future<Output=T>> Future for Envelope<T, F> {
            type Output = T;
            fn poll(&mut self, cx: &mut Context) -> Poll<T> { self.inner.poll(cx) }
        }
    "#;
    accepts(&format!("{declarations}
        async fn project<T,F:Future<Output=T>>(item:Envelope<T,F>)->T {{item.await}}
        async fn direct<T,F:Future<Output=T>>(item:F)->T {{item.await}}
    "));
    refuses_type_mismatch("async fn wrong<F:Future<Output=Int>>(item:F)->Text {item.await}");
    refuses_type_mismatch("async fn invented<T>(item:T)->Int {item.await}");
}

#[test]
fn an_unrelated_output_member_cannot_own_the_await_operator() {
    accepts(r#"
        type Decoy is protocol { type Output; };
        async fn project<F: Future<Output=Int> + Decoy<Output=Text>>(item:F)->Int {
            item.await
        }
    "#);
}

#[test]
fn constructing_a_future_recovers_only_the_outputs_proved_by_its_fields() {
    let declarations = r#"
        mount core.async.future.{Poll, Context};
        type Pair<A, B, X:Future<Output=A>, Y:Future<Output=B>> is { first:X, second:Y };
        implement<A,B,X:Future<Output=A>,Y:Future<Output=B>> Future for Pair<A,B,X,Y> {
            type Output = (A,B);
            fn poll(&mut self, _cx:&mut Context)->Poll<(A,B)> { Poll.Pending }
        }
    "#;
    accepts(&format!("{declarations}
        async fn collect<A,B,X:Future<Output=A>,Y:Future<Output=B>>(x:X,y:Y)->(A,B) {{
            Pair {{first:x, second:y}}.await
        }}
    "));
    refuses_type_mismatch(&format!("{declarations}
        async fn swapped<A,B,X:Future<Output=A>,Y:Future<Output=B>>(x:X,y:Y)->(B,A) {{
            Pair {{first:x, second:y}}.await
        }}
    "));
}

#[test]
fn structural_await_distinguishes_a_nominal_future_from_a_tuple_implementation() {
    accepts(r#"
        async fn plain<A,B>(pair:(A,B))->(A,B) { pair.await }
        async fn callable<F>(f:fn(F)->Int)->fn(F)->Int { f.await }
    "#);
    let implemented = r#"
        mount core.async.future.{Poll, Context};
        implement Future for (Int,Int) {
            type Output = Bool;
            fn poll(&mut self, _cx:&mut Context)->Poll<Bool> { Poll.Ready(true) }
        }
    "#;
    accepts(&format!("{implemented} async fn actual(pair:(Int,Int))->Bool {{ pair.await }}"));
    refuses_type_mismatch(&format!("{implemented} async fn wrong(pair:(Int,Int))->(Int,Int) {{ pair.await }}"));
}
