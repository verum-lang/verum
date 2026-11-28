//! Sum payloads share the ordinary value-copy and affine-transfer boundary.
#![cfg(feature = "codegen")]
#[path = "fixtures/canonical_drop.rs"]
mod canonical_drop;

use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenResult, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn compile(source: &str, unit: bool) -> CodegenResult<VbcModule> {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("payload"));
    canonical_drop::initialize(&mut codegen);
    if unit {
        codegen.collect_unit_declarations(&[&ast])?;
        codegen.compile_unit_items(&[&ast], ItemFailurePolicy::Strict)?;
        codegen.finalize_module_from_state()
    } else {
        codegen.compile_additional_module(&ast)
    }
}

fn run(source: &str) {
    for unit in [false, true] {
        let module = compile(source, unit).expect("payload compilation");
        let wire = verum_vbc::serialize::serialize_module(&module).expect("wire encode");
        let decoded = verum_vbc::deserialize::deserialize_module(&wire).expect("wire decode");
        for (route, module) in [("source", module), ("wire", decoded)] {
            verum_vbc::validate::validate_module(&module).expect("declaration validation");
            let entry = module.find_function_by_name("payload.probe").unwrap();
            let result = Interpreter::new(module.into()).execute_function(entry);
            assert_eq!(result.unwrap().try_as_i64(), Some(73), "unit={unit}, {route}");
        }
    }
}

#[test]
fn a_borrowed_field_supplies_the_value_declared_by_the_new_variant() {
    run(r#"
        type Number is Absent | Value(Int);
        fn rebuild(input: &Number) -> Number {
            match input { Number.Value(n) => Number.Value(n), Number.Absent => Number.Absent }
        }
        fn probe() -> Int {
            let mut original = Number.Value(73);
            let copy = rebuild(&original);
            match &mut original { Number.Value(ref mut n) => *n = 19, Number.Absent => {} }
            match copy { Number.Value(n) => n, Number.Absent => 0 }
        }
    "#);
}

#[test]
fn generic_and_named_payloads_select_their_declared_value_slots() {
    run(r#"
        type Sample<T> is Absent | Value(T);
        type Row is Empty | Named { value: Int };
        fn copy_sample<T>(input: &Sample<T>) -> Sample<T> {
            match input { Sample.Value(n) => Sample.Value(n), Sample.Absent => Sample.Absent }
        }
        fn copy_row(input: &Row) -> Row {
            match input { Row.Named { value } => Row.Named { value }, Row.Empty => Row.Empty }
        }
        fn probe() -> Int {
            let mut original = Sample.Value(30);
            let a = copy_sample<Int>(&original);
            match &mut original { Sample.Value(ref mut n) => *n = 0, Sample.Absent => {} }
            let mut row = Row.Named { value: 43 };
            let b = copy_row(&row);
            match &mut row { Row.Named {value: ref mut n} => *n = 0, Row.Empty => {} }
            let x = match a { Sample.Value(n) => n, Sample.Absent => 0 };
            x + match b { Row.Named {value} => value, Row.Empty => 0 }
        }
    "#);
}

#[test]
fn a_reference_variant_field_keeps_the_selected_referent() {
    run(r#"
        type Incoming is Empty | Value(Int);
        type Observation is Empty | Loan(&Int);
        fn select(input: &Incoming) -> Observation {
            match input { Incoming.Value(n) => Observation.Loan(n), Incoming.Empty => Observation.Empty }
        }
        fn probe() -> Int {
            let original = Incoming.Value(73);
            match select(&original) { Observation.Loan(reference) => *reference, Observation.Empty => 0 }
        }
    "#);
}

#[test]
fn a_concrete_affine_field_cannot_receive_its_borrowed_owner() {
    let source = r#"
        type affine Ticket is { number: Int };
        type Boxed is Empty | Item(Ticket);
        fn duplicate(input: &Ticket) -> Boxed { Boxed.Item(input) }
    "#;
    for unit in [false, true] {
        let error = compile(source, unit).expect_err("copying an exclusive owner");
        assert!(error.to_string().contains("exclusive borrowed owner"), "{error}");
    }
}

#[test]
fn named_payload_copies_the_place_before_later_mutation() {
    run(r#"
        type Cell is { value: Int };
        type Choice<T> is Empty | Value(T);
        fn probe() -> Int {
            let mut source = Cell { value: 73 };
            let result = Choice.Value(source);
            source.value = 0;
            match result { Choice.Value(value) => value.value, _ => 0 }
        }
    "#);
}

#[test]
fn a_returned_nested_payload_survives_its_original_local_cleanup() {
    run(r#"
        mount core.base.protocols.Drop;
        type Cell is { value: Int };
        implement Drop for Cell { fn drop(&mut self) { self.value = 0; } }
        type Choice<T> is Empty | Value(T);
        fn produce() -> Choice<Choice<Cell>> {
            let source = Cell { value: 73 };
            Choice.Value(Choice.Value(source))
        }
        fn probe() -> Int {
            match produce() {
                Choice.Value(Choice.Value(value)) => value.value,
                _ => 0,
            }
        }
    "#);
}

#[test]
fn explicit_dereference_payload_uses_its_declared_copy_discipline() {
    run(r#"
        type Cell is { value: Int };
        type Choice<T> is Empty | Value(T);
        fn probe() -> Int {
            let mut source = Cell { value: 73 };
            let result = {
                let borrowed = &source;
                Choice.Value(*borrowed)
            };
            source.value = 0;
            match result { Choice.Value(value) => value.value, _ => 0 }
        }
    "#);
}

#[test]
fn reference_payload_keeps_the_original_referent() {
    let source = r#"
        type Cell is { value: Int };
        type Choice<T> is Empty | Value(T);
        fn probe() -> Int {
            let source = Cell { value: 73 };
            let borrowed = &source;
            let result: Choice<&Cell> = Choice.Value(borrowed);
            match result { Choice.Value(value) => value.value, _ => 0 }
        }
    "#;
    run(source);
    for unit in [false, true] {
        let module = compile(source, unit).unwrap();
        let entry = module.find_function_by_name("payload.probe").unwrap();
        let function = module.get_function(entry).unwrap();
        let start = function.bytecode_offset as usize;
        let body = verum_vbc::bytecode::decode_instructions(
            &module.bytecode[start..start + function.bytecode_length as usize],
        ).unwrap();
        assert!(!body.iter().any(|instruction| matches!(instruction,
            verum_vbc::Instruction::Clone { .. })), "reference cannot copy its referent: {body:?}");
    }
}

#[test]
fn borrowed_affine_payload_cannot_be_copied_into_a_sum() {
    let source = r#"
        type affine Cell is { value: Int };
        type Choice<T> is Empty | Value(T);
        fn copy(source: &Cell) -> Choice<Cell> { Choice.Value(*source) }
    "#;
    for unit in [false, true] {
        let error = match compile(source, unit) {
            Err(error) => error,
            Ok(_) => panic!("borrow cannot acquire payload ownership, unit={unit}"),
        };
        assert!(error.to_string().contains("exclusive borrowed owner"), "{error}");
    }
}

#[test]
fn an_inferred_conditional_sum_preserves_its_payload_declaration() {
    run(r#"
        type Signal<T> is NoSignal | Finished | Sample(T);
        fn sample(value: Int) -> Signal<Int> {
            let selected = if value < 0 {
                if value < -8 { Signal.Finished } else { Signal.NoSignal }
            } else if value < 10 {
                Signal.Sample(value)
            } else {
                Signal.NoSignal
            };
            match selected {
                Signal.Sample(item) => Signal.Sample(item),
                Signal.Finished => Signal.Finished,
                _ => Signal.NoSignal,
            }
        }
        fn probe() -> Int {
            let valid = match sample(3) { Signal.Sample(item) => item, _ => 0 };
            let empty = match sample(15) { Signal.NoSignal => 30, _ => 0 };
            let finished = match sample(-9) { Signal.Finished => 40, _ => 0 };
            valid + empty + finished
        }
    "#);
}

#[test]
fn an_inferred_conditional_sum_retains_its_borrowed_affine_payload() {
    let source = r#"
        type affine Ticket is { number: Int };
        type Signal<T> is NoSignal | Sample(T);
        fn duplicate(input: &Ticket, choose: Bool) -> Signal<Ticket> {
            let selected = if choose { Signal.NoSignal } else { Signal.Sample(input) };
            match selected {
                Signal.Sample(borrowed) => Signal.Sample(*borrowed),
                Signal.NoSignal => Signal.NoSignal,
            }
        }
    "#;
    for unit in [false, true] {
        let error = compile(source, unit).expect_err("the selected payload is still borrowed");
        assert!(error.to_string().contains("exclusive borrowed owner"), "{error}");
    }
}

#[test]
fn a_nullary_variant_of_another_sum_cannot_supply_ownership_evidence() {
    let source = r#"
        type Signal<T> is Empty | Sample(T);
        type Different<T> is Empty | Other(T);
        type Container<T> is One(T);
        fn mixed(choose: Bool) -> Container<Signal<Int>> {
            let selected = if choose { Different.Empty } else { Signal.Sample(73) };
            Container.One(selected)
        }
    "#;
    for unit in [false, true] {
        let error = compile(source, unit).expect_err("the empty variant belongs to a different sum");
        assert!(error.to_string().contains("retained source declaration"), "{error}");
    }
}

#[test]
fn a_nested_associated_result_keeps_the_payload_under_its_selected_protocol() {
    run(r#"
        type Envelope<T> is Empty | Wrapped(T);
        type Reader is protocol {
            type Value;
            fn read(&mut self) -> Envelope<Self.Value>;
        };
        type Source is {};
        implement Reader for Source {
            type Value = Envelope<Int>;
            fn read(&mut self) -> Envelope<Envelope<Int>> {
                Envelope.Wrapped(Envelope.Wrapped(73))
            }
        }
        fn keep<T>(value: T) -> T { value }
        fn extract<T, R: Reader<Value = Envelope<T>>>(mut source: R, fallback: T) -> T {
            match source.read() {
                Envelope.Wrapped(item) => match item {
                    Envelope.Wrapped(payload) => keep(payload),
                    Envelope.Empty => fallback,
                },
                Envelope.Empty => fallback,
            }
        }
        fn probe() -> Int { extract<Int, Source>(Source {}, 0) }
    "#);
}

#[test]
fn an_unrelated_nested_sum_cannot_supply_the_protocol_payload() {
    let source = r#"
        type Envelope<T> is Empty | Wrapped(T);
        type Other<T> is Absent | Value(T);
        type Reader is protocol { type Value; fn read(&self) -> Envelope<Self.Value>; };
        fn reject<T, R: Reader<Value = Other<T>>>(source: R, fallback: T) -> T {
            match source.read() {
                Envelope.Wrapped(item) => match item {
                    Envelope.Wrapped(payload) => payload,
                    _ => fallback,
                },
                _ => fallback,
            }
        }
    "#;
    for unit in [false, true] {
        let error = compile(source, unit).err().expect("foreign variant must be refused");
        assert!(error.to_string().contains("variant"), "{error}");
    }
}

#[test]
fn an_or_pattern_binds_only_the_selected_payload_path() {
    run(r#"
        type Inner is Empty | Number(Int);
        type Choice is Short(Int) | Long(Int, Inner) | Flag(Bool);
        fn pick(choice: &Choice) -> Int {
            match choice {
                Choice.Short(n) | Choice.Long(_, Inner.Number(n)) => *n,
                Choice.Long(_, Inner.Empty) | Choice.Flag(true) => 8,
                _ => 0,
            }
        }
        fn probe() -> Int {
            pick(&Choice.Short(17)) + pick(&Choice.Long(200, Inner.Number(48))) + pick(&Choice.Flag(true))
        }
    "#);
}

#[test]
fn mutable_or_pattern_bindings_retain_the_selected_field() {
    run(r#"
        type Pair is Left(Int, Int) | Right(Int, Int);
        fn set(pair: &mut Pair) {
            match pair { Pair.Left(ref mut n, _) | Pair.Right(_, ref mut n) => *n = 41 }
        }
        fn probe() -> Int {
            let mut pair = Pair.Right(32, 8);
            set(&mut pair);
            match pair { Pair.Right(first, second) => first + second, _ => 0 }
        }
    "#);
}

#[test]
fn or_pattern_choices_reject_incompatible_binding_names_and_types() {
    for (pattern, diagnostic) in [
        ("Choice.Left(x) | Choice.Right(y)", "different names"),
        ("Choice.Left(x) | Choice.Flag(x)", "incompatible or-pattern binding"),
        ("Choice.Left(ref x) | Choice.Right(x)", "incompatible or-pattern binding"),
    ] {
        let source = format!("type Choice is Left(Int) | Right(Int) | Flag(Bool); fn probe(value: Choice) -> Int {{ match value {{ {pattern} => 73, _ => 0 }} }}");
        for unit in [false, true] {
            let error = compile(&source, unit).expect_err("incompatible bindings");
            assert!(error.to_string().contains(diagnostic), "{error}");
        }
    }
}

#[test]
fn an_or_pattern_tests_each_guard_once_and_keeps_leftmost_precedence() {
    run(r#"
        type Pair is Both(Int, Int);
        fn count(step: Int) -> Int {
            static mut CALLS: Int = 0;
            CALLS += step;
            CALLS
        }
        fn choose(pair: &Pair, first: Bool) -> Int {
            match pair {
                (Pair.Both(x, _) if count(1) > 0 && first) | Pair.Both(_, x) => *x,
                _ => 0,
            }
        }
        fn probe() -> Int {
            let pair = Pair.Both(20, 51);
            choose(&pair, true) + choose(&pair, false) + count(0)
        }
    "#);
}
