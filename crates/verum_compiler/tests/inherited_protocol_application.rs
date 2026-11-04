//! Protocol applications retain declared kinds in the source checker and VBC.
use std::sync::Arc;
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::Interpreter,
};

fn source_errors(source: &str) -> Vec<Text> {
    let file_id = FileId::new(0);
    let ast = FastParser::new().parse_module_str(source, file_id).unwrap();
    let id = ModuleId::new(0);
    let owner = ModulePath::from_str("applications");
    let mut module = ModuleInfo::new(id, owner.clone(), ast.clone(), file_id, source.into());
    module.exports = extract_exports_from_module(&ast, id, &owner).unwrap();
    let mut registry = ModuleRegistry::new();
    registry.register(module);
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("applications");
    checker.set_current_module_path("applications");
    checker.set_module_registry_direct(registry);
    let mut errors: Vec<Text> = checker
        .prepare_module_types(&ast)
        .into_iter()
        .map(|e| format!("{e:?}").into())
        .collect();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|e| e.is_error())
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors
}

#[test]
fn direct_record_field_keeps_owner() {
    let errors = source_errors(
        r#"module applications;
 type Buffer<N:meta Int> is {value:Int};
 fn main()->Int { let buffer:Buffer<3> = Buffer {value:7}; buffer.value }
 "#,
    );
    assert!(errors.is_empty(), "{errors:?}");
}
#[test]
fn inherited_method_argument_keeps_protocol_application() {
    for (n, accepted) in [(3, true), (4, false)] {
        let errors = source_errors(&format!(
            r#"module applications;
 type Buffer<N:meta Int> is {{value:Int}};
 type Carrier<T> is protocol {{fn echo(&self,value:T)->T {{value}}}};
 type Point is {{value:Int}};
 implement Carrier<Buffer<3>> for Point {{}}
 fn main()->Int {{ let point=Point{{value:0}}; let buffer:Buffer<{n}> = Buffer {{value:7}}; let result=point.echo(buffer); 7 }}
 "#
        ));
        assert_eq!(errors.is_empty(), accepted, "value={n}: {errors:?}");
    }
}
#[test]
fn inherited_method_result_keeps_protocol_application() {
    for (n, accepted) in [(3, true), (4, false)] {
        let errors = source_errors(&format!(
            r#"module applications;
 type Buffer<N:meta Int> is {{value:Int}};
 type Carrier<T> is protocol {{fn echo(&self,value:T)->T {{value}}}};
 type Point is {{value:Int}};
 implement Carrier<Buffer<3>> for Point {{}}
 fn main()->Int {{ let point=Point{{value:0}}; let buffer:Buffer<3> = Buffer {{value:7}}; let result:Buffer<{n}> = point.echo(buffer); 7 }}
 "#
        ));
        assert_eq!(errors.is_empty(), accepted, "value={n}: {errors:?}");
    }
}

#[test]
fn inherited_result_retains_record_owner() {
    let errors = source_errors(
        r#"module applications;
        type Buffer<N: meta Int> is { value: Int };
        type Carrier<T> is protocol { fn echo(&self, value: T) -> T { value } };
        type Point is { value: Int };
        implement Carrier<Buffer<3>> for Point {}
        fn main() -> Int {
            let point = Point { value: 0 };
            let buffer: Buffer<3> = Buffer { value: 7 };
            let echoed = point.echo(buffer);
            echoed.value
        }
    "#,
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn unused_owner_parameter_does_not_shift_method_parameter() {
    for (argument, accepted) in [("true", true), ("3", false)] {
        let errors = source_errors(&format!(
            r#"module applications;
            type Select<A, B> is protocol {{ fn echo(&self, value: B) -> B {{ value }} }};
            type Point is {{ value: Int }};
            implement Select<Int, Bool> for Point {{}}
            fn main() -> Bool {{ let point = Point {{ value: 0 }}; point.echo({argument}) }}
        "#
        ));
        assert_eq!(errors.is_empty(), accepted, "{argument}: {errors:?}");
    }
}

#[test]
fn method_parameter_shadows_owner_without_capture() {
    let errors = source_errors(
        r#"module applications;
        type Carrier<T> is protocol {
            fn local<T>(&self, value: T) -> T { value }
            fn owner(&self, value: T) -> T { value }
        };
        type Point is { value: Int };
        implement Carrier<Int> for Point {}
        fn main() -> Bool {
            let point = Point { value: 0 };
            let first: Int = point.local(7);
            let second: Bool = point.local(true);
            let original: Int = point.owner(3);
            second
        }
    "#,
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn method_parameters_are_fresh_for_each_call() {
    let errors = source_errors(
        r#"module applications;
        type Carrier<T> is protocol { fn local<U>(&self, owner: T, value: U) -> U { value } };
        type Point is { value: Int };
        implement Carrier<Int> for Point {}
        fn main() -> Bool {
            let point = Point { value: 0 };
            let first: Int = point.local(1, 7);
            let second: Bool = point.local(1, true);
            second
        }
    "#,
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn generic_receiver_preserves_actual_owner_substitution() {
    for (argument, accepted) in [("true", true), ("3", false)] {
        let errors = source_errors(&format!(
            r#"module applications;
            type Carrier<T> is protocol {{ fn echo(&self, value: T) -> T {{ value }} }};
            type Pair<A, B> is {{ left: A, right: B }};
            implement<A, B> Carrier<B> for Pair<A, B> {{}}
            fn main() -> Bool {{
                let point: Pair<Int, Bool> = Pair {{ left: 0, right: true }};
                point.echo({argument})
            }}
        "#
        ));
        assert_eq!(errors.is_empty(), accepted, "{argument}: {errors:?}");
    }
}

#[test]
fn inherited_signature_and_execution_survive_vbc_roundtrip() {
    let source = r#"module applications;
        type Buffer<N: meta Int> is { value: Int };
        type Carrier<T> is protocol { fn echo(&self, value: T) -> T { value } };
        type Point is { value: Int };
        implement Carrier<Buffer<3>> for Point {}
        fn main() -> Int {
            let point = Point { value: 0 };
            let buffer: Buffer<3> = Buffer { value: 7 };
            let echoed = point.echo(buffer);
            echoed.value
        }
    "#;
    assert!(source_errors(source).is_empty());
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(0))
        .unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("applications"))
        .compile_module(&ast)
        .unwrap();
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    let entry = module.find_function_by_name("applications.main").unwrap();
    let value = Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .unwrap();
    assert_eq!(value.try_as_i64(), Some(7));
}

#[test]
fn bounded_method_uses_declared_owner_slots() {
    for (argument, accepted) in [("true", true), ("3", false)] {
        let errors = source_errors(&format!(
            r#"module applications;
            type Select<A, B> is protocol {{ fn echo(&self, value: B) -> B {{ value }} }};
            fn probe<P: Select<Int, Bool>>(point: P) -> Bool {{ point.echo({argument}) }}
        "#
        ));
        assert_eq!(errors.is_empty(), accepted, "{argument}: {errors:?}");
    }
}

#[test]
fn bounded_method_local_parameter_is_not_an_owner_argument() {
    let errors = source_errors(
        r#"module applications;
        type Carrier<A> is protocol { fn echo<B>(&self, value: B) -> B { value } };
        fn probe<P: Carrier<Int>>(point: P) -> Bool {
            let first: Int = point.echo(7);
            let second: Bool = point.echo(true);
            second
        }
    "#,
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn inherited_bound_transports_reordered_owner_arguments() {
    for (argument, accepted) in [("true", true), ("3", false)] {
        let errors = source_errors(&format!(
            r#"module applications;
            type Base<A, B> is protocol {{ fn echo(&self, value: B) -> B {{ value }} }};
            type Derived<X, Y> is protocol extends Base<Y, X> {{}};
            fn probe<P: Derived<Bool, Int>>(point: P) -> Bool {{ point.echo({argument}) }}
        "#
        ));
        assert_eq!(errors.is_empty(), accepted, "{argument}: {errors:?}");
    }
}

fn imported_errors(consumer: &str, providers: &[(&str, &str)]) -> List<Text> {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("demo");
    checker.set_current_module_path("demo.main");
    let mut registry = ModuleRegistry::new();
    for (index, (owner, source)) in providers
        .iter()
        .chain(std::iter::once(&("demo.main", consumer)))
        .enumerate()
    {
        let file_id = FileId::new(index as u32);
        let ast = FastParser::new()
            .parse_module_str(source, file_id)
            .expect("source grammar");
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let mut module = ModuleInfo::new(id, path.clone(), ast.clone(), file_id, (*source).into());
        module.exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        registry.register(module);
    }
    checker.set_module_registry_direct(registry.clone());
    let ast = FastParser::new()
        .parse_module_str(consumer, FileId::new(providers.len() as u32))
        .expect("consumer grammar");
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "demo.main", &registry) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
    for item in &ast.items {
        let result = match &item.kind {
            ItemKind::Type(decl) => checker.register_type_declaration(decl),
            ItemKind::Impl(decl) => checker.register_impl_block(decl),
            ItemKind::Function(decl) => checker.register_function_signature(decl),
            _ => continue,
        };
        if let Err(error) = result {
            errors.push(format!("{error:?}").into());
        }
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|e| e.is_error())
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors
}

#[test]
fn imported_inherited_method_keeps_owner_argument() {
    let provider = r#"module api;
        public type Buffer<N: meta Int> is { value: Int };
        public type Carrier<T> is protocol { fn echo(&self, value: T) -> T { value } };
        public type Point is { value: Int };
        implement Carrier<Buffer<3>> for Point {}
    "#;
    for (n, accepted) in [(3, true), (4, false)] {
        let source = format!(
            r#"module main;
            mount demo.api.{{Buffer, Point}};
            fn probe(point: Point, value: Buffer<{n}>) -> Buffer<{n}> {{ point.echo(value) }}
        "#
        );
        let found = imported_errors(&source, &[("demo.api", provider)]);
        assert_eq!(found.is_empty(), accepted, "{n}: {found:?}");
    }
}

#[test]
fn method_kind_binders_are_not_erased_into_ordinary_type_variables() {
    for parameters in ["F<_>", "F<_, _>", "F: Type -> Type"] {
        let errors = source_errors(&format!(
            r#"module applications;
            type Carrier is protocol {{ fn marker<{parameters}>(&self) -> Int {{ 7 }} }};
        "#
        ));
        assert!(
            errors
                .iter()
                .any(|error| error.contains("quantified signature carrier")),
            "{parameters}: {errors:?}"
        );
    }
}

#[test]
fn all_callable_method_bounds_are_retained_and_checked() {
    for (second, accepted) in [("Int", true), ("Bool", false)] {
        let errors = source_errors(&format!(
            r#"module applications;
            type Carrier is protocol {{
                fn call<F: fn(Int) -> Int + fn({second}) -> {second}>(&self, value: F) -> Int {{ 7 }}
            }};
            type Point is {{ value: Int }};
            implement Carrier for Point {{}}
            fn identity(value: {second}) -> {second} {{ value }}
            fn main() -> Int {{ let point = Point {{ value: 0 }}; point.call(identity) }}
        "#
        ));
        assert_eq!(errors.is_empty(), accepted, "{second}: {errors:?}");
    }
}

#[test]
fn bounded_method_local_protocol_obligations_are_enforced() {
    for (argument, accepted) in [("true", true), ("3", false)] {
        let errors = source_errors(&format!(
            r#"module applications;
            type Allowed is protocol {{}};
            implement Allowed for Bool {{}}
            type Carrier is protocol {{ fn echo<U: Allowed>(&self, value: U) -> U {{ value }} }};
            fn probe<P: Carrier>(point: P) -> Int {{ let ignored = point.echo({argument}); 7 }}
        "#
        ));
        assert_eq!(errors.is_empty(), accepted, "{argument}: {errors:?}");
    }
}

#[test]
fn reordered_implementation_binders_follow_actual_receiver_pattern() {
    for (argument, accepted) in [("3", true), ("true", false)] {
        let errors = source_errors(&format!(
            r#"module applications;
            type Carrier<T> is protocol {{ fn echo(&self, value: T) -> T {{ value }} }};
            type Pair<A, B> is {{ left: A, right: B }};
            implement<A, B> Carrier<A> for Pair<B, A> {{}}
            fn probe(point: Pair<Bool, Int>) -> Int {{ point.echo({argument}) }}
        "#
        ));
        assert_eq!(errors.is_empty(), accepted, "{argument}: {errors:?}");
    }
}

#[test]
fn inherited_local_protocol_obligations_are_checked_at_call() {
    for (argument, accepted) in [("true", true), ("3", false)] {
        let errors = source_errors(&format!(
            r#"module applications;
            type Allowed is protocol {{}};
            implement Allowed for Bool {{}}
            type Carrier is protocol {{ fn echo<U: Allowed>(&self, value: U) -> U {{ value }} }};
            type Point is {{ value: Int }};
            implement Carrier for Point {{}}
            fn probe(point: Point) -> Int {{ let value = point.echo({argument}); 7 }}
        "#
        ));
        assert_eq!(errors.is_empty(), accepted, "{argument}: {errors:?}");
    }
}

#[test]
fn conditional_self_requirement_is_checked_per_call_not_per_impl() {
    for (called, allowed, accepted) in [
        (false, false, true),
        (true, true, true),
        (true, false, false),
    ] {
        let evidence = if allowed {
            "implement Allowed for Point {}"
        } else {
            ""
        };
        let call = if called { "point.check()" } else { "7" };
        let errors = source_errors(&format!(
            r#"module applications;
            type Allowed is protocol {{}};
            type Carrier is protocol {{ fn check(&self) -> Int where type Self: Allowed {{ 7 }} }};
            type Point is {{ value: Int }};
            implement Carrier for Point {{}}
            {evidence}
            fn probe(point: Point) -> Int {{ {call} }}
        "#
        ));
        if !accepted {
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("method call requirement")),
                "wrong rejection: {errors:?}"
            );
        }
        assert_eq!(
            errors.is_empty(),
            accepted,
            "called={called}, allowed={allowed}: {errors:?}"
        );
    }
}

#[test]
fn explicit_override_preserves_declared_self_requirement() {
    for (allowed, accepted) in [(true, true), (false, false)] {
        let evidence = if allowed {
            "implement Allowed for Point {}"
        } else {
            ""
        };
        let errors = source_errors(&format!(
            r#"module applications;
            type Allowed is protocol {{}};
            type Carrier is protocol {{ fn check(&self) -> Int where type Self: Allowed; }};
            type Point is {{ value: Int }};
            implement Carrier for Point {{ fn check(&self) -> Int {{ 7 }} }}
            {evidence}
            fn probe(point: Point) -> Int {{ point.check() }}
        "#
        ));
        if !accepted {
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("method call requirement")),
                "wrong rejection: {errors:?}"
            );
        }
        assert_eq!(errors.is_empty(), accepted, "allowed={allowed}: {errors:?}");
    }
}

#[test]
fn associated_subject_requirement_uses_actual_implementation() {
    for (item, accepted) in [("Bool", true), ("Int", false)] {
        let errors = source_errors(&format!(
            r#"module applications;
            type Allowed is protocol {{}};
            implement Allowed for Bool {{}}
            type Carrier is protocol {{ type Item; fn check(&self) -> Int where type Self.Item: Allowed {{ 7 }} }};
            type Point is {{ value: Int }};
            implement Carrier for Point {{ type Item = {item}; }}
            fn probe(point: Point) -> Int {{ point.check() }}
        "#
        ));
        if !accepted {
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("method call requirement")),
                "wrong rejection: {errors:?}"
            );
        }
        assert_eq!(errors.is_empty(), accepted, "{item}: {errors:?}");
    }
}

#[test]
fn local_where_requirement_keeps_method_binder_identity() {
    for (argument, accepted) in [("true", true), ("7", false)] {
        let errors = source_errors(&format!(
            r#"module applications;
            type Allowed is protocol {{}};
            implement Allowed for Bool {{}}
            type Carrier<A> is protocol {{ fn check<F>(&self, value: F) -> Int where type F: Allowed {{ 7 }} }};
            type Point is {{ value: Int }};
            implement Carrier<Int> for Point {{}}
            fn probe(point: Point) -> Int {{ point.check({argument}) }}
        "#
        ));
        if !accepted {
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("method call requirement")),
                "wrong rejection: {errors:?}"
            );
        }
        assert_eq!(errors.is_empty(), accepted, "{argument}: {errors:?}");
    }
}

#[test]
fn bounded_call_proves_where_requirement_from_callers_assumptions() {
    for (extra, accepted) in [(" + Allowed", true), ("", false)] {
        let errors = source_errors(&format!(
            r#"module applications;
            type Allowed is protocol {{}};
            type Carrier is protocol {{ fn check(&self) -> Int where type Self: Allowed {{ 7 }} }};
            fn probe<P: Carrier{extra}>(point: P) -> Int {{ point.check() }}
        "#
        ));
        if !accepted {
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("method call requirement")),
                "wrong rejection: {errors:?}"
            );
        }
        assert_eq!(errors.is_empty(), accepted, "{extra}: {errors:?}");
    }
}

#[test]
fn owner_where_requirement_is_substituted_before_discharge() {
    for (owner, accepted) in [("Bool", true), ("Int", false)] {
        let errors = source_errors(&format!(
            r#"module applications;
            type Allowed is protocol {{}};
            implement Allowed for Bool {{}}
            type Carrier<T> is protocol {{ fn check(&self) -> Int where type T: Allowed {{ 7 }} }};
            type Point is {{ value: Int }};
            implement Carrier<{owner}> for Point {{}}
            fn probe(point: Point) -> Int {{ point.check() }}
        "#
        ));
        if !accepted {
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("method call requirement")),
                "wrong rejection: {errors:?}"
            );
        }
        assert_eq!(errors.is_empty(), accepted, "{owner}: {errors:?}");
    }
}

#[test]
fn explicit_override_preserves_method_local_requirement() {
    for (argument, accepted) in [("true", true), ("3", false)] {
        let errors = source_errors(&format!(
            r#"module applications;
            type Allowed is protocol {{}};
            implement Allowed for Bool {{}}
            type Carrier is protocol {{
                fn echo<R>(&self, value: R) -> R where type R: Allowed;
            }};
            type Point is {{ value: Int }};
            implement Carrier for Point {{ fn echo<R>(&self, value: R) -> R {{ value }} }}
            fn probe() -> Int {{ let point = Point {{ value: 0 }}; let result = point.echo({argument}); 7 }}
        "#
        ));
        if !accepted {
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("method call requirement")),
                "{errors:?}"
            );
        }
        assert_eq!(errors.is_empty(), accepted, "{argument}: {errors:?}");
    }
}

#[test]
fn return_selected_override_retains_only_its_own_requirements() {
    for (output, accepted) in [("Bool", true), ("Int", false)] {
        let errors = source_errors(&format!(
            r#"module applications;
            type Allowed is protocol {{}};
            implement Allowed for Bool {{}}
            type Carrier<T> is protocol {{ fn convert(&self) -> T where type T: Allowed; }};
            type Point is {{ value: Int }};
            implement Carrier<Int> for Point {{ fn convert(&self) -> Int {{ 7 }} }}
            implement Carrier<Bool> for Point {{ fn convert(&self) -> Bool {{ true }} }}
            fn probe(point: Point) -> {output} {{ let value: {output} = point.convert(); value }}
        "#
        ));
        if !accepted {
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("method call requirement")),
                "{errors:?}"
            );
        }
        assert_eq!(errors.is_empty(), accepted, "{output}: {errors:?}");
    }
}
