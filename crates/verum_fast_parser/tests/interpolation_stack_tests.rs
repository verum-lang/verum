use verum_ast::FileId;
use verum_fast_parser::VerumParser;

#[test]
fn interpolation_and_rejection_share_an_ordinary_worker_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let parser = VerumParser::new();
            for leaf in [r#"f"{value as Int:04x}""#, r#"html"<b>{value}</b>""#] {
                let source = format!(
                    "fn render(value: Int) {{ {}{leaf}{}; }}",
                    "render(".repeat(16),
                    ")".repeat(16),
                );
                parser.parse_module_str(&source, FileId::new(0)).unwrap();
                assert!(
                    parser
                        .parse_module_str(&source[..source.len() - 2], FileId::new(0))
                        .is_err()
                );
            }
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core");
            for file in ["base/primitives.vr", "text/text.vr", "mem/arena.vr"] {
                let source = std::fs::read_to_string(root.join(file)).unwrap();
                parser
                    .parse_module_str(&source, FileId::new(0))
                    .expect(file);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn call_nesting_reaches_the_shared_limit_without_native_stack_overflow() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let parser = VerumParser::new();
            for depth in [16, 64, 127] {
                let source = format!("{}value{}", "render(".repeat(depth), ")".repeat(depth));
                parser.parse_expr_str(&source, FileId::new(0)).unwrap();
            }
            for depth in [128, 256] {
                let source = format!("{}value{}", "render(".repeat(depth), ")".repeat(depth));
                let error = parser.parse_expr_str(&source, FileId::new(0)).unwrap_err();
                assert!(
                    error
                        .iter()
                        .any(|diagnostic| diagnostic.to_string().contains("Maximum nesting depth")),
                    "{error:?}"
                );
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
