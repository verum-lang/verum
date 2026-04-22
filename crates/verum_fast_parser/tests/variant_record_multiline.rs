#![allow(dead_code, unused_imports, unused_variables)]
use verum_ast::FileId;
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

fn parse(source: &str) -> Result<(), String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).map(|_| ()).map_err(|e| {
        e.into_iter().map(|err| format!("{:?}", err)).collect::<Vec<_>>().join("\n")
    })
}

#[test]
fn multiline_variant_record_with_generic() {
    let src = r#"
public type CoalescedPacket is
      InitialPkt   { frames: List<Int>, pn: UInt64, pn_len: UInt8,
                     largest_acked: Maybe<UInt64>, token: List<Byte>,
                     keys: Int }
    | HandshakePkt { frames: List<Int>, pn: UInt64, pn_len: UInt8,
                     largest_acked: Maybe<UInt64>, keys: Int }
    | OneRttPkt    { frames: List<Int>, pn: UInt64, pn_len: UInt8,
                     largest_acked: Maybe<UInt64>, keys: Int,
                     spin_bit: Bool, key_phase: Bool };
    "#;
    let r = parse(src);
    if let Err(e) = &r { eprintln!("{}", e); }
    assert!(r.is_ok());
}
