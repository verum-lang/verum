//! One declaration-owned value instance connects record fields and nominal arguments.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
fn errors(source: &str) -> List<Text> {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&ast)
        .into_iter()
        .map(|e| format!("{e:?}").into())
        .collect();
    for item in &ast.items {
        if let Err(e) = checker.check_item(item) {
            errors.push(format!("{e:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| format!("{e:?}").into()),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| format!("{d:?}").into()),
    );
    errors
}
const TYPES: &str =
    "type Cell<N: meta Int> is { value: () }; type Frame<N: meta Int> is { cell: Cell<N> };";
fn accept(source: &str) {
    let es = errors(source);
    assert!(es.is_empty(), "{es:?}");
}
fn reject(source: &str) {
    let es = errors(source);
    assert!(
        es.iter().any(|e| e.contains("Mismatch")
            || e.contains("exact value")
            || e.contains("domain")
            || e.contains("wrong kind")
            || e.contains("lifetime parameter")),
        "missing causal refusal: {es:?}"
    );
}
#[test]
fn contextual_record_keeps_its_value_argument() {
    accept(&format!(
        "{TYPES} fn make(cell:Cell<7>)->Frame<7>{{ Frame {{cell:cell}} }}"
    ));
}
#[test]
fn inferred_record_keeps_its_value_argument() {
    accept(&format!(
        "{TYPES} fn make(cell:Cell<7>)->Frame<7>{{ let frame=Frame {{cell:cell}}; frame }}"
    ));
}
#[test]
fn projection_keeps_the_receivers_value_argument() {
    accept(&format!(
        "{TYPES} fn read<N:meta Int>(frame:Frame<N>)->Cell<N>{{frame.cell}}"
    ));
}
#[test]
fn contextual_record_cannot_change_a_constant_index() {
    reject(&format!(
        "{TYPES} fn make(cell:Cell<7>)->Frame<8>{{ Frame {{cell:cell}} }}"
    ));
}
#[test]
fn contextual_record_cannot_capture_an_unrelated_value() {
    reject(&format!(
        "{TYPES} fn make<N:meta Int,M:meta Int>(cell:Cell<N>)->Frame<M>{{ Frame {{cell:cell}} }}"
    ));
}
#[test]
fn projection_cannot_change_the_receivers_value_argument() {
    reject(&format!(
        "{TYPES} fn read<N:meta Int,M:meta Int>(frame:Frame<N>)->Cell<M>{{frame.cell}}"
    ));
}
#[test]
fn mixed_record_slots_keep_their_declared_roles() {
    accept(&format!(
        "{TYPES} type Mixed<A,N:meta Int,B> is {{first:A,cell:Cell<N>,last:B}}; fn make(cell:Cell<7>)->Mixed<Int,7,Bool>{{Mixed {{first:1,cell:cell,last:true}}}}"
    ));
}
#[test]
fn inferred_record_still_discharges_declared_value_domain() {
    reject(
        "type Cell<N:meta Int> is {value:()}; type Positive<N:meta Int{>=0}> is {cell:Cell<N>}; fn make(cell:Cell<-1>){let frame=Positive {cell:cell};}",
    );
}
#[test]
fn erased_lifetime_preserves_following_type_slot() {
    accept("fn id<'a,T>(value:T)->T{value} fn probe()->Int{id<'static,Int>(7)}");
}
#[test]
fn erased_lifetime_slot_cannot_accept_a_type() {
    reject("fn id<'a,T>(value:T)->T{value} fn probe()->Int{id<Int,Int>(7)}");
}

#[test]
fn transparent_alias_is_normalized_as_a_value_carrier() {
    accept(
        "type Cell<N:meta Int> is {value:()}; type Count = Int; fn size<N:meta Count>(cell:Cell<N>)->Int{N} fn probe(cell:Cell<7>)->Int{size<7>(cell)}",
    );
}
#[test]
fn different_transparent_carrier_does_not_become_int() {
    reject(
        "type Cell<N:meta Int> is {value:()}; type Flag = Bool; fn size<N:meta Flag>(cell:Cell<N>)->Int{0}",
    );
}
#[test]
fn normalizing_a_refined_carrier_keeps_its_domain() {
    reject(
        "type Cell<N:meta Int> is {value:()}; type Count = Int{>=0}; fn size<N:meta Count>(cell:Cell<N>)->Int{N} fn probe(cell:Cell<-1>)->Int{size<-1>(cell)}",
    );
}
#[test]
fn refined_alias_accepts_positive_and_rejects_zero() {
    let head = "type Cell<N:meta Int> is {value:()}; type Count = Int{>0}; fn size<N:meta Count>(cell:Cell<N>)->Int{N}";
    accept(&format!(
        "{head} fn probe(cell:Cell<7>)->Int{{size<7>(cell)}}"
    ));
    reject(&format!(
        "{head} fn probe(cell:Cell<0>)->Int{{size<0>(cell)}}"
    ));
}
#[test]
fn nested_refined_alias_preserves_both_domains() {
    let head = "type Cell<N:meta Int> is {value:()}; type Positive = Int{>0}; type Small = Positive{<10}; fn size<N:meta Small>(cell:Cell<N>)->Int{N}";
    accept(&format!(
        "{head} fn probe(cell:Cell<7>)->Int{{size<7>(cell)}}"
    ));
    for n in [0, 10] {
        reject(&format!(
            "{head} fn probe(cell:Cell<{n}>)->Int{{size<{n}>(cell)}}"
        ));
    }
}
