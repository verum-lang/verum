//! Actual-source controls for declaration-owned family application.
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> Vec<String> {
    errors_with_bodies(source, true)
}
fn errors_with_bodies(source: &str, check_bodies: bool) -> Vec<String> {
    let module = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("families");
    let mut errors = checker
        .prepare_module_declarations(&module)
        .iter()
        .map(|error| format!("{error:?}"))
        .collect::<Vec<_>>();
    for item in module.items.iter().filter(|_| check_bodies) {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}"));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
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

const SIMPLE: &str = "module families; type Rel is protocol {type Out<A>;}; type Pair<A,B> is {first:A,second:B}; type Owner is {value:Bool}; implement Rel for Owner {type Out<X> = Pair<Bool,X>;}";

#[test]
fn fixed_rhs_arguments_are_not_replaced_by_family_arguments() {
    let errors = errors(&format!(
        "{SIMPLE} fn probe(value:Owner.Out<Int>)->Pair<Bool,Int>{{value}}"
    ));
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn different_rhs_result_is_rejected() {
    let errors = errors(&format!(
        "{SIMPLE} fn probe(value:Owner.Out<Int>)->Pair<Int,Bool>{{value}}"
    ));
    assert!(
        !errors.is_empty(),
        "family result must remain Pair<Bool,Int>"
    );
}

#[test]
fn actual_receiver_order_and_family_locals_have_distinct_scopes() {
    let source = "module families; type Rel is protocol {type Out<X>;}; type Owner<A,B> is {left:A,right:B}; implement<A,B> Rel for Owner<B,A> {type Out<X> = (A,B,X);} type Concrete is Owner<Bool,Int>; fn probe(value:Concrete.Out<Text>)->(Int,Bool,Text){value}";
    let errors = errors(source);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn family_local_shadows_the_owner_name_without_capturing_its_identity() {
    let source = "module families; type Rel is protocol {type Out<X>;}; type Pair<A,B> is {left:A,right:B}; type Owner<A> is {value:A}; implement<A> Rel for Owner<A> {type Out<A> = Pair<Bool,A>;} type Concrete is Owner<Text>; fn probe(value:Concrete.Out<Int>)->Pair<Bool,Int>{value}";
    let errors = errors(source);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn required_local_argument_cannot_disappear() {
    let errors = errors(&format!(
        "{SIMPLE} fn probe(value:Owner.Out<Int,Text>)->Int{{42}}"
    ));
    assert!(
        errors
            .iter()
            .any(|error| error.contains("E405") || error.contains("E407")),
        "{errors:?}"
    );
}

#[test]
fn a_family_implementation_cannot_strengthen_an_unbounded_local_domain() {
    let source = "module families; type Mark is protocol {}; type Rel is protocol {type Out<X>;}; type Owner is {}; implement Rel for Owner {type Out<X:Mark> = X;}";
    let errors = errors(source);
    assert!(
        errors.iter().any(|error| error.contains("E405")),
        "{errors:?}"
    );
}

const DATABASE: &str = "module families; type Row is protocol {}; type DbError is {code:Int}; type Result<T,E> is Ok(T) | Err(E); type Stream is protocol {type Item;};";
#[test]
fn symbolic_database_family_keeps_its_row_domain() {
    let source = format!(
        "{DATABASE} type Connection is protocol {{type Rows<R:Row>: Stream<Item=Result<R,DbError>>; fn query_stream<R:Row>(&self)->Result<Self.Rows<R>,DbError>;}};"
    );
    let errors = errors(&source);
    assert!(errors.is_empty(), "{errors:?}");
}
#[test]
fn symbolic_database_method_must_prove_the_row_domain() {
    let source = format!(
        "{DATABASE} type Connection is protocol {{type Rows<R:Row>: Stream<Item=Result<R,DbError>>; fn query_stream<R>(&self)->Result<Self.Rows<R>,DbError>;}};"
    );
    let errors = errors(&source);
    assert!(
        errors.iter().any(|error| error.contains("E405")),
        "{errors:?}"
    );
}
#[test]
fn abstract_family_application_uses_caller_assumptions() {
    let source = "module families; type Row is protocol {}; type Source is protocol {type Rows<R:Row>;}; fn preserve<S:Source,R:Row>(rows:S.Rows<R>)->S.Rows<R>{rows}";
    let errors = errors(source);
    assert!(errors.is_empty(), "{errors:?}");
}
#[test]
fn abstract_family_application_rejects_an_unproved_domain() {
    let source = "module families; type Row is protocol {}; type Source is protocol {type Rows<R:Row>;}; fn preserve<S:Source,R>(rows:S.Rows<R>)->S.Rows<R>{rows}";
    let errors = errors(source);
    assert!(
        errors.iter().any(|error| error.contains("E405")),
        "{errors:?}"
    );
}

#[test]
fn registration_cannot_strengthen_the_declared_family_domain() {
    let source = "module families; type Mark is protocol {}; type Rel is protocol {type Out<X>;}; type Owner is {}; implement Rel for Owner {type Out<X:Mark> = X;}";
    let errors = errors_with_bodies(source, false);
    assert!(
        errors.iter().any(|error| error.contains("E405")),
        "{errors:?}"
    );
}
#[test]
fn registration_accepts_the_same_domain_under_a_renamed_parameter() {
    let source = "module families; type Mark is protocol {}; type Rel is protocol {type Out<X:Mark>;}; type Owner is {}; implement Rel for Owner {type Out<Y:Mark> = Y;}";
    let errors = errors_with_bodies(source, false);
    assert!(errors.is_empty(), "{errors:?}");
}
#[test]
fn registration_cannot_publish_an_unproved_result_bound() {
    let source = "module families; type Mark is protocol {}; type Rel is protocol {type Out<X>:Mark;}; type Owner is {}; type Bad is {}; implement Rel for Owner {type Out<Y> = Bad;}";
    let errors = errors_with_bodies(source, false);
    assert!(
        errors.iter().any(|error| error.contains("E405")),
        "{errors:?}"
    );
}
#[test]
fn registration_accepts_a_universally_proved_result_bound() {
    let source = "module families; type Mark is protocol {}; type Wrapper<T> is {value:T}; implement<T> Mark for Wrapper<T> {} type Rel is protocol {type Out<X>:Mark;}; type Owner is {}; implement Rel for Owner {type Out<Y> = Wrapper<Y>;}";
    let errors = errors_with_bodies(source, false);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn symbolic_family_domain_is_checked_inside_a_callable_bound() {
    let source = "module families; type Row is protocol {}; type Source is protocol {type Rows<R:Row>; fn map<R:Row,F:fn(Self.Rows<R>)->Int>(&self, callback:F)->Int;};";
    let errors = errors(source);
    assert!(errors.is_empty(), "{errors:?}");
}
#[test]
fn a_callable_bound_cannot_hide_an_unproved_family_domain() {
    let source = "module families; type Row is protocol {}; type Source is protocol {type Rows<R:Row>; fn map<R,F:fn(Self.Rows<R>)->Int>(&self, callback:F)->Int;};";
    let errors = errors(source);
    assert!(
        errors.iter().any(|error| error.contains("E405")),
        "{errors:?}"
    );
}
#[test]
fn a_family_implementation_requires_its_callable_owner_contract() {
    let source = "module families; type Rel is protocol {type Out<X>;}; type Owner<F> is {callback:F}; implement<F:fn()->Int> Rel for Owner<F> {type Out<X> = (F,X);} type Bad is Owner<async fn()->Int>; fn probe(value:Bad.Out<Text>)->(async fn()->Int,Text){value}";
    let errors = errors(source);
    assert!(
        errors.iter().any(|error| error.contains("E405")),
        "{errors:?}"
    );
}
#[test]
fn a_family_implementation_accepts_its_callable_owner_contract() {
    let source = "module families; type Rel is protocol {type Out<X>;}; type Owner<F> is {callback:F}; implement<F:fn()->Int> Rel for Owner<F> {type Out<X> = (F,X);} type Good is Owner<fn()->Int>; fn probe(value:Good.Out<Text>)->(fn()->Int,Text){value}";
    let errors = errors(source);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn unsaturated_family_is_not_an_ordinary_associated_type() {
    let errors = errors(&format!(
        "{SIMPLE} fn identity(value:Owner.Out)->Owner.Out{{value}}"
    ));
    assert!(
        errors.iter().any(|error| error.contains("E405")),
        "{errors:?}"
    );
}
#[test]
fn a_family_default_does_not_erase_required_local_arguments() {
    let source = "module families; type Rel is protocol {type Out<X> = Int;}; type Owner is {}; implement Rel for Owner {type Out<X> = Int;} fn identity(value:Owner.Out)->Owner.Out{value}";
    let errors = errors(source);
    assert!(
        errors.iter().any(|error| error.contains("E405")),
        "{errors:?}"
    );
}

#[test]
fn a_value_autoref_cannot_prove_a_callable_owner_condition() {
    let source = "module families; type Rel is protocol {type Out<X>;}; type Owner<F> is {callback:F}; implement<F:fn()->&Text> Rel for Owner<F> {type Out<X> = (F,X);} type Bad is Owner<fn()->Text>; fn probe(value:Bad.Out<Int>)->(fn()->Text,Int){value}";
    let errors = errors(source);
    assert!(
        errors.iter().any(|error| error.contains("E405")),
        "{errors:?}"
    );
}
#[test]
fn an_exact_reference_result_satisfies_a_callable_owner_condition() {
    let source = "module families; type Rel is protocol {type Out<X>;}; type Owner<F> is {callback:F}; implement<F:fn()->&Text> Rel for Owner<F> {type Out<X> = (F,X);} type Good is Owner<fn()->&Text>; fn probe(value:Good.Out<Int>)->(fn()->&Text,Int){value}";
    let errors = errors(source);
    assert!(errors.is_empty(), "{errors:?}");
}
#[test]
fn inherited_family_retains_its_declared_local_domain() {
    let source = "module families; type Row is protocol {}; type Parent<A> is protocol {type Out<R:Row>;}; type Child is protocol extends Parent<Bool> {}; fn preserve<S:Child,R:Row>(value:S.Out<R>)->S.Out<R>{value}";
    let errors = errors(source);
    assert!(errors.is_empty(), "{errors:?}");
}
#[test]
fn inherited_family_does_not_become_a_zero_argument_type() {
    let source = "module families; type Row is protocol {}; type Parent<A> is protocol {type Out<R:Row>;}; type Child is protocol extends Parent<Bool> {}; fn preserve<S:Child>(value:S.Out)->S.Out{value}";
    let errors = errors(source);
    assert!(
        errors.iter().any(|error| error.contains("E405")),
        "{errors:?}"
    );
}

#[test]
fn an_identity_body_cannot_hide_an_invalid_family_owner_contract() {
    let source = "module families; type Rel is protocol {type Out<X>;}; type Owner<F> is {callback:F}; implement<F:fn()->Int> Rel for Owner<F> {type Out<X> = (F,X);} type Bad is Owner<async fn()->Int>; fn identity(value:Bad.Out<Text>)->Bad.Out<Text>{value}";
    let errors = errors(source);
    assert!(
        errors.iter().any(|error| error.contains("E405")),
        "{errors:?}"
    );
}
