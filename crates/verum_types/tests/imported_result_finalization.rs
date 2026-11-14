//! Source-owned result inference across imports and independent instantiations.
use verum_ast::visitor::{Visitor, walk_expr};
use verum_ast::{Expr, ExprKind, FileId};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
use verum_types::TypeChecker;

fn check_imported_result(result: &str) {
    let mut registry = ModuleRegistry::new();
    for (owner,source) in [
        ("owner",format!("public type Cell is {{public pointer:&Int,public value:Int}}; public fn update(cell:&mut Cell,key:&Int){result} {{cell.value=73;}}")),
        ("decoy","public type Cell is {public pointer:&Int,public value:Int}; public fn update(cell:&mut Cell,key:&Int)->Unit {cell.pointer=key;}".into()),
        ("consumer","mount decoy.*; mount owner.{self as service,Cell as Carrier,update as selected}; fn preserve(mut cell:Carrier)->Carrier {let local=4;selected(&mut cell,&local);cell}".into()),
    ] {
        let id=registry.allocate_id();
        let file=FileId::new(id.as_u32());
        let ast=FastParser::new().parse_module_str(&source,file).unwrap();
        let path=ModulePath::from_str(owner);
        let exports=extract_exports_from_module(&ast,id,&path).unwrap();
        let mut info=ModuleInfo::new(id,path,ast,file,source.into());
        info.exports=exports;
        registry.register(info);
    }
    let module = registry.get_by_path("consumer").unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry);
    let mut found = checker.prepare_module_declarations(&module.ast);
    for item in &module.ast.items {
        if let Err(error) = checker.check_item(item) {
            found.push(error);
        }
    }
    found.extend(checker.take_deferred_errors());
    struct Calls<'a>(&'a TypeChecker);
    impl Visitor for Calls<'_> {
        fn visit_expr(&mut self, expr: &Expr) {
            if matches!(
                &expr.kind,
                ExprKind::Call { .. } | ExprKind::MethodCall { .. }
            ) {
                eprintln!(
                    "SELECTED_SIGNATURE {:?}: {:?}",
                    expr.span,
                    self.0.type_registry.lookup_call_target(expr.span)
                );
            }
            walk_expr(self, expr);
        }
    }
    let mut calls = Calls(&checker);
    for item in &module.ast.items {
        calls.visit_item(item);
    }
    let found: List<Text> = found
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|error| error.to_string().into())
        .collect();
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn unannotated_selected_source_result_must_be_finalized() {
    check_imported_result("");
}

#[test]
fn explicit_unit_selected_source_result_is_accepted() {
    check_imported_result("->Unit");
}

fn source_errors(owner: &str, consumer: &str) -> List<Text> {
    let mut registry = ModuleRegistry::new();
    for (name, source) in [("owner", owner), ("consumer", consumer)] {
        let id = registry.allocate_id();
        let file = FileId::new(id.as_u32());
        let ast = FastParser::new().parse_module_str(source, file).unwrap();
        let path = ModulePath::from_str(name);
        let exports = extract_exports_from_module(&ast, id, &path).unwrap();
        let mut info = ModuleInfo::new(id, path, ast, file, source.into());
        info.exports = exports;
        registry.register(info);
    }
    let module = registry.get_by_path("consumer").unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry);
    let mut found = checker.prepare_module_declarations(&module.ast);
    for item in &module.ast.items {
        if let Err(error) = checker.check_item(item) {
            found.push(error);
        }
    }
    found.extend(checker.take_deferred_errors());
    found
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|error| error.to_string().into())
        .collect()
}

#[test]
fn an_imported_result_cannot_be_chosen_by_the_consumer() {
    let found = source_errors(
        "public fn identity(value:Int) { value }",
        "mount owner.identity; fn rejected()->Bool { identity(73) }",
    );
    assert!(
        found
            .iter()
            .any(|error| error.contains("expected 'Bool'") && error.contains("Int")),
        "{found:?}"
    );
}

#[test]
fn generic_parameter_results_are_instantiated_independently() {
    let found = source_errors(
        "public fn identity<T>(value:T) { value }",
        "mount owner.identity as selected; fn number()->Int { selected(73) } fn flag()->Bool { selected(true) }",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn generic_projected_results_use_the_declaring_record() {
    let found = source_errors(
        "public type Cell<T> is {public value:T}; public fn select<T>(cell:&Cell<T>) {cell.value}",
        "mount owner.{Cell as Carrier,select}; type Cell is {value:Bool}; fn number(cell:&Carrier<Int>)->Int {select(cell)}",
    );
    assert!(found.is_empty(), "{found:?}");
    let rejected = source_errors(
        "public type Cell<T> is {public value:T}; public fn select<T>(cell:&Cell<T>) {cell.value}",
        "mount owner.{Cell as Carrier,select}; type Cell is {value:Bool}; fn rejected(cell:&Carrier<Int>)->Bool {select(cell)}",
    );
    assert!(
        rejected
            .iter()
            .any(|error| error.contains("expected 'Bool'") && error.contains("Int")),
        "{rejected:?}"
    );
}

#[test]
fn explicit_return_and_tuple_results_share_the_structural_derivation() {
    let found = source_errors(
        "public fn pair(value:Int) {let selected=value; return (selected,true);}",
        "mount owner.pair; fn accepted()->(Int,Bool) {pair(73)}",
    );
    assert!(found.is_empty(), "{found:?}");
    let rejected = source_errors(
        "public fn pair(value:Int) {let selected=value; return (selected,true);}",
        "mount owner.pair; fn rejected()->(Bool,Int) {pair(73)}",
    );
    assert!(
        !rejected.is_empty(),
        "tuple components must not be inferred from the caller"
    );
}

#[test]
fn inferred_unit_does_not_hide_a_reference_field_write() {
    let found = source_errors(
        "public type Cell is {public pointer:&Int,public value:Int}; public fn replace(cell:&mut Cell,source:&Int) {cell.pointer=source;}",
        "mount owner.{Cell,replace}; fn rejected(input:&Int)->Cell {let mut cell=Cell{pointer:input,value:0}; {let local=73;replace(&mut cell,&local);}cell}",
    );
    assert!(
        found
            .iter()
            .any(|error| error.contains("local") && error.contains("does not live long enough")),
        "{found:?}"
    );
}

#[test]
fn a_same_named_consumer_function_cannot_supply_the_result() {
    let found = source_errors(
        "public fn selected() {true}",
        "mount owner.selected as imported; fn selected()->Int {73} fn rejected()->Int {imported()}",
    );
    assert!(
        found
            .iter()
            .any(|error| error.contains("expected 'Int'") && error.contains("Bool")),
        "{found:?}"
    );
}
