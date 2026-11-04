//! Lifetime annotations are erased; retained arguments preserve type/value order.
use verum_common::List;
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
fn errors(source: &str) -> List<String> {
    let ast = Parser::new(source).parse_module().expect("valid grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let mut errors: List<_> = checker
        .prepare_module_declarations(&ast)
        .into_iter()
        .map(|error| format!("{error:?}"))
        .collect();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}"));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| format!("{error:?}")),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .map(|diagnostic| format!("{diagnostic:?}")),
    );
    errors
}
fn accepts(source: &str) {
    let errors = errors(source);
    assert!(errors.is_empty(), "{errors:?}");
}
fn rejects(source: &str) {
    let errors = errors(source);
    assert!(!errors.is_empty(), "unsoundly accepted: {source}");
    assert!(
        errors
            .iter()
            .all(|e| !e.contains("TypeNotFound") && !e.contains("unbound variable")),
        "unrelated unresolved symbol: {errors:?}"
    );
}
const LEADING: &str = "type Holder<'a,T> is {item:&'a mut T}; implement<'a,T> Holder<'a,T> {fn get(&self)->&T{self.item}} fn wrap<'a,T>(item:&'a mut T)->Holder<'a,T>{Holder{item:item}}";
#[test]
fn leading_lifetime_preserves_returned_reference() {
    accepts(LEADING);
}
#[test]
fn original_entry_returns_borrowed_value() {
    accepts(&format!(
        "{LEADING} fn main()->Int{{let mut value=7;let holder=wrap(&mut value);*holder.get()}}"
    ));
}
#[test]
fn interleaved_lifetime_preserves_each_type_slot() {
    accepts(
        "type Holder<T,'a,U> is {item:&'a mut T,other:U}; implement<T,'a,U> Holder<T,'a,U>{fn get(&self)->&T{self.item}} fn wrap<T,'a,U>(item:&'a mut T,other:U)->Holder<T,'a,U>{Holder{item:item,other:other}}",
    );
}
#[test]
fn erased_annotation_matches_unannotated_nominal_type() {
    accepts(
        "type Holder<'a,T> is {item:&'a mut T}; fn erase<'a,T>(holder:Holder<'a,T>)->Holder<T>{holder}",
    );
}
#[test]
fn record_cannot_change_retained_type_argument() {
    rejects(
        "type Holder<'a,T> is {item:&'a mut T}; fn bad<'a>(item:&'a mut Bool)->Holder<'a,Int>{Holder{item:item}}",
    );
}
#[test]
fn lifetime_erasure_preserves_existing_reference_capability() {
    let with_annotation = errors(
        "type Holder<'a,T> is {item:&'a mut T}; fn get<'a,T>(holder:&Holder<'a,T>)->&mut T{holder.item}",
    );
    let without_annotation = errors(
        "type Holder<T> is {item:&mut T}; fn get<T>(holder:&Holder<T>)->&mut T{holder.item}",
    );
    assert!(without_annotation.is_empty(), "{without_annotation:?}");
    assert_eq!(with_annotation, without_annotation);
}
#[test]
fn shared_receiver_cannot_create_a_mutable_field_reference() {
    rejects(
        "type Holder<'a,T> is {item:T}; fn bad<'a,T>(holder:&Holder<'a,T>)->&mut T{&mut holder.item}",
    );
}
#[test]
fn alias_substitution_uses_only_retained_parameters() {
    accepts(
        "type Holder<'a,T> is {item:&'a mut T}; type View<'a,T> = Holder<'a,T>; fn alias<'a,T>(holder:Holder<'a,T>)->View<'a,T>{holder}",
    );
}
#[test]
fn retained_value_domain_is_checked_after_lifetime() {
    accepts(
        "type Cell<'a,N:meta Int{it>=0},T> is {value:T}; fn good()->Cell<'static,7,Int>{Cell{value:7}}",
    );
    rejects(
        "type Cell<'a,N:meta Int{it>=0},T> is {value:T}; fn bad()->Cell<'static,-1,Int>{Cell{value:7}}",
    );
}
#[test]
fn type_default_follows_retained_arguments() {
    accepts(
        "type Pair<'a,T,U=Int> is {left:T,right:U}; fn good()->Pair<'static,Bool>{Pair{left:true,right:7}}",
    );
}
const BOUNDS: &str = "type Measure is protocol{fn amount(&self)->Int;}; type Good is {value:Int}; type Bad is {value:Int}; implement Measure for Good{fn amount(&self)->Int{self.value}} type Holder<'a,T> is {item:&'a mut T}; fn wrap<'a,T>(item:&'a mut T)->Holder<'a,T>{Holder{item:item}} fn inspect<'a,T:Measure>(holder:&Holder<'a,T>)->Int{holder.item.amount()}";
#[test]
fn retained_type_keeps_protocol_bound() {
    accepts(&format!(
        "{BOUNDS} fn good(value:&mut Good)->Int{{let holder=wrap(value);inspect(&holder)}}"
    ));
}
#[test]
fn lifetime_cannot_displace_protocol_bound() {
    rejects(&format!(
        "{BOUNDS} fn bad(value:&mut Bad)->Int{{let holder=wrap(value);inspect(&holder)}}"
    ));
}
#[test]
fn lifetime_annotation_does_not_shadow_an_actual_type() {
    accepts(
        "type Count = Int; type Holder<'Count,T> is {value:T,count:Count}; fn good()->Holder<'static,Bool>{Holder{value:true,count:7}}",
    );
}
#[test]
fn lifetime_annotation_does_not_declare_a_type() {
    let errors = errors("type Holder<'a,T> is {value:T,ghost:a};");
    assert!(
        errors.iter().any(|error| error.contains("a")
            && (error.contains("TypeNotFound")
                || error.contains("Undefined")
                || error.contains("E407"))),
        "erased annotation fabricated a type binder: {errors:?}"
    );
}
