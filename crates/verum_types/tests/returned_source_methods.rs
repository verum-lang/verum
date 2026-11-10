//! T1711: a returned source value carries its declaring type's public methods.
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text, ToText};
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

#[test]
fn two_mount_paths_preregister_one_implementation_owner() {
    let library = r#"
        public type Stamp is protocol { fn stamp(&self) -> Int; };
        public type Ticket<T> is { value:T };
        implement<T> Stamp for Ticket<T> { fn stamp(&self) -> Int { 7 } }
        public fn first() -> Int { 1 }
        public fn second() -> Int { 2 }
    "#;
    for mounts in [
        "mount demo.tickets.first; mount old.tickets.second;",
        "mount old.tickets.second; mount demo.tickets.first;",
    ] {
        let (_, errors) = check_with_module_aliases(
            &format!("{mounts} fn probe() -> Int {{ first() + second() }}"),
            &[("demo.tickets", library)], &[("old.tickets", "demo.tickets")],
        );
        assert!(errors.is_empty(), "{mounts} {errors:?}");
    }
}

fn check_with_checker(consumer: &str, modules: &[(&str, &str)]) -> (TypeChecker, List<Text>) {
    check_with_module_aliases(consumer, modules, &[])
}

fn check_with_module_aliases(
    consumer: &str,
    modules: &[(&str, &str)],
    aliases: &[(&str, &str)],
) -> (TypeChecker, List<Text>) {
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
    for (alias, owner) in aliases {
        registry.register_path_alias(*alias, *owner);
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
    assert_eq!(
        errors.len(),
        1,
        "expected only the missing-method refusal: {errors:?}"
    );
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
            &format!(
                "{mounts} fn integer() -> Int {{ first().read() }} fn boolean() -> Bool {{ second().read() }}"
            ),
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
            let signature = &checker
                .context_mut()
                .env
                .lookup("issue")
                .expect("mounted function")
                .ty;
            let Type::Function { return_type, .. } = signature else {
                panic!("not a function: {signature:?}");
            };
            assert_eq!(
                return_type.to_text(),
                "demo.provider.Receipt",
                "reverse={reverse}; {consumer}: {signature:?}"
            );
        }
    }
}

fn check_generic_factory(entry: &str) {
    let model = "public type Receipt<T> is { value: T }; implement<T> Receipt<T> { public fn read(&self) -> T { self.value } }";
    let factory = "mount demo.model.{Receipt as Issued}; public fn issue<T>(value: T) -> Issued<T> { Issued { value: value } }";
    let other = "public type Issued<T> is { value: Bool }; implement<T> Issued<T> { public fn read(&self) -> Bool { self.value } }";
    {
        let consumer = format!(
            "mount demo.other.Issued; mount {entry}.issue; fn probe() -> Int {{ issue(37).read() }}"
        );
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

#[test]
fn generic_return_uses_the_declaring_modules_import_alias() {
    check_generic_factory("demo.factory");
}

#[test]
fn reexported_generic_return_uses_the_declaring_modules_import_alias() {
    check_generic_factory("demo.api");
}

const HIDDEN_PROVIDER: &str = r#"
    private type Receipt is { value: Int };
    implement Receipt {
        public fn read(&self) -> Int { self.hidden() }
        private fn hidden(&self) -> Int { self.value }
    }
    public fn issue() -> Receipt { Receipt { value: 37 } }
"#;

#[test]
fn inferred_private_type_retains_its_public_method() {
    let errors = check(
        "mount demo.provider.issue; fn probe() -> Int { issue().read() }",
        &[("demo.provider", HIDDEN_PROVIDER)],
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn inferred_private_type_does_not_expose_its_private_method() {
    let errors = check(
        "mount demo.provider.issue; fn probe() -> Int { issue().hidden() }",
        &[("demo.provider", HIDDEN_PROVIDER)],
    );
    assert_missing_method(&errors, "hidden");
    assert!(errors[0].contains("demo.provider.Receipt"), "{errors:?}");
}

#[test]
fn inferred_private_type_does_not_publish_its_bare_name() {
    let errors = check(
        "mount demo.provider.issue; fn forbidden(value: Receipt) -> Int { 0 }",
        &[("demo.provider", HIDDEN_PROVIDER)],
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("TypeNotFound") && error.contains("Receipt")),
        "the factory must not import its private return type name: {errors:?}",
    );
}

fn borrowed_generic_method_errors(method: &str, return_type: &str) -> List<Text> {
    let model = "public type Stored<T> is { value: T }; implement<T> Stored<T> { public fn read(&self) -> T { self.value } private fn hidden(&self) -> T { self.value } }";
    let provider = "mount demo.model.{Stored as Items}; public type Holder is { items: Items<Int> }; implement Holder { public fn items(&self) -> &Items<Int> { &self.items } }";
    let other = "public type Items<T> is { value: Bool }; implement<T> Items<T> { public fn read(&self) -> Bool { self.value } }";
    check(
        &format!("mount demo.other.Items; mount demo.provider.Holder; fn probe(value: &Holder) -> {return_type} {{ value.items().{method}() }}"),
        &[("demo.model", model), ("demo.provider", provider), ("demo.other", other)],
    )
}

#[test]
fn borrowed_method_result_keeps_its_generic_import_owner() {
    let errors = borrowed_generic_method_errors("read", "Int");
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn borrowed_method_result_cannot_use_a_same_leaf_return_type() {
    let errors = borrowed_generic_method_errors("read", "Bool");
    assert!(errors.iter().any(|error| error.contains("Mismatch")), "{errors:?}");
    assert!(!errors.iter().any(|error| error.contains("MethodNotFound")), "{errors:?}");
}

#[test]
fn borrowed_method_result_keeps_the_returned_owners_private_methods() {
    let errors = borrowed_generic_method_errors("hidden", "Int");
    assert_missing_method(&errors, "hidden");
}

fn module_alias_method_errors(method: &str, result: &str) -> List<Text> {
    let model = "public type Stored<T> is { value: T }; implement<T> Stored<T> { public fn first(&self) -> T { self.value } public fn second(&self) -> T { self.value } private fn hidden(&self) -> T { self.value } }";
    let provider = "mount short_model.{Stored as Items}; public type Holder is { items: Items<Int> }; implement Holder { public fn items(&self) -> &Items<Int> { &self.items } }";
    let consumer = format!("mount short_provider.Holder; fn probe(value: &Holder) -> {result} {{ let first: Int = value.items().first(); value.items().{method}() }}");
    check_with_module_aliases(
        &consumer,
        &[("demo.model", model), ("demo.provider", provider)],
        &[("short_model", "demo.model"), ("short_provider", "demo.provider")],
    ).1
}

#[test]
fn module_path_aliases_preserve_borrowed_receiver_method_identity() {
    let errors = module_alias_method_errors("second", "Int");
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn module_path_aliases_preserve_borrowed_receiver_payload_type() {
    let errors = module_alias_method_errors("second", "Bool");
    assert!(errors.iter().any(|error| error.contains("Mismatch")), "{errors:?}");
    assert!(!errors.iter().any(|error| error.contains("MethodNotFound")), "{errors:?}");
}

#[test]
fn module_path_aliases_preserve_borrowed_receiver_private_methods() {
    let errors = module_alias_method_errors("hidden", "Int");
    assert_missing_method(&errors, "hidden");
}

#[test]
fn a_source_index_projects_its_key_and_output_through_a_shared_field() {
    let modules = [
        ("core.base.protocols", r#"
            public type Index<Idx> is protocol { type Output; fn index(&self, key:Idx)->&Self.Output; };
            public type IndexMut<Idx> is protocol extends Index<Idx> { fn index_mut(&mut self, key:Idx)->&mut Self.Output; };
        "#),
        ("demo.provider", r#"
            mount core.base.protocols.{Index,IndexMut};
            public type Table<K,V> is { key:K, entry:V };
            implement<K,V> Index<K> for Table<K,V> { type Output=V; fn index(&self, key:K)->&V { &self.entry } }
            implement<K,V> IndexMut<K> for Table<K,V> { fn index_mut(&mut self, key:K)->&mut V { &mut self.entry } }
        "#),
    ];
    let header = "mount demo.provider.Table; type Report is { rows: Table<Text,Bool> };";
    let valid = check(&format!(r#"{header} fn read(report:&Report)->Bool {{ let item = &report.rows["x"]; *item }}"#), &modules);
    assert!(valid.is_empty(), "{valid:?}");
    for body in [
        r#"fn wrong_key(report:&Report)->Bool { report.rows[3] }"#,
        r#"fn wrong_output(report:&Report)->Int { report.rows["x"] }"#,
    ] {
        let errors = check(&format!("{header} {body}"), &modules);
        assert!(errors.iter().any(|error| error.contains("Mismatch")), "{body}: {errors:?}");
    }
    let invalid_write = check(&format!(r#"{header} fn write(report:&Report) {{ report.rows["x"] = true; }}"#), &modules);
    assert!(invalid_write.iter().any(|error| error.contains("SharedReferenceWrite")), "{invalid_write:?}");
}

#[test]
fn awaiting_a_returned_source_value_projects_the_declared_future_output() {
    let modules = [
        ("core.async.future", "public type Future is protocol { type Output; };"),
        ("demo.foreign", "public type Future is protocol { type Output; };"),
        ("demo.jobs", r#"
            mount core.async.future.Future;
            mount demo.foreign.{Future as ForeignFuture};
            public type Ticket<T> is { value: T };
            public fn issue<T>(value: T) -> Ticket<T> { Ticket { value } }
            public type Other is protocol { type Output; };
            implement<T> Other for Ticket<T> { type Output = Int; }
            implement<T> Future for Ticket<T> { type Output = (T, Bool); }
            public type Noise<T> is { value: T };
            public fn noise<T>(value: T) -> Noise<T> { Noise { value } }
            implement<T> ForeignFuture for Noise<T> { type Output = T; }
        "#),
    ];
    let header = "mount demo.jobs.{issue,noise};";
    let accepted = check(
        &format!(r#"
            {header}
            async fn read() -> Text {{
                let (value, flag) = issue("ready").await;
                if flag {{ value }} else {{ "" }}
            }}
            async fn choose() -> Text {{
                select {{ pair = issue("ready").await => pair.0 }}
            }}
            async fn together() -> Bool {{
                let (first, second) = join(issue(7), issue("ready")).await;
                second.1
            }}
        "#),
        &modules,
    );
    assert!(accepted.is_empty(), "{accepted:?}");
    for expression in [r#"issue("ready").await"#, "noise(7).await"] {
        let errors = check(
            &format!("{header} async fn wrong() -> Int {{ {expression} }}"),
            &modules,
        );
        assert!(errors.iter().any(|error| error.contains("Mismatch")), "{expression}: {errors:?}");
    }
}

#[test]
fn a_sync_callback_can_call_an_imported_method_with_a_proven_sync_body() {
    let modules = [("demo.jobs", r#"
        public type Job is { number: Int };
        implement Job {
            public fn read(&self) -> Int { self.number }
            public async fn later(&self) -> Int { self.number }
            public fn indirect(&self, callback: async fn() -> Int) -> Int { callback() }
        }
    "#)];
    let header = r#"
        mount demo.jobs.Job;
        fn require(item: Job, read: fn(Job) -> Int) -> Int { read(item) }
    "#;
    let valid = check(
        &format!("{header} fn probe(item: Job) -> Int {{ require(item, |job| job.read()) }}"),
        &modules,
    );
    assert!(valid.is_empty(), "{valid:?}");
    for call in ["job.later()", "job.indirect(callback)"] {
        let errors = check(
            &format!("{header} fn probe(item: Job, callback: async fn() -> Int) -> Int {{ require(item, |job| {call}) }}"),
            &modules,
        );
        assert!(errors.iter().any(|error| error.contains("Mismatch") && error.contains("async fn")), "{call}: {errors:?}");
    }
}
