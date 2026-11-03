use verum_ast::FileId;
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn errors(source: &str) -> Vec<String> {
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
    let mut messages: Vec<_> = errors.iter().map(ToString::to_string).collect();
    messages.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| d.message().to_string()),
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
        assert_eq!(errors(source), Vec::<String>::new(), "{source}");
    }
}
