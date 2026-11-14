//! Branches and loops preserve loans only through exact, typed write footprints.
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let module = FastParser::new().parse_module_str(source, FileId::new(1)).unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("cursor_probe");
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) { errors.push(error); }
    }
    errors.extend(checker.take_deferred_errors());
    errors.iter().chain(checker.diagnostic_sources()).map(|error| error.to_string().into()).collect()
}

#[test]
fn conditional_scalar_writes_preserve_the_unmodified_payload() {
    let found = errors("type Cursor is { public input:&Int, public pos:Int }; fn step(p:&mut Cursor) {if p.pos < 73 {p.pos=p.pos+1;} else {p.pos=0;}} fn preserve(mut p:Cursor)->Cursor {step(&mut p);p}");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn repeated_scalar_writes_preserve_the_unmodified_payload() {
    let found = errors("type Cursor is { public input:&Int, public pos:Int }; fn step(p:&mut Cursor) {while p.pos < 73 {p.pos+=1;}} fn preserve(mut p:Cursor)->Cursor {step(&mut p);p}");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn selected_nested_steps_use_the_same_write_contract() {
    let found = errors("type Cursor is { public input:&Int, public pos:Int }; fn step(p:&mut Cursor) {if p.pos < 73 {p.pos+=1;}} fn walk(p:&mut Cursor) {while p.pos < 73 {step(p);}} fn preserve(mut p:Cursor)->Cursor {walk(&mut p);p}");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn declared_slice_reads_do_not_turn_a_cursor_write_into_a_payload_write() {
    let found = errors("type Cursor is { public bytes:&[Byte], public pos:Int }; fn step(p:&mut Cursor) {if p.pos < p.bytes.len() {let b=p.bytes[p.pos];p.pos+=1;if b==0x0A_u8 {p.pos=0;}}} fn walk(p:&mut Cursor) {while p.pos < p.bytes.len() {step(p);break;}} fn preserve(mut p:Cursor)->Cursor {walk(&mut p);p}");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn conditional_reference_injection_still_retains_the_shorter_loan() {
    let found = errors("type Cursor is { public input:&Int, public pos:Int }; fn step(p:&mut Cursor, value:&Int) {if p.pos < 73 {p.input=value;}} fn invalid(input:&Int)->Cursor {let mut p=Cursor{input:input,pos:0};{let local=73;step(&mut p,&local);}p}");
    assert!(found.iter().any(|error|error.contains("does not live long enough") || error.contains("cannot establish the lifetime")), "{found:?}");
}

#[test]
fn loops_cannot_hide_an_unknown_or_recursive_writer() {
    for declarations in [
        "fn step(p:&mut Cursor, callback:fn(&mut Cursor)) {while p.pos < 73 {callback(p);p.pos+=1;}} fn invalid(mut p:Cursor, callback:fn(&mut Cursor))->Cursor {step(&mut p,callback);p}",
        "fn step(p:&mut Cursor) {while p.pos < 73 {step(p);}} fn invalid(mut p:Cursor)->Cursor {step(&mut p);p}",
    ] {
        let found = errors(&format!("type Cursor is {{ public input:&Int, public pos:Int }}; {declarations}"));
        assert!(found.iter().any(|error|error.contains("cannot establish the lifetime")), "{declarations}: {found:?}");
    }
}

#[test]
fn a_same_shaped_method_is_not_a_structural_operation_receipt() {
    let found = errors("type Reader is {public action:fn()->Int}; implement Reader {fn len(&self)->Int {(self.action)()}} type Cursor is {public input:&Int,public reader:Reader,public pos:Int}; fn step(p:&mut Cursor) {p.pos=p.reader.len();} fn invalid(mut p:Cursor)->Cursor {step(&mut p);p}");
    assert!(found.iter().any(|error|error.contains("cannot establish the lifetime")), "{found:?}");
}

#[test]
fn a_value_producing_branch_does_not_become_a_unit_signature() {
    let found = errors("fn choose(flag:Bool) {if flag {73} else {0}} fn check()->Int {choose(true)}");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn selected_call_depth_uses_bounded_work_on_an_ordinary_stack() {
    let mut source = Text::from("type Cursor is {public input:&Int,public pos:Int}; fn step0(p:&mut Cursor) {p.pos+=1;}");
    for depth in 1..=80 { source.push_str(&format!("fn step{depth}(p:&mut Cursor) {{step{}(p);}}",depth-1)); }
    source.push_str("fn preserve(mut p:Cursor)->Cursor {step80(&mut p);p}");
    let found = errors(&source);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn exhausted_write_proof_retains_unknown_mutation() {
    let body = "p.pos=1;".repeat(2200);
    let found = errors(&format!("type Cursor is {{public input:&Int,public pos:Int}}; fn step(p:&mut Cursor) {{{body}}} fn invalid(mut p:Cursor)->Cursor {{step(&mut p);p}}"));
    assert!(found.iter().any(|error|error.contains("cannot establish the lifetime")), "{found:?}");
}

#[test]
fn a_nested_cursor_step_preserves_the_borrowed_callers_payload() {
    let found = errors(r#"
        type Cursor is {public bytes:&[Byte],public pos:Int,public line:Int,public column:Int};
        fn step(p:&mut Cursor) {
            if p.pos < p.bytes.len() {
                let b=p.bytes[p.pos];
                p.pos=p.pos+1;
                if b==0x0A_u8 {p.line=p.line+1;p.column=1;}
                else {p.column=p.column+1;}
            }
        }
        fn walk(p:&mut Cursor) {
            while p.pos < p.bytes.len() {
                let b=p.bytes[p.pos];
                if b==0x20_u8 || b==0x09_u8 || b==0x0A_u8 || b==0x0D_u8 {step(p);}
                else {break;}
            }
        }
        fn parse(p:&mut Cursor)->Int {walk(p);step(p);walk(p);p.pos}
    "#);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_proved_scalar_writer_does_not_restore_previously_unknown_payloads() {
    let found = errors(r#"
        type Cursor is {public input:&Int,public pos:Int};
        fn invoke_action(p:&mut Cursor,action:fn(&mut Cursor)) {action(p);}
        fn touch(p:&mut Cursor) {if p.pos<73 {p.pos+=1;}}
        fn invalid(mut p:Cursor,action:fn(&mut Cursor))->Cursor {
            invoke_action(&mut p,action);touch(&mut p);p
        }
    "#);
    assert!(found.iter().any(|error|error.contains("cannot establish the lifetime")),"{found:?}");
}

#[test]
fn a_returned_borrowed_field_keeps_its_actual_source() {
    let found = errors(r#"
        type Cursor is {public input:&Int,public pos:Int};
        fn read(p:&Cursor)->&Int {let ignored=p.pos;p.input}
        fn replace(p:&mut Cursor,value:&Int) {p.input=value;}
        fn invalid(input:&Int)->&Int {
            let mut p=Cursor{input,pos:0};
            {let local=73;replace(&mut p,&local);}
            read(&p)
        }
    "#);
    assert!(found.iter().any(|error|error.contains("local") && error.contains("does not live long enough")),"{found:?}");
}
