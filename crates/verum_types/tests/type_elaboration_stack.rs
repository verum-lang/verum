//! Recursive conversion must not reserve the workspace of every type rule.
use verum_ast::{Span, Type as AstType, TypeKind};
use verum_types::{TypeChecker, TypeError, ty::Type};

#[test]
fn nested_generic_elaboration_uses_a_normal_thread_stack() {
    std::thread::Builder::new().stack_size(2 * 1024 * 1024).spawn(|| {
        use verum_ast::{FileId, GenericArg, Ident, Path};
        use verum_common::List;
        let module = verum_fast_parser::FastParser::new().parse_module_str(
            "type Envelope<T> is { value: T };", FileId::new(0),
        ).unwrap();
        let mut checker = TypeChecker::new();
        checker.register_primitives();
        assert!(checker.prepare_module_declarations(&module).is_empty());
        for item in &module.items { checker.check_item(item).unwrap(); }
        for depth in [12, 60, 64, 12] {
            let mut source = AstType::new(TypeKind::Int, Span::dummy());
            for _ in 0..depth {
                source = AstType::new(TypeKind::Generic {
                    base: Box::new(AstType::new(TypeKind::Path(Path::from_ident(
                        Ident::new("Envelope", Span::dummy()),
                    )), Span::dummy())),
                    args: List::from_iter([GenericArg::Type(source)]),
                }, Span::dummy());
            }
            let result = checker.ast_to_type(&source);
            if depth == 64 {
                assert!(matches!(result, Err(TypeError::RecursionLimit(_))), "{result:?}");
                continue;
            }
            let mut result = result.unwrap();
            for _ in 0..depth {
                let Type::Named { path, mut args } = result else {
                    panic!("the declared generic structure must survive elaboration");
                };
                assert!(matches!(path.segments.last(), Some(verum_ast::ty::PathSegment::Name(name))
                    if name.name.as_str() == "Envelope"));
                assert_eq!(args.len(), 1);
                result = args.remove(0);
            }
            assert_eq!(result, Type::Int);
        }
    }).unwrap().join().unwrap();
}

#[test]
fn nested_reference_elaboration_uses_a_normal_thread_stack() {
    const CHILD: &str = "VERUM_TYPE_ELABORATION_STACK_CHILD";
    if std::env::var_os(CHILD).is_some() {
        std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(|| {
                let mut checker = TypeChecker::new();
                checker.register_primitives();
                // A limit refusal must also restore the recursion budget.
                for depth in [12, 63, 64, 12] {
                    let mut source = AstType::new(TypeKind::Int, Span::dummy());
                    for _ in 0..depth {
                        source = AstType::new(
                            TypeKind::Reference {
                                mutable: false,
                                inner: Box::new(source),
                            },
                            Span::dummy(),
                        );
                    }
                    let result = checker.ast_to_type(&source);
                    if depth == 64 {
                        assert!(
                            matches!(result, Err(TypeError::RecursionLimit(_))),
                            "{result:?}"
                        );
                        continue;
                    }
                    let mut result = result.unwrap();
                    for _ in 0..depth {
                        let Type::Reference {
                            mutable: false,
                            inner,
                        } = result
                        else {
                            panic!("the source reference structure must survive elaboration");
                        };
                        result = *inner;
                    }
                    assert_eq!(result, Type::Int);
                }
            })
            .unwrap()
            .join()
            .unwrap();
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "nested_reference_elaboration_uses_a_normal_thread_stack",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
