#![allow(dead_code, unused_imports)]
use verum_ast::FileId;
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

fn parse_file(path: &str) -> Result<(), String> {
    let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(&src, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).map(|_| ()).map_err(|errs| {
        errs.into_iter().map(|e| format!("{:?}", e)).collect::<Vec<_>>().join("\n")
    })
}

#[test]
fn sqlite_l6_connection() {
    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let p = format!(
        "{}/../../core/database/sqlite/native/l6_session/connection.vr",
        root
    );
    let r = parse_file(&p);
    if let Err(e) = &r {
        eprintln!("{}", e);
    }
    assert!(r.is_ok());
}
