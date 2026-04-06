use verum_ast::FileId;
use verum_lexer::Lexer;
use verum_parser::VerumParser;

fn main() {
    let path = std::env::args().nth(1).expect("Please provide a file path");
    let source =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("Failed to read {}: {}", path, e));

    let file_id = FileId::new(1);
    let lexer = Lexer::new(&source, file_id);
    let parser = VerumParser::new();

    match parser.parse_module(lexer, file_id) {
        Ok(_module) => {
            println!("Parsing succeeded!");
        }
        Err(errors) => {
            println!("Parsing failed with {} errors:\n", errors.len());
            for (i, error) in errors.iter().enumerate() {
                println!("Error {}: {:?}", i + 1, error);
            }
        }
    }
}
