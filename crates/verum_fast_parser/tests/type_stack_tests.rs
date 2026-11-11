use verum_ast::{FileId, TypeKind, ty::GenericArg};
use verum_fast_parser::VerumParser;

#[test]
fn nested_types_parse_on_an_ordinary_worker_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let parser = VerumParser::new();
            // 127 generic nodes plus the leaf reach the shared depth budget.
            let depth = 127;
            let input = format!("{}Int{}", "Envelope<".repeat(depth), ">".repeat(depth));
            let mut ty = parser.parse_type_str(&input, FileId::new(0)).unwrap();
            for _ in 0..depth {
                let TypeKind::Generic { mut args, .. } = ty.kind else {
                    panic!("generic layer lost")
                };
                assert_eq!(args.len(), 1);
                let GenericArg::Type(inner) = args.pop().unwrap() else {
                    panic!("type argument lost")
                };
                ty = inner;
            }
            assert!(matches!(ty.kind, TypeKind::Int));
            assert!(
                parser
                    .parse_type_str(&input[..input.len() - 1], FileId::new(0))
                    .is_err()
            );
            for depth in [128, 256] {
                let excessive = format!("{}Int{}", "Envelope<".repeat(depth), ">".repeat(depth));
                assert!(parser.parse_type_str(&excessive, FileId::new(0)).is_err());
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
