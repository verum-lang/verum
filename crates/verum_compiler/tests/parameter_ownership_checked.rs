//! Checked method selection retains argument and projected-result ownership.
use verum_ast::{Visitor, expr::ResolvedCallTarget};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
use verum_vbc::{
    codegen::VbcCodegen, deserialize::deserialize_module, interpreter::Interpreter,
    serialize::serialize_module,
};

fn checked(source: &str) -> Result<verum_ast::Module, verum_common::Text> {
    let mut ast = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    let mut errors = checker.prepare_module_declarations(&ast);
    errors.extend(
        ast.items
            .iter()
            .filter_map(|item| checker.check_item(item).err()),
    );
    errors.extend(checker.take_deferred_errors());
    if !errors.is_empty() || !checker.diagnostic_sources().is_empty() {
        return Err(format!("{errors:?} {:?}", checker.diagnostic_sources()).into());
    }
    checker.commit_resolved_call_targets(&mut ast);
    checker.commit_resolved_call_targets(&mut ast);
    Ok(ast)
}

fn checked_run(source: &str) -> i64 {
    let ast = checked(source).unwrap();
    struct Selected(usize);
    impl Visitor for Selected {
        fn visit_expr(&mut self, expr: &verum_ast::Expr) {
            if matches!(
                expr.resolved_call_target,
                Some(ResolvedCallTarget::StaticCall { .. })
            ) {
                self.0 += 1;
            }
            verum_ast::visitor::walk_expr(self, expr);
        }
    }
    let mut selected = Selected(0);
    for item in &ast.items {
        selected.visit_item(item);
    }
    assert!(
        selected.0 > 0,
        "control must use the checker's selected call route"
    );
    let original = VbcCodegen::new().compile_module(&ast).unwrap();
    let module = deserialize_module(&serialize_module(&original).unwrap()).unwrap();
    let execute = |image: verum_vbc::VbcModule| {
        let main = image.find_function_by_name("main").unwrap();
        Interpreter::try_new_validated(image.into()).unwrap()
            .execute_function(main).unwrap().try_as_i64().unwrap()
    };
    let result = execute(original);
    assert_eq!(execute(module), result);
    result
}

#[test]
fn checked_consuming_method_preserves_the_callers_nested_owner() {
    assert_eq!(
        checked_run(
            r#"
        type Leaf is { value: Int };
        type Branch is { leaf: Leaf };
        implement Branch { fn consume(self) {} }
        fn main() -> Int {
            let original = Branch { leaf: Leaf { value: 73 } };
            original.consume();
            original.leaf.value
        }
    "#
        ),
        73
    );
}

#[test]
fn checked_parameter_projection_survives_its_parameter_cleanup() {
    assert_eq!(
        checked_run(
            r#"
        type Leaf is { value: Int };
        type Branch is { leaf: Leaf };
        implement Branch { fn project(self) -> Leaf { self.leaf } }
        fn main() -> Int {
            let original = Branch { leaf: Leaf { value: 73 } };
            let projected = original.project();
            original.leaf.value + projected.value
        }
    "#
        ),
        146
    );
}

const GUARDED_METHODS: &str = r#"
    type Deref is protocol { type Target; fn deref(&self) -> &Self.Target; };
    type DerefMut is protocol extends Deref { fn deref_mut(&mut self) -> &mut Self.Target; };
    type Counter is { value: Int };
    implement Counter {
        fn write(&mut self, next: Int) { self.value = next; }
        fn read(&self) -> Int { self.value }
    }
    type Guard<T> is { inner: T };
    implement<T> Deref for Guard<T> {
        type Target = T;
        fn deref(&self) -> &T { &self.inner }
    }
"#;
const WRITE_GUARD: &str = r#"
    implement<T> DerefMut for Guard<T> {
        fn deref_mut(&mut self) -> &mut T { &mut self.inner }
    }
"#;

#[test]
fn checked_mutable_method_writes_through_each_guard() {
    let source = format!(r#"{GUARDED_METHODS} {WRITE_GUARD}
        fn main() -> Int {{
            let mut outer = Guard {{ inner: Guard {{ inner: Counter {{ value: 2 }} }} }};
            let before = outer.read();
            outer.write(71);
            before + outer.inner.inner.value
        }}
    "#);
    assert_eq!(checked_run(&source), 73);
}

#[test]
fn shared_only_guards_and_shared_borrows_refuse_mutable_methods() {
    for (implementation, receiver, reason) in [
        ("", "&mut Guard<Counter>", "DerefMut"),
        (WRITE_GUARD, "&Guard<Counter>", "SharedReferenceWrite"),
    ] {
        let source = format!("{GUARDED_METHODS} {implementation} fn update(guard: {receiver}) {{ guard.write(71); }}");
        let error = checked(&source).expect_err("the actual receiver lacks exclusive access");
        assert!(error.contains(reason), "{error}");
    }
}
