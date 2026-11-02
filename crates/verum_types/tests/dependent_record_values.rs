//! Record introduction, runtime-place stability and preservation under updates.
use verum_ast::FileId;
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

const PACK: &str = "
    public type Witness<A> is { public value: A };
    public type Pack is { public carrier: Type, public witness: Witness<carrier> };
    public type Pair is { public left: Pack, public right: Pack };
";

fn errors(body: &str) -> Vec<String> {
    let source = format!("{PACK}\n{body}");
    let module = FastParser::new()
        .parse_module_str(&source, FileId::new(1))
        .unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("sample");
    let mut problems: Vec<_> = checker
        .prepare_module_declarations(&module)
        .into_iter()
        .map(|e| e.to_string())
        .collect();
    for item in &module.items {
        if let Err(e) = checker.check_item(item) {
            problems.push(e.to_string());
        }
    }
    problems.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| e.to_string()),
    );
    problems.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| format!("{d:?}")),
    );
    problems
}

#[test]
fn construction_substitutes_the_actual_carrier_in_declaration_order() {
    for body in [
        "fn pack(w: Witness<Int>) -> Pack { Pack { witness: w, carrier: Int } }",
        "fn repack(x: &Pack) -> Pack { Pack { witness: x.witness, carrier: x.carrier } }",
        "fn nested(p: &Pair) -> Pack { Pack { carrier: p.left.carrier, witness: p.left.witness } }",
        "fn copy(x: Pack) -> Pack { Pack { ..x } }",
        "fn replace(mut x: Pack, w: Witness<Int>) { x = Pack { carrier: Int, witness: w }; }",
    ] {
        assert_eq!(errors(body), Vec::<String>::new(), "{body}");
    }
}

#[test]
fn witness_cannot_be_exchanged_between_carriers() {
    for body in [
        "fn wrong(w: Witness<Bool>) -> Pack { Pack { carrier: Int, witness: w } }",
        "fn mixed(x: &Pack, y: &Pack) -> Pack { Pack { carrier: x.carrier, witness: y.witness } }",
        "fn siblings(p: &Pair) -> Pack { Pack { carrier: p.left.carrier, witness: p.right.witness } }",
        "fn changed(x: Pack, y: &Pack) -> Pack { Pack { carrier: y.carrier, ..x } }",
        "fn shadow(x: &Pack, y: &Pack) -> Pack { let w = x.witness; { let x = y; Pack { carrier: x.carrier, witness: w } } }",
    ] {
        let problems = errors(body);
        assert!(
            problems
                .iter()
                .any(|e| e.to_lowercase().contains("mismatch")),
            "{body}: {problems:?}"
        );
    }
}

#[test]
fn partial_carrier_changes_and_mutable_borrows_require_reconstruction() {
    for body in [
        "fn change(x: &mut Pack, y: &Pack) { x.carrier = y.carrier; }",
        "fn expose(x: &mut Pack) -> &mut Type { &mut x.carrier }",
    ] {
        let problems = errors(body);
        assert!(
            problems
                .iter()
                .any(|e| e.contains("invalidate a dependent field")),
            "{body}: {problems:?}"
        );
    }
}

#[test]
fn mutable_bindings_cannot_give_a_witness_a_stable_index() {
    for body in [
        "type Exclusive is &mut Pack; fn stale(x: Exclusive) -> Pack { let w = x.witness; Pack { carrier: x.carrier, witness: w } }",
        "type Exclusive is &mut Pack; type Nested is { public child: Exclusive }; fn stale(x: Nested) -> Pack { let w = x.child.witness; Pack { carrier: x.child.carrier, witness: w } }",
        "fn stale(mut x: Pack, y: Pack) -> Pack { let old = x.witness; x = y; Pack { carrier: x.carrier, witness: old } }",
        "fn stale(x: &mut Pack, y: Pack) -> Pack { let old = x.witness; *x = y; Pack { carrier: x.carrier, witness: old } }",
    ] {
        let problems = errors(body);
        assert!(
            problems
                .iter()
                .any(|e| e.contains("immutable local receiver")),
            "{body}: {problems:?}"
        );
    }
}

#[test]
fn overwriting_a_place_cannot_reinterpret_an_existing_witness() {
    for body in [
        "fn replace(x: Pack, y: Pack) -> Pack { let w = x.witness; x = y; Pack { carrier: x.carrier, witness: w } }",
        "fn replace(p: Pair, y: Pack) -> Pack { let w = p.left.witness; p.left = y; Pack { carrier: p.left.carrier, witness: w } }",
        "fn borrow(x: Pack) { let w = x.witness; let r = &mut x; }",
    ] {
        let problems = errors(body);
        assert!(
            problems
                .iter()
                .any(|e| e.contains("cannot mutate a receiver")),
            "{body}: {problems:?}"
        );
    }
}

#[test]
fn method_receivers_preserve_the_carrier_of_an_existing_witness() {
    let methods = "implement Pack {
        public fn swap_in(&mut self, y: Pack) { *self = y; }
        public fn inspect(&self) -> Int { 1 }
    }";
    let shared = format!("{methods}
        fn stable(x: Pack) -> Pack {{ let w = x.witness; x.inspect(); Pack {{ carrier: x.carrier, witness: w }} }}");
    assert_eq!(errors(&shared), Vec::<String>::new());
    let replace = format!("{methods}
        fn stale(x: Pack, y: Pack) -> Pack {{ let w = x.witness; x.swap_in(y); Pack {{ carrier: x.carrier, witness: w }} }}");
    let problems = errors(&replace);
    assert!(
        problems
            .iter()
            .any(|e| e.contains("cannot mutate a receiver")),
        "{problems:?}"
    );
}

#[test]
fn method_autoref_respects_the_complete_access_path() {
    let methods = "implement Pack {
        fn overwrite(&mut self, y: Pack) { *self = y; }
        fn view(&self) -> &Pack { self }
        fn separate(&self, y: Pack) -> Pack { y }
    }";
    for body in [
        "fn reject(x: &Pack, y: Pack) { x.overwrite(y); }",
        "fn reject(p: &Pair, y: Pack) { p.left.overwrite(y); }",
        "fn reject(x: Pack, y: Pack) { x.view().overwrite(y); }",
        "type Alias is Pack; fn reject(x: &Alias, y: Pack) { x.overwrite(y); }",
    ] {
        let problems = errors(&format!("{methods}\n{body}"));
        assert!(
            problems.iter().any(|e| e.contains("shared reference")),
            "{body}: {problems:?}"
        );
    }
    for body in [
        "fn allow(x: &mut Pack, y: Pack) { x.overwrite(y); }",
        "fn allow(x: &Pack, y: Pack, z: Pack) { let w = x.witness; x.separate(y).overwrite(z); }",
    ] {
        assert_eq!(
            errors(&format!("{methods}\n{body}")),
            Vec::<String>::new(),
            "{body}"
        );
    }
}

#[test]
fn protocol_implementation_cannot_escalate_receiver_permission() {
    let declared = "type Inspect is protocol { fn inspect(&self, y: Pack); };";
    let valid = "implement Inspect for Pack { fn inspect(&self, y: Pack) {} }
                 fn accept(x: Pack, y: Pack) { let w = x.witness; x.inspect(y); }";
    assert_eq!(
        errors(&format!("{declared}\n{valid}")),
        Vec::<String>::new()
    );
    let generics = "fn observe<T: Inspect>(x: &T, y: Pack) { x.inspect(y); }";
    assert_eq!(
        errors(&format!("{declared}\n{generics}")),
        Vec::<String>::new()
    );
    let invalid = "implement Inspect for Pack { fn inspect(&mut self, y: Pack) { *self = y; } }";
    let problems = errors(&format!("{declared}\n{invalid}"));
    assert!(
        problems
            .iter()
            .any(|e| e.contains("E427") && e.contains("receiver")),
        "{problems:?}"
    );
}

#[test]
fn tier_conversion_cannot_strengthen_a_nested_reference() {
    for input in ["&(&Pack)", "&(&mut Pack)"] {
        let problems = errors(&format!(
            "fn reject(p: {input}) -> &checked mut Pack {{ p.to_checked_mut() }}"
        ));
        assert!(
            problems.iter().any(|e| e.contains("shared reference")),
            "{input}: {problems:?}"
        );
    }
    assert_eq!(
        errors("fn allow(p: &mut Pack) -> &checked mut Pack { p.to_checked_mut() }"),
        Vec::<String>::new()
    );
}

#[test]
fn reference_results_preserve_the_origin_of_a_dependent_projection() {
    let declarations = "fn borrow<T>(x: &T) -> &T { x }
        fn relay(x: &Pack) -> &Pack { let a = x; let b = a; return b; }
        fn second(a: &Pack, b: &Pack) -> &Pack { b }
        fn choose(c: Bool, a: &Pack, b: &Pack) -> &Pack { if c { a } else { b } }
        implement Pack { fn swap_in(&mut self, y: Pack) { *self = y; } }";
    for body in [
        "fn same(x: Pack) -> Pack { let r = borrow(&x); Pack { carrier: x.carrier, witness: r.witness } }",
        "fn chain(x: &Pack) -> Pack { let a = borrow(x); let b = borrow(a); Pack { carrier: b.carrier, witness: x.witness } }",
        "fn nested(p: Pair) -> Pack { let a = borrow(&p.left); Pack { carrier: p.left.carrier, witness: a.witness } }",
        "fn forwarded(x: &Pack) -> Pack { let a = relay(x); Pack { carrier: x.carrier, witness: a.witness } }",
        "fn positional(x: &Pack, y: &Pack) -> Pack { let a = second(x, y); Pack { carrier: y.carrier, witness: a.witness } }",
        "fn end(mut x: Pack, y: Pack) { { let a = borrow(&x); let _ = a; } x = y; }",
    ] {
        let problems = errors(&format!("{declarations}\n{body}"));
        assert!(problems.is_empty(), "{body}: {problems:?}");
    }
    for body in [
        "fn stale(mut x: Pack, y: Pack) -> Pack { let r = borrow(&x); let w = r.witness; x = y; Pack { carrier: r.carrier, witness: w } }",
        "fn stale(mut x: Pack, y: Pack) -> Pack { let r = borrow(&x); let a = r; let w = a.witness; x.swap_in(y); Pack { carrier: r.carrier, witness: w } }",
        "fn second_call(mut x: Pack, y: Pack) { fn with_alias(a: &Pack, b: &mut Pack, y: Pack) { let w = a.witness; *b = y; let _ = Pack { carrier: a.carrier, witness: w }; } let r = borrow(&x); with_alias(r, &mut x, y); }",
        "fn wrong(x: &Pack, y: &Pack) -> Pack { let a = second(x, y); Pack { carrier: x.carrier, witness: a.witness } }",
        "fn indirect(c: Bool, mut x: Pack, y: Pack) { fn consume(a: &Pack, b: &mut Pack) {} let r = choose(c, &x, &y); let s = r; consume(s, &mut x); }",
        "fn opaque(c: Bool, x: &Pack, y: &Pack) -> Pack { let a = choose(c, x, y); Pack { carrier: x.carrier, witness: a.witness } }",
        "fn shadow(x: &Pack, y: &Pack) -> Pack { let a = borrow(x); let x = y; Pack { carrier: x.carrier, witness: a.witness } }",
        "fn nested_slot(mut x: &Pack, y: &Pack) { let a = borrow(&x); x = y; let _ = a; }",
        "fn changed_before_projection(mut x: Pack, y: Pack) -> Pack { let a = borrow(&x); x = y; Pack { carrier: a.carrier, witness: a.witness } }",
    ] {
        let problems = errors(&format!("{declarations}\n{body}"));
        assert!(problems.iter().any(|msg| msg.contains("cannot mutate a receiver") || msg.contains("original carrier") || msg.contains("immutable local receiver") || msg.contains("mismatch") || msg.contains("different record receiver") || msg.contains("already borrowed")), "{body}: {problems:?}");
    }
}
