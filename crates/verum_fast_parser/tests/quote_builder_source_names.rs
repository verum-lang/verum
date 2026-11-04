//! T2282: a constructor and a consuming setter have distinct declaration names.
use verum_ast::{Expr, ExprKind, ItemKind};
use verum_ast::decl::{FunctionParamKind, ImplDecl, ImplItemKind};
use verum_ast::source_implementation::AssociatedImplementationSource;
use verum_ast::visitor::{Visitor, walk_expr};
use verum_fast_parser::Parser;

const QUOTE: &str = include_str!("../../../core/meta/quote.vr");
const CALLERS: &str = include_str!("../../../core-tests/meta/quote/unit_test.vr");

fn quote_impl(source: &str) -> ImplDecl {
    Parser::new(source).parse_module().expect("quote grammar").items.into_iter()
        .find_map(|item| match item.kind { ItemKind::Impl(declaration) => Some(declaration), _ => None })
        .expect("QuoteBuilder implementation")
}

#[test]
fn actual_quote_source_retains_both_operations_with_distinct_roles() {
    let declaration = quote_impl(QUOTE);
    let source = AssociatedImplementationSource::new("core.meta.quote".into(), &declaration)
        .expect("actual source carrier");
    assert_eq!(source.origin_module(), "core.meta.quote");
    let retained = source.declaration().expect("validated source reader");
    assert_eq!(retained.items.len(), declaration.items.len());
    let functions: std::collections::BTreeMap<_, _> = retained.items.iter().filter_map(|item| {
        if let ImplItemKind::Function(function) = &item.kind { Some((function.name.name.as_str(), function)) } else { None }
    }).collect();
    let constructor = functions.get("new_spanned").expect("named constructor");
    assert_eq!(constructor.params.len(), 1);
    assert!(matches!(constructor.params[0].kind, FunctionParamKind::Regular { .. }));
    let setter = functions.get("with_span").expect("consuming setter");
    assert_eq!(setter.params.len(), 2);
    assert!(matches!(setter.params[0].kind, FunctionParamKind::SelfValueMut));
    assert!(functions.values().all(|function| function.body.is_none()));
}

#[test]
fn restoring_the_original_collision_refuses_the_actual_source_carrier() {
    assert!(QUOTE.contains("public fn new_spanned(span: MetaSpan)"));
    let original = QUOTE.replacen("public fn new_spanned(span: MetaSpan)", "public fn with_span(span: MetaSpan)", 1);
    let error = AssociatedImplementationSource::new("core.meta.quote".into(), &quote_impl(&original)).unwrap_err();
    assert!(error.contains("repeated declaration"), "{error}");
}

#[test]
fn receiver_and_arity_do_not_disambiguate_duplicate_member_names() {
    let source = "type Builder is {value:Int}; implement Builder {fn span(value:Int)->Self {Self {value}} fn span(mut self,value:Int)->Self {self.value=value; self}}";
    let error = AssociatedImplementationSource::new("fixture".into(), &quote_impl(source)).unwrap_err();
    assert!(error.contains("repeated declaration"), "{error}");
}

#[test]
fn identical_names_in_distinct_implementations_remain_valid() {
    let source = "type First is {value:Int}; type Second is {value:Int}; implement First {fn new(value:Int)->Self {Self {value}}} implement Second {fn new(value:Int)->Self {Self {value}}}";
    let module = Parser::new(source).parse_module().unwrap();
    let mut implementations = 0;
    for item in &module.items {
        if let ItemKind::Impl(declaration) = &item.kind {
            AssociatedImplementationSource::new("fixture".into(), declaration).unwrap();
            implementations += 1;
        }
    }
    assert_eq!(implementations, 2);
}

#[test]
fn actual_constructor_caller_migrates_and_the_consuming_setter_stays() {
    #[derive(Default)] struct Calls { constructors: usize, setters: usize, obsolete: usize }
    impl Visitor for Calls {
        fn visit_expr(&mut self, expression: &Expr) {
            if let ExprKind::MethodCall { receiver, method, .. } = &expression.kind {
                let is_type = matches!(&receiver.kind, ExprKind::Path(path) if path.as_ident().is_some_and(|id| id.name.as_str() == "QuoteBuilder"));
                match (is_type, method.name.as_str()) {
                    (true, "new_spanned") => self.constructors += 1,
                    (true, "with_span") => self.obsolete += 1,
                    (false, "with_span") => self.setters += 1,
                    _ => {}
                }
            }
            walk_expr(self, expression);
        }
    }
    let module = Parser::new(CALLERS).parse_module().expect("migrated real caller grammar");
    let mut calls = Calls::default();
    for item in &module.items { calls.visit_item(item); }
    assert_eq!((calls.constructors, calls.setters, calls.obsolete), (1, 1, 0));
}
