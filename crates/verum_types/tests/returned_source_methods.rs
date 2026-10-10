//! T1711: a returned source value carries its declaring type's public methods.
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::{Type, TypeChecker};

const PROVIDER: &str = r#"
    public type Receipt is { value: Int };
    implement Receipt {
        public fn read(&self) -> Int { self.value }
        private fn hidden(&self) -> Int { self.value }
    }
    public type Result<T, E> is | Ok(T) | Err(E);
    public fn issue() -> Receipt { Receipt { value: 37 } }
    public fn checked_issue() -> Result<Receipt, Text> { Result.Ok(issue()) }
"#;

fn check_with_checker(consumer: &str, modules: &[(&str, &str)]) -> (TypeChecker, List<Text>) {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("demo");
    checker.set_current_module_path("demo.main");
    let mut registry = ModuleRegistry::new();
    for (index, (owner, source)) in modules
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
        .parse_module_str(consumer, FileId::new(modules.len() as u32))
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
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    (checker, errors)
}

fn check(consumer: &str, modules: &[(&str, &str)]) -> List<Text> {
    check_with_checker(consumer, modules).1
}

fn assert_missing_method(errors: &List<Text>, method: &str) {
    assert_eq!(errors.len(), 1, "expected only the missing-method refusal: {errors:?}");
    assert!(
        errors[0].contains("MethodNotFound") && errors[0].contains(method),
        "wrong refusal: {errors:?}"
    );
}

#[test]
fn returned_public_receiver_has_methods_without_direct_type_mount() {
    let errors = check(
        "mount demo.provider.issue; fn probe() -> Int { issue().read() }",
        &[("demo.provider", PROVIDER)],
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn direct_type_mount_is_the_positive_control() {
    let errors = check(
        "mount demo.provider.{Receipt, issue}; fn probe() -> Int { issue().read() }",
        &[("demo.provider", PROVIDER)],
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn returned_result_payload_has_methods_without_direct_payload_mount() {
    let errors = check(
        "mount demo.provider.{Result, checked_issue}; fn probe() -> Int { match checked_issue() { Result.Ok(value) => value.read(), Result.Err(_) => 0 } }",
        &[("demo.provider", PROVIDER)],
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn returned_same_leaf_receivers_keep_their_declaring_methods_in_both_orders() {
    let other = "public type Receipt is { value: Bool }; implement Receipt { public fn read(&self) -> Bool { self.value } } public fn issue() -> Receipt { Receipt { value: true } }";
    for reverse in [false, true] {
        let modules = if reverse {
            [("demo.other", other), ("demo.provider", PROVIDER)]
        } else {
            [("demo.provider", PROVIDER), ("demo.other", other)]
        };
        let mounts = if reverse {
            "mount demo.other.{issue as second}; mount demo.provider.{issue as first};"
        } else {
            "mount demo.provider.{issue as first}; mount demo.other.{issue as second};"
        };
        let errors = check(
            &format!("{mounts} fn integer() -> Int {{ first().read() }} fn boolean() -> Bool {{ second().read() }}"),
            &modules,
        );
        assert!(errors.is_empty(), "reverse={reverse}: {errors:?}");
    }
}

#[test]
fn foreign_same_leaf_receiver_does_not_acquire_another_owners_method() {
    let other = "public type Receipt is { value: Int }; public fn issue() -> Receipt { Receipt { value: 11 } }";
    for reverse in [false, true] {
        let modules = if reverse {
            [("demo.other", other), ("demo.provider", PROVIDER)]
        } else {
            [("demo.provider", PROVIDER), ("demo.other", other)]
        };
        let errors = check(
            "mount demo.provider.Receipt; mount demo.other.issue; fn probe() -> Int { issue().read() }",
            &modules,
        );
        assert_missing_method(&errors, "read");
        assert!(errors[0].contains("demo.other.Receipt"), "{errors:?}");
    }
}

#[test]
fn returned_receiver_cannot_call_a_private_method() {
    let errors = check(
        "mount demo.provider.issue; fn probe() -> Int { issue().hidden() }",
        &[("demo.provider", PROVIDER)],
    );
    assert_missing_method(&errors, "hidden");
}

#[test]
fn direct_type_mount_cannot_expose_a_private_method() {
    let errors = check(
        "mount demo.provider.{Receipt, issue}; fn probe() -> Int { issue().hidden() }",
        &[("demo.provider", PROVIDER)],
    );
    assert_missing_method(&errors, "hidden");
}

#[test]
fn owner_local_public_method_can_call_its_private_helper() {
    let errors = check(
        "type Receipt is { value: Int }; implement Receipt { private fn hidden(&self) -> Int { self.value } public fn read(&self) -> Int { self.hidden() } } fn probe(value: Receipt) -> Int { value.read() }",
        &[],
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn source_function_return_keeps_the_declaring_owner_without_a_type_mount() {
    for consumer in [
        "mount demo.provider.issue;",
        "mount demo.other.Receipt; mount demo.provider.issue;",
    ] {
        let other = "public type Receipt is { value: Bool };";
        for reverse in [false, true] {
            let modules = if reverse {
                [("demo.other", other), ("demo.provider", PROVIDER)]
            } else {
                [("demo.provider", PROVIDER), ("demo.other", other)]
            };
            let (mut checker, errors) = check_with_checker(consumer, &modules);
            assert!(errors.is_empty(), "{errors:?}");
            let signature = &checker.context_mut().env.lookup("issue").expect("mounted function").ty;
            let Type::Function { return_type, .. } = signature else {
                panic!("not a function: {signature:?}");
            };
            assert_eq!(return_type.to_text(), "demo.provider.Receipt", "reverse={reverse}; {consumer}: {signature:?}");
        }
    }
}

#[test]
fn generic_return_uses_the_declaring_modules_import_alias() {
    let model = "public type Receipt<T> is { value: T }; implement<T> Receipt<T> { public fn read(&self) -> T { self.value } }";
    let factory = "mount demo.model.{Receipt as Issued}; public fn issue<T>(value: T) -> Issued<T> { Issued { value: value } }";
    let other = "public type Issued<T> is { value: Bool }; implement<T> Issued<T> { public fn read(&self) -> Bool { self.value } }";
    for entry in ["demo.factory", "demo.api"] {
        let consumer = format!("mount demo.other.Issued; mount {entry}.issue; fn probe() -> Int {{ issue(37).read() }}");
        let errors = check(
            &consumer,
            &[
                ("demo.model", model),
                ("demo.factory", factory),
                ("demo.other", other),
                ("demo.api", "public mount demo.factory.issue;"),
            ],
        );
        assert!(errors.is_empty(), "entry={entry}: {errors:?}");
    }
}
