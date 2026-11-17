//! T2860/T2869: exact callable equations justify checked bodies and callers.
use verum_ast::{ItemKind, Module};
use verum_common::{List, Shared, Text};
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, core_metadata::*};

const OWNER: &str = "callable.provider";
const DECLARATION: &str = "public fn invoke<F>(callback:F)->Int where F=fn(Int)->Int {callback(5)}";
const POSITIVE: &str = "fn add(value:Int)->Int {value+42} fn run()->Int {invoke(add)}";
const NEGATIVE: &str = "fn wrong(value:Bool)->Int {42} fn run()->Int {invoke(wrong)}";

fn parse(source: &str) -> Module {
    Parser::new(source).parse_module().expect("source grammar")
}

fn errors(checker: &mut TypeChecker, source: &str) -> List<Text> {
    let module = parse(source);
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    errors
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|error| format!("{error:?}").into())
        .collect()
}

fn source_errors(source: &str) -> List<Text> {
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path(OWNER);
    errors(&mut checker, source)
}

#[track_caller]
fn rejects(errors: &[Text]) {
    assert!(
        errors.iter().any(|error| error.contains("Mismatch")
            || error.contains("E405")
            || error.contains("equality")
            || error.contains("RigidTypeVariable")),
        "{errors:#?}"
    );
}

#[test]
fn declared_exact_callable_equation_justifies_only_its_own_body_and_calls() {
    let positive = source_errors(&format!("{DECLARATION} {POSITIVE}"));
    assert!(positive.is_empty(), "checked exact body: {positive:#?}");
    rejects(&source_errors(&format!("{DECLARATION} {NEGATIVE}")));
    rejects(&source_errors(&format!(
        "{DECLARATION} async fn asynchronous(value:Int)->Int {{value}} fn run()->Int {{invoke(asynchronous)}}"
    )));
    rejects(&source_errors(&format!(
        "{DECLARATION} fn add(value:Int)->Int {{value}} fn run()->Int {{invoke<async fn(Int)->Int>(add)}}"
    )));
    rejects(&source_errors(
        "fn wrong_body<F>(callback:F)->Int where F=fn(Int)->Int {callback(true)}",
    ));
    let unproved = "fn unproved<F>(callback:F)->Int {callback(5)}";
    rejects(&source_errors(&format!("{DECLARATION} {unproved}")));
}

fn metadata() -> CoreMetadata {
    let checked = source_errors(DECLARATION);
    assert!(
        checked.is_empty(),
        "body must check before publication: {checked:#?}"
    );
    let declaration = parse(DECLARATION);
    let ItemKind::Function(function) = &declaration.items[0].kind else {
        panic!("function declaration");
    };
    let signature = verum_ast::source_signature::source_signature(function, false)
        .unwrap()
        .expect("actual source producer retains the equation");
    assert!(signature.body.is_none());
    let mut metadata = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..Default::default()
    };
    metadata.functions.insert(
        format!("{OWNER}.invoke").into(),
        FunctionDescriptor {
            name: "invoke".into(),
            module_path: OWNER.into(),
            origin_module_path: None,
            source_signature: Some(signature),
            generic_params: List::new(),
            params: List::from_single(ParamDescriptor {
                name: "callback".into(),
                ty: "__opaque_src".into(),
                declared_ty: "__opaque_src".into(),
                has_default: false,
                default_literal: None,
            }),
            return_type: "Int".into(),
            contexts: List::new(),
            is_async: false,
            is_unsafe: false,
            intrinsic_id: None,
            parent_type: None,
            impl_generic_names: List::new(),
            is_const: false,
            decl_span: None,
            is_public: true,
            explicit_type_param_ids: None,
        },
    );
    decode_metadata(&encode_metadata(&metadata).unwrap()).unwrap()
}

#[test]
fn checked_callable_body_publishes_the_same_exact_obligation_to_metadata_callers() {
    let metadata = Shared::new(metadata()).into_arc();
    for eager in [false, true] {
        for (source, valid) in [(POSITIVE, true), (NEGATIVE, false)] {
            let mut checker = if eager {
                TypeChecker::new_with_core_eager(metadata.clone())
            } else {
                TypeChecker::new_with_core(metadata.clone())
            };
            checker.register_primitives();
            checker.set_current_module_path("callable.client");
            let failures = errors(
                &mut checker,
                &source.replace("invoke", &format!("{OWNER}.invoke")),
            );
            if valid {
                assert!(failures.is_empty(), "eager={eager}: {failures:#?}");
            } else {
                rejects(&failures);
            }
        }
    }
}

#[test]
fn equations_seed_named_function_values_and_selected_method_arguments() {
    for caller in [
        "fn add(value:Int)->Int {value+42} fn run()->Int {let selected=invoke; selected(add)}",
        "fn add(value:Int)->Int {value+42} fn run()->Int {invoke<fn(Int)->Int>(add)}",
    ] {
        let failures = source_errors(&format!("{DECLARATION} {caller}"));
        assert!(failures.is_empty(), "{caller}: {failures:#?}");
    }
    let failures = source_errors(
        r#"
        type Holder is ();
        implement Holder {
            fn invoke<F>(&self,callback:F)->Int where F=fn(Int)->Int {callback(5)}
        }
        fn add(value:Int)->Int {value+42}
        fn run(holder:&Holder)->Int {holder.invoke(add)}
    "#,
    );
    assert!(failures.is_empty(), "selected method: {failures:#?}");
}

#[test]
fn equations_seed_composite_expected_types_without_solving_unrelated_binders() {
    let source = r#"
        fn apply<T,F>(value:T,callback:F)->T where F=fn(T)->T {callback(value)}
        fn add(value:Int)->Int {value+42}
        fn run()->Int {apply(5,add)}
    "#;
    let failures = source_errors(source);
    assert!(failures.is_empty(), "{failures:#?}");
    rejects(&source_errors(&format!(
        "{DECLARATION} fn unconstrained<F>(callback:F)->Int {{invoke(callback)}}"
    )));
}
