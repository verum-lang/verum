//! A selected source function has no caller-local captured environment (T2765).
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn diagnostics(body: &str) -> List<Text> {
    let source = format!("type Payload is {{public ptr:&Int,public marker:Int}};
        pure fn closed(value:Int)->Int {{value}}
        fn execute(operation:fn(Int)->Int)->Int {{operation(73)}}
        {body}");
    let module = FastParser::new().parse_module_str(&source, FileId::new(1)).unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("callable_environment");
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) { errors.push(error); }
    }
    errors.extend(checker.take_deferred_errors());
    errors.iter().chain(checker.diagnostic_sources())
        .map(|error| error.to_string().into()).collect()
}

fn rejected(body: &str) {
    let errors = diagnostics(body);
    assert!(errors.iter().any(|error| error.contains("cannot establish the lifetime")
        || error.contains("does not live long enough")
        || error.contains("borrowed")), "{body}: {errors:#?}");
}

#[test]
fn a_named_source_function_preserves_unrelated_payload_origins() {
    let errors = diagnostics("fn receive(input:&Int)->Payload {
        let kept=Payload{ptr:input,marker:2}; execute(closed);
        Payload{ptr:kept.ptr,marker:kept.marker}}");
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn a_named_source_function_preserves_an_owned_scalar_capture() {
    let errors = diagnostics("fn receive()->Int {
        let factor=3; let scaled:fn(Int)->Int=|value| factor*value;
        execute(closed); execute(scaled)}");
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn a_named_source_function_preserves_a_live_shared_capture() {
    let errors = diagnostics("fn receive(input:&Int)->Int {
        let read:fn(Int)->Int=|value| *input+value; execute(closed);
        execute(read)}");
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn a_real_borrow_still_cannot_escape_after_a_closed_function_argument() {
    rejected("fn receive()->Payload {let local=3; let loan=&local;
        let read:fn()->Int=|| *loan; execute(closed);
        Payload{ptr:loan,marker:read()}}");
}

#[test]
fn an_opaque_parameter_cannot_borrow_a_named_functions_provenance() {
    rejected("fn receive(input:&Int,closed:fn(Int)->Int)->Payload {
        let kept=Payload{ptr:input,marker:2}; execute(closed);
        Payload{ptr:kept.ptr,marker:kept.marker}}");
}

#[test]
fn a_local_shadow_cannot_borrow_a_named_functions_provenance() {
    rejected("fn receive(input:&Int,operation:fn(Int)->Int)->Payload {
        let closed=operation; let kept=Payload{ptr:input,marker:2}; execute(closed);
        Payload{ptr:kept.ptr,marker:kept.marker}}");
}

#[test]
fn a_mutating_capture_keeps_its_payload_lifetime_refusal() {
    rejected("fn receive(input:&Int)->Payload {let mut kept=Payload{ptr:input,marker:2};
        {let local=3; execute(|value| {kept.ptr=&local; value});}
        Payload{ptr:kept.ptr,marker:kept.marker}}");
}

#[test]
fn a_nested_function_is_not_a_module_level_environment_proof() {
    rejected("fn receive(input:&Int)->Payload {let mut kept=Payload{ptr:input,marker:2};
        {let local=3; fn nested(value:Int)->Int {kept.ptr=&local; value} execute(nested);}
        Payload{ptr:kept.ptr,marker:kept.marker}}");
}
