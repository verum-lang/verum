//! Diagnostic source fixture for T1551; not a passing lifecycle gate.
//! Run main against one coherent verum_types test dependency graph.
use verum_ast::{FileId, ItemKind, Module};
use verum_parser::{syntax_to_ast, EventBasedParser, Parser};
fn describe(route: &str, module: &Module) {
    for item in &module.items {
        if let ItemKind::Type(decl) = &item.kind {
            println!("{route}: {} {:?}", decl.name.name, decl.resource_modifier);
        }
    }
}
fn parser_controls() {
    for source in [
        "type Plain is { id: Int };",
        "type affine Affine is { id: Int };",
        "type linear Linear is { id: Int };",
    ] {
        println!("source: {source}");
        let mut parser = Parser::new(source);
        let fast = parser.parse_module().expect("valid grammar control");
        describe("semantic", &fast);
        let parsed = EventBasedParser::new().parse(source, FileId::new(0));
        println!("event_parse_errors: {:?}", parsed.errors);
        let converted = syntax_to_ast(source, &parsed.syntax(), FileId::new(0));
        println!("sink_errors: {:?}", converted.errors);
        describe("event_sink", &converted.module);
    }
}

use verum_types::infer::TypeChecker;
fn parse(s: &str) -> Module {
    Parser::new(s)
        .parse_module()
        .expect("grammar-valid control")
}
fn register(checker: &mut TypeChecker, module: &Module, owner: &str) {
    checker.set_current_module_path(owner);
    for item in &module.items {
        if let ItemKind::Type(decl) = &item.kind {
            checker
                .register_type_declaration(decl)
                .expect("register source declaration");
        }
    }
}
fn probe(ordinary_first: bool, which: &str, borrowed: bool, include_alpha: bool) {
    let mut checker = TypeChecker::new();
    let alpha = parse("public type affine Token is { id: Int };");
    let beta = parse("public type Token is { pad: Int, id: Int };");
    if !include_alpha {
        register(&mut checker, &beta, "beta");
    } else if ordinary_first {
        register(&mut checker, &beta, "beta");
        register(&mut checker, &alpha, "alpha");
    } else {
        register(&mut checker, &alpha, "alpha");
        register(&mut checker, &beta, "beta");
    }
    checker.set_current_module_path("consumer");
    let source = if borrowed {
        format!("fn observe(x: &{which}.Token) {{}} fn consume(x: {which}.Token) {{}} fn probe(x: {which}.Token) {{ observe(&x); observe(&x); consume(x); }}")
    } else {
        format!("fn consume(x: {which}.Token) {{}} fn probe(x: {which}.Token) {{ consume(x); consume(x); }}")
    };
    let module = parse(&source);
    let mut errors = Vec::new();
    for item in &module.items {
        if let ItemKind::Function(decl) = &item.kind {
            if let Err(e) = checker.register_function_signature(decl) {
                errors.push(format!("register:{e:?}"));
            }
        }
    }
    for item in &module.items {
        if let Err(e) = checker.check_item(item) {
            errors.push(format!("check:{e:?}"));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| format!("deferred:{e:?}")),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| format!("diag:{e:?}")),
    );
    println!("include_alpha={include_alpha} ordinary_first={ordinary_first} type={which}.Token borrowed={borrowed} errors={errors:?}");
}
fn checker_controls() {
    probe(false, "beta", false, false);
    for order in [false, true] {
        probe(order, "alpha", false, true);
        probe(order, "beta", false, true);
        probe(order, "alpha", true, true);
    }
}

fn main() {
    parser_controls();
    checker_controls();
}
