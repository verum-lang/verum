#![no_main]
use libfuzzer_sys::fuzz_target;
use verum_ast::FileId;
use verum_fast_parser::FastParser;

fuzz_target!(|data: &[u8]| {
    if let Ok(fragment) = std::str::from_utf8(data) {
        if fragment.len() > 2048 {
            return;
        }

        let file_id = FileId::new(0);
        let parser = FastParser::new();

        // Wrap in a function body so the module parser reaches expression context.
        let source = format!("fn main() {{ {} }}", fragment);
        let _ = parser.parse_module_str(&source, file_id);

        // Also exercise the dedicated expression parser.
        let _ = parser.parse_expr_str(fragment, file_id);
    }
});
