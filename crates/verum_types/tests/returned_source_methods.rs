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
fn imported_generic_pointer_payloads_preserve_the_pointee_and_access_mode() {
    let modules = [
        (
            "demo.device",
            "public type Address<T> is (*mut T); public type Register<T> is (*volatile T);",
        ),
        ("demo.host", "public type Address<T> is (*const T);"),
    ];
    let mounts = "mount demo.device.{Address as Device, Register}; mount demo.host.Address as Host;";
    for (carrier, pointee) in [
        ("Device", "*mut Int"),
        ("Host", "*const Int"),
        ("Register", "*volatile Int"),
    ] {
        let source = format!("{mounts} fn read(value: {carrier}<Int>) -> {pointee} {{ value.0 }}");
        let (checker, errors) = check_with_checker(&source, &modules);
        assert!(errors.is_empty(), "{source}: {errors:?}");
        let projection = "value.0";
        let start = source.find(projection).unwrap() as u32;
        let span = verum_ast::Span::new(
            start, start + projection.len() as u32, FileId::new(modules.len() as u32),
        );
        let ty = checker.type_registry.lookup_expr(span).expect("inferred field projection");
        assert_eq!(ty.to_string(), pointee);
    }
    for (carrier, wrong) in [
        ("Device", "*mut Bool"),
        ("Host", "*const Bool"),
        ("Register", "*volatile mut Int"),
    ] {
        let source = format!("{mounts} fn wrong(value: {carrier}<Int>) -> {wrong} {{ value.0 }}");
        let errors = check(&source, &modules);
        assert!(errors.iter().any(|error| error.contains("Mismatch")), "{source}: {errors:?}");
    }
}

#[test]
fn plain_call_bounds_load_the_returned_values_own_implementation() {
    let modules = [
        ("demo.capability", "public type Stamp is protocol { fn stamp(&self) -> Int; };"),
        ("demo.tickets", r#"
            mount demo.capability.Stamp;
            public type Ticket<T> is { value: T };
            implement Stamp for Ticket<Int> { fn stamp(&self) -> Int { self.value } }
            public fn issue() -> Ticket<Int> { Ticket { value: 42 } }
        "#),
        ("demo.foreign", r#"
            public type Ticket<T> is { value: T };
            public fn issue() -> Ticket<Int> { Ticket { value: 7 } }
        "#),
        ("demo.api", r#"
            mount demo.capability.Stamp;
            mount demo.tickets.{Ticket, issue};
            public fn issued() -> Ticket<Int> { issue() }
            public fn accept<T: Stamp>(value: T) -> Int { value.stamp() }
        "#),
    ];
    let accepted = check("mount demo.api.{accept, issued}; fn probe() -> Int { accept(issued()) }", &modules);
    assert!(accepted.is_empty(), "{accepted:?}");

    let refused = check(
        "mount demo.api.accept; mount demo.foreign.issue; fn probe() -> Int { accept(issue()) }",
        &modules,
    );
    assert!(refused.iter().any(|error| error.contains("Stamp")), "{refused:?}");
}

fn check_projected_call(extra: &str, expression: &str, result: &str) -> List<Text> {
    let handle = format!(r#"
        mount demo.producer.Produce;
        mount demo.returned.{{Receipt, receipt}};
        public type Handle<T> is {{ data:T }};
        implement<T> Produce for Handle<T> {{
            type Output = Receipt<T>;
            fn produce(self) -> Receipt<T> {{ receipt(self.data) }}
        }}
        public fn handle<T>(data:T) -> Handle<T> {{ Handle {{data}} }}
        {extra}
    "#);
    check(&format!(r#"
        mount demo.driver.{{driver, drive}};
        mount demo.producer.ready;
        fn probe() -> {result} {{ let rt = driver(); {expression} }}
    "#), &[
        ("demo.producer", r#"
            public type Produce is protocol { type Output; fn produce(self) -> Self.Output; };
            public type Ready<T> is { data:T };
            implement<T> Produce for Ready<T> { type Output = T; fn produce(self) -> T { self.data } }
            public fn ready<T>(data:T) -> Ready<T> { Ready {data} }
        "#),
        ("demo.handle", &handle),
        ("demo.returned", r#"
            public type Receipt<T> is { data:T };
            implement<T> Receipt<T> { public fn take(self) -> T { self.data } }
            public fn receipt<T>(data:T) -> Receipt<T> { Receipt {data} }
        "#),
        ("demo.driver", r#"
            mount demo.producer.Produce;
            mount demo.handle.{Handle, handle};
            public type Driver is {};
            implement Driver {
                public fn spawn<F:Produce>(&self, value:F) -> Handle<F.Output> { handle(value.produce()) }
                public fn drive<F:Produce>(&self, value:F) -> F.Output { value.produce() }
            }
            public fn drive<F:Produce>(value:F) -> F.Output { value.produce() }
            public fn driver() -> Driver { Driver {} }
        "#),
    ])
}

#[test]
fn declared_output_owner_survives_an_unrelated_homonym() {
    let other = "public type Other is protocol { type Output; }; implement<T> Other for Handle<T> { type Output = Bool; }";
    for expression in [
        "rt.drive(rt.spawn(ready(42))).take()",
        "drive(rt.spawn(ready(42))).take()",
        "let apply = drive; apply(rt.spawn(ready(42))).take()",
    ] {
        let errors = check_projected_call(other, expression, "Int");
        assert!(errors.is_empty(), "{expression}: {errors:#?}");
    }
}

#[test]
fn declared_output_owner_survives_a_forwarding_blanket() {
    let other = "public type IntoProduce is protocol { type Output; }; implement<F:Produce> IntoProduce for F { type Output = F.Output; }";
    let errors = check_projected_call(other, "rt.drive(rt.spawn(ready(42))).take()", "Int");
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn a_homonymous_output_cannot_change_a_calls_result() {
    let other = "public type Other is protocol { type Output; }; implement<T> Other for Handle<T> { type Output = Bool; }";
    let errors = check_projected_call(other, "rt.drive(rt.spawn(ready(42)))", "Bool");
    assert!(!errors.is_empty(), "foreign associated result was accepted");
}

#[test]
fn converging_receiver_arguments_do_not_merge_their_declared_output_owners() {
    for reverse in [false, true] {
        let first = "public type Measure is protocol { type Value; fn measure(self) -> Self.Value; };";
        let second = "public type Flag is protocol { type Value; fn flag(self) -> Self.Value; };";
        let protocols = if reverse { format!("{second} {first}") } else { format!("{first} {second}") };
        let library = format!(r#"
            {protocols}
            public type Reading is {{}};
            implement Measure for Reading {{ type Value = Int; fn measure(self) -> Int {{ 42 }} }}
            implement Flag for Reading {{ type Value = Bool; fn flag(self) -> Bool {{ true }} }}
            public fn measure<T:Measure>(value:T) -> T.Value {{ value.measure() }}
            public fn flag<T:Flag>(value:T) -> T.Value {{ value.flag() }}
            public fn both<A:Measure,B:Flag>(a:A,b:B) -> (A.Value,B.Value) {{ (a.measure(),b.flag()) }}
        "#);
        for expression in ["both(a,b)", "(measure(a),flag(b))"] {
            let errors = check(&format!(
                "mount demo.reading.{{Reading,measure,flag,both}}; fn probe(a:Reading,b:Reading) -> (Int,Bool) {{ {expression} }}"
            ), &[("demo.reading", &library)]);
            assert!(errors.is_empty(), "{expression}, reverse={reverse}: {errors:#?}");
        }
    }
}

#[test]
fn a_reference_projection_uses_the_reference_implementation() {
    let library = r#"
        public type Measure is protocol { type Value; fn measure(self) -> Self.Value; };
        public type Reading is { data:Int };
        implement Measure for Reading { type Value = Int; fn measure(self) -> Int { self.data } }
        implement Measure for &Reading { type Value = Bool; fn measure(self) -> Bool { true } }
        public fn measure<T:Measure>(value:T) -> T.Value { value.measure() }
    "#;
    for (result, accepted) in [("Bool", true), ("Int", false)] {
        let errors = check(&format!(
            "mount demo.reading.{{Reading,measure}}; fn probe(value:&Reading) -> {result} {{ measure(value) }}"
        ), &[("demo.reading", library)]);
        assert_eq!(errors.is_empty(), accepted, "{result}: {errors:#?}");
    }
}

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

const OPERATIONS: &str = r#"
        public type Never is !;
        public type Try is protocol { type Output; type Residual; };
        public type FromResidual<R> is protocol { fn from_residual(r: R) -> Self; };
    "#;
const CONVERSIONS: &str = r#"
        public type From<T> is protocol { fn from(value: T) -> Self; };
        implement<T> From<T> for T { fn from(value: T) -> T { value } }
    "#;
const RESULT: &str = r#"
        mount core.base.ops.{Try, FromResidual, Never};
        mount core.base.protocols.From;
        public type Result<T,E> is Ok(T) | Err(E);
        implement<T,E> Try for Result<T,E> { type Output = T; type Residual = Result<Never,E>; }
    "#;
const RESIDUAL_IMPL: &str = r#"
        implement<T,E,F> FromResidual<Result<Never,F>> for Result<T,E> where E: From<F> {
            fn from_residual(r: Result<Never,F>) -> Result<T,E> {
                match r { Err(e) => Err(E.from(e)), Ok(never) => never }
            }
        }
    "#;

#[test]
fn a_closed_try_conversion_requires_its_declared_residual_and_error_implementations() {
    let error_impl = "implement From<Failure> for Changed { fn from(v: Failure) -> Changed { Changed { code: v.code } } }";
    for (residual, conversion, admitted) in [
        (RESIDUAL_IMPL, error_impl, true), (RESIDUAL_IMPL, "", false), ("", error_impl, false),
    ] {
        let declaration = format!("{RESULT} {residual}");
        let consumer = format!(r#"
            mount core.base.protocols.From;
            mount core.base.ops.FromResidual;
            mount core.base.result.{{Result, Ok, Err}};
            type Failure is {{ code: Int }};
            type Changed is {{ code: Int }};
            {conversion}
            fn propagate(input: Result<(),Failure>) -> Result<Int,Changed> {{ input?; Ok(41) }}
        "#);
        let errors = check(&consumer, &[
            ("core.base.protocols", CONVERSIONS), ("core.base.ops", OPERATIONS),
            ("core.base.result", &declaration),
        ]);
        if admitted {
            assert!(errors.is_empty(), "{errors:?}");
        } else {
            assert!(errors.iter().any(|e| e.contains("FromResidual")), "{errors:?}");
        }
    }
}

#[test]
fn a_residual_obligation_finds_only_the_blanket_from_its_own_protocol() {
    let result = format!("{RESULT} {RESIDUAL_IMPL}");
    let source = "mount core.base.result.{Result, Ok}; fn forward(x: Result<Int,Int>) -> Result<Int,Int> { Ok(x?) }";
    let modules = |conversions| [
        ("core.base.result", result.as_str()), ("core.base.ops", OPERATIONS),
        ("core.base.protocols", conversions),
        ("foreign.protocols", CONVERSIONS),
    ];
    let errors = check(source, &modules(CONVERSIONS));
    assert!(errors.is_empty(), "{errors:?}");
    let missing = "public type From<T> is protocol { fn from(value: T) -> Self; };";
    let errors = check(&format!("mount foreign.protocols.From; {source}"), &modules(missing));
    assert!(errors.iter().any(|error| error.contains("FromResidual")), "{errors:?}");
}

#[test]
fn callable_results_retain_their_declaring_sum_through_type_composition() {
    let result = format!("{RESULT} {RESIDUAL_IMPL}");
    let modules = [
        ("core.base.result", result.as_str()), ("core.base.ops", OPERATIONS),
        ("core.base.protocols", CONVERSIONS),
    ];
    let imports = "mount core.base.result.{Result, Ok};";
    for body in [
        "fn call<E,F: fn(Int)->Result<Int,E>>(f:F)->Result<Int,E> { Ok(f(3)?) }",
        "type Response<T> is Result<T,Int>; fn call(f:fn()->Response<Int>)->Result<Int,Int> { Ok(f()?) }",
        "fn call(f:fn()->Result<Result<Int,Int>,Int>)->Result<Int,Int> { let v=f()?; Ok(v?) }",
        "fn call(f:fn()->(Result<Int,Int>,Bool))->Result<Int,Int> { let (v,_) = f(); Ok(v?) }",
        "fn call(f:fn<T>(T)->Result<T,Int>)->Result<Int,Int> { Ok(f(3)?) }",
        "fn read()->Result<Int,Int> { Ok(4) } type S is protocol { async fn go(&mut self)->Result<Int,Int>; }; type L is {}; implement S for L { async fn go(&mut self)->Result<Int,Int> { Ok(read()?) } }",
    ] {
        let errors = check(&format!("{imports} {body}"), &modules);
        assert!(errors.is_empty(), "{body}: {errors:?}");
    }
    let foreign = "type Shadow<A,E> is Ok(A) | Err(E); fn wrong(f:fn()->Shadow<Int,Int>)->Result<Int,Int> { Ok(f()?) }";
    let errors = check(&format!("{imports} {foreign}"), &modules);
    assert!(errors.iter().any(|error| error.contains("NotResultOrMaybe")), "{errors:?}");
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
    let model = "public type Receipt<T> is { value: T }; implement<T> Receipt<T> { public fn read(&self) -> T { self.value } private fn hidden(&self) -> T { self.value } }";
    let factory = "mount demo.model.{Receipt as Issued}; public fn issue<T>(value: T) -> Issued<T> { Issued { value } } public fn deliver(decoy: Int) -> Bool { true }";
    let other = "public type Issued<T> is { value: Bool }; implement<T> Issued<T> { public fn read(&self) -> Bool { self.value } }";
    let mut modules = [
        ("demo.model", model),
        ("demo.factory", factory),
        ("demo.other", other),
        ("demo.api", "public mount demo.factory.issue;"),
    ];
    for reverse in [false, true] {
        if reverse { modules.reverse(); }
        for (import, call) in [
            (format!("mount {entry}.issue;"), "issue"),
            (format!("mount {entry}.{{issue as deliver}};"), "deliver"),
        ] {
            let imports = format!("mount demo.other.Issued; {import}");
            for (method, result, expected) in [
                ("read", "Int", None),
                ("read", "Bool", Some("Mismatch")),
                ("hidden", "Int", Some("MethodNotFound")),
            ] {
                let source = format!("{imports} fn probe() -> {result} {{ {call}(37).{method}() }}");
                let (mut checker, errors) = check_with_checker(&source, &modules);
                match expected {
                    None => {
                        assert!(errors.is_empty(), "{source}: {errors:?}");
                        assert_eq!(
                            checker.take_resolved_mount_bindings().get(&Text::from(call)).map(Text::as_str),
                            Some("demo.factory.issue"), "{source}",
                        );
                    }
                    Some(kind) => {
                        assert_eq!(errors.len(), 1, "{source}: {errors:?}");
                        assert!(errors[0].contains(kind), "{source}: {errors:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn renamed_callable_retains_its_own_default_argument_count() {
    let modules = [("demo.provider", "public fn issue(value: Int = 73) -> Int { value } public fn deliver(left: Int, right: Int) -> Bool { true }")];
    let mount = "mount demo.provider.{issue as deliver};";
    let ok = check(&format!("{mount} fn read() -> Int {{ deliver() }}"), &modules);
    assert!(ok.is_empty(), "{ok:?}");
    let wrong = check(&format!("{mount} fn read() -> Int {{ deliver(1, 2) }}"), &modules);
    assert_eq!(wrong.len(), 1, "{wrong:?}");
    assert!(wrong[0].contains("E102") && wrong[0].contains("at most 1 argument"), "{wrong:?}");
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

#[test]
fn source_deref_uses_the_declared_target_and_requires_mutable_evidence_for_write() {
    let modules = [
        ("core.base.protocols", r#"
            public type Deref is protocol { type Target; fn deref(&self) -> &Self.Target; };
            public type DerefMut is protocol extends Deref { fn deref_mut(&mut self) -> &mut Self.Target; };
        "#),
        ("demo.foreign", "public type Deref is protocol { type Target; };"),
        ("demo.locks", r#"
            mount core.base.protocols.{Deref,DerefMut};
            mount demo.foreign.{Deref as ForeignDeref};
            public type Guard<Tag,Item> is { tag: Tag, value: Item };
            implement<Tag,Item> Deref for Guard<Tag,Item> {
                type Target = Item; fn deref(&self) -> &Item { &self.value }
            }
            implement<Item> DerefMut for Guard<Int,Item> {
                fn deref_mut(&mut self) -> &mut Item { &mut self.value }
            }
            public fn readonly() -> Guard<Text,Bool> { Guard { tag: "read", value: true } }
            public fn writable() -> Guard<Int,Bool> { Guard { tag: 1, value: false } }
            public type Payload is { public bit: Bool };
            public fn shared_field() -> Guard<Text,Payload> { Guard { tag: "r", value: Payload { bit: true } } }
            public fn mutable_field() -> Guard<Int,Payload> { Guard { tag: 0, value: Payload { bit: true } } }
            public type Other<T> is { value: T };
            implement<T> ForeignDeref for Other<T> { type Target = T; }
            public fn other() -> Other<Bool> { Other { value: false } }
        "#),
    ];
    let header = "mount demo.locks.{readonly,writable,shared_field,mutable_field,other};";
    let accepted = check(
        &format!("{header} fn read()->Bool {{ *readonly() }} fn write()->Bool {{ let mut g=writable(); *g=true; *g }} fn field() {{ let mut p=mutable_field(); p.bit=false; }}"),
        &modules,
    );
    assert!(accepted.is_empty(), "{accepted:?}");
    for (body, reason) in [
        ("fn wrong()->Text { *readonly() }", "Mismatch"),
        ("fn sibling()->Bool { *other() }", "Cannot dereference"),
        ("fn wrong() { let mut g=readonly(); *g=true; }", "DerefMut"),
        ("fn field() { let mut p=shared_field(); p.bit=false; }", "DerefMut"),
    ] {
        let errors = check(&format!("{header} {body}"), &modules);
        assert!(errors.iter().any(|error| error.contains(reason)), "{body}: {errors:?}");
    }
}

#[test]
fn inherent_calls_must_prove_the_declaring_implementations_conditions() {
    let declarations = "
        type Writable is protocol { fn cookie(&self) -> Int; };
        implement Writable for Int { fn cookie(&self) -> Int { 67 } }
        implement<T: Writable> [T] { public fn stamp(&self) -> Int { 71 } }
    ";
    let valid = check_with_checker(&format!("{declarations} fn use_int(xs: &[Int]) -> Int {{ xs.stamp() }}"), &[]).1;
    assert!(valid.is_empty(), "{valid:?}");
    let invalid = check_with_checker(&format!("{declarations} fn use_bool(xs: &[Bool]) -> Int {{ xs.stamp() }}"), &[]).1;
    assert!(invalid.iter().any(|error| error.contains("E405")), "{invalid:?}");
}

#[test]
fn a_type_mounted_through_a_module_alias_keeps_its_static_method_owner() {
    let modules = [
        ("demo.tickets", r#"
            public type Ticket<T> is { public value: T };
            implement<T> Ticket<T> { public fn wrap(item:T)->Ticket<T> { Ticket { value:item } } }
        "#),
        ("demo.other", r#"
            public type Ticket<T> is { item:T };
            implement<T> Ticket<T> { public fn foreign(item:T)->Bool { true } }
        "#),
    ];
    let aliases = [("old.tickets", "demo.tickets")];
    let header = "mount demo.other.{Ticket as Other}; mount old.tickets.Ticket;";
    for (body, accepted) in [
        ("fn use_it() -> Bool { Ticket.wrap(true).value }", true),
        ("fn wrong() -> Int { Ticket.wrap(true).value }", false),
        ("fn wrong() -> Bool { Ticket.foreign(true) }", false),
    ] {
        let errors = check_with_module_aliases(&format!("{header} {body}"), &modules, &aliases).1;
        assert_eq!(errors.is_empty(), accepted, "{body}: {errors:?}");
    }
}

#[test]
fn imported_structural_methods_keep_their_element_and_visibility() {
    let provider = "implement<T> [T] { public fn sample(&self, item: T) -> T { item } private fn hidden(&self) -> Int { 42 } }";
    let modules = [("demo.slices", provider)];
    let positive = check_with_checker(
        "mount demo.slices.*; fn read(items: &[Int]) -> Int { items.sample(31) }", &modules,
    ).1;
    assert!(positive.is_empty(), "{positive:?}");
    for consumer in [
        "mount demo.slices.*; fn read(items: &[Int]) -> Int { items.sample(true) }",
        "mount demo.slices.*; fn read(items: &[Int]) -> Bool { items.sample(31) }",
        "mount demo.slices.*; fn read(items: &[Int]) -> Int { items.hidden() }",
        "fn read(items: &[Int]) -> Int { items.sample(31) }",
    ] {
        let errors = check_with_checker(consumer, &modules).1;
        assert!(!errors.is_empty(), "{consumer}");
    }
}

#[test]
fn structural_method_mounts_refuse_conflicts_and_incompatible_carriers() {
    let generic = "implement<T> [T] { public fn stamp(&self) -> Int { 1 } }";
    let numbers = "implement [Int] { public fn numeric(&self) -> Int { 1 } }";
    for modules in [
        [("demo.first", generic), ("demo.second", generic)],
        [("demo.second", generic), ("demo.first", generic)],
    ] {
        let errors = check_with_checker(
            "mount demo.first.*; mount demo.second.*; fn f(x: &[Int]) -> Int { x.stamp() }",
            &modules,
        ).1;
        assert!(errors.iter().any(|diagnostic| diagnostic.contains("AmbiguousName") && diagnostic.contains("demo.first") && diagnostic.contains("demo.second")), "{errors:?}");
    }
    let ok = check_with_checker("mount demo.ext.*; fn f(x: &[Int]) -> Int { x.numeric() }", &[("demo.ext", numbers)]).1;
    assert!(ok.is_empty(), "{ok:?}");
    let errors = check_with_checker("mount demo.ext.*; fn f(x: &[Bool]) -> Int { x.numeric() }", &[("demo.ext", numbers)]).1;
    assert!(errors.iter().any(|diagnostic| diagnostic.contains("MethodNotFound")), "{errors:?}");
}

#[test]
fn a_structural_method_requires_its_receivers_declared_conditions() {
    let provider = "
        public type Writable is protocol { fn cookie(&self) -> Int; };
        implement Writable for Int { fn cookie(&self) -> Int { 67 } }
        implement<T: Writable> [T] { public fn stamp(&self) -> Int { 71 } }
    ";
    let modules = [("demo.ext", provider)];
    let good = check_with_checker("mount demo.ext.*; fn read(xs: &[Int]) -> Int { xs.stamp() }", &modules).1;
    assert!(good.is_empty(), "{good:?}");
    let bad = check_with_checker("mount demo.ext.*; fn read(xs: &[Bool]) -> Int { xs.stamp() }", &modules).1;
    assert!(bad.iter().any(|diagnostic| diagnostic.contains("E405")), "{bad:?}");
}

#[test]
fn imported_sync_callbacks_follow_only_their_declared_callees_and_atomic_operands() {
    let modules = [
        ("demo.ops", r#"
            public fn observed<T>(cell: &T) -> T { @intrinsic("atomic_load", cell, 0) }
            public fn update<T>(cell: &mut T, value:T) { @intrinsic("atomic_store", cell, value, 0) }
            public async fn suspended(cell: &Int) -> Int { *cell }
        "#),
        ("demo.jobs", r#"
            mount demo.ops.{observed as fetch, update};
            public type Counter is { count:Int };
            implement Counter {
                public fn current(&self) -> Int { fetch(&self.count) }
                public fn set(&mut self, value:Int) { update(&mut self.count, value); }
                public fn parameter(&self, fetch:async fn(&Int)->Int) -> Int { fetch(&self.count) }
                public fn shadow(&self, cb:async fn(&Int)->Int) -> Int { let fetch = cb; fetch(&self.count) }
            }
        "#),
    ];
    let header = "mount demo.jobs.Counter; fn sync<F:fn(Counter)->Int>(c:Counter, handler:F)->Int { handler(c) }";
    let valid = check(&format!("{header} fn read(c:Counter)->Int {{ sync(c, |item| item.current()) }}"), &modules);
    assert!(valid.is_empty(), "{valid:?}");
    for call in ["item.parameter(cb)", "item.shadow(cb)"] {
        let errors = check(&format!("{header} fn wrong(c:Counter, cb:async fn(&Int)->Int)->Int {{ sync(c, |item| {call}) }}"), &modules);
        assert!(errors.iter().any(|error| error.contains("Mismatch")), "{call}: {errors:?}");
    }
    for (body, reason) in [
        (r#"fn wrong(a:&Bool)->Int { @intrinsic("atomic_load", a, 0) }"#, "Mismatch"),
        (r#"fn wrong(a:&mut Bool) { @intrinsic("atomic_store", a, 3, 0); }"#, "Mismatch"),
        (r#"fn wrong(a:&Bool) { @intrinsic("atomic_store", a, true, 0); }"#, "SharedReferenceWrite"),
    ] {
        let errors = check(body, &[]);
        assert!(errors.iter().any(|error| error.contains(reason)), "{body}: {errors:?}");
    }
}

#[test]
fn a_selected_protocol_method_projects_its_item_inside_variant_payloads() {
    let modules = [("demo.streams", r#"
        public type Stage<T> is | Pending | Ready(T);
        public type Message<T> is | Empty | Value(T);
        public type Pollable is protocol {
            type Item;
            fn pull(&self) -> Stage<Message<Self.Item>> { Stage.Pending }
        };
        public type Other is protocol { type Item; };
        public type Handle<T> is { item:T };
        implement<T> Other for Handle<T> { type Item = Int; }
        implement<T> Pollable for Handle<T> { type Item = T; }
        public fn obtain<T>(item:T)->Handle<T> { Handle { item } }
    "#)];
    let header = "mount demo.streams.{Stage,Message,Pollable,Other,obtain};";
    for (wanted, otherwise, accepted) in [("Text", r#""""#, true), ("Int", "0", false)] {
        let body = format!(r#"{header} fn read() -> {wanted} {{
            match obtain("ok").pull() {{
                Stage.Ready(Message.Value(value)) => value,
                _ => {otherwise},
            }}
        }}"#);
        let errors = check(&body, &modules);
        assert_eq!(errors.is_empty(), accepted, "{body}: {errors:?}");
    }
}

#[test]
fn implementation_headers_and_bodies_share_the_receivers_declared_parameters() {
    let declarations = r#"
        type Info is { code:Int };
        type Window<T> is { value:T };
        type Heading is protocol { fn heading(&self)->Int; };
        type Extent is protocol { fn length(&self)->Int; };
        implement Heading for Window<Info> { fn heading(&self)->Int { self.value.code } }
        implement Extent for Window<Info> { fn length(&self)->Int { self.heading() } }
        implement Heading for Window<Int8> { fn heading(&self)->Int { self.value as Int } }
        implement Extent for Window<Int8> { fn length(&self)->Int { self.heading() } }
    "#;
    let valid = check(declarations, &[]);
    assert!(valid.is_empty(), "{valid:?}");
    let different = check(&format!("{declarations} type Twin is {{ code:Int }}; fn bad(x:Window<Twin>)->Int {{ x.length() }}"), &[]);
    assert!(different.iter().any(|error| error.contains("MethodNotFound")), "{different:?}");
    let collision = check(r#"
        type T is { code:Int };
        type Storage<T> is { data:T };
        type Read is protocol { fn read(&self)->Int; };
        implement Read for Storage<T> { fn read(&self)->Int { self.data.code } }
        fn good(x:Storage<T>)->Int { x.read() }
    "#, &[]);
    assert!(collision.is_empty(), "{collision:?}");
    let unknown_bound = check(r#"
        type Job<F> is { action:F }; type Run is protocol {};
        implement<F> Run for Job<F> where F:fn(Int)->Z {}
    "#, &[]);
    assert!(unknown_bound.iter().any(|error| error.contains("Z") && error.contains("TypeNotFound")), "{unknown_bound:?}");
    let implicit = check("type Boxed<T> is { value:T }; implement Boxed<T> { fn item(&self)->&T { &self.value } }", &[]);
    assert!(implicit.is_empty(), "{implicit:?}");
    let repeated = check(r#"
        type Pair<A,B> is { first:A, second:B };
        implement Pair<B,B> { fn item(&self)->&B { &self.first } }
    "#, &[]);
    assert!(repeated.is_empty(), "{repeated:?}");
    for missing in ["Missing", "Z"] {
        for header in [
            format!("implement Heading for Window<{missing}> {{ fn heading(&self)->Int {{ 0 }} }}"),
            format!("implement Window<{missing}> {{ fn bad(&self)->Int {{ 0 }} }}"),
        ] {
            let errors = check(&format!("{declarations} {header}"), &[]);
            assert!(errors.iter().any(|error| error.contains(missing) && error.contains("TypeNotFound")), "{header}: {errors:?}");
        }
    }
}

#[test]
fn imported_concrete_receivers_do_not_bind_a_same_leaf_type_from_the_consumer() {
    let modules = [
        ("demo.store", r#"
            public type Key is { public value:Int };
            public type Shelf<Element> is { item:Element };
            implement Shelf<Key> { public fn read(&self)->Int { self.item.value } }
            implement Shelf<Element> { public fn get(&self)->&Element { &self.item } }
        "#),
        ("demo.other", "public type Key is { public value:Int };"),
    ];
    let positive = check(
        "mount demo.store.{Shelf,Key}; fn read(x:Shelf<Key>)->Int { x.read() + x.get().value }",
        &modules,
    );
    assert!(positive.is_empty(), "{positive:?}");
    let negative = check(
        "mount demo.store.Shelf; mount demo.other.Key; fn read(x:Shelf<Key>)->Int { x.read() }",
        &modules,
    );
    assert!(negative.iter().any(|error| error.contains("Mismatch")
        && error.contains("demo.other.Key") && error.contains("demo.store.Key")), "{negative:?}");
}

#[test]
fn chained_protocol_results_keep_the_imported_sum_and_its_payload() {
    let packet = "public type Packet<T> is Empty | Wrap(T);
        implement<T> Packet<T> {
            public fn populated(&self) -> Bool { match self { Wrap(_) => true, Empty => false } }
            public fn transform<U, F: fn(T) -> U>(self, f: F) -> Packet<U> {
                match self { Wrap(value) => Packet.Wrap(f(value)), Empty => Packet.Empty }
            }
        }";
    let other = "public type Packet<T> is Empty | Wrap(T); implement<T> Packet<T> { public fn foreign(&self) -> Bool { true } }";
    let modules = [("demo.payload", packet), ("demo.other", other)];
    let source = "mount demo.payload.Packet; type Feed is protocol {
        type Element; fn pull(&mut self) -> Packet<Self.Element>;
        fn populated(mut self) -> Bool { self.pull().populated() }
    };
    type Numbers is {};
    implement Feed for Numbers { type Element = Int; fn pull(&mut self) -> Packet<Int> { Packet.Wrap(3) } }";
    let accepted = format!("{source} fn read(n: &mut Numbers) -> Bool {{ n.pull().transform(|v| v + 1).populated() }}");
    let errors = check_with_checker(&accepted, &modules).1;
    assert!(errors.is_empty(), "{errors:?}");
    let rejected = format!("{source} fn read(n: &mut Numbers) -> Bool {{ n.pull().foreign() }}");
    let errors = check_with_checker(&rejected, &modules).1;
    assert!(errors.iter().any(|diagnostic| diagnostic.contains("MethodNotFound") && diagnostic.contains("foreign")), "{errors:?}");
}

#[test]
fn imported_primitive_impls_use_the_extensions_private_type_mount() {
    let sources = [
        ("fixture.payload", "public type Parcel is { public amount: Int };"),
        ("fixture.decoy", "public type Parcel is { public wrong: Bool };"),
        ("fixture.ext", r#"
            mount fixture.payload.Parcel as LocalReceipt;
            implement Int {
                public fn receipt_for_value(self) -> LocalReceipt { LocalReceipt { amount: self } }
            }
            public fn activate() -> Int { 1 }
        "#),
    ];
    let prefix = "mount fixture.decoy.Parcel as LocalReceipt; mount fixture.ext.activate;";
    let accepted = format!("{prefix} fn read(i: Int) -> Int {{ i.receipt_for_value().amount }}");
    let errors = check(&accepted, &sources);
    assert!(errors.is_empty(), "{errors:?}");

    let rejected = format!("{prefix} fn read(i: Int) -> Bool {{ i.receipt_for_value().wrong }}");
    let errors = check(&rejected, &sources);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("UnknownField")
            && errors[0].contains("fixture.payload.Parcel")
            && errors[0].contains("wrong"),
        "{errors:?}"
    );
}

#[test]
fn imported_for_loop_items_follow_into_iterator_even_with_another_item_first() {
    let sources = [
        ("core.base.iterator", "public type IntoIterator is protocol { type Item; type IntoIter; fn into_iter(self) -> Self.IntoIter; };"),
        ("fixture.batch", r#"
            mount core.base.iterator.IntoIterator;
            public type Other is protocol { type Item; };
            public type Payload is { public amount: Int };
            public type Batch<T> is { item: T };
            implement<T> Other for Batch<T> { type Item = Bool; }
            implement<T> IntoIterator for Batch<T> {
                type Item = T;
                type IntoIter = Self;
                fn into_iter(self) -> Self { self }
            }
            public fn batch() -> Batch<Payload> { Batch { item: Payload { amount: 8 } } }
        "#),
    ];
    let source = "mount fixture.batch.batch; fn read() -> Int { let mut value = 0; for item in batch() { value = item.amount; } value }";
    let errors = check(source, &sources);
    assert!(errors.is_empty(), "{errors:?}");

    let errors = check(&source.replace("item.amount", "item.foreign"), &sources);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("UnknownField")
            && errors[0].contains("fixture.batch.Payload")
            && errors[0].contains("foreign"),
        "{errors:?}"
    );
}

#[test]
fn a_nominal_with_an_unrelated_item_member_is_not_an_iterable() {
    let sources = [("fixture.pretend", r#"
        public type Other is protocol { type Item; };
        public type RecordIter is { public n: Int };
        implement Other for RecordIter { type Item = Int; }
        public fn record() -> RecordIter { RecordIter { n: 9 } }
    "#)];
    for body in [
        "for _ in record() {}",
        "let _ = [n for n in record()];",
    ] {
        let errors = check(&format!("mount fixture.pretend.record; fn check_record() {{ {body} }}"), &sources);
        assert!(errors.iter().any(|error| error.contains("E405") && error.contains("fixture.pretend.RecordIter")), "{body}: {errors:?}");
    }
}

#[test]
fn bound_range_literals_and_foreign_nominals_keep_distinct_iteration_semantics() {
    let source = "fn total() -> Int { let domain = 2..5; let inclusive = 1..=3; let mut sum = 0;
        for n in domain { sum = sum + n; } for n in inclusive { sum = sum + n; } sum }";
    let errors = check(source, &[]);
    assert!(errors.is_empty(), "{errors:?}");
    let modules = [("fixture.foreign", "public type Range<T> is { public value: T }; public fn range() -> Range<Int> { Range { value: 8 } }")];
    let errors = check("mount fixture.foreign.range; fn rejected() { for _ in range() {} }", &modules);
    assert!(errors.iter().any(|e| e.contains("E405") && e.contains("fixture.foreign.Range")), "{errors:?}");
}
