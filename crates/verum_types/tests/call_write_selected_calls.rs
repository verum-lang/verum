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
    checker.with_module_declarations(&module, |checker| {
        for item in &module.items {
            if let Err(error) = checker.check_item(item) {
                errors.push(error);
            }
        }
    });
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

#[test]
fn a_selected_writer_can_return_a_sum_with_structural_cleanup() {
    let found = errors(
        "type Failure is {}; type Outcome<T,E> is Written(T) | Failed(E);
         type Writer is { target:&mut Cell };
         implement Cell { fn set(&mut self,n:Int)->Outcome<(),Failure> {
             self.value=n; Outcome.Written(())
         } }
         implement Writer { fn write(&mut self,n:Int)->Outcome<(),Failure> { self.target.set(n) } }",
        "fn preserve(mut writer:Writer)->Writer {
            let _=writer.write(1); let _=writer.write(2); writer
         }",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_sum_result_cannot_certify_an_unsafe_field_rebinding() {
    let found = errors(
        "type Failure is {}; type Outcome<T,E> is Written(T) | Failed(E);
         type Writer is { target:&mut Cell };
         implement Cell { fn set(&mut self,p:&Int)->Outcome<(),Failure> {
             self.pointer=p; Outcome.Written(())
         } }
         implement Writer { fn write(&mut self,p:&Int)->Outcome<(),Failure> { self.target.set(p) } }",
        "fn reject(mut writer:Writer)->Writer {
            {let local=7; let _=writer.write(&local);} writer
         }",
    );
    assert!(found.iter().any(|error| error.contains("does not live long enough")
        || error.contains("cannot establish the lifetime")), "{found:?}");
}

#[test]
fn a_selected_writer_result_rejects_custom_cleanup_on_an_empty_error_record() {
    let found = errors(
        "type Drop is protocol {fn drop(&mut self);};
         type Failure is {}; implement Drop for Failure {fn drop(&mut self) {}}
         type Outcome<T,E> is Written(T) | Failed(E);
         type Writer is { target:&mut Cell };
         implement Cell { fn set(&mut self,n:Int)->Outcome<(),Failure> {
             self.value=n; Outcome.Written(())
         } }
         implement Writer { fn write(&mut self,n:Int)->Outcome<(),Failure> { self.target.set(n) } }",
        "fn preserve(mut writer:Writer)->Writer {
            let _=writer.write(1); let _=writer.write(2); writer
         }",
    );
    assert!(found.iter().any(|error| error.contains("cannot establish the lifetime")), "{found:?}");
}

fn receiver_writes(extra: &str, body: &str) -> List<Text> {
    let declarations = format!("type Sink is {{value:Int}};
        implement Sink {{ fn put(&mut self,n:Int) {{ self.value=n; }} }}
        type Writer is {{output:&mut Sink, adjust:Bool}};
        {extra}
        implement Writer {{
            fn dispatch(&mut self,n:Int)->Int {{
                match self.adjust {{ true=>self.write(n+1), false=>self.write(n) }}
            }}
            fn write(&mut self,n:Int)->Int {{ {body} }}
            fn mode(&self)->Bool {{ self.adjust }}
        }}");
    errors(&declarations, "fn caller(output:&mut Sink)->Bool {
        let mut writer=Writer{output:output,adjust:true};
        let _=writer.dispatch(1); let _=writer.dispatch(2); writer.mode()
    }")
}

#[test]
fn branching_selected_methods_only_reborrow_the_output_slot() {
    let found = receiver_writes("", "let mut left=n;
        while left>0 { self.output.put(left); left=left-1; } n");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn forwarding_checks_each_branch_for_unverified_or_escaping_receiver_uses() {
    for (extra, body) in [
        ("", "if n>0 { let mut local=Sink{value:1}; self.output=&mut local; } n"),
        ("fn other_body(value:&mut Writer);", "if n>0 { other_body(self); } n"),
        ("fn other_body()->Int;", "if n>0 { let _=other_body(); } n"),
        ("", "self.dispatch(n)"),
        ("type Marker is {tag:Int};", "let a=Marker{tag:n}; let _=a==a; self.output.put(n); n"),
        ("", "let mutate=|| { self.output.put(n); }; mutate(); n"),
        ("type Drop is protocol {fn drop(&mut self);}; type Deferred is {sink:&mut Sink};
          implement Drop for Deferred {fn drop(&mut self) {self.sink.put(9);}}",
         "let guard=Deferred{sink:self.output}; n"),
        ("type Drop is protocol {fn drop(&mut self);}; type Deferred is {};
          implement Drop for Deferred {fn drop(&mut self) {}} fn guard()->Deferred {Deferred{}}",
         "guard(); self.output.put(n); n"),
    ] {
        let found = receiver_writes(extra, body);
        assert!(found.iter().any(|error| error.contains("cannot establish the lifetime")
            || error.contains("does not live long enough")), "{body}: {found:?}");
    }
}
