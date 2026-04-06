#![no_main]
use libfuzzer_sys::fuzz_target;
use verum_ast::FileId;
use verum_lexer::Lexer;

fuzz_target!(|data: &[u8]| {
    if let Ok(source) = std::str::from_utf8(data) {
        if source.len() > 4096 {
            return;
        }

        let file_id = FileId::new(0);
        let lexer = Lexer::new(source, file_id);

        // Drain every token. The lexer must never panic, only return Ok/Err.
        for result in lexer {
            let _ = result;
        }
    }
});
