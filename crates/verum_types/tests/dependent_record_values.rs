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
