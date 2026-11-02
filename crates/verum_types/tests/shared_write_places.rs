//! A caller can retain dependent evidence across a helper taking a shared
//! reference. The helper's own body must enforce the same write permission.
use verum_ast::FileId;
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn errors(source: &str) -> Vec<String> {
    let source = format!("public type Boxed is {{ public value: Int }};\n{source}");
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
fn helpers_cannot_write_or_reborrow_shared_referents() {
    for body in [
        "fn change(p: &Boxed, y: Boxed) { *p = y; }",
        "fn change<T>(p: &T, y: T) { *p = y; }",
        "fn change(p: &Boxed) { p.value = 1; }",
        "fn change(p: &Boxed) { p.value += 1; }",
        "fn change(p: &Boxed) -> &mut Boxed { &mut *p }",
        "fn change(p: &Boxed) -> &mut Int { &mut p.value }",
        "type Alias is &Boxed; fn change(p: Alias) { p.value = 1; }",
        "fn change(p: &mut &Boxed) { p.value = 1; }",
        "type Outer is { public inner: &Boxed }; fn change(p: &mut Outer) { p.inner.value += 2; }",
        "fn borrow(p: &Boxed) -> &Boxed { p } fn change(p: &Boxed) { borrow(p).value = 1; }",
    ] {
        let messages = errors(body);
        assert!(
            messages
                .iter()
                .any(|m| m.contains("cannot write through a shared reference")),
            "{body}: {messages:?}"
        );
    }
}

#[test]
fn exclusive_references_allow_writes_and_shared_references_can_be_rebound() {
    for body in [
        "fn change(p: &mut Boxed, y: Boxed) { *p = y; }",
        "fn change(p: &mut Boxed) { p.value += 1; }",
        "fn change(p: &mut &Boxed, y: &Boxed) { *p = y; }",
        "fn change(mut p: &Boxed, y: &Boxed) { p = y; }",
        "fn read(p: &Boxed) -> Int { p.value }",
        "type Alias is &mut Boxed; fn change(p: Alias) { p.value = 1; }",
        "type Alias is &Boxed; fn read(p: Alias) -> Int { p.value }",
        "type Outer is { public inner: Boxed }; fn change(p: &mut Outer) { p.inner.value += 2; }",
        "type affine Handle is { id: Int }; fn take(h: Handle) -> Boxed { Boxed { value: h.id } } fn change() { let h = Handle { id: 2 }; take(h).value = 1; }",
        "fn borrow(p: &mut Boxed) -> &mut Boxed { p } fn change(p: &mut Boxed) { borrow(p).value = 1; }",
    ] {
        assert!(errors(body).is_empty(), "{body}: {:?}", errors(body));
    }
}
