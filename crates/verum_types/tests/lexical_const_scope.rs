use verum_ast::{FileId, ItemKind};
use verum_common::{ConstValue, List, Text};
use verum_fast_parser::FastParser;
use verum_types::{TypeChecker, const_eval::ConstEvaluator};

fn module(source: &str) -> verum_ast::Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(0))
        .expect("actual source grammar")
}
fn expression(source: &str) -> verum_ast::Expr {
    FastParser::new()
        .parse_expr_str(source, FileId::new(0))
        .expect("actual expression grammar")
}
fn checker() -> TypeChecker {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("fixture");
    checker.set_current_module_path("fixture.main");
    checker
}
fn check(checker: &mut TypeChecker, source: &str) -> List<Text> {
    let start = checker.diagnostics().len();
    let ast = module(source);
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&ast)
        .into_iter()
        .map(|error| error.to_string().into())
        .collect();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error.to_string().into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| error.to_string().into()),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .skip(start)
            .filter(|error| error.is_error())
            .map(|error| format!("{error:?}").into()),
    );
    errors
}
fn accept(checker: &mut TypeChecker, source: &str) {
    let errors = check(checker, source);
    assert!(errors.is_empty(), "{source}\n{errors:?}");
}
const GLOBAL: &str = "const Limit:Int=2;";
const AFTER: &str = "fn after(value:[Int;2])->[Int;Limit]{value}";
#[test]
fn successful_body_does_not_rebind_module_constant() {
    let mut c = checker();
    accept(&mut c, GLOBAL);
    accept(
        &mut c,
        "fn inside(value:[Int;3])->[Int;3]{const Limit:Int=3; let local:[Int;Limit]=value; local}",
    );
    accept(&mut c, AFTER);
    assert_eq!(
        c.eval_meta_param(&expression("Limit")).unwrap(),
        ConstValue::Int(2)
    );
}
#[test]
fn failed_ordinary_and_protocol_bodies_restore_outer_count() {
    for body in [
        "fn wrong()->Bool{const Limit:Int=3;42}",
        "type Rule is protocol {fn wrong()->Bool{const Limit:Int=3;42}};",
    ] {
        let mut c = checker();
        accept(&mut c, GLOBAL);
        assert!(!check(&mut c, body).is_empty());
        accept(&mut c, AFTER);
        assert_eq!(
            c.eval_meta_param(&expression("Limit")).unwrap(),
            ConstValue::Int(2)
        );
    }
}
#[test]
fn nested_block_restores_its_parent_local_count() {
    let mut c = checker();
    accept(&mut c, GLOBAL);
    accept(
        &mut c,
        "fn inside(three:[Int;3],four:[Int;4])->[Int;3]{const Limit:Int=3; {const Limit:Int=4; let nested:[Int;Limit]=four;} let outer:[Int;Limit]=three; outer}",
    );
    accept(&mut c, AFTER);
}
#[test]
fn wrong_local_size_is_not_recovered_from_outer_constant() {
    let mut c = checker();
    accept(&mut c, GLOBAL);
    assert!(
        !check(
            &mut c,
            "fn wrong(two:[Int;2])->[Int;2]{const Limit:Int=3; let local:[Int;Limit]=two; two}"
        )
        .is_empty()
    );
    accept(&mut c, AFTER);
}
#[test]
fn qualified_global_count_ignores_local_shadow() {
    let mut c = checker();
    accept(&mut c, GLOBAL);
    accept(
        &mut c,
        "fn inside(two:[Int;2])->[Int;2]{const Limit:Int=3; let global:[Int;cog.fixture.main.Limit]=two; global}",
    );
    accept(&mut c, AFTER);
}
#[test]
fn invalid_inner_count_cannot_borrow_outer_value_and_does_not_poison_it() {
    let mut c = checker();
    accept(&mut c, GLOBAL);
    assert!(
        !check(
            &mut c,
            "fn wrong(two:[Int;2])->[Int;2]{const Limit:Int=1/0;let local:[Int;Limit]=two;two}"
        )
        .is_empty()
    );
    accept(&mut c, AFTER);
}
#[test]
fn meta_block_error_restores_rich_bindings() {
    let mut evaluator = ConstEvaluator::new();
    evaluator.bind("value", ConstValue::Int(2));
    assert!(evaluator.eval(&expression("{let value=3;1/0}")).is_err());
    assert!(evaluator.eval(&expression("{let value=3;1/0;42}")).is_err());
    assert_eq!(evaluator.get(&"value".into()), Some(ConstValue::Int(2)));
    assert_eq!(
        evaluator
            .eval(&expression("{let value=3;{let value=4;}value}"))
            .unwrap(),
        ConstValue::Int(3)
    );
    assert_eq!(evaluator.get(&"value".into()), Some(ConstValue::Int(2)));
}
#[test]
fn meta_callee_uses_its_parameters_and_globals_without_caller_capture() {
    let mut evaluator = ConstEvaluator::new();
    evaluator.bind("value", ConstValue::Int(2));
    for item in module("meta fn global()->Int{value} meta fn identity(value:Int)->Int{value} meta fn wrong(value:Int)->Int{1/0}").items {
        if let ItemKind::Function(function)=item.kind {evaluator.register_function(&function);}
    }
    assert_eq!(
        evaluator
            .eval(&expression("{let value=7;global()}"))
            .unwrap(),
        ConstValue::Int(2)
    );
    assert_eq!(
        evaluator
            .eval(&expression("{let value=7;identity(value)}"))
            .unwrap(),
        ConstValue::Int(7)
    );
    assert!(
        evaluator
            .eval(&expression("{let value=7;wrong(value)}"))
            .is_err()
    );
    assert_eq!(evaluator.get(&"value".into()), Some(ConstValue::Int(2)));
}

#[test]
fn lazy_imported_signature_uses_its_owner_count_and_survives_local_scope() {
    use verum_modules::{
        ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
    };
    let provider = "public const Limit:Int=2; public fn consume(value:[Int;Limit])->Int{42}";
    let mut registry = ModuleRegistry::new();
    let ast = module(provider);
    let id = ModuleId::new(1);
    let owner = ModulePath::from_str("fixture.foreign");
    let mut info = ModuleInfo::new(
        id,
        owner.clone(),
        ast.clone(),
        FileId::new(1),
        provider.into(),
    );
    info.exports = extract_exports_from_module(&ast, id, &owner).unwrap();
    registry.register(info);
    let mut c = checker();
    c.set_module_registry_direct(registry);
    c.set_current_module_path("fixture.foreign");
    accept(&mut c, "public const Limit:Int=2;");
    c.set_current_module_path("fixture.main");
    accept(&mut c, GLOBAL);
    accept(
        &mut c,
        "fn inside(two:[Int;2])->Int{const Limit:Int=3;mount fixture.foreign.{consume};consume(two)}",
    );
    accept(
        &mut c,
        "mount fixture.foreign.{consume};fn after(two:[Int;2])->Int{consume(two)}",
    );
    let errors = check(
        &mut c,
        "fn wrong(three:[Int;3])->Int{consume(three)}",
    );
    assert!(!errors.is_empty(), "imported count was erased");
}
#[test]
fn global_rich_constant_keeps_its_own_evaluation_locals() {
    let mut c = checker();
    accept(&mut c, "const VALUE:Int={let local=2;local};");
    assert_eq!(c.eval_meta_param(&expression("VALUE")).unwrap(), ConstValue::Int(2));
    accept(&mut c, "fn same(value:[Int;2])->[Int;VALUE]{value}");
}
