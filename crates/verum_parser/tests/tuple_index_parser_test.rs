#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
use verum_ast::{ExprKind, FileId, ItemKind};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

#[test]
fn test_tuple_index_parsing() {
    let source = r#"
fn main() {
    let pair = (10.0, 20.0);
    let x = pair.0;
    let y = pair.1;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    match result {
        Ok(module) => {
            println!("Parsed successfully!");
            println!("Module has {} items", module.items.len());

            // Find the main function
            let main_fn = module
                .items
                .iter()
                .find_map(|item| {
                    if let ItemKind::Function(f) = &item.kind {
                        if f.name.name.as_str() == "main" {
                            Some(f)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .expect("Should have main function");

            println!("Main function body: {:#?}", main_fn.body);

            // Check that we have TupleIndex expressions
            if let Some(verum_ast::decl::FunctionBody::Block(block)) = &main_fn.body {
                let has_tuple_index = find_tuple_index(&block.stmts);
                assert!(has_tuple_index, "Should have TupleIndex expression");
            } else {
                panic!("Expected block body for main function");
            }
        }
        Err(errors) => {
            println!("Parse errors:");
            for err in errors.iter() {
                println!("  {}", err);
            }
            panic!("Parsing failed with {} errors", errors.len());
        }
    }
}

fn find_tuple_index(stmts: &[verum_ast::Stmt]) -> bool {
    for stmt in stmts {
        if let verum_ast::StmtKind::Let {
            value: Some(expr), ..
        } = &stmt.kind
            && matches!(expr.kind, ExprKind::TupleIndex { .. }) {
                println!("Found TupleIndex expression!");
                return true;
            }
    }
    false
}
