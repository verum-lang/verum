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
use std::fs;
use std::path::Path;
use verum_ast::FileId;
use verum_lexer::Lexer;
use verum_fast_parser::RecursiveParser;

fn parse_file(path: &Path) -> Result<(), String> {
    let source = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(&source, file_id);
    let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
    let mut parser = RecursiveParser::new(&tokens[..], file_id);

    // Parse all items in the file using parse_module
    let module_result = parser.parse_module();

    if let Err(e) = module_result {
        return Err(format!("Parse error in {}: {:?}", path.display(), e));
    }

    if !parser.errors.is_empty() {
        let errors: Vec<String> = parser.errors.iter().map(|e| format!("{:?}", e)).collect();
        return Err(format!(
            "Parser errors in {}:\n{}",
            path.display(),
            errors.join("\n")
        ));
    }

    Ok(())
}

#[test]
fn test_parse_fibonacci() {
    let path = Path::new("../../examples/fibonacci.vr");
    if path.exists() {
        parse_file(path).expect("Failed to parse fibonacci.vr");
    } else {
        eprintln!("Skipping fibonacci.vr - file not found at {:?}", path);
    }
}

#[test]
fn test_parse_cbgr_demo() {
    let path = Path::new("../../examples/cbgr_demo.vr");
    if path.exists() {
        parse_file(path).expect("Failed to parse cbgr_demo.vr");
    } else {
        eprintln!("Skipping cbgr_demo.vr - file not found at {:?}", path);
    }
}
