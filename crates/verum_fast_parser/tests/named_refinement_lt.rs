#![allow(dead_code, unused_imports)]
use verum_ast::FileId;
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

fn parse(source: &str) -> Result<(), String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).map(|_| ()).map_err(|errs| {
        errs.into_iter().map(|e| format!("{:?}", e)).collect::<Vec<_>>().join("\n")
    })
}

#[test] fn named_refinement_lt() {
    let src = "type NegativeInt is Int { x: x < 0 };";
    let r = parse(src);
    if let Err(e) = &r { eprintln!("{}", e); }
    assert!(r.is_ok());
}

#[test] fn named_refinement_gt() {
    assert!(parse("type PositiveInt is Int { x: x > 0 };").is_ok());
}

#[test] fn named_refinement_eq() {
    assert!(parse("type ZeroInt is Int { x: x == 0 };").is_ok());
}

#[test] fn named_refinement_dot_method() {
    // Named refinement with method call, e.g. `s: s.is_valid()`.
    assert!(parse(r#"type ValidEmail is Text { s: s.contains("@") };"#).is_ok());
}

#[test] fn record_variant_still_works_with_generic() {
    let src = r#"
        type Coal is
              A { pn: Maybe<Int> }
            | B { k: Int };
    "#;
    assert!(parse(src).is_ok());
}

#[test] fn record_variant_qualified_type() {
    let src = r#"
        type T is
              A { v: module.Inner }
            | B;
    "#;
    assert!(parse(src).is_ok());
}
