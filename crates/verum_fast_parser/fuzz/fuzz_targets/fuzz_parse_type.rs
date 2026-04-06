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

        // Wrap as a type alias so the module parser enters type context.
        let source = format!("type T is {};", fragment);
        let _ = parser.parse_module_str(&source, file_id);

        // Also exercise the dedicated type parser.
        let _ = parser.parse_type_str(fragment, file_id);
    }
});
