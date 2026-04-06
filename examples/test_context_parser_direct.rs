// Direct parser test for context system

fn main() {
    let source = r#"
context Logger {
    fn log(message: Text);
    fn error(message: Text);
}

fn process_data(data: Text) using Logger {
    Logger.log("Processing data");
}

fn main() {
    process_data("test");
}
"#;

    println!("Testing context parsing...\n");
    println!("Source:");
    println!("{}", source);
    println!("\n{}", "=".repeat(60));

    let mut parser = verum_parser::Parser::new(source);
    let result = parser.parse_module();

    match result {
        Ok(module) => {
            println!("\nParsing SUCCESSFUL!");
            println!("\nModule contains {} items:", module.items.len());
            for (i, item) in module.items.iter().enumerate() {
                println!("  {}. {:?}", i + 1, item.kind);
            }
        }
        Err(errors) => {
            println!("\nParsing FAILED with {} error(s):", errors.len());
            for (i, error) in errors.iter().enumerate() {
                println!("\nError {}:", i + 1);
                println!("  {:?}", error);
            }
        }
    }
}
