#![cfg(feature = "codegen")]
//! Await combines constructor and contextual declarations without confusing
//! owner binders with the enclosing callable's generic parameter IDs.
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    module::VbcModule,
    serialize::serialize_module,
};

const ENV: &str = r#"
type Poll<T> is Ready(T) | Pending;
type Waker is { marker: Int };
type Context is { waker: &Waker };
implement Context {
    fn from_waker(waker: &Waker) -> Context { Context { waker: waker } }
}
fn noop_waker() -> Waker { Waker { marker: 0 } }
type Thread is ();
implement Thread { fn yield_now() {} }
type Future is protocol {
    type Output;
    fn poll(&mut self, cx: &mut Context) -> Poll<Self.Output>;
};
type Maybe<T> is None | Some(T);
type Tagged<P, T> is { value: T };
implement<P, T> Future for Tagged<P, T> {
    type Output = Maybe<P>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Maybe<P>> {
        let value = self.value;
        let f = || value;
        f();
        Poll.Ready(Maybe.None)
    }
}
type Ready<T> is { value: T };
implement<T> Future for Ready<T> {
    type Output = T;
    fn poll(&mut self, _cx: &mut Context) -> Poll<T> { Poll.Ready(self.value) }
}
"#;

fn compile(body: &str, bootstrap: bool) -> Result<VbcModule, String> {
    let source = format!("{ENV}\n{body}");
    let ast = Parser::new(&source).parse_module().expect("source grammar");
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("future_declarations"));
    if bootstrap {
        compiler
            .collect_unit_declarations(&[&ast])
            .map_err(|error| error.to_string())?;
        compiler
            .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
            .map_err(|error| error.to_string())?;
        compiler
            .finalize_module_from_state()
            .map_err(|error| error.to_string())
    } else {
        compiler
            .compile_module(&ast)
            .map_err(|error| error.to_string())
    }
}

fn run(body: &str, expected: i64) {
    for bootstrap in [false, true] {
        let module = compile(body, bootstrap)
            .unwrap_or_else(|error| panic!("bootstrap={bootstrap}: {error}"));
        let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        let entry = module
            .find_function_by_name("future_declarations.probe")
            .unwrap();
        let result = Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap_or_else(|error| panic!("bootstrap={bootstrap}: {error}"));
        assert!(result.is_int(), "bootstrap={bootstrap}: {result:?}");
        assert_eq!(result.as_i64(), expected, "bootstrap={bootstrap}");
    }
}

fn refuse(body: &str, message: &str) {
    for bootstrap in [false, true] {
        let error = compile(body, bootstrap)
            .expect_err("missing/conflicting declaration must refuse compilation");
        assert!(error.contains(message), "bootstrap={bootstrap}: {error}");
    }
}

#[test]
fn contextual_output_and_field_values_use_distinct_owner_and_caller_binders() {
    run(
        r#"
async fn choose<X, P, T>(x: X, value: T) -> Maybe<P> {
    Tagged { value: value }.await
}

async fn probe() -> Int {
    match choose<Int, Bool, Int>(0, 7).await { Maybe.None => 37, Maybe.Some(_) => 0 }
}
"#,
        37,
    );
}

#[test]
fn method_shadow_binders_remain_distinct_from_the_generic_receiver() {
    run(
        r#"
type Factory<P> is { marker: P };
implement<P> Factory<P> {
    async fn choose<P, T>(&self, value: T) -> Maybe<P> { Tagged { value: value }.await }
}
async fn probe() -> Int {
    let factory: Factory<Int> = Factory { marker: 0 };
    match factory.choose<Bool, Int>(7).await { Maybe.None => 37, Maybe.Some(_) => 0 }
}
"#,
        37,
    );
}

#[test]
fn parenthesized_annotated_initializer_supplies_only_its_own_output_context() {
    run(
        r#"
async fn probe() -> Int {
    let result: Maybe<Bool> = (((Tagged { value: 7 }.await)));
    match result { Maybe.None => 37, Maybe.Some(_) => 0 }
}
"#,
        37,
    );
}

#[test]
fn annotated_output_conflicting_with_the_constructed_future_is_rejected() {
    refuse(
        r#"
async fn probe() -> Int {
    let value: Bool = (Ready { value: 7 }.await);
    if value { 37 } else { 0 }
}
"#,
        "Future.Output conflicts with its contextual declaration",
    );
}

#[test]
fn discarded_phantom_future_cannot_take_evidence_from_enclosing_return_type() {
    refuse(
        r#"
async fn probe() -> Int {
    Tagged { value: 7 }.await;
    37
}
"#,
        "has no complete ownership declaration",
    );
}

#[test]
fn nested_await_does_not_consume_a_binary_initializers_bool_annotation() {
    run(
        r#"
async fn probe() -> Int {
    let equal: Bool = (Ready { value: 7 }.await) == 7;
    if equal { 37 } else { 0 }
}
"#,
        37,
    );
}

#[test]
fn renamed_reordered_impl_parameters_normalize_associated_output_to_owner_scope() {
    let body = r#"
type Pair<X, Y> is { left: X, right: Y };
implement<A, B> Future for Pair<B, A> {
    type Output = A;
    fn poll(&mut self, _cx: &mut Context) -> Poll<A> { Poll.Ready(self.right) }
}
async fn probe() -> Int {
    let ready: Bool = Pair { left: 5, right: true }.await;
    if ready { 37 } else { 0 }
}
"#;
    for bootstrap in [false, true] {
        let module = compile(body, bootstrap)
            .unwrap_or_else(|error| panic!("bootstrap={bootstrap}: {error}"));
        let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        let pair = module
            .types
            .iter()
            .find(|ty| {
                module
                    .get_string(ty.name)
                    .is_some_and(|name| name == "Pair" || name == "future_declarations.Pair")
            })
            .unwrap();
        let output = pair
            .protocols
            .iter()
            .flat_map(|protocol| &protocol.associated_types)
            .find(|(name, _)| module.get_string(*name) == Some("Output"))
            .unwrap();
        assert_eq!(
            output.1,
            verum_vbc::types::TypeRef::Generic(pair.type_params[1].id),
            "Output=A must refer to the owner's second argument, not an impl-local ID"
        );
        let entry = module
            .find_function_by_name("future_declarations.probe")
            .unwrap();
        let result = Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap_or_else(|error| panic!("bootstrap={bootstrap}: {error}"));
        assert!(result.is_int(), "bootstrap={bootstrap}: {result:?}");
        assert_eq!(result.as_i64(), 37, "bootstrap={bootstrap}");
    }
}

#[test]
fn lifetime_only_future_keeps_borrowed_fields_and_composite_output_declaration() {
    run(
        r#"
type Result<T, E> is Ok(T) | Err(E);
type Borrowed<'a> is { value: &'a mut Int };
implement<'a> Future for Borrowed<'a> {
    type Output = Result<Int, Bool>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Result<Int, Bool>> {
        *self.value = *self.value + 1;
        Poll.Ready(Result.Ok(*self.value))
    }
}
async fn read<'a>(value: &'a mut Int) -> Result<Int, Bool> { Borrowed { value: value }.await }
async fn probe() -> Int {
    let mut value = 36;
    match read(&mut value).await {
        Result.Ok(answer) => { assert(value == 37); answer },
        Result.Err(_) => 0,
    }
}
"#,
        37,
    );
}

#[test]
fn unannotated_local_constructor_preserves_its_complete_future_declaration() {
    run(
        r#"
async fn probe() -> Int {
    let ready = Ready { value: 7 };
    ready.await
}
"#,
        7,
    );
}

#[test]
fn contextual_field_instantiates_a_declared_static_constructor() {
    run(
        r#"
type Cell<T> is { stored: Maybe<T> };
implement<T> Cell<T> {
    fn empty() -> Cell<T> { Cell { stored: Maybe.None } }
}
type Held<T> is { cell: Cell<T> };
implement<T> Future for Held<T> {
    type Output = Maybe<T>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Maybe<T>> { Poll.Ready(Maybe.None) }
}
async fn probe() -> Int {
    let value: Maybe<Bool> = Held { cell: Cell.empty() }.await;
    match value { Maybe.None => 37, Maybe.Some(_) => 0 }
}
"#,
        37,
    );
}

#[test]
fn future_output_resolves_nested_associated_members_of_its_declared_receiver() {
    run(r#"
type Stream is protocol { type Item; };
type AsyncIterator is protocol { type Item; };
type Source<T> is { value: T };
implement<T> Stream for Source<T> { type Item = T; }
implement<T> AsyncIterator for Source<T> { type Item = T; }
type Next<S: Stream> is { source: &mut S };
implement<S: Stream> Future for Next<S> {
    type Output = Maybe<S.Item>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Maybe<S.Item>> { Poll.Ready(Maybe.None) }
}
async fn probe() -> Int {
    let mut source = Source { value: 7 };
    let result: Maybe<Int> = Next { source: &mut source }.await;
    match result { Maybe.None => 37, Maybe.Some(_) => 0 }
}
"#, 37);
}

#[test]
fn conflicting_associated_members_cannot_authorize_future_output() {
    refuse(r#"
type Stream is protocol { type Item; };
type Other is protocol { type Item; };
type Source<T> is { value: T };
implement<T> Stream for Source<T> { type Item = T; }
implement<T> Other for Source<T> { type Item = Bool; }
type Next<S: Stream> is { source: &mut S };
implement<S: Stream> Future for Next<S> {
    type Output = Maybe<S.Item>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Maybe<S.Item>> { Poll.Ready(Maybe.None) }
}
async fn probe() -> Int {
    let mut source = Source { value: 7 };
    let result: Maybe<Int> = Next { source: &mut source }.await;
    37
}
"#, "Future.Output has no ownership declaration");
}
