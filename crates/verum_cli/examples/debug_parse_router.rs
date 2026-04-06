use verum_ast::FileId;
use verum_lexer::Lexer;
use verum_parser::VerumParser;

fn main() {
    let source = std::fs::read_to_string("registry/verum-registry/src/server/router.vr")
        .expect("Failed to read file");

    let file_id = FileId::new(1);
    let lexer = Lexer::new(&source, file_id);
    let parser = VerumParser::new();

    match parser.parse_module(lexer, file_id) {
        Ok(_module) => {
            println!("✓ Parsing succeeded!");
        }
        Err(errors) => {
            println!("✗ Parsing failed with {} errors:\n", errors.len());
            for (i, error) in errors.iter().enumerate() {
                println!("Error {}: {:?}", i + 1, error);
            }
        }
    }
}
