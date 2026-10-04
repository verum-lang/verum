#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::{FunctionDescriptor, VbcModule};
use verum_vbc::types::{TypeId, TypeRef};

fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source).parse_module().expect("parse");
    VbcCodegen::with_config(CodegenConfig::new("callable"))
        .compile_module(&ast)
        .expect("compile")
}

fn nominal(module: &VbcModule, name: &str) -> TypeRef {
    let desc = module
        .types
        .iter()
        .find(|t| module.get_string(t.name) == Some(name))
        .expect(name);
    TypeRef::Concrete(desc.id)
}

fn closures(module: &VbcModule) -> Vec<&FunctionDescriptor> {
    module
        .functions
        .iter()
        .filter(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n.contains("$closure$"))
        })
        .collect()
}

#[test]
fn annotated_nominal_parameter_reaches_closure_descriptor() {
    let module = compile(
        "type Payload is { value: Int }; fn probe() -> Int { let f = |x: Payload| -> Payload { x }; 0 }",
    );
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, nominal(&module, "Payload"));
    assert_eq!(f.return_type, nominal(&module, "Payload"));
}

#[test]
fn closure_result_is_inferred_in_its_parameter_scope() {
    let module =
        compile("type Payload is { value: Int }; fn probe() -> Int { let f = |x: Payload| x; 0 }");
    assert_eq!(
        closures(&module)[0].return_type,
        nominal(&module, "Payload")
    );
}

#[test]
fn declared_callable_argument_types_contextualize_lambda_without_adapter_name() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Stage<T> is { value: T };
implement<T> Stage<T> { fn transform<R, F: fn(T) -> R>(self, f: F) -> F { f } }
fn probe(stage: Stage<Payload>) -> Int { let f = stage.transform(|x| x); 0 }
"#,
    );
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, nominal(&module, "Payload"));
    assert_eq!(f.return_type, nominal(&module, "Payload"));
}

#[test]
fn sibling_closures_keep_distinct_nominal_parameter_and_result_types() {
    let module = compile(
        r#"
type First is { value: Int };
type Second is { value: Int };
fn probe() -> Int { let f = |x: First| x; let g = |x: Second| x; 0 }
"#,
    );
    let fs = closures(&module);
    assert_eq!(fs.len(), 2);
    for (f, name) in fs.iter().zip(["First", "Second"]) {
        assert_eq!(f.params[1].type_ref, nominal(&module, name));
        assert_eq!(f.return_type, nominal(&module, name));
    }
}

#[test]
fn explicit_primitive_closure_signature_remains_exact() {
    let module = compile("fn probe() -> Int { let f = |x: Bool| -> Bool { x }; 0 }");
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, TypeRef::Concrete(TypeId::BOOL));
    assert_eq!(f.return_type, TypeRef::Concrete(TypeId::BOOL));
}

#[test]
fn contextual_callback_slots_are_independent() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Stage<T> is { value: T };
implement<T> Stage<T> { fn both(self, first: fn(T) -> T, second: fn(Bool) -> Bool) -> Int { 0 } }
fn probe(stage: Stage<Payload>) -> Int { stage.both(|x| x, |x| x) }
"#,
    );
    let fs = closures(&module);
    assert_eq!(fs.len(), 2);
    assert_eq!(fs[0].params[1].type_ref, nominal(&module, "Payload"));
    assert_eq!(fs[1].params[1].type_ref, TypeRef::Concrete(TypeId::BOOL));
    assert_eq!(fs[0].return_type, nominal(&module, "Payload"));
    assert_eq!(fs[1].return_type, TypeRef::Concrete(TypeId::BOOL));
}

#[test]
fn unrelated_same_named_method_cannot_supply_callback_context() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Decoy is { other: Int };
type Stage<T> is { value: T };
type Other is { value: Int };
implement Other { fn transform(self, f: fn(Decoy) -> Decoy) -> Int { 0 } }
implement<T> Stage<T> { fn transform(self, f: fn(T) -> T) -> Int { 0 } }
fn probe(stage: Stage<Payload>) -> Int { stage.transform(|x| x) }
"#,
    );
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, nominal(&module, "Payload"));
    assert_eq!(f.return_type, nominal(&module, "Payload"));
}

#[test]
fn nested_nominal_annotation_keeps_all_type_arguments() {
    let module = compile(
        r#"
type Payload is { value: Int };
type Envelope<T> is { value: T };
fn probe() -> Int { let f = |x: Envelope<Payload>| x; 0 }
"#,
    );
    let TypeRef::Concrete(base) = nominal(&module, "Envelope") else {
        panic!("nominal")
    };
    let expected = TypeRef::Instantiated {
        base,
        args: vec![nominal(&module, "Payload")],
    };
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, expected);
    assert_eq!(f.return_type, expected);
}

#[test]
fn reference_annotation_is_preserved_in_parameter_and_result() {
    use verum_vbc::types::{CbgrTier, Mutability};
    let module =
        compile("type Payload is { value: Int }; fn probe() -> Int { let f = |x: &Payload| x; 0 }");
    let expected = TypeRef::Reference {
        inner: Box::new(nominal(&module, "Payload")),
        mutability: Mutability::Immutable,
        tier: CbgrTier::Tier0,
    };
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, expected);
    assert_eq!(f.return_type, expected);
}

#[test]
fn caller_generic_annotation_stays_generic_despite_same_named_nominal() {
    use verum_vbc::types::TypeParamId;
    let module = compile("type T is { value: Int }; fn probe<T>() -> Int { let f = |x: T| x; 0 }");
    let f = closures(&module)[0];
    assert_eq!(f.params[1].type_ref, TypeRef::Generic(TypeParamId(0)));
    assert_eq!(f.return_type, TypeRef::Generic(TypeParamId(0)));
}
