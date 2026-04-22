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

fn check(rel: &str) {
    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let p = format!("{}/../../{}", root, rel);
    let r = parse_file(&p);
    if let Err(e) = &r {
        eprintln!("=== {} ===\n{}", rel, e);
    }
    assert!(r.is_ok(), "file must parse: {}", rel);
}

#[test] fn bbr() { check("core/net/quic/recovery/cc/bbr.vr"); }
#[test] fn cubic() { check("core/net/quic/recovery/cc/cubic.vr"); }
#[test] fn new_reno() { check("core/net/quic/recovery/cc/new_reno.vr"); }
#[test] fn early_data() { check("core/net/tls13/handshake/early_data.vr"); }
#[test] fn coalesce() { check("core/net/quic/connection_sm/coalesce.vr"); }
#[test] fn packet() { check("core/net/quic/packet.vr"); }
