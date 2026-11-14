//! Selected source calls require their own stored-value contract.
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn errors(declarations: &str, caller: &str) -> List<Text> {
    let source = format!(
        "type Cell is {{ public pointer:&Int, public value:Int }}; {declarations} {caller}"
    );
    let module = FastParser::new()
        .parse_module_str(&source, FileId::new(1))
        .unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("write_probe");
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    errors
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|e| e.to_string().into())
        .collect()
}

#[test]
fn selected_closed_call_writes_preserve_sibling_payloads() {
    let found = errors(
        "fn selected(value:&Int)->Int {73} fn update(cell:&mut Cell, value:&Int) {cell.value=selected(value);}",
        "fn preserve(mut cell:Cell)->Cell {let local=4;update(&mut cell,&local);cell}",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn callback_result_writes_do_not_gain_a_selected_source_contract() {
    let found = errors(
        "fn update(cell:&mut Cell, action:fn(&Int)->Int) {cell.value=action(&cell.value);}",
        "fn preserve(mut cell:Cell, action:fn(&Int)->Int)->Cell {update(&mut cell,action);cell}",
    );
    assert!(found.iter().any(|e| e.contains("cannot establish the lifetime")), "{found:?}");
}

#[test]
fn a_source_call_result_cannot_hide_stored_reference_writes() {
    let found = errors(
        "fn selected(value:&Int)->&Int {value} fn update(cell:&mut Cell, value:&Int) {cell.pointer=selected(value);}",
        "fn preserve(input:&Int)->Cell {let mut cell=Cell{pointer:input,value:0}; {let local=73;update(&mut cell,&local);}cell}",
    );
    assert!(found.iter().any(|e| e.contains("local") && e.contains("does not live long enough")), "{found:?}");
}

#[test]
fn a_selected_readonly_method_keeps_its_declared_receiver() {
    let found = errors(
        "type Source is {public value:Int}; implement Source {fn read(&self)->Int {self.value}} fn update(cell:&mut Cell, source:&Source) {cell.value=source.read();}",
        "fn preserve(mut cell:Cell)->Cell {let local=Source{value:73};update(&mut cell,&local);cell}",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_bodyless_function_is_not_a_source_result_contract() {
    let found = errors(
        "fn selected(value:&Int)->Int; fn update(cell:&mut Cell, value:&Int) {cell.value=selected(value);}",
        "fn preserve(mut cell:Cell)->Cell {let local=4;update(&mut cell,&local);cell}",
    );
    assert!(found.iter().any(|e| e.contains("cannot establish the lifetime")), "{found:?}");
}

#[test]
fn a_callback_parameter_cannot_inherit_a_same_named_functions_body() {
    let found = errors(
        "fn selected(value:&Int)->Int {73} fn update(cell:&mut Cell, value:&Int, selected:fn(&Int)->Int) {cell.value=selected(value);}",
        "fn preserve(mut cell:Cell, action:fn(&Int)->Int)->Cell {let local=4;update(&mut cell,&local,action);cell}",
    );
    assert!(found.iter().any(|e| e.contains("cannot establish the lifetime")), "{found:?}");
}

#[test]
fn a_scalar_result_does_not_erase_the_selected_callees_writes() {
    let found = errors(
        "fn selected(cell:&mut Cell,value:&Int)->Int {cell.pointer=value;73} fn update(cell:&mut Cell,value:&Int) {cell.value=selected(cell,value);}",
        "fn preserve(input:&Int)->Cell {let mut cell=Cell{pointer:input,value:0}; {let local=73;update(&mut cell,&local);}cell}",
    );
    assert!(found.iter().any(|e| e.contains("local") && e.contains("does not live long enough")), "{found:?}");
}

#[test]
fn a_selected_scalar_result_does_not_hide_owned_local_cleanup() {
    let found = errors(
        "type Drop is protocol {fn drop(&mut self);}; type affine Deferred is {target:&mut Cell,source:&Int}; implement Drop for Deferred {fn drop(&mut self) {self.target.pointer=self.source;}} fn selected(cell:&mut Cell,value:&Int)->Int {let guard=Deferred{target:cell,source:value};73} fn update(cell:&mut Cell,value:&Int) {cell.value=selected(cell,value);}",
        "fn preserve(input:&Int)->Cell {let mut cell=Cell{pointer:input,value:0}; {let local=73;update(&mut cell,&local);}cell}",
    );
    assert!(found.iter().any(|e| e.contains("local") && e.contains("does not live long enough")), "{found:?}");
}

#[test]
fn recursive_calls_do_not_create_an_unbounded_provenance_proof() {
    let found = errors(
        "fn selected(value:&Int)->Int {selected(value)} fn update(cell:&mut Cell,value:&Int) {cell.value=selected(value);}",
        "fn preserve(mut cell:Cell)->Cell {let local=4;update(&mut cell,&local);cell}",
    );
    assert!(found.iter().any(|e| e.contains("cannot establish the lifetime")), "{found:?}");
}
