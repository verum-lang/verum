use verum_ast::{ItemKind, ty::{TypeBoundKind, TypeKind, WherePredicate, WherePredicateKind}};
use verum_fast_parser::Parser;

fn predicate(source: &str) -> WherePredicate {
    let module = Parser::new(source).parse_module().expect("grammar-approved where clause");
    let ItemKind::Function(function) = &module.items[0].kind else { panic!("function") };
    let clause = function.generic_where_clause.as_ref().expect("type where clause retained");
    assert_eq!(clause.predicates.len(), 1);
    clause.predicates[0].clone()
}

#[test]
fn bare_and_explicit_type_equalities_retain_the_same_semantic_shape() {
    for subject in ["T", "T.Item", "Wrapper<T>", "(T, Int)", "&T", "[T]"] {
        for prefix in ["", "type "] {
            let source = format!("fn choose<T>(item: T) -> Int where {prefix}{subject} = Outcome<Int> {{ 0 }}");
            let predicate = predicate(&source);
            let WherePredicateKind::Type { ty, bounds } = predicate.kind else { panic!("type equality became a value predicate") };
            assert_eq!(&source[ty.span.start as usize..ty.span.end as usize], subject);
            let [bound] = bounds.as_slice() else { panic!("one equality") };
            let TypeBoundKind::Equality(target) = &bound.kind else { panic!("equality bound") };
            assert!(matches!(target.kind, TypeKind::Generic { .. }));
            assert_eq!(&source[target.span.start as usize..target.span.end as usize], "Outcome<Int>");
        }
    }
}

#[test]
fn protocol_bounds_and_value_comparisons_remain_distinct() {
    let bound = predicate("fn bounded<T>(item: T) -> Int where T: Family { 0 }");
    assert!(matches!(bound.kind, WherePredicateKind::Type { bounds, .. }
        if matches!(bounds.as_slice(), [bound] if matches!(bound.kind, TypeBoundKind::Protocol(_)))));
    for comparison in ["amount > 0", "amount == 0"] {
        let source = format!("fn checked_value(amount: Int) -> Int where {comparison} {{ amount }}");
        let module = Parser::new(&source).parse_module().unwrap();
        let ItemKind::Function(function) = &module.items[0].kind else { panic!("function") };
        let clause = function.generic_where_clause.as_ref().expect("where predicate retained");
        assert!(matches!(clause.predicates.as_slice(), [predicate] if matches!(predicate.kind, WherePredicateKind::Value { .. })));
    }
}

#[test]
fn callable_capabilities_and_exact_function_equalities_have_distinct_carriers() {
    use verum_ast::ty::GenericParamKind;
    for signature in ["fn(Int) -> Int", "fn<A>(A) -> A", "async fn(Int) -> Int", "async fn<A>(A) -> A using [Clock]"] {
        let source = format!("fn directional<F: {signature}>(callback: F) -> Int {{ 0 }}");
        let module = Parser::new(&source).parse_module().unwrap();
        let ItemKind::Function(function) = &module.items[0].kind else { panic!("function") };
        let GenericParamKind::Type { bounds, .. } = &function.generics[0].kind else { panic!("type parameter") };
        assert!(matches!(bounds.as_slice(), [bound] if matches!(bound.kind, TypeBoundKind::Callable(_))));
        for prefix in ["", "type "] {
            for (operator, exact) in [(":", false), ("=", true)] {
                let source = format!("fn chosen<F>(callback: F) -> Int where {prefix}F {operator} {signature} {{ 0 }}");
                let parsed = predicate(&source);
                let WherePredicateKind::Type { bounds, .. } = parsed.kind else { panic!("type predicate") };
                assert!(matches!(bounds.as_slice(), [bound] if if exact {
                    matches!(bound.kind, TypeBoundKind::Equality(_))
                } else { matches!(bound.kind, TypeBoundKind::Callable(_)) }));
                let module = Parser::new(&source).parse_module().unwrap();
                let formatted = verum_ast::pretty::format_item(&module.items[0]);
                let reparsed = predicate(&formatted);
                let WherePredicateKind::Type { bounds: recovered, .. } = reparsed.kind else { panic!("type predicate") };
                assert_eq!(std::mem::discriminant(&bounds[0].kind), std::mem::discriminant(&recovered[0].kind));
            }
        }
    }
}

#[test]
fn explicit_callable_type_arguments_keep_async_rank2_and_context_syntax() {
    use verum_ast::{ExprKind, FunctionBody, GenericArg};
    for signature in ["fn(Int) -> Int", "async fn(Int) -> Int", "async fn<A>(A) -> A using [Clock]"] {
        let source = format!("fn run() {{ invoke<{signature}>(callback) }}");
        let module = Parser::new(&source).parse_module().unwrap();
        let ItemKind::Function(function) = &module.items[0].kind else { panic!("function") };
        let Some(FunctionBody::Block(body)) = &function.body else { panic!("body") };
        let ExprKind::Call { type_args, .. } = &body.expr.as_ref().unwrap().kind else { panic!("explicit call") };
        let [GenericArg::Type(argument)] = type_args.as_slice() else { panic!("one type argument") };
        assert_eq!(&source[argument.span.start as usize..argument.span.end as usize], signature);
        assert!(matches!(argument.kind, TypeKind::Function { .. } | TypeKind::Rank2Function { .. }));
    }
}
