use verum_ast::FileId;
use verum_lexer::Lexer;
use verum_parser::VerumParser;

fn main() {
    let source = r#"
type Router is {
    routes: List<Int>,
};

implement Router {
    pub fn find(self) -> Int {
        for route in self.routes {
            print(route);
        }
        return 1;
    }
}
"#;

    let file_id = FileId::new(1);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();

    match parser.parse_module(lexer, file_id) {
        Ok(module) => {
            println!("✓ Parsing succeeded!");
            println!("{:#?}", module);
        }
        Err(errors) => {
            println!("✗ Parsing failed with {} errors:\n", errors.len());
            for (i, error) in errors.iter().enumerate() {
                println!("Error {}: {:?}", i + 1, error);
            }
        }
    }
}
