#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_vbc::bytecode::decode_instruction;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::Instruction;
use verum_vbc::interpreter::Interpreter;

const ENV: &str = r#"
module user_receiver;
type Deref is protocol { type Target; fn deref(&self) -> &Self.Target; };
type Guard<T> is { marker: Int, value: T };
implement<T> Deref for Guard<T> {
    type Target = T;
    fn deref(&self) -> &T { &self.value }
}
implement<T> Guard<T> {
    fn own(&self) -> Int { self.marker }
}
type Cell is { padding: Int, value: Int };
implement Cell {
    fn read(&self) -> Int { self.value }
    fn own(&self) -> Int { 99 }
}
"#;

fn check(body: &str, expected: Option<i64>) -> List<Text> {
    let ast = Parser::new(&format!("{ENV}\n{body}"))
        .parse_module()
        .expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("user_receiver"))
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
    let entry_id = entry.id;
    let end = (entry.bytecode_offset + entry.bytecode_length) as usize;
    let mut offset = entry.bytecode_offset as usize;
    let mut operations = List::new();
    while offset < end {
        match decode_instruction(&module.bytecode, &mut offset).expect("instruction") {
            Instruction::CallM { method_id, .. } => operations.push(Text::from(
                module
                    .get_string(verum_vbc::types::StringId(method_id))
                    .unwrap(),
            )),
            Instruction::Call { func_id, .. }
            | Instruction::TailCall { func_id, .. }
            | Instruction::CallG { func_id, .. } => operations.push(Text::from(
                module
                    .get_string(module.functions[func_id as usize].name)
                    .unwrap(),
            )),
            Instruction::Deref { .. } => operations.push(Text::from("*")),
            Instruction::Len { .. } => operations.push(Text::from("Len")),
            _ => {}
        }
    }
    if let Some(expected) = expected {
        let value = Interpreter::new(Arc::new(module))
            .execute_function(entry_id)
            .expect("execute");
        assert!(value.is_int(), "expected Int, got {value:?}");
        assert_eq!(value.as_i64(), expected);
    }
    operations
}

#[test]
fn user_deref_receiver_emits_the_same_calls_as_explicit_deref() {
    let implicit = check("fn probe(g: Guard<Cell>) -> Int { g.read() }", None);
    let explicit = check("fn probe(g: Guard<Cell>) -> Int { (*g).read() }", None);
    assert_eq!(implicit, explicit);
    assert!(
        implicit.iter().any(|op| op.ends_with("Guard.deref")),
        "{implicit:?}"
    );
    assert!(implicit.iter().any(|op| op == "*"), "{implicit:?}");
    assert!(
        implicit.iter().any(|op| op.ends_with("Cell.read")),
        "{implicit:?}"
    );
}

#[test]
fn user_deref_receiver_executes_the_target_method() {
    check(
        "fn probe() -> Int { let g: Guard<Cell> = Guard { marker: 13, value: Cell { padding: 99, value: 7 } }; g.read() }",
        Some(7),
    );
}

#[test]
fn wrapper_methods_win_before_and_during_a_deref_chain() {
    check(
        "fn probe() -> Int { let g: Guard<Cell> = Guard { marker: 13, value: Cell { padding: 41, value: 7 } }; g.own() }",
        Some(13),
    );
    let ops = check("fn probe(g: Guard<Cell>) -> Int { g.own() }", None);
    assert!(ops.iter().any(|op| op.ends_with("Guard.own")), "{ops:?}");
    assert!(
        !ops.iter()
            .any(|op| op.ends_with(".deref") || op.ends_with("Cell.own")),
        "{ops:?}"
    );
    let ops = check(
        r#"
type Outer<T> is { value: T };
implement<T> Deref for Outer<T> { type Target = T; fn deref(&self) -> &T { &self.value } }
fn probe() -> Int {
    let g: Outer<Guard<Cell>> = Outer { value: Guard { marker: 13, value: Cell { padding: 41, value: 7 } } };
    g.own()
}
"#,
        Some(13),
    );
    assert!(ops.iter().any(|op| op.ends_with("Outer.deref")), "{ops:?}");
    assert!(ops.iter().any(|op| op.ends_with("Guard.own")), "{ops:?}");
    assert!(!ops.iter().any(|op| op.ends_with("Guard.deref")), "{ops:?}");
}

#[test]
fn nested_same_constructor_deref_reaches_the_terminal_method() {
    let ops = check(
        "fn probe() -> Int { let g: Guard<Guard<Cell>> = Guard { marker: 13, value: Guard { marker: 13, value: Cell { padding: 41, value: 7 } } }; g.read() }",
        Some(7),
    );
    assert_eq!(
        ops.iter().filter(|op| op.ends_with("Guard.deref")).count(),
        2,
        "{ops:?}"
    );
    assert!(ops.iter().any(|op| op.ends_with("Cell.read")), "{ops:?}");
}

#[test]
fn generic_deref_target_uses_the_declared_parameter_position() {
    let ops = check(
        r#"
type PairGuard<Noise, Item> is { noise: Noise, value: Item };
implement<Noise, Item> Deref for PairGuard<Noise, Item> {
    type Target = Item;
    fn deref(&self) -> &Item { &self.value }
}
fn probe() -> Int {
    let g: PairGuard<Int, Cell> = PairGuard { noise: 53, value: Cell { padding: 41, value: 7 } };
    g.read()
}
"#,
        Some(7),
    );
    assert!(
        ops.iter().any(|op| op.ends_with("PairGuard.deref")),
        "{ops:?}"
    );
    assert!(ops.iter().any(|op| op.ends_with("Cell.read")), "{ops:?}");
}

#[test]
fn nongeneric_deref_and_temporary_receiver_use_one_evaluation() {
    let ops = check(
        r#"
type Wrapper is { value: Cell };
implement Deref for Wrapper { type Target = Cell; fn deref(&self) -> &Cell { &self.value } }
fn make() -> Wrapper { Wrapper { value: Cell { padding: 41, value: 7 } } }
fn probe() -> Int { make().read() }
"#,
        Some(7),
    );
    assert_eq!(
        ops.iter()
            .filter(|op| op.ends_with(".make") || *op == "make")
            .count(),
        1,
        "{ops:?}"
    );
    assert!(
        ops.iter().any(|op| op.ends_with("Wrapper.deref")),
        "{ops:?}"
    );
    assert!(ops.iter().any(|op| op.ends_with("Cell.read")), "{ops:?}");
}

#[test]
fn qualified_user_wrapper_keeps_the_target_instantiation() {
    let ops = check(
        r#"
fn make() -> user_receiver.Guard<Cell> { Guard { marker: 13, value: Cell { padding: 41, value: 7 } } }
fn probe() -> Int { make().read() }
"#,
        Some(7),
    );
    assert!(ops.iter().any(|op| op.ends_with("Guard.deref")), "{ops:?}");
    assert!(ops.iter().any(|op| op.ends_with("Cell.read")), "{ops:?}");
}

#[test]
fn reference_to_user_wrapper_still_invokes_its_deref_method() {
    let ops = check("fn probe(g: &Guard<Cell>) -> Int { g.read() }", None);
    assert!(ops.iter().any(|op| op.ends_with("Guard.deref")), "{ops:?}");
    assert!(ops.iter().any(|op| op.ends_with("Cell.read")), "{ops:?}");
}

#[test]
fn iterable_target_never_keeps_the_guard_method_identity() {
    let ops = check(
        "fn probe() -> Int { let g: Guard<List<Int>> = Guard { marker: 13, value: [7, 11] }; let mut total = 0; for item in g.iter() { total = total + item; } total }",
        Some(18),
    );
    assert!(ops.iter().any(|op| op.ends_with("Guard.deref")), "{ops:?}");
    assert!(ops.iter().any(|op| op.ends_with("List.iter")), "{ops:?}");
    assert!(!ops.iter().any(|op| op.ends_with("Guard.iter")), "{ops:?}");
}

#[test]
fn cyclic_deref_declarations_do_not_recurse_while_resolving_a_method() {
    let ops = check(
        r#"
type Cycle is { value: Int };
implement Deref for Cycle { type Target = Cycle; fn deref(&self) -> &Cycle { self } }
fn probe(g: Cycle) -> Int { g.missing() }
"#,
        None,
    );
    assert!(!ops.iter().any(|op| op.ends_with(".deref")), "{ops:?}");
}

#[test]
fn expanding_deref_declarations_do_not_recurse_while_resolving_a_method() {
    let ops = check(
        r#"
type Growing<T> is { next: &Growing<Growing<T>> };
implement<T> Deref for Growing<T> {
    type Target = Growing<Growing<T>>;
    fn deref(&self) -> &Growing<Growing<T>> { self.next }
}
fn probe(g: Growing<Cell>) -> Int { g.missing() }
"#,
        None,
    );
    assert!(!ops.iter().any(|op| op.ends_with(".deref")), "{ops:?}");
}

#[test]
fn explicit_reference_deref_keeps_wrapper_method_precedence() {
    let ops = check("fn probe(g: &Guard<Cell>) -> Int { (*g).own() }", None);
    assert!(ops.iter().any(|op| op.ends_with("Guard.own")), "{ops:?}");
    assert!(
        !ops.iter()
            .any(|op| op.ends_with("Guard.deref") || op.ends_with("Cell.own")),
        "{ops:?}"
    );
    check(
        r#"
fn through_reference(g: &Guard<Cell>) -> Int { (*g).own() }
fn probe() -> Int {
    let g: Guard<Cell> = Guard { marker: 13, value: Cell { padding: 41, value: 7 } };
    through_reference(&g)
}
"#,
        Some(13),
    );
}

#[test]
fn raw_reference_deref_still_strips_its_reference_prefix() {
    let ops = check(
        "fn probe(g: &unsafe Guard<Cell>) -> Int { (*g).own() }",
        None,
    );
    assert!(ops.iter().any(|op| op.ends_with("Guard.own")), "{ops:?}");
    assert!(
        !ops.iter()
            .any(|op| op.ends_with("Guard.deref") || op.ends_with("Cell.own")),
        "{ops:?}"
    );
}

#[test]
fn associated_target_return_keeps_the_concrete_target_type() {
    let ops = check(
        r#"
type AssociatedGuard<T> is { value: T };
implement<T> Deref for AssociatedGuard<T> {
    type Target = T;
    fn deref(&self) -> &Self.Target { &self.value }
}
fn probe() -> Int {
    let g: AssociatedGuard<Cell> = AssociatedGuard { value: Cell { padding: 41, value: 7 } };
    g.read()
}
"#,
        Some(7),
    );
    assert!(
        ops.iter().any(|op| op.ends_with("AssociatedGuard.deref")),
        "{ops:?}"
    );
    assert!(ops.iter().any(|op| op.ends_with("Cell.read")), "{ops:?}");
}

#[test]
fn single_letter_target_is_not_an_unresolved_generic_parameter() {
    let ops = check(
        r#"
type U is { padding: Int, answer: Int };
implement U { fn read(&self) -> Int { self.answer } }
fn probe() -> Int {
    let g: Guard<U> = Guard { marker: 13, value: U { padding: 41, answer: 7 } };
    g.read()
}
"#,
        Some(7),
    );
    assert!(ops.iter().any(|op| op.ends_with("Guard.deref")), "{ops:?}");
    assert!(ops.iter().any(|op| op.ends_with("U.read")), "{ops:?}");
}

#[test]
fn duplicating_deref_expansion_has_a_bounded_resolution_budget() {
    let ops = check(
        r#"
type Pair<Left, Right> is { left: Left, right: Right };
type Growing<T> is { next: &Growing<Pair<T, T>> };
implement<T> Deref for Growing<T> {
    type Target = Growing<Pair<T, T>>;
    fn deref(&self) -> &Growing<Pair<T, T>> { self.next }
}
fn probe(g: Growing<Cell>) -> Int { g.missing() }
"#,
        None,
    );
    assert!(!ops.iter().any(|op| op.ends_with(".deref")), "{ops:?}");
}

#[test]
fn parenthesizing_a_reference_operand_preserves_explicit_deref() {
    for operand in ["g", "(g)", "((g))"] {
        let body = format!("fn probe(g: &Guard<Cell>) -> Int {{ (*{operand}).own() }}");
        let ops = check(&body, None);
        assert!(
            ops.iter().any(|op| op.ends_with("Guard.own")),
            "{operand}: {ops:?}"
        );
        assert!(
            !ops.iter()
                .any(|op| op.ends_with("Guard.deref") || op.ends_with("Cell.own")),
            "{operand}: {ops:?}"
        );
        check(
            &format!(
                r#"
fn through_reference(g: &Guard<Cell>) -> Int {{ (*{operand}).own() }}
fn probe() -> Int {{
    let g: Guard<Cell> = Guard {{ marker: 13, value: Cell {{ padding: 41, value: 7 }} }};
    through_reference(&g)
}}
"#
            ),
            Some(13),
        );
    }
}

#[test]
fn associated_target_with_reference_arguments_resolves_container_methods() {
    for return_type in ["&Self.Target", "&List<&T>"] {
        let ops = check(
            &format!(
                r#"
type RefList<T> is {{ values: List<&T> }};
implement<T> Deref for RefList<T> {{
    type Target = List<&T>;
    fn deref(&self) -> {return_type} {{ &self.values }}
}}
fn probe(g: RefList<Cell>) -> Int {{ g.len() }}
"#
            ),
            None,
        );
        assert!(
            ops.iter().any(|op| op.ends_with("RefList.deref")),
            "{return_type}: {ops:?}"
        );
        assert!(
            ops.iter().any(|op| op.ends_with("List.len") || op == "Len"),
            "{return_type}: {ops:?}"
        );
    }
}

#[test]
fn caller_generic_target_never_uses_a_same_named_nominal_deref() {
    let ops = check(
        r#"
type Reader is protocol { fn read(&self) -> Int; };
type Decoy is { value: Int };
implement Decoy { fn read(&self) -> Int { self.value } }
type U is { value: Decoy };
implement Deref for U { type Target = Decoy; fn deref(&self) -> &Decoy { &self.value } }
fn probe<U: Reader>(g: Guard<U>) -> Int { g.read() }
"#,
        None,
    );
    assert!(ops.iter().any(|op| op.ends_with("Guard.deref")), "{ops:?}");
    assert!(
        !ops.iter()
            .any(|op| op.ends_with("U.deref") || op.ends_with("Decoy.read")),
        "{ops:?}"
    );
}
