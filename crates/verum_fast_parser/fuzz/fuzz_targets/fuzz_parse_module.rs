#![no_main]
use libfuzzer_sys::fuzz_target;
use verum_ast::FileId;
use verum_fast_parser::FastParser;
use verum_lexer::Lexer;

fuzz_target!(|data: &[u8]| {
    // Only process valid UTF-8 — the parser operates on &str.
    if let Ok(source) = std::str::from_utf8(data) {
        // Reject extremely large inputs to keep iteration speed high.
        if source.len() > 4096 {
            return;
        }

        let file_id = FileId::new(0);

        // Path 1: parse_module (lexer + parser)
        let lexer = Lexer::new(source, file_id);
        let parser = FastParser::new();
        let _ = parser.parse_module(lexer, file_id);

        // Path 2: parse_module_str (source-level error analysis)
        let _ = parser.parse_module_str(source, file_id);
    }
});
