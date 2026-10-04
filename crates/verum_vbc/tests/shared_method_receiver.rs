#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::bytecode::decode_instruction;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::Instruction;

// Inspect the compiler boundary without a baked stdlib. Runtime coverage uses
// the actual Shared/atomic implementations in shared_method_receiver.vr.
const ENV: &str = r#"
type Deref is protocol { type Target; fn deref(&self) -> &Self.Target; };
type Shared<T> is { value: T };
implement<T> Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T { &self.value }
}
implement<T> Shared<T> {
    fn strong_count(&self) -> Int { 1 }
}
type Cell is { padding: Int, value: Int };
implement Cell {
    fn load(&self) -> Int { self.value }
    fn unique_read(&self) -> Int { self.value }
    fn strong_count(&self) -> Int { 999 }
}
"#;

fn operations(body: &str) -> Vec<String> {
    let ast = Parser::new(&format!("{ENV}\n{body}"))
        .parse_module()
        .expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("shared_receiver"))
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
        .expect("probe");
    let end = (entry.bytecode_offset + entry.bytecode_length) as usize;
    let mut offset = entry.bytecode_offset as usize;
    let mut result = Vec::new();
    while offset < end {
        match decode_instruction(&module.bytecode, &mut offset).expect("instruction") {
            Instruction::CallM { method_id, .. } => result.push(
                module
                    .get_string(verum_vbc::types::StringId(method_id))
                    .unwrap()
                    .to_string(),
            ),
            Instruction::Call { func_id, .. } => result.push(
                module
                    .get_string(module.functions[func_id as usize].name)
                    .unwrap()
                    .to_string(),
            ),
            Instruction::Deref { .. } => result.push("*".to_string()),
            _ => {}
        }
    }
    result
}

#[test]
fn implicit_receiver_matches_explicit_deref_for_colliding_and_unique_methods() {
    for method in ["load", "unique_read"] {
        let implicit = operations(&format!(
            "fn probe(s: Shared<Cell>) -> Int {{ s.{method}() }}"
        ));
        let explicit = operations(&format!(
            "fn probe(s: Shared<Cell>) -> Int {{ (*s).{method}() }}"
        ));
        assert_eq!(implicit, explicit, "method {method}");
        assert!(
            implicit.iter().any(|op| op == "Shared.deref"),
            "{implicit:?}"
        );
        assert!(implicit.iter().any(|op| op == "*"), "{implicit:?}");
        assert!(
            implicit
                .iter()
                .any(|op| op.ends_with(&format!("Cell.{method}"))),
            "{implicit:?}"
        );
    }
}

#[test]
fn wrapper_methods_take_precedence_over_inner_homonyms() {
    let ops = operations("fn probe(s: Shared<Cell>) -> Int { s.strong_count() }");
    assert!(
        ops.iter().any(|op| op.ends_with("Shared.strong_count")),
        "{ops:?}"
    );
    assert!(
        !ops.iter()
            .any(|op| op == "Shared.deref" || op.ends_with("Cell.strong_count")),
        "{ops:?}"
    );
}

#[test]
fn a_reference_to_shared_still_dereferences_the_wrapper() {
    let ops = operations("fn probe(s: &Shared<Cell>) -> Int { s.load() }");
    assert!(ops.iter().any(|op| op == "Shared.deref"), "{ops:?}");
    assert!(ops.iter().any(|op| op.ends_with("Cell.load")), "{ops:?}");
}

#[test]
fn temporary_receiver_is_evaluated_once() {
    let ops = operations(
        "fn make() -> Shared<Cell> { Shared { value: Cell { padding: 41, value: 777 } } } fn probe() -> Int { make().load() }",
    );
    assert_eq!(
        ops.iter()
            .filter(|op| op.ends_with(".make") || *op == "make")
            .count(),
        1,
        "{ops:?}"
    );
    assert!(ops.iter().any(|op| op == "Shared.deref"), "{ops:?}");
}
