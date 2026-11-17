#![cfg(feature = "codegen")]

use verum_ast::Module;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, CodegenResult, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter, module::VbcModule, types::TypeRef,
};

fn parse(source: &str) -> Module {
    Parser::new(source).parse_module().expect("parse declared equality source")
}

fn compile(body: &str) -> CodegenResult<VbcModule> {
    let provider = parse(r#"
        module provider;
        public type Outcome<T> is Idle | Selected(T);
        public type Family is protocol { type Item; };
    "#);
    let consumer = parse(&format!(r#"
        module consumer;
        mount provider.{{Outcome, Family}};
        type Rival<T> is Selected(T) | Idle;
        type Shell<T> is {{ item: T }};
        fn consume(value: Int) -> Int {{ value }}
        {body}
    "#));
    let files = [&provider, &consumer];
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    codegen.collect_unit_declarations(&files)?;
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(&files, ItemFailurePolicy::Strict)?;
    codegen.finalize_module_from_state()
}

fn source_and_wire(module: VbcModule) -> [VbcModule; 2] {
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    ).unwrap();
    [module, wire]
}

#[test]
fn an_exact_equality_refines_payloads_without_rewriting_the_published_parameter() {
    let module = compile(r#"
        fn choose<T>(item: T) -> Int where T = Outcome<Int> {
            match item { Outcome.Selected(value) => consume(value), Outcome.Idle => 0 }
        }
        fn run() -> Int { choose(Outcome.Selected(37)) }
    "#).unwrap();
    for module in source_and_wire(module) {
        let choose = module.functions.iter().find(|function|
            module.get_string(function.name) == Some("consumer.choose")).unwrap();
        assert!(matches!(choose.semantic_parameter_types().unwrap(), [Some(TypeRef::Generic(_))]),
            "the published parameter retains its source binder");
        let entry = module.find_function_by_name("consumer.run").unwrap();
        assert_eq!(Interpreter::new(module.into()).execute_function(entry).unwrap().try_as_i64(), Some(37));
    }
}

#[test]
fn declared_aliases_normalize_inside_equations_without_erasing_nominal_owners() {
    let module = compile(r#"
        type Alias<T> is Outcome<T>;
        type Export<U> is Alias<U>;
        fn choose<T>(item: T) -> Int where T = Export<Int>, T = Outcome<Int> {
            match item { Outcome.Selected(value) => consume(value), Outcome.Idle => 0 }
        }
        fn run() -> Int { choose(Outcome.Selected(39)) }
    "#).unwrap();
    for module in source_and_wire(module) {
        let choose = module.get_function(module.find_function_by_name("consumer.choose").unwrap()).unwrap();
        assert!(matches!(choose.semantic_parameter_types().unwrap(), [Some(TypeRef::Generic(_))]));
        let entry = module.find_function_by_name("consumer.run").unwrap();
        assert_eq!(Interpreter::new(module.into()).execute_function(entry).unwrap().try_as_i64(), Some(39));
    }
    for declaration in [
        "type Alias<T> is Rival<T>;",
        "type Alias<T> is (Outcome<T>);",
    ] {
        let error = compile(&format!(r#"
            {declaration}
            fn rejected<T>(item: T) -> Int where T = Alias<Int>, T = Outcome<Int> {{ 0 }}
        "#)).err().expect("an alias target and a nominal wrapper retain their own declarations");
        assert!(matches!(error.kind, verum_vbc::codegen::CodegenErrorKind::InvalidSourceSignature(_)), "{error}");
    }
}

#[test]
fn projected_and_transitive_equalities_keep_the_exact_subject() {
    let module = compile(r#"
        fn projected<F: Family>(item: F.Item) -> Int where F.Item = Outcome<Int> {
            match item { Outcome.Selected(value) => consume(value), Outcome.Idle => 0 }
        }
        fn transitive<T, U>(item: T) -> Int where T = U, U = Outcome<Int> {
            match item { Outcome.Selected(value) => consume(value), Outcome.Idle => 0 }
        }
        fn run() -> Int { transitive<Outcome<Int>, Outcome<Int>>(Outcome.Selected(41)) }
    "#).unwrap();
    for module in source_and_wire(module) {
        let projected = module.functions.iter().find(|function|
            module.get_string(function.name) == Some("consumer.projected")).unwrap();
        assert!(matches!(projected.semantic_parameter_types().unwrap(),
            [Some(TypeRef::AssociatedProjection { assoc, .. })] if assoc == "Item"));
        let entry = module.find_function_by_name("consumer.run").unwrap();
        assert_eq!(Interpreter::new(module.into()).execute_function(entry).unwrap().try_as_i64(), Some(41));
    }
}

#[test]
fn missing_or_unrelated_equalities_cannot_supply_a_pattern_payload() {
    for header in [
        "fn rejected<T>(item: T) -> Int",
        "fn rejected<T, U>(item: T) -> Int where U = Outcome<Int>",
        "fn rejected<F: Family, G: Family>(item: F.Item) -> Int where G.Item = Outcome<Int>",
    ] {
        let error = compile(&format!(r#"
            {header} {{
                match item {{ Outcome.Selected(value) => consume(value), Outcome.Idle => 0 }}
            }}
        "#)).err().expect("an unproved subject must not borrow the ambient constructor's payload").to_string();
        assert!(error.contains("source type") || error.contains("no declared tag"), "{header}: {error}");
    }
}

#[test]
fn conflicting_and_recursive_equations_are_refused_before_body_use() {
    for (premises, expected) in [
        ("T = Outcome<Int>, T = Rival<Int>", "conflicting"),
        ("T = Outcome<Int>, T = Outcome<Bool>", "conflicting"),
        ("T = Shell<T>", "cyclic"),
        ("T = U, U = Shell<T>", "cyclic"),
    ] {
        let error = compile(&format!("fn rejected<T, U>(item: T) -> Int where {premises} {{ 0 }}"))
            .err().expect("invalid equations must not publish body evidence").to_string();
        assert!(error.contains(expected) && error.contains("declared type equalit"), "{premises}: {error}");
    }
}

#[test]
fn a_method_equality_does_not_escape_into_a_sibling_body() {
    let error = compile(r#"
        implement<T> Shell<T> {
            fn proved(&self) -> Int where T = Outcome<Int> {
                match self.item { Outcome.Selected(value) => consume(value), Outcome.Idle => 0 }
            }
            fn unproved(&self) -> Int {
                match self.item { Outcome.Selected(value) => consume(value), Outcome.Idle => 0 }
            }
        }
    "#).err().expect("the next method must not inherit the previous method's equality").to_string();
    assert!(error.contains("source type") || error.contains("no declared tag"), "{error}");
    assert!(error.contains("unproved"), "the proved body must compile first: {error}");
}

#[test]
fn an_impossible_default_is_omitted_without_discarding_an_open_or_matching_default() {
    let module = compile(r#"
        type Access is protocol {
            type Item;
            fn borrowed<T>(&self) -> Int where Self.Item = &T { 9 }
            fn ordinary(&self) -> Int { 7 }
        };
        type Scalar is { item: Int };
        implement Access for Scalar { type Item = Int; }
        type Borrowed<T> is { item: &T };
        implement<T> Access for Borrowed<T> { type Item = &T; }
        implement<T> Access for Shell<T> { type Item = T; }
        fn run() -> Int { let scalar = Scalar { item: 1 }; scalar.ordinary() }
    "#).unwrap();
    for module in source_and_wire(module) {
        let exists = |suffix: &str| module.functions.iter().any(|function|
            module.get_string(function.name).is_some_and(|name| name.ends_with(suffix)));
        assert!(!exists("Scalar.borrowed"));
        assert!(exists("Borrowed.borrowed"));
        assert!(exists("Shell.borrowed"));
        let entry = module.find_function_by_name("consumer.run").unwrap();
        assert_eq!(Interpreter::new(module.into()).execute_function(entry).unwrap().try_as_i64(), Some(7));
    }
}

#[test]
fn exact_function_equality_constrains_the_body_without_becoming_a_capability() {
    let module = compile(r#"
        fn invoke<F>(callback: F) -> Int where F = fn(Int) -> Int { callback(5) }
        fn add(value: Int) -> Int { value + 42 }
        fn run() -> Int { invoke(add) }
    "#).unwrap();
    for module in source_and_wire(module) {
        let entry = module.find_function_by_name("consumer.run").unwrap();
        assert_eq!(Interpreter::new(module.into()).execute_function(entry).unwrap().try_as_i64(), Some(47));
    }
    let error = compile(r#"
        fn contradictory<F>(callback: F) -> Int
        where F = fn(Int) -> Int, F = fn(Int) -> Bool { 0 }
    "#).err().expect("exact function equations must not be ignored").to_string();
    assert!(error.contains("conflicting or unsupported declared type equality"), "{error}");
}
