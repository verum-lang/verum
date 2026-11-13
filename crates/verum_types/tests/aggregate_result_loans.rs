use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let declarations = "
        type Node is { public n: Int };
        type Batch<T> is Empty | Full(T);
        type Sample is { public ptr: &Int, public total: Int };
        implement Node {
            fn view(&self) -> Batch<(&Int, &Int)> { Batch.Full((&self.n, &self.n)) }
            fn change(&mut self) { self.n = 2; }
            fn count(&self) -> Int { self.n }
        }
        fn discard(x: Node) {}
    ";
    let source = format!("{declarations}\n{source}");
    let module = FastParser::new()
        .parse_module_str(&source, FileId::new(1))
        .unwrap();
    let mut checker = TypeChecker::new();
    checker.set_current_module_path("sample");
    checker.register_primitives();
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    let mut messages: List<_> = errors
        .iter()
        .map(|error| error.to_string().into())
        .collect();
    messages.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| Text::from(d.message())),
    );
    messages
}

#[test]
fn references_in_returned_data_cannot_outlive_their_storage() {
    for body in [
        "fn leak() -> Batch<(&Int, &Int)> { let x = Node { n: 1 }; x.view() }",
        "fn leak() -> (&Int, &Int) { let x = 1; (&x, &x) }",
        "fn leak() -> Batch<(&Int, &Int)> { let x = Node { n: 1 }; let held = x.view(); held }",
        "fn leak(x: Node) -> Batch<(&Int, &Int)> { x.view() }",
    ] {
        let diagnostics = errors(body);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.contains("does not live long enough")),
            "{body}: {diagnostics:?}"
        );
    }
}

#[test]
fn a_bound_payload_keeps_its_owner_borrowed() {
    let nested = "type Outer is { public node: Node }; fn take(o: Outer) {}
        fn run() { let o = Outer { node: Node { n: 1 } }; match o.node.view() {
            Batch.Full(pair) => { take(o); let _last = *pair.0; }, Batch.Empty => {}
        } }";
    assert!(
        errors(nested).iter().any(|d| d.contains("cannot move")),
        "{nested}"
    );
    for (action, expected) in [
        ("discard(x)", "cannot move"),
        ("let moved = x", "cannot move"),
        ("x.change()", "cannot borrow"),
    ] {
        let source = format!(
            "fn test() {{
            let mut x = Node {{ n: 1 }};
            match x.view() {{
                Batch.Full(pair) => {{ {action}; let _last = *pair.0; }},
                Batch.Empty => {{}}
            }}
        }}"
        );
        assert!(
            errors(&source).iter().any(|d| d.contains(expected)),
            "{source}"
        );
    }
}

#[test]
fn consuming_receivers_forward_stored_loans_without_borrowing_their_own_slot() {
    let declarations = "
        implement Node { fn finish(mut self, value: &Int) -> Sample { self.n = 2; Sample { ptr: value, total: self.n } } }
        implement Sample { fn finish(self) -> Sample { self } }
    ";
    let good = format!("{declarations} fn build(value: &Int) -> Sample {{
        let config = Node {{ n: 1 }}; config.finish(value) }}");
    assert_eq!(errors(&good), List::<Text>::new());
    for body in [
        "fn leak() -> Sample { let local = 9; let stored = Sample { ptr: &local, total: 1 }; stored.finish() }",
        "fn leak() -> Sample { let local = 9; let config = Node { n: 1 }; config.finish(&local) }",
        "fn leak() -> Batch<(&Int, &Int)> { let config = Node { n: 1 }; config.view() }",
    ] {
        let diagnostics = errors(&format!("{declarations} {body}"));
        assert!(diagnostics.iter().any(|d| d.contains("does not live long enough")), "{body}: {diagnostics:?}");
    }
}

#[test]
fn forwarding_shared_inputs_and_mutating_outside_the_borrow_are_valid() {
    for source in [
        "fn forward(x: &Int) -> Sample { let n = Node { n: 1 }; Sample { ptr: x, total: n.count() } }",
        "fn forward(x: &Node) -> Batch<(&Int, &Int)> { x.view() }",
        "fn forward(x: Batch<(&Int, &Int)>) -> Batch<(&Int, &Int)> { x }",
        "fn forward(x: &Int) -> (&Int, Int) { let counter = Node { n: 1 }; (x, counter.count()) }",
        "fn run() { let mut x = Node { n: 1 }; match x.view() {
            Batch.Full(pair) => { let _last = *pair.0; }, Batch.Empty => x.change()
        } x.change(); }",
    ] {
        assert_eq!(errors(source), List::<Text>::new(), "{source}");
    }
}

#[test]
fn concrete_owned_generic_results_do_not_retain_transient_input_loans() {
    for source in [
        "type Packed is (Int);
         fn copy_count(x: &Int) -> Packed { Packed(*x) }
         fn owned() -> Packed { let x = 7; copy_count(&x) }",
        "type Packed is (Int, Bool);
         fn copy_count(x: &Int) -> Packed { Packed(*x, true) }
         fn owned() -> Packed { let x = 7; copy_count(&x) }",
        "type Marker is ();
         fn copy_count(x: &Int) -> Marker { Marker }
         fn owned() -> Marker { let x = 7; copy_count(&x) }",
        "fn copy_count(x: &Int) -> Batch<Int> { Batch.Full(*x) }
         fn owned() -> Batch<Int> { let x = 7; copy_count(&x) }",
        "type Parcel<T> is { public value: T };
         fn copy_count(x: &Int) -> Parcel<Int> { Parcel { value: *x } }
         fn owned() -> Parcel<Int> { let x = 7; copy_count(&x) }",
        "type Parcel<T> is { public value: T };
         fn copy_count(x: &Int) -> Batch<Parcel<Int>> { Batch.Full(Parcel { value: *x }) }
         fn owned() -> Batch<Parcel<Int>> { let x = 7; copy_count(&x) }",
        "type Marker<T> is { public count: Int };
         fn copy_count(x: &Int) -> Marker<&Int> { Marker { count: *x } }
         fn owned() -> Marker<&Int> { let x = 7; copy_count(&x) }",
        "type Parcel<T> is { public value: T };
         type Selected<A, B> is Parcel<B>;
         fn copy_count(x: &Int) -> Selected<&Int, Int> { Parcel { value: *x } }
         fn owned() -> Selected<&Int, Int> { let x = 7; copy_count(&x) }",
        "type Chain<T> is End | Link(T, Chain<T>);
         fn copy_count(x: &Int) -> Chain<Int> { Chain.Link(*x, Chain.End) }
         fn owned() -> Chain<Int> { let x = 7; copy_count(&x) }",
    ] {
        assert_eq!(errors(source), List::<Text>::new(), "{source}");
    }
}

#[test]
fn instantiated_fields_still_reject_local_reference_escape() {
    for source in [
        "type Packed is (&Int);
         fn hold(x: &Int) -> Packed { Packed(x) }
         fn leak() -> Packed { let x = 7; hold(&x) }",
        "type Packed is (Bool, &Int);
         fn hold(x: &Int) -> Packed { Packed(true, x) }
         fn leak() -> Packed { let x = 7; hold(&x) }",
        "type Parcel<T> is { public value: T };
         fn hold(x: &Int) -> Parcel<&Int> { Parcel { value: x } }
         fn leak() -> Parcel<&Int> { let x = 7; hold(&x) }",
        "type Parcel<T> is { public value: T };
         type Selected<A, B> is Parcel<B>;
         fn hold(x: &Int) -> Selected<Int, &Int> { Parcel { value: x } }
         fn leak() -> Selected<Int, &Int> { let x = 7; hold(&x) }",
        "type Fixed<T> is { public count: T, public ptr: &Int };
         fn hold(x: &Int) -> Fixed<Int> { Fixed { count: 3, ptr: x } }
         fn leak() -> Fixed<Int> { let x = 7; hold(&x) }",
        "type Chain<T> is End | Link(T, Chain<T>);
         fn hold(x: &Int) -> Chain<&Int> { Chain.Link(x, Chain.End) }
         fn leak() -> Chain<&Int> { let x = 7; hold(&x) }",
        "type Parcel<T> is { public value: T };
         fn hold(x: &Int) -> Parcel<fn() -> Int> { Parcel { value: || *x } }
         fn leak() -> Parcel<fn() -> Int> { let x = 7; hold(&x) }",
    ] {
        let diagnostics = errors(source);
        assert!(
            diagnostics
                .iter()
                .any(|message| message.contains("does not live long enough")),
            "{source}: {diagnostics:?}"
        );
    }
}

#[test]
fn distinct_nested_parameter_scopes_keep_the_selected_payload() {
    let declarations = "type Inner<T> is { public value: T };
        type Outer<T, U> is { public second: Inner<U>, public first: T };";
    let owned = format!(
        "{declarations}
        fn copy_count(x: &Int) -> Outer<Bool, Int> {{
            Outer {{ first: true, second: Inner {{ value: *x }} }}
        }}
        fn owned() -> Outer<Bool, Int> {{ let x = 7; copy_count(&x) }}"
    );
    assert_eq!(errors(&owned), List::<Text>::new());
    let borrowed = format!(
        "{declarations}
        fn hold(x: &Int) -> Outer<Int, &Int> {{
            Outer {{ first: 3, second: Inner {{ value: x }} }}
        }}
        fn leak() -> Outer<Int, &Int> {{ let x = 7; hold(&x) }}"
    );
    let diagnostics = errors(&borrowed);
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("does not live long enough")),
        "{diagnostics:?}"
    );
}

#[test]
fn unbounded_instantiation_cannot_establish_reference_free_storage() {
    let diagnostics = errors(
        "type Expanding<T> is End | Next(Expanding<(T, T)>);
        fn make(x: &Int) -> Expanding<Int> { Expanding.End }
        fn probe() -> Expanding<Int> { let x = 7; make(&x) }",
    );
    // The shape analysis has a finite work budget. An unresolved recursive
    // instance keeps its input origins; exhaustion cannot prove them absent.
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("does not live long enough")),
        "{diagnostics:?}"
    );
}
