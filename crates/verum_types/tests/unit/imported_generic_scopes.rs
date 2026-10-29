use super::*;
use verum_fast_parser::Parser;

const SOURCE: &str = r#"
type Imported<T> is { value: T };
implement<T> Imported<T> {
    fn choose<T>(&self, value: T) -> T { value }
    fn original(&self) -> T { self.value }
    fn create<T>(value: T) -> T { value }
    fn after_static(&self) -> T { self.value }
}
"#;

fn declarations() -> (TypeChecker, verum_ast::Module) {
    let module = Parser::new(SOURCE).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Type(decl) = &item.kind {
            checker.register_type_declaration(decl).unwrap();
        }
    }
    (checker, module)
}

#[test]
fn lazy_import_preserves_the_enclosing_type_parameters_and_self() {
    let (mut checker, module) = declarations();
    let outer = Type::Var(TypeVar::fresh());
    checker.ctx.define_type("T", outer.clone());
    checker.ctx.define_type("Self", Type::Bool);
    checker
        .import_impl_blocks_for_type(&module, "Imported")
        .unwrap();
    assert_eq!(checker.ctx.lookup_type("T"), Some(&outer));
    assert_eq!(checker.ctx.lookup_type("Self"), Some(&Type::Bool));
}

#[test]
fn lazy_import_does_not_leave_new_parameter_names_in_the_caller() {
    let (mut checker, module) = declarations();
    checker.ctx.remove_type(&Text::from("T"));
    checker.ctx.remove_type(&Text::from("Self"));
    checker
        .import_impl_blocks_for_type(&module, "Imported")
        .unwrap();
    assert!(checker.ctx.lookup_type("T").is_none());
    assert!(checker.ctx.lookup_type("Self").is_none());
}

#[test]
fn method_shadowing_preserves_impl_and_method_quantifier_identity() {
    let (mut checker, module) = declarations();
    checker
        .import_impl_blocks_for_type(&module, "Imported")
        .unwrap();
    let methods = checker.inherent_methods.read();
    let methods = methods
        .get(&Text::from("Imported"))
        .expect("imported methods");
    let choose = methods.get(&Text::from("choose")).unwrap();
    assert_eq!(choose.impl_var_count, 1);
    assert_eq!(choose.vars.len(), 2);
    assert_ne!(choose.vars[0], choose.vars[1]);
    let Type::Function {
        params,
        return_type,
        ..
    } = &choose.ty
    else {
        panic!("method")
    };
    assert_eq!(params.as_slice(), &[Type::Var(choose.vars[1])]);
    assert_eq!(return_type.as_ref(), &Type::Var(choose.vars[1]));
    for name in ["original", "after_static"] {
        let method = methods.get(&Text::from(name)).unwrap();
        assert_eq!(method.vars.len(), 1, "{name}");
        let Type::Function { return_type, .. } = &method.ty else {
            panic!("method")
        };
        assert_eq!(return_type.as_ref(), &Type::Var(choose.vars[0]), "{name}");
    }
}
