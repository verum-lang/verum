use verum_ast::{FileId, Span};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::{
    TypeChecker, refinement::PredicateProvenance, ty::Type, type_registry::TypeRegistry,
};

fn check(source: &str) -> (List<Text>, TypeRegistry) {
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(1))
        .expect("valid Verum");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let mut errors = checker.prepare_module_declarations(&ast);
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    let mut messages: List<_> = errors
        .iter()
        .map(|err| Text::from(format!("{err}")))
        .collect();
    messages.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| Text::from(format!("{d:?}"))),
    );
    (messages, checker.take_type_registry())
}

fn assert_reads(source: &str, reads: &[(&str, bool)]) {
    let (errors, types) = check(source);
    assert!(errors.is_empty(), "{source}\n{errors:?}");
    for &(marker, narrowed) in reads {
        let start = source.find(marker).expect("unique marker") + marker.rfind("= ").unwrap() + 2;
        let name = &source[start..source[start..].find(';').unwrap() + start];
        let span = Span::new(start as u32, (start + name.len()) as u32, FileId::new(1));
        let ty = types
            .lookup_expr(span)
            .unwrap_or_else(|| panic!("missing type for {marker}"));
        if narrowed {
            assert!(
                matches!(ty, Type::Refined { predicate, .. } if predicate.provenance == PredicateProvenance::Narrowed),
                "{marker}: {ty:?}"
            );
        } else {
            assert_eq!(ty, &Type::Int, "{marker}");
        }
    }
}

#[test]
fn writes_use_the_storage_type_and_end_only_the_written_values_facts() {
    let source = "fn test(v: Int) { let mut x = v; let mut other = v; if x >= 0 { let before = x; other = -1; let kept = x; x = -1; let after = x; } }";
    assert_reads(
        source,
        &[
            ("before = x;", true),
            ("kept = x;", true),
            ("after = x;", false),
        ],
    );
}

#[test]
fn comparisons_depend_on_both_bindings_and_respect_lexical_shadowing() {
    let source = "fn test(v: Int) { let mut x = v; let mut limit = v; if x < limit { { let mut x = v; let shadow = x; x = -1; } let outer = x; limit += 1; let after = x; } }";
    assert_reads(
        source,
        &[
            ("shadow = x;", false),
            ("outer = x;", true),
            ("after = x;", false),
        ],
    );
}

#[test]
fn an_exclusive_call_invalidates_the_guard_but_a_shared_call_preserves_it() {
    let source = "fn reset(p: &mut Int) { *p = -1; } fn inspect(p: &Int) {} fn test(v: Int) { let mut x = v; if x >= 0 { inspect(&x); let kept = x; reset(&mut x); let after = x; } }";
    assert_reads(source, &[("kept = x;", true), ("after = x;", false)]);
}

#[test]
fn writing_through_a_stored_exclusive_reference_retires_the_guard() {
    let source = "type Cell is {public value:Int}; type Holder is {public cell:&mut Cell}; implement Holder {fn reset(&mut self) {self.cell.value=0;}} fn test(holder:&mut Holder, x:Int) {if x < holder.cell.value {let before = x;holder.reset();let after = x;}}";
    assert_reads(source, &[("before = x;", true), ("after = x;", false)]);
    let independent = "type Cell is {public value:Int}; type Holder is {public cell:&mut Cell}; implement Holder {fn reset(&mut self) {self.cell.value=0;}} fn test(holder:&mut Holder, input:Int) {let mut x=input; if x >= 0 {holder.reset();let kept = x;}}";
    assert_reads(independent, &[("kept = x;", true)]);
}

#[test]
fn forwarding_a_write_through_a_reference_field_retires_its_guard() {
    let source = "type Cell is {public value:Int}; type Holder is {public cell:&mut Cell};
        type Outcome<T> is Written(T) | Failed;
        implement Cell {fn reset(&mut self)->Outcome<()> {self.value=0; Outcome.Written(())}}
        implement Holder {fn reset(&mut self)->Outcome<()> {self.cell.reset()}}
        fn test(holder:&mut Holder, x:Int) {
            if x < holder.cell.value {let before = x;let _=holder.reset();let after = x;}
        }";
    assert_reads(source, &[("before = x;", true), ("after = x;", false)]);
}

#[test]
fn changing_a_value_cannot_bypass_its_declared_refinement() {
    let (errors, _) = check("fn test() { let mut x: Int{it > 0} = 3; if x == 3 { x = -1; } }");
    assert!(
        errors
            .iter()
            .any(|err| err.contains("refinement") || err.contains("Refinement")),
        "{errors:?}"
    );
}

#[test]
fn a_reborrow_and_an_existing_alias_retire_the_referents_evidence() {
    let source = "fn reset(p: &mut Int) { *p = -1; } fn test(v: Int, x: Int) { let mut limit = v; let alias = &mut limit; if x < *alias { let before = x; reset(alias); let after = x; } }";
    assert_reads(source, &[("before = x;", true), ("after = x;", false)]);
    let source =
        "fn test(p: &mut Int, x: Int) { if x < *p { let before = x; *p = -1; let after = x; } }";
    assert_reads(source, &[("before = x;", true), ("after = x;", false)]);
}

#[test]
fn mutable_copies_and_exclusive_references_use_the_storage_contract() {
    let source = "fn test(v: Int) { let mut x = v; if x >= 0 { let mut copy = x; copy = -1; let hold = x; let p = &mut x; *p = -1; } }";
    assert_reads(source, &[("hold = x;", true)]);
}

#[test]
fn sibling_branches_and_their_join_use_the_reachable_evidence() {
    let source = "type Pack is { carrier: Type, payload: carrier }; fn test(v: Int, change: Bool) { let mut x = v; if x >= 0 { if change { x = -1; } else { let sibling = x; } let joined = x; } }";
    assert_reads(source, &[("sibling = x;", true), ("joined = x;", false)]);
    let source = "fn test(v: Int, change: Bool) { let mut x = v; if x >= 0 { if change { x = -1; return; } let kept = x; } }";
    assert_reads(source, &[("kept = x;", true)]);
}

#[test]
fn persistent_smt_assumptions_end_at_writes_and_visibility_changes() {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.context_mut().enter_scope();
    checker.context_mut().env.insert_mono("x", Type::Int);
    let parse = |source| {
        FastParser::new()
            .parse_expr_str(source, FileId::new(1))
            .unwrap()
    };
    checker.synth_expr(&parse("if x < 0 { loop {} }")).unwrap();
    assert_eq!(checker.get_evidence_assumptions().len(), 1);
    checker.context_mut().enter_scope();
    checker.context_mut().env.insert_mono("x", Type::Int);
    assert!(checker.get_evidence_assumptions().is_empty());
    checker.synth_expr(&parse("x = -1")).unwrap();
    checker.context_mut().exit_scope();
    assert_eq!(checker.get_evidence_assumptions().len(), 1);
    checker.synth_expr(&parse("x = -1")).unwrap();
    assert!(checker.get_evidence_assumptions().is_empty());
}

#[test]
fn repeated_bodies_cannot_assume_an_earlier_iteration_or_invocation_kept_a_value() {
    let source = "fn test(v: Int, again: Bool) { let mut x = v; if x >= 0 { while again { let prior = x; x = -1; } } }";
    assert_reads(source, &[("prior = x;", false)]);
    let source = "fn test(v: Int, again: Bool) { let mut x = v; if x >= 0 { while again { break; } let kept = x; } }";
    assert_reads(source, &[("kept = x;", true)]);
    let source = "fn test(v: Int) { let mut x = v; if x >= 0 { { let run = || { let prior = x; }; } let kept = x; } }";
    assert_reads(source, &[("prior = x;", false), ("kept = x;", true)]);
}

#[test]
fn aggregate_copies_own_their_storage_type() {
    let source = "fn test(v: Int) { let mut x = v; if x >= 0 { let mut pair = (x, x); pair.0 = -1; let before = x; x = -1; let after = x; } }";
    assert_reads(source, &[("before = x;", true), ("after = x;", false)]);
}

#[test]
fn a_match_arm_sees_only_writes_that_can_reach_it() {
    let source = "fn test(v: Int, flag: Bool) { let mut x = v; if x >= 0 { match flag { true => { x = -1; }, false => { let sibling = x; } }; let joined = x; } }";
    assert_reads(source, &[("sibling = x;", true), ("joined = x;", false)]);
}

#[test]
fn destructuring_writes_retire_observations_in_the_enclosing_block() {
    let source = "fn test(v: Int) { let mut x = v; let mut y = v; if x >= 0 { let before = x; { (x, y) = (-1, 0); } let after = x; } }";
    assert_reads(source, &[("before = x;", true), ("after = x;", false)]);
}

#[test]
fn a_write_through_an_opaque_callable_cannot_preserve_a_guard() {
    let source = "fn test(x: Int, p: &mut Int, update: fn()) { if x < *p { let before = x; update(); let after = x; } }";
    assert_reads(source, &[("before = x;", true), ("after = x;", false)]);
}

#[test]
fn exclusive_references_preserve_a_declared_storage_refinement() {
    let (errors, _) =
        check("fn test() { let mut x: Int{it > 0} = 3; if x == 3 { let p = &mut x; *p = -1; } }");
    assert!(
        errors
            .iter()
            .any(|err| err.contains("refinement") || err.contains("Refinement")),
        "{errors:?}"
    );
    let (errors, _) = check("fn test() { let mut x: Int{it > 0} = 3; if x == 3 { x = 2; } }");
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn an_opaque_pointer_read_may_alias_a_write_to_a_known_referent() {
    let source = "fn test(x: Int, p: *mut Int, q: &mut Int) { unsafe { if x < *p { let before = x; *q = -1; let after = x; } } }";
    assert_reads(source, &[("before = x;", true), ("after = x;", false)]);
}
