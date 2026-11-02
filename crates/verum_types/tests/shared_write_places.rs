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

#[test]
fn a_live_shared_alias_blocks_method_activation_and_compound_writes() {
    let declarations = "
        type Witness<A> is { value: A };
        type Pack is { carrier: Type, witness: Witness<carrier> };
        implement Pack {
            fn swap_in(&mut self, y: Pack) { *self = y; }
            fn inspect(&self) -> Int { 1 }
        }";
    for mutation in ["x.swap_in(y);", "x.inspect(); x.swap_in(y);"] {
        let body = format!(
            "{declarations}
            fn stale(x: Pack, y: Pack) -> Pack {{ let r = &x; let w = r.witness;
                {mutation} Pack {{ carrier: r.carrier, witness: w }}
            }}"
        );
        let messages = errors(&body);
        assert!(
            messages.iter().any(|m| m.contains("dependent type index")),
            "{body}: {messages:?}"
        );
    }
    for body in [
        "implement Boxed { fn inspect(&self) {} fn reset(&mut self) { self.value = 0; } } fn f(x: Boxed) -> Int { let r = &x; x.inspect(); x.reset(); r.value }",
        "fn f(mut x: Int) -> Int { let r = &x; x += 1; *r }",
        "fn f(x: Boxed) -> Int { let r = &x; x.value += 1; r.value }",
        "fn f(mut x: Int) -> Int { let r = &x; let w = &mut (x); *r }",
    ] {
        let messages = errors(body);
        assert!(
            messages.iter().any(|m| m.contains("borrow")),
            "{body}: {messages:?}"
        );
    }
    for body in [
        "fn f(x: Pack) -> Pack { let r = &x; let w = r.witness; x.inspect(); Pack { carrier: r.carrier, witness: w } }",
        "fn f(x: Pack, y: Pack) { { let r = &x; r.inspect(); } x.swap_in(y); }",
        "fn f(mut n: Int) -> Int { let w = &mut n; *w += 1; *w }",
    ] {
        let messages = errors(&format!("{declarations}\n{body}"));
        assert!(messages.is_empty(), "{body}: {messages:?}");
    }
}

#[test]
fn writes_respect_overlapping_field_and_element_loans() {
    let declarations = "type Table is { row: [Int; 2], spare: Int };";
    for (write, reject) in [
        ("t.row[0] += 1;", true),
        ("t.row[i] += 1;", true),
        ("let w = &mut t.row;", true),
        ("{ let also = &t.row[0]; } t.row[0] += 1;", true),
        ("t.row[1] += 1;", false),
        ("t.spare += 1;", false),
    ] {
        let source = format!(
            "{declarations} fn probe(t: Table, i: Int) -> Int {{
            let r = &t.row[0]; {write} *r
        }}"
        );
        let messages = errors(&source);
        if reject {
            assert!(
                messages.iter().any(|m| m.contains("borrow")),
                "{write}: {messages:?}"
            );
        } else {
            assert!(messages.is_empty(), "{write}: {messages:?}");
        }
    }
}

#[test]
fn a_scalar_dereference_can_end_only_its_last_non_escaping_use() {
    for source in [
        "fn change(mut x: Int) -> Int { let r = &x; let a = *r; x = 2; *r }",
        "fn change(mut x: Int) -> Int { let r = &x; let a = *r; x = 2; let b = *r; b }",
        "fn change(mut x: Int) -> Int { let r = &x; let keep = r; let a = *r; x = 2; *keep }",
        "fn change(mut x: Int) -> Int { let r = &x; let keep = &*r; let a = *r; x = 2; *keep }",
        "fn change(mut x: Int) -> Int { let r = &x; { let a = *r; x = 2; } *r }",
        "fn change(mut x: Int) -> Int { let r = &x; let a = *r; let b = { x = 2; *r }; b }",
    ] {
        let messages = errors(source);
        assert!(
            messages.iter().any(|m| m.contains("borrow")),
            "{source}: {messages:?}"
        );
    }
    for source in [
        "fn change(mut x: Int) -> Int { let r = &x; let a = *r; x = 2; a }",
        "fn change(mut x: Int) -> Int { let r = &x; let a = *r; let b = *r; x = 2; a + b }",
        "fn test_mut(mut x: Int) -> Int { let r = &mut x; *r = 2; let s = &x; *s }",
        "fn test_field(x: Boxed) -> Int { let r = &x.value; let v = *r; x.value = 2; v }",
        "fn check(b: Bool) {} fn change(mut x: Int) { let r = &x; check(*r == 1); x = 2; }",
    ] {
        let messages = errors(source);
        assert!(messages.is_empty(), "{source}: {messages:?}");
    }
}

#[test]
fn entering_a_for_loop_does_not_cancel_a_live_loan() {
    let source = "fn change(mut x: [Int; 2]) -> Int {
        let r = &x; for item in &mut x {} r[0]
    }";
    let messages = errors(source);
    assert!(
        messages.iter().any(|m| m.contains("borrow")),
        "{messages:?}"
    );
}

#[test]
fn argument_loans_survive_nested_calls_until_the_outer_invocation() {
    let declarations = "
        fn borrow<T>(p: &T) -> &T { p }
        fn choose(c: Bool, a: &Boxed, b: &Boxed) -> &Boxed { if c { a } else { b } }
        fn take(a: &Boxed, b: &mut Boxed) {}
        fn reverse(a: &mut Boxed, b: &Boxed) {}
        fn read(p: &Boxed) -> Int { p.value }
        fn number(a: Int, b: &mut Boxed) {}
        type Pair is { left: Boxed, right: Boxed };
        implement Boxed {
            fn update(&mut self, other: &Boxed) { self.value = other.value; }
            fn view(&self) -> &Boxed { self }
            fn copy_box(&self) -> Boxed { Boxed { value: self.value } }
        }";
    for (body, reject) in [
        ("take(borrow(borrow(&x)), &mut x);", true),
        ("reverse(&mut x, borrow(&x));", true),
        ("take(choose(c, &x, &y), &mut y);", true),
        ("take(if c { &x } else { &y }, &mut x);", true),
        ("take({ let r = borrow(&x); r }, &mut x);", true),
        ("x.update(borrow(&x));", true),
        ("x.update(y.view());", false),
        ("x.update(x.view());", true),
        ("x.copy_box().update(&x);", false),
        ("take(borrow(&p.left), &mut p.right);", false),
        ("number(read(&x), &mut x);", false),
        ("take(borrow(&x), &mut y);", false),
    ] {
        let source = format!(
            "{declarations} fn probe(c: Bool, mut x: Boxed, mut y: Boxed, mut p: Pair) {{ {body} }}"
        );
        let diagnostics = errors(&source);
        if reject {
            assert!(
                diagnostics.iter().any(|m| m.contains("borrow")),
                "{body}: {diagnostics:?}"
            );
        } else {
            assert!(diagnostics.is_empty(), "{body}: {diagnostics:?}");
        }
    }
}

#[test]
fn implicit_clone_borrows_only_duplicable_layouts() {
    for source in [
        "type Role is | Up | Down; type Entry is { role: Role }; fn read(e: &Entry) -> Role { e.role.clone() }",
        "type Digest is (Text); fn read(p: &Digest) -> Digest { p.clone() }",
        "fn show(b: &Boxed) -> Text { b.value.to_text() }",
        "type Pair is { x: Int, y: Text }; fn read(p: &Pair) -> Pair { p.clone() }",
        "type Wrap<T> is { item: T }; fn read(p: &Wrap<Int>) -> Wrap<Int> { p.clone() }",
    ] {
        let messages = errors(source);
        assert!(messages.is_empty(), "{source}: {messages:?}");
    }
    for source in [
        "type Outer is { inner: Boxed }; fn mutate(p: &Outer) -> Int { let mut replica = p.clone(); replica.inner.value = 13; p.inner.value }",
        "type Code is (Text); type Box is { code: Code }; fn duplicate(t: &Box) -> Box { t.clone() }",
        "type affine Status is | Idle | Busy; fn duplicate(t: &Status) -> Status { t.clone() }",
        "type affine Status is | Idle | Busy; type Box is { status: Status }; fn duplicate(t: &Box) -> Box { t.clone() }",
        "type affine Ticket is { id: Int }; fn duplicate(t: &Ticket) -> Ticket { t.clone() }",
        "type affine Ticket is { id: Int }; type Box is { ticket: Ticket }; fn duplicate(t: &Box) -> Box { t.clone() }",
        "type Access is { ptr: &mut Int }; fn duplicate(a: &Access) -> Access { a.clone() }",
        "type Wrap<T> is { item: T }; type Pair is { a: Wrap<Int>, b: Wrap<&mut Int> }; fn duplicate(p: &Pair) -> Pair { p.clone() }",
        "type Wrap<T> is { item: T }; fn duplicate<T>(p: &Wrap<T>) -> Wrap<T> { p.clone() }",
    ] {
        let messages = errors(source);
        assert!(messages.iter().any(|m| m.contains("implicit clone")), "{source}: {messages:?}");
    }
    let messages = errors("implement Boxed { fn clone(&mut self) -> Boxed { self.value += 1; Boxed { value: self.value } } } fn duplicate(p: &Boxed) -> Boxed { p.clone() }");
    assert!(messages.iter().any(|m| m.contains("shared reference")), "{messages:?}");
}
