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

        // Wrap as a let-binding so the module parser enters pattern context.
        let as_let = format!("fn main() {{ let {} = x; }}", fragment);
        let _ = parser.parse_module_str(&as_let, file_id);

        // Also try match arms — exercises more pattern variants.
        let as_match = format!("fn main() {{ match x {{ {} => 0 }} }}", fragment);
        let _ = parser.parse_module_str(&as_match, file_id);
    }
});
