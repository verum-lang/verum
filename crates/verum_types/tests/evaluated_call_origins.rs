//! T2860: call results use evaluated values; later source reads see mutations.
use verum_ast::FileId;
use verum_common::List;
use verum_diagnostics::Diagnostic;
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Diagnostic> {
    let module = FastParser::new().parse_module_str(source, FileId::new(1))
        .expect("source grammar");
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("evaluated.origins");
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) { errors.push(error); }
    }
    errors.extend(checker.take_deferred_errors());
    errors.iter().chain(checker.diagnostic_sources())
        .map(verum_types::TypeError::to_diagnostic).collect()
}

#[track_caller]
fn accepted(source: &str) {
    let errors = errors(source);
    assert!(errors.is_empty(), "{errors:#?}");
}

#[track_caller]
fn lifetime_rejected(source: &str) {
    let errors = errors(source);
    assert!(errors.iter().any(|error| error.code() == Some("E312")
        || (matches!(error.code(), Some("E310" | "E311")) && error.message().starts_with("cannot borrow"))),
        "{errors:#?}");
}

#[test]
fn generic_callback_forwards_the_evaluated_value_but_cannot_reread_its_source() {
    accepted("fn apply<T,F>(value:T,callback:F)->T where F=fn(T)->T {callback(value)}");
    lifetime_rejected("fn apply<T,F>(value:T,callback:F)->T where F=fn(T)->T {callback(value); value}");
}

#[test]
fn a_callback_result_keeps_reference_payloads_captured_before_its_body_scope() {
    let prefix = "fn invoke<F>(callback:F)->&Int where F=fn()->&Int {callback()}";
    accepted(&format!("{prefix} fn receive(input:&Int)->&Int {{let selected=|| input; invoke(selected)}}"));
    accepted(&format!("{prefix} fn receive()->Int {{let local=4; let loan=&local; let selected=|| loan; *invoke(selected)}}"));
    lifetime_rejected(&format!("{prefix} fn escape()->&Int {{let local=4; let loan=&local; let selected=|| loan; invoke(selected)}}"));
    lifetime_rejected(&format!("{prefix} fn escape()->&Int {{let local=4; let loan=&local; invoke(|| loan)}}"));
}

#[test]
fn immutable_scalar_capture_does_not_borrow_its_old_binding_slot() {
    accepted("fn make()->fn()->Int {let local=42; || local} fn run()->Int {let selected=make(); selected()}");
}

#[test]
fn mutable_capture_storage_cannot_be_relabelled_as_an_independent_scalar_copy() {
    accepted("fn run()->Int {let mut local=1; let selected=|| {local+=1; local}; selected()}");
    lifetime_rejected("fn escape()->fn()->Int {let mut local=1; || {local+=1; local}}");
    lifetime_rejected("fn escape()->fn()->Int {let mut local=1; || local}");
}

#[test]
fn a_reference_into_owned_capture_storage_requires_a_result_lifetime_contract() {
    lifetime_rejected("fn escape()->&Int {let local=4; let selected=|| &local; selected()}");
    lifetime_rejected("fn escape()->&Int {let selected=|| {let local=4; &local}; selected()}");
    lifetime_rejected("type IntRef = &Int; fn escape(input:&Int)->&IntRef {let selected=|| {let result=&input; result}; selected()}");
    accepted("fn receive(input:&Int)->&Int {let selected=|| {let result=input; result}; selected()}");
}

#[test]
fn incoming_borrowed_storage_outlives_owned_parameter_slots() {
    lifetime_rejected("fn install<T>(destination:&mut &T,value:T) {*destination=&value;}");
    accepted("fn install<T>(destination:&mut &T,value:&T) {*destination=value;}");
}

#[test]
fn callback_capture_loans_cannot_escape_through_incoming_writable_storage() {
    accepted(&format!("{PAYLOAD} fn install(target:&mut Payload,input:&Int) {{let callback:fn(&mut Payload)=|destination| {{destination.ptr=input;}}; callback(target);}}"));
    lifetime_rejected(&format!("{PAYLOAD} fn install(target:&mut Payload) {{let local=3; let loan=&local; let callback:fn(&mut Payload)=|destination| {{destination.ptr=loan;}}; callback(target);}}"));
}

#[test]
fn branch_and_loop_overwrites_do_not_dominate_the_unexecuted_path() {
    for update in [
        "if change {{value=Payload{ptr:input,marker:8};}}",
        "while change {{value=Payload{ptr:input,marker:8}; break;}}",
    ] {
        lifetime_rejected(&format!("{PAYLOAD} fn escape(input:&Int,change:Bool)->Payload {{let local=3; let mut value=Payload{{ptr:&local,marker:7}}; {update} value}}"));
    }
}

#[test]
fn short_circuit_and_deferred_overwrites_cannot_erase_current_payload_loans() {
    for update in [
        "false && {value=Payload{ptr:input,marker:8}; false};",
        "true || {value=Payload{ptr:input,marker:8}; true};",
        "defer {value=Payload{ptr:input,marker:8};}",
        "errdefer {value=Payload{ptr:input,marker:8};}",
    ] {
        lifetime_rejected(&format!("{PAYLOAD} fn escape(input:&Int)->Payload {{let local=3; let mut value=Payload{{ptr:&local,marker:7}}; {update} value}}"));
    }
}

const PAYLOAD: &str = r#"
    type Payload is {public ptr:&Int,public marker:Int};
    fn replace(target:&mut Payload) {target.ptr=&target.marker;}
    fn first(value:Payload,marker:())->Payload {value}
"#;

#[test]
fn later_arguments_do_not_retroactively_reread_an_owned_earlier_argument() {
    accepted(&format!("{PAYLOAD} fn receive(input:&Int)->Payload {{let mut value=Payload{{ptr:input,marker:7}}; first(value,replace(&mut value))}}"));
    lifetime_rejected(&format!("{PAYLOAD} fn receive(input:&Int)->Payload {{let mut value=Payload{{ptr:input,marker:7}}; first(value,replace(&mut value)); value}}"));
}

#[test]
fn snapshots_do_not_hide_shorter_argument_loans_or_post_call_payload_writes() {
    lifetime_rejected(&format!("{PAYLOAD} fn escape()->Payload {{let local=3; let value=Payload{{ptr:&local,marker:7}}; first(value,())}}"));
    lifetime_rejected(&format!("{PAYLOAD} fn receive(input:&Int)->Payload {{let mut value=Payload{{ptr:input,marker:7}}; replace(&mut value); first(value,())}}"));
}

#[test]
fn tuple_elements_keep_their_value_origins_in_evaluation_order() {
    let source = format!("{PAYLOAD} fn first_tuple(value:(Payload,()))->(Payload,()) {{value}}");
    lifetime_rejected(&format!("{source} fn escape(input:&Int)->(Payload,()) {{let local=3; let mut value=Payload{{ptr:&local,marker:7}}; first_tuple((value,{{value=Payload{{ptr:input,marker:8}}; ()}}))}}"));
    accepted(&format!("{PAYLOAD} fn last_tuple(value:((),Payload))->((),Payload) {{value}} fn receive(input:&Int)->((),Payload) {{let local=3; let mut value=Payload{{ptr:&local,marker:7}}; last_tuple(({{value=Payload{{ptr:input,marker:8}}; ()}},value))}}"));
}

#[test]
fn record_fields_and_shorthand_keep_origins_at_their_actual_evaluation() {
    let source = format!("{PAYLOAD} type Bundle is {{public value:Payload,public marker:()}}; fn forward(value:Bundle)->Bundle {{value}}");
    for first in ["value:value", "value"] {
        lifetime_rejected(&format!("{source} fn escape(input:&Int)->Bundle {{let local=3; let mut value=Payload{{ptr:&local,marker:7}}; forward(Bundle{{{first},marker:{{value=Payload{{ptr:input,marker:8}}; ()}}}})}}"));
    }
}

#[test]
fn array_elements_keep_their_evaluated_payload_origins() {
    let source = format!("{PAYLOAD} fn forward(value:[Payload;2])->[Payload;2] {{value}}");
    lifetime_rejected(&format!("{source} fn escape(input:&Int)->[Payload;2] {{let local=3; let mut value=Payload{{ptr:&local,marker:7}}; forward([value,{{value=Payload{{ptr:input,marker:8}}; value}}])}}"));
    accepted(&format!("{source} fn receive(input:&Int)->[Payload;2] {{let local=3; let mut value=Payload{{ptr:&local,marker:7}}; forward([{{value=Payload{{ptr:input,marker:8}}; value}},value])}}"));
}

#[test]
fn a_reference_operand_does_not_snapshot_the_future_contents_of_its_referent() {
    lifetime_rejected(&format!("{PAYLOAD} fn read(source:&Payload,marker:())->Payload {{Payload{{ptr:source.ptr,marker:source.marker}}}} fn receive(input:&Int)->Payload {{let mut value=Payload{{ptr:input,marker:7}}; read(&value,replace(&mut value))}}"));
}

#[test]
fn contextual_borrowing_keeps_the_source_place_loan() {
    let source = format!("{PAYLOAD} fn selected(source:&Payload)->&Int {{&source.marker}}");
    accepted(&format!("{source} fn receive(source:&Payload)->&Int {{selected(source)}}"));
    lifetime_rejected(&format!("{source} fn escape(input:&Int)->&Int {{let value=Payload{{ptr:input,marker:7}}; selected(value)}}"));
}

#[test]
fn mutually_exclusive_argument_loans_still_conflict_before_invocation() {
    lifetime_rejected("fn selected(left:&mut Int,right:&Int)->Int {*left+*right} fn run()->Int {let mut value=7; selected(&mut value,&value)}");
}

#[test]
fn invoking_an_unknown_callback_does_not_rewrite_its_binding_slot() {
    accepted("fn invoke(action:fn()->Int)->Int {let first=action(); first+action()}");
    accepted("fn invoke(action:fn()->Int)->Int {action()} fn twice(action:fn()->Int)->Int {invoke(action)+invoke(action)}");
    accepted("fn calculate(input:Int)->Int {fn spread(mut value:Int)->Int {value=value*2; value} spread(input)+spread(input)}");
}

#[test]
fn unknown_environments_cannot_reach_unpassed_fresh_local_slots() {
    accepted(&format!("{PAYLOAD} fn receive(input:&Int,action:fn())->Payload {{let value=Payload{{ptr:input,marker:7}}; action(); value}}"));
    // A copied payload is stable even when the unknown environment can reach
    // its old input storage. The environment cannot rewrite this fresh slot.
    accepted(&format!("{PAYLOAD} fn receive(input:&Payload,action:fn())->Payload {{let value=Payload{{ptr:input.ptr,marker:7}}; action(); value}}"));
}

#[test]
fn unknown_environments_still_invalidate_reachable_incoming_referents() {
    lifetime_rejected(&format!("{PAYLOAD} fn receive(input:&mut Payload,action:fn())->&Int {{action(); input.ptr}}"));
    lifetime_rejected(&format!("{PAYLOAD} fn receive(input:&mut Payload,action:fn(&mut Payload))->&Int {{action(input); input.ptr}}"));
}

#[test]
fn shared_inputs_cannot_be_retained_in_exclusively_borrowed_slots() {
    for exclusive in ["&mut Text", "&checked mut Text", "&unsafe mut Text"] {
        let prefix = format!("type Sink is {{public buffer:{exclusive}}}; type SinkRef = &mut Sink;");
        accepted(&format!("{prefix} fn render(target:SinkRef,write:fn(SinkRef,&Text)) {{let text=\"literal\"; write(target,&text);}}"));
    }
    lifetime_rejected("type Sink is {public buffer:&Text}; fn render(target:&mut Sink,write:fn(&mut Sink,&Text)) {let text=\"literal\"; write(target,&text);}");
    lifetime_rejected("type Sink is {public buffer:&mut Text}; fn render(target:&mut Sink,write:fn(&mut Sink,&mut Text)) {let mut text=\"literal\"; write(target,&mut text);}");
}

#[test]
fn exclusive_carriers_still_expose_nested_shared_reference_slots() {
    lifetime_rejected("type Cell is {public pointer:&Int}; type Sink is {public target:&mut Cell}; fn render(target:&mut Sink,write:fn(&mut Sink,&Int)) {let local=7; write(target,&local);}");
    lifetime_rejected("type Cell is {public pointer:&Int}; type Sink<T> is {public target:&mut T}; fn render(target:&mut Sink<Cell>,write:fn(&mut Sink<Cell>,&Int)) {let local=7; write(target,&local);}");
}
