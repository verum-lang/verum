#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::Instruction;
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::StringTable;
use verum_vbc::types::{TypeDescriptor, TypeId, TypeKind};
use verum_vbc::value::Value;

const NOMINALS: &str = r#"
type ScoreProtocol is protocol { fn score_marker(&self) -> Int; };
type ScoreLeft is ();
type ScoreRight is ();
implement ScoreProtocol for ScoreLeft { fn score_marker(&self) -> Int { 11 } }
implement ScoreProtocol for ScoreRight { fn score_marker(&self) -> Int { 29 } }
fn invoke<T: ScoreProtocol>(value: T) -> Int { value.score_marker() }
"#;

fn run(source: &str) -> Value {
    let ast = Parser::new(source).parse_module().expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("nominal_unit"))
        .compile_module(&ast)
        .expect("compile");
    let entry = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "probe" || n.ends_with(".probe"))
        })
        .expect("probe")
        .id;
    Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .expect("execute")
}

#[test]
fn distinct_nominal_unit_values_dispatch_through_their_own_protocol_impls() {
    let value = run(&format!(
        "{NOMINALS}\nfn probe() -> Int {{ invoke(ScoreLeft) * 100 + invoke(ScoreRight) }}"
    ));
    assert_eq!(value.as_i64(), 1129);
}

#[test]
fn nullary_call_constructors_preserve_the_same_nominal_identity() {
    let value = run(&format!(
        "{NOMINALS}\nfn probe() -> Int {{ invoke(ScoreLeft()) * 100 + invoke(ScoreRight()) }}"
    ));
    assert_eq!(value.as_i64(), 1129);
}

#[test]
fn a_resolved_qualified_constructor_uses_the_nominal_unit_lowering() {
    let ast = Parser::new(NOMINALS).parse_module().expect("parse");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("nominal_unit"));
    let module = codegen.compile_module(&ast).expect("compile");
    let expected = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("ScoreLeft"))
        .expect("ScoreLeft")
        .id
        .0;
    // Install the resolved import binding using its original constructor
    // descriptor; this exercises the qualified-call lowering independently
    // of source module discovery.
    let constructor = codegen
        .ctx_mut()
        .lookup_function("ScoreLeft")
        .expect("constructor")
        .clone();
    codegen
        .ctx_mut()
        .register_function("markers.ScoreLeft".to_owned(), constructor);
    codegen.ctx_mut().begin_function("probe", &[], None);
    let expr = Parser::new("markers.ScoreLeft()")
        .parse_expr()
        .expect("parse call");
    codegen.compile_expr(&expr).expect("compile call");
    assert!(codegen.ctx_mut().instructions.iter().any(|i| matches!(i,
        Instruction::New { type_id, field_count: 0, .. } if *type_id == expected
    )));
}

#[test]
fn heap_of_a_generic_nominal_unit_keeps_its_payload_identity() {
    let value = run(&format!(
        r#"{NOMINALS}
type Deref is protocol {{ type Target; fn deref(&self) -> &Self.Target; }};
type Heap<T> is {{ value: T }};
implement<T> Deref for Heap<T> {{
    type Target = T;
    fn deref(&self) -> &T {{ &self.value }}
}}
fn invoke_heap<T: ScoreProtocol>(value: Heap<T>) -> Int {{ value.score_marker() }}
fn probe() -> Int {{
    invoke_heap(Heap.new(ScoreLeft)) * 100 + invoke_heap(Heap.new(ScoreRight))
}}
"#
    ));
    assert_eq!(value.as_i64(), 1129);
}

#[test]
fn ordinary_unit_remains_the_canonical_unit_value() {
    assert!(run("fn probe() { () }").is_unit());
}

#[test]
fn unit_sum_variants_keep_their_variant_tags() {
    let value = run(
        "type Choice is First | Second; fn probe() -> Int { let first = Choice.First; let second = Choice.Second; match (first, second) { (Choice.First, Choice.Second) => 7, _ => -1 } }",
    );
    assert_eq!(value.as_i64(), 7);
}

#[test]
fn empty_record_bare_values_also_keep_nominal_identity() {
    let source = NOMINALS.replace("is ();", "is {};");
    let value = run(&format!(
        "{source}\nfn probe() -> Int {{ invoke(ScoreLeft) * 100 + invoke(ScoreRight) }}"
    ));
    assert_eq!(value.as_i64(), 1129);
}

#[test]
fn imported_unit_descriptors_construct_their_declared_type_ids() {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    let mut strings = StringTable::new();
    for (name, id) in [("Left", 1300), ("Right", 1301)] {
        let descriptor = TypeDescriptor {
            id: TypeId(id),
            name: strings.intern(name),
            kind: TypeKind::Unit,
            ..Default::default()
        };
        codegen.register_archive_type_qualified(
            descriptor,
            name.to_owned(),
            Some("markers"),
            Some(&strings),
        );
    }
    codegen.ctx_mut().begin_function("probe", &[], None);
    for name in ["Left", "Right"] {
        let expr = Parser::new(name).parse_expr().expect("parse value");
        codegen.compile_expr(&expr).expect("compile imported value");
    }
    let allocated: Vec<u32> = codegen
        .ctx_mut()
        .instructions
        .iter()
        .filter_map(|i| match i {
            Instruction::New {
                type_id,
                field_count: 0,
                ..
            } => Some(*type_id),
            _ => None,
        })
        .collect();
    assert_eq!(allocated, [1300, 1301]);
}
