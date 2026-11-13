//! Assignments retain only loans stored in the destination's checked value.
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let source = format!("type Cursor is {{first:Int64, second:Int64, done:Bool}};\n{source}");
    let ast = FastParser::new()
        .parse_module_str(&source, FileId::new(1))
        .expect("grammar");
    let mut checker = TypeChecker::new();
    checker.set_current_module_path("sample");
    checker.register_primitives();
    let mut errors = checker.prepare_module_declarations(&ast);
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    let mut messages: List<Text> = errors
        .iter()
        .map(|error| format!("{error:?}").into())
        .collect();
    messages.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| d.message().into()),
    );
    messages
}

#[test]
fn scalar_copy_from_reference_does_not_borrow_the_write_receiver() {
    for source in [
        "implement Cursor { fn update(&mut self, p:&Int64) { let mut first:Int64=0_i64; let mut second:Int64=1_i64; first=p; second=p; self.first=first; self.second=second; self.done=first<second; }}",
        "type Number is Value(Int64) | Missing; implement Cursor { fn update(&mut self, p:&Number) { let mut first:Int64=0_i64; match p { Value(x)=>{ first=x; } Missing=>{} } self.first=first; self.second=first; self.done=true; }}",
        "fn update(p:&mut Cursor, n:&Int64) { let mut value:Int64=0_i64; value=n; p.first=value; p.second=value; }",
        "implement Cursor { fn update(&mut self, p:&Int64) { self.first=p; self.second=p; self.done=true; }}",
    ] {
        let errors = errors(source);
        assert!(errors.is_empty(), "{source}\n{errors:?}");
    }
}

#[test]
fn live_shared_alias_still_blocks_projected_writes() {
    for source in [
        "implement Cursor { fn update(&mut self, value:Int64)->Int64 { let keep=&*self; self.first=value; keep.first }}",
        "fn update(p:&mut Cursor,value:Int64)->Int64 { let keep=&*p; p.second=value; keep.second }",
        "fn update(p:&mut Cursor,value:Int64)->Int64 { let keep=&p.first; p.first=value; *keep }",
    ] {
        let errors = errors(source);
        assert!(
            errors
                .iter()
                .any(|message| message.contains("BorrowConflict") || message.contains("borrow")),
            "{source}\n{errors:?}"
        );
    }
}

#[test]
fn reference_bearing_assignment_keeps_its_real_referent_alive() {
    for source in [
        "fn keep(mut x:Int)->Int { let mut saved:(&Int,Int)=(&x,1); saved=(&x,2); x=3; *saved.0 }",
        "type Int64 is { value:&Int }; fn keep(mut x:Int)->Int { let mut saved=Int64 {value:&x}; saved=Int64 {value:&x}; x=2; *saved.value }",
    ] {
        let errors = errors(source);
        assert!(
            errors
                .iter()
                .any(|message| message.contains("BorrowConflict") || message.contains("borrow")),
            "{source}\n{errors:?}"
        );
    }
}
