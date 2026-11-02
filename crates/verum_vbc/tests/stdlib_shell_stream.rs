//! Execute the stdlib's line framing on the wire-decoded VM representation.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_ast::ItemKind;
use verum_fast_parser::Parser;
use verum_vbc::{codegen::VbcCodegen, interpreter::Interpreter};

#[test]
fn stream_strips_one_line_ending_without_changing_utf8_content() {
    let mut source = Parser::new(include_str!("../../../core/shell/stream.vr"))
        .parse_module()
        .unwrap();
    source.items.retain(|item| {
        matches!(&item.kind,
        ItemKind::Function(decl) if decl.name.name == "strip_trailing_newline")
    });
    let probe = Parser::new(
        r#"
        fn main() -> Int {
            if strip_trailing_newline("é\r\n") != "é" { return 1; }
            if strip_trailing_newline("a\n") != "a" { return 2; }
            if strip_trailing_newline("line") != "line" { return 3; }
            if strip_trailing_newline("\n\n") != "\n" { return 4; }
            if strip_trailing_newline("") != "" { return 5; }
            0
        }
    "#,
    )
    .parse_module()
    .unwrap();
    source.items.extend(probe.items);
    let mut producer = VbcCodegen::new();
    producer.register_builtin_variants();
    let module = producer.compile_module(&source).unwrap();
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    let main = module.find_function_by_name("main").unwrap();
    let mut vm = Interpreter::new(Arc::new(module));
    vm.run_global_ctors().unwrap();
    assert_eq!(vm.execute_function(main).unwrap().try_as_i64(), Some(0));
}
