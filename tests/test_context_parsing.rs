// Test context system parsing

use verum_parser::Parser;

#[test]
fn test_basic_context_parsing() {
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

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    match &result {
        Ok(module) => {
            println!("Parsed successfully!");
            println!("Module items: {}", module.items.len());
            for item in &module.items {
                println!("  Item: {:?}", item.kind);
            }
        }
        Err(errors) => {
            println!("Parse errors:");
            for error in errors {
                println!("  Error: {:?}", error);
            }
        }
    }

    assert!(result.is_ok(), "Should parse successfully");
}

#[test]
fn test_multiple_contexts() {
    let source = r#"
context Database {
    fn query(sql: Text) -> Text;
}

context Logger {
    fn log(message: Text);
}

fn operation() using [Database, Logger] {
    Logger.log("test");
}
"#;

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    match &result {
        Err(errors) => {
            println!("Parse errors for multiple contexts:");
            for error in errors {
                println!("  Error: {:?}", error);
            }
        }
        Ok(_) => println!("Multiple contexts parsed successfully!"),
    }

    assert!(result.is_ok(), "Should parse multiple contexts");
}

#[test]
fn test_context_group() {
    let source = r#"
context Database {
    fn query(sql: Text) -> Text;
}

context Logger {
    fn log(message: Text);
}

using WebContext = [Database, Logger];

fn handle_request() using WebContext {
    // test
}
"#;

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    match &result {
        Err(errors) => {
            println!("Parse errors for context group:");
            for error in errors {
                println!("  Error: {:?}", error);
            }
        }
        Ok(_) => println!("Context group parsed successfully!"),
    }

    assert!(result.is_ok(), "Should parse context group");
}

#[test]
fn test_provide_statement() {
    let source = r#"
context Logger {
    fn log(message: Text);
}

type ConsoleLogger = {
    prefix: Text
}

fn main() {
    provide Logger = ConsoleLogger { prefix: "APP" };
}
"#;

    let mut parser = Parser::new(source);
    let result = parser.parse_module();

    match &result {
        Err(errors) => {
            println!("Parse errors for provide statement:");
            for error in errors {
                println!("  Error: {:?}", error);
            }
        }
        Ok(_) => println!("Provide statement parsed successfully!"),
    }

    assert!(result.is_ok(), "Should parse provide statement");
}
