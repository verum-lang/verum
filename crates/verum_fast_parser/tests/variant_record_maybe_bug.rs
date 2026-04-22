#![allow(dead_code, unused_imports, unused_variables, unused_mut)]
use verum_ast::{FileId, ItemKind, Module, TypeDeclBody};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

fn parse_module(source: &str) -> Result<Module, String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| format!("{:?}", e))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

#[test]
fn repro_1_maybe_in_first_variant_record_no_leading_pipe() {
    // FAILS
    let src = r#"
        type Coal is
              A { pn: Maybe<Int> }
            | B { k: Int };
    "#;
    let r = parse_module(src);
    eprintln!("repro_1 result: {:?}", r.is_ok());
    if let Err(e) = &r { eprintln!("{}", e); }
    assert!(r.is_ok(), "repro_1 should parse");
}

#[test]
fn repro_2_int_in_first_variant_record_no_leading_pipe() {
    // PASSES
    let src = r#"
        type Coal is
              A { pn: Int }
            | B { k: Int };
    "#;
    assert!(parse_module(src).is_ok());
}

#[test]
fn repro_3_leading_pipe_with_maybe() {
    // PASSES
    let src = r#"
        type Coal is
            | A { pn: Maybe<Int> }
            | B { k: Int };
    "#;
    assert!(parse_module(src).is_ok());
}

#[test]
fn repro_4_single_variant_maybe() {
    // PASSES
    let src = r#"
        type Coal is A { pn: Maybe<Int> };
    "#;
    assert!(parse_module(src).is_ok());
}

#[test]
fn repro_5_second_variant_has_maybe() {
    // PASSES
    let src = r#"
        type Coal is
              A { pn: Int }
            | B { k: Maybe<Int> };
    "#;
    assert!(parse_module(src).is_ok());
}

#[test]
fn repro_6_generic_in_tuple_variant_no_leading_pipe() {
    let src = r#"
        type Coal is A(Maybe<Int>) | B(Int);
    "#;
    let r = parse_module(src);
    eprintln!("repro_6 result: {:?}", r.is_ok());
    if let Err(e) = &r { eprintln!("{}", e); }
    assert!(r.is_ok());
}

#[test]
fn repro_7_maybe_two_generic_args_in_record() {
    let src = r#"
        type Coal is
              A { pn: Result<Int, Text> }
            | B { k: Int };
    "#;
    let r = parse_module(src);
    eprintln!("repro_7 result: {:?}", r.is_ok());
    if let Err(e) = &r { eprintln!("{}", e); }
    assert!(r.is_ok());
}
