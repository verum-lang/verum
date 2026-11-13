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

#[test]
fn a_match_retains_the_value_selected_by_each_live_arm() {
    for source in [
        "fn project(stable: &Int) -> Sample { let scratch = 42; match &scratch { _ => Sample { ptr: stable, total: 7 } } }",
        "fn project(stable: &Int) -> Sample { let scratch = 42; let result = match &scratch { _ => Sample { ptr: stable, total: 7 } }; result }",
    ] {
        assert_eq!(errors(source), List::<Text>::new(), "{source}");
    }
    for source in [
        "fn invalid(stable: &Int) -> Sample { let scratch = 42; match &scratch { actual => Sample { ptr: actual, total: 7 } } }",
        "fn invalid(stable: &Int) -> Sample { let scratch = 42; let result = match &scratch { actual => Sample { ptr: actual, total: 7 } }; result }",
        "fn invalid(stable: &Int, c: Bool) -> Sample { let scratch = 42; match &scratch { actual if c => Sample { ptr: actual, total: 7 }, _ => Sample { ptr: stable, total: 7 } } }",
        "fn retain(value: &Int) -> Sample { Sample { ptr: value, total: 1 } }
         fn invalid() -> Sample { let local = 7; match &local { value => retain(value) } }",
        "type Deferred is { public read: fn() -> Int };
         fn invalid() -> Deferred { let local = 7; match &local { value => Deferred { read: || *value } } }",
    ] {
        let diagnostics = errors(source);
        assert!(diagnostics.iter().any(|error| error.contains("does not live long enough")), "{source}: {diagnostics:?}");
    }
}

#[test]
fn borrowing_a_projected_field_preserves_every_possible_referent() {
    let declarations = "
        fn select(a: &Node, b: &Node, c: Bool) -> &Node { if c { a } else { b } }
        fn view_field(value: &Int) -> Sample { Sample { ptr: value, total: 1 } }
    ";
    let valid = format!("{declarations}
        fn good(a: &Node, b: &Node, c: Bool) -> Sample {{
            let handle = select(a, b, c); view_field(&handle.n) }}");
    assert_eq!(errors(&valid), List::<Text>::new());
    for body in [
        "fn bad(a: &Node, c: Bool) -> Sample { let local = Node {n: 7};
            let handle = select(a, &local, c); view_field(&handle.n) }",
        "fn bad(a: &Node, c: Bool) -> Sample { let local = Node {n: 7};
            let handle = select(&local, a, c); view_field(&handle.n) }",
        "type Handle is &Node; type Cell is {public slot: &Handle};
         fn bad(a: &Node,b: &Node,c: Bool) -> Cell { let handle = select(a,b,c); Cell {slot: &handle} }",
    ] {
        let diagnostics = errors(&format!("{declarations} {body}"));
        assert!(diagnostics.iter().any(|error| error.contains("does not live long enough")),
            "{body}: {diagnostics:?}");
    }
}

#[test]
fn static_call_results_can_only_receive_caller_loans_through_the_declared_boundary() {
    let declarations = "
        type Reply is Empty | Field(&Int);
        type Factory is ();
        implement Factory {
            fn owned(count: Int) -> Reply { Reply.Empty }
            fn borrowed(value: &Int) -> Reply { Reply.Field(value) }
        }";
    let positive = format!("{declarations}
        fn accept() -> Reply {{ let local = 7; match &local {{ item => Factory.owned(*item) }} }}");
    assert_eq!(errors(&positive), List::<Text>::new());
    for body in [
        "fn reject() -> Reply { let local = 7; match &local { item => Factory.borrowed(item) } }",
        "fn reject() -> Reply { let local = 7; match &local { _ => Factory.borrowed(&local) } }",
        "fn reject(select: fn(&Int) -> Reply) -> Reply { let local = 7; match &local { item => select(item) } }",
        "context Gate { fn ping(); }
         implement Factory { fn uses(count: Int) -> Reply using [Gate] { Gate.ping(); Reply.Empty } }
         fn reject() -> Reply using [Gate] { let local = 7; match &local { item => Factory.uses(*item) } }",
        "implement Factory { fn borrowed_slice(items: [Int]) -> Reply { Reply.Empty } }
         fn reject(items: [Int]) -> Reply { let local = 7; match &local { _ => Factory.borrowed_slice(items) } }",
        "implement Factory { fn borrowed_raw(items: *const Int) -> Reply { Reply.Empty } }
         fn reject(items: *const Int) -> Reply { let local = 7; match &local { _ => Factory.borrowed_raw(items) } }",
        "implement Factory { fn borrowed_volatile(items: *volatile Int) -> Reply { Reply.Empty } }
         fn reject(items: *volatile Int) -> Reply { let local = 7; match &local { _ => Factory.borrowed_volatile(items) } }",
        "implement Factory { fn borrowed_unsafe(items: &unsafe Int) -> Reply { Reply.Empty } }
         fn reject(items: &unsafe Int) -> Reply { let local = 7; match &local { _ => Factory.borrowed_unsafe(items) } }",
    ] {
        let diagnostics = errors(&format!("{declarations} {body}"));
        assert!(
            !diagnostics.is_empty()
                && diagnostics.iter().all(|error| error.contains("does not live long enough")),
            "{body}: {diagnostics:?}"
        );
    }
}


#[test]
fn copied_fields_preserve_payload_origins_independently_of_owner_storage() {
    for source in [
        "fn forward(value: Sample) -> Sample { let alias = &value; Sample { ptr: alias.ptr, total: alias.total } }",
        "fn forward(value: Sample) -> Sample { let alias = &checked value; Sample { ptr: alias.ptr, total: alias.total } }",
        "type Source is &Sample; fn forward(value: Source) -> Sample { Sample { ptr: value.ptr, total: value.total } }",
        "fn relay(value: &Sample) -> &Sample { value } fn forward(value: Sample) -> Sample { Sample { ptr: relay(&value).ptr, total: value.total } }",
        "type Wrapper is { public value: Sample }; fn forward(value: Wrapper, condition: Bool) -> Sample { let alias = &value; match condition { true => alias.value, false => alias.value } }",
        "fn forward(value: &Sample) -> Sample { Sample { ptr: value.ptr, total: value.total } }",
        "fn forward(value: Sample) -> Sample { let first = &value; let second = &first; Sample { ptr: second.ptr, total: second.total } }",
        "type Wrapper is { public value: Sample }; fn forward(value: Wrapper) -> Sample { let alias = &value; Sample { ptr: alias.value.ptr, total: alias.value.total } }",
        "fn forward(value: (&Int, Int)) -> (&Int, Int) { let alias = &value; (alias.0, alias.1) }",
        "fn forward(ptr: &Int) -> Sample { let value = Sample { ptr: ptr, total: 1 }; let alias = &value; Sample { ptr: alias.ptr, total: alias.total } }",
        "fn forward(ptr: &Int) -> Sample { let value = Sample { ptr: ptr, total: 1 }; let alias = &value; { let n = 2; let value = Sample { ptr: &n, total: 3 }; Sample { ptr: alias.ptr, total: alias.total } } }",
    ] {
        let diagnostics = errors(source);
        assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
    }
}

#[test]
fn copied_fields_keep_real_local_payload_loans_and_explicit_storage_borrows() {
    for source in [
        "fn leak() -> Sample { let n = 1; let value = Sample { ptr: &n, total: 2 }; let alias = &value; Sample { ptr: alias.ptr, total: alias.total } }",
        "fn relay(value: &Sample) -> &Sample { value } fn leak() -> Sample { let n = 1; let value = Sample { ptr: &n, total: 2 }; Sample { ptr: relay(&value).ptr, total: value.total } }",
        "type Wrapper is { public value: Sample }; fn leak(condition: Bool) -> Sample { let n = 1; let value = Wrapper { value: Sample { ptr: &n, total: 2 } }; let alias = &value; match condition { true => alias.value, false => alias.value } }",
        "type Wrapper is { public value: Sample }; fn leak() -> Sample { let n = 1; let value = Wrapper { value: Sample { ptr: &n, total: 2 } }; let alias = &value; Sample { ptr: alias.value.ptr, total: alias.value.total } }",
        "fn leak() -> (&Int, Int) { let n = 1; let value = (&n, 2); let alias = &value; (alias.0, alias.1) }",
        "fn leak(value: Node) -> &Int { let alias = &value; &alias.n }",
        "fn leak() -> Sample { let n = 1; let value = Sample { ptr: &n, total: 2 }; let first = &value; let second = &first; Sample { ptr: second.ptr, total: second.total } }",
        "fn leak(ptr: &Int) -> Sample { let n = 1; let value = Sample { ptr: &n, total: 2 }; let alias = &value; { let value = Sample { ptr: ptr, total: 3 }; Sample { ptr: alias.ptr, total: alias.total } } }",
    ] {
        let diagnostics = errors(source);
        assert!(diagnostics.iter().any(|message| message.contains("does not live long enough")), "{source}: {diagnostics:?}");
    }
}


#[test]
fn copied_reference_fields_keep_the_external_referent_borrowed() {
    let source = "fn change() -> Int {
        let mut n = 1;
        let value = Sample { ptr: &n, total: 2 };
        let alias = &value;
        let copied = Sample { ptr: alias.ptr, total: alias.total };
        n = 3;
        *copied.ptr
    }";
    let diagnostics = errors(source);
    assert!(diagnostics.iter().any(|message| message.contains("borrow")), "{diagnostics:?}");
}


#[test]
fn copied_payloads_cross_supported_source_reference_chains() {
    std::thread::spawn(|| {
        for depth in [4, 8, 16] {
            let mut source = Text::from("fn forward(value: Sample) -> Sample {");
            let mut previous = Text::from("value");
            for index in 0..depth {
                let current: Text = format!("alias{index}").into();
                source.push_str(&format!("let {current} = &{previous};"));
                previous = current;
            }
            source.push_str(&format!("Sample {{ ptr: {previous}.ptr, total: {previous}.total }} }}"));
            let diagnostics = errors(&source);
            assert!(diagnostics.is_empty(), "depth {depth}: {diagnostics:?}");
        }
    }).join().expect("ordinary worker stack");
}
