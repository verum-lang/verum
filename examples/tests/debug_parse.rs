use verum_ast::FileId;
use verum_lexer::Lexer;
use verum_parser::VerumParser;

fn main() {
    let source = std::fs::read_to_string("/Users/taaliman/projects/luxquant/axiom/examples/tests/context_system_test.vr")
        .expect("Failed to read file");

    let file_id = FileId::new(0);
    let lexer = Lexer::new(&source, file_id);
    let parser = VerumParser::new();

    match parser.parse_module(lexer, file_id) {
        Ok(module) => {
            println!("Parsed successfully!");
            println!("{:#?}", module);
        }
        Err(errors) => {
            println!("Parse errors:");
            for error in errors {
                println!("  {}", error);
            }
        }
    }
}
