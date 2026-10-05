//! Shared actual emission authority; no speculative reference loads are enabled.
use super::VbcToLlvmLowering;
use crate::llvm::{
    LoweringConfig,
    native_call::{ArgumentView, ResultView},
};

use verum_fast_parser::Parser;
use verum_llvm::{context::Context, module::Linkage, values::AnyValue};
use verum_vbc::{codegen::VbcCodegen, module::VbcModule};

fn compile(source: &str) -> VbcModule {
    VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().unwrap())
        .unwrap()
}
fn id(module: &VbcModule, name: &str) -> u32 {
    module
        .functions
        .iter()
        .find(|fd| module.get_string(fd.name) == Some(name))
        .unwrap()
        .id
        .0
}

#[test]
fn native_receipts_follow_forward_calls_and_actual_result_words() {
    let vbc = compile(
        "fn probe(value: Int) -> Int { later(value) } fn later(value: Int) -> Int { value + 7 }",
    );
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("calls").with_debug_info(false),
    );
    lower.lower_module(&vbc).unwrap();
    let facts = lower.native_calls.resolve(lower.module());
    let calls = facts.get(&id(&vbc, "probe")).unwrap();
    assert_eq!(calls.len(), 1);
    let (callee, receipt) = &calls[0];
    assert_eq!(*callee, id(&vbc, "later"));
    assert_eq!(receipt.arguments.len(), 1);
    assert_eq!(receipt.result_view, ResultView::Word);
    assert_eq!(receipt.result, receipt.call.try_as_basic_value().basic());
    assert!(matches!(
        receipt.arguments[0].view,
        ArgumentView::Register(_)
    ));
    assert!(
        receipt.instruction
            < vbc
                .get_function(verum_vbc::module::FunctionId(id(&vbc, "probe")))
                .unwrap()
                .instructions
                .as_ref()
                .unwrap()
                .len()
    );
}

#[test]
fn reference_arguments_record_the_original_cell_adapter() {
    let vbc = compile(
        "type Cell is { padding: Int, value: Int }; fn put(p: &mut Int) { *p = 77; } fn probe(c: &mut Cell) { put(&mut c.value); }",
    );
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("cells").with_debug_info(false),
    );
    lower.lower_module(&vbc).unwrap();
    let facts = lower.native_calls.resolve(lower.module());
    let calls = facts.get(&id(&vbc, "probe")).unwrap();
    let (_, receipt) = calls
        .iter()
        .find(|(callee, _)| *callee == id(&vbc, "put"))
        .unwrap();
    assert!(matches!(
        receipt.arguments[0].view,
        ArgumentView::FieldCellOrRegister(_)
    ));
    assert!(
        receipt.arguments[0]
            .value
            .print_to_string()
            .to_str()
            .unwrap()
            .contains("refield_arg_addr")
    );
}

#[test]
fn changed_callee_or_caller_body_cannot_reuse_source_receipts() {
    let vbc = compile(
        "fn probe(value: Int) -> Int { selected(value) } fn selected(value: Int) -> Int { value + 7 }",
    );
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("mutation").with_debug_info(false),
    );
    lower.lower_module(&vbc).unwrap();
    let probe = id(&vbc, "probe");
    assert_eq!(
        lower
            .native_calls
            .resolve(lower.module())
            .get(&probe)
            .unwrap()
            .len(),
        1
    );
    let selected = lower.module().get_function("selected").unwrap();
    selected.set_linkage(Linkage::External);
    assert_eq!(
        lower
            .native_calls
            .resolve(lower.module())
            .get(&probe)
            .unwrap()
            .len(),
        1,
        "linkage does not rewrite a body"
    );
    // SAFETY: no execution is active; no instruction handles from a removed
    // body are read. The resolver checks the live caller seal first.
    unsafe {
        for block in selected.get_basic_blocks() {
            block.delete().unwrap();
        }
    }
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(selected, "replacement"));
    builder
        .build_return(Some(&context.i64_type().const_int(99, false)))
        .unwrap();
    assert!(
        lower
            .native_calls
            .resolve(lower.module())
            .get(&probe)
            .unwrap()
            .is_empty()
    );
    let caller = lower.module().get_function("probe").unwrap();
    unsafe {
        for block in caller.get_basic_blocks() {
            block.delete().unwrap();
        }
    }
    builder.position_at_end(context.append_basic_block(caller, "replacement"));
    builder
        .build_return(Some(&context.i64_type().const_int(3, false)))
        .unwrap();
    assert!(
        !lower
            .native_calls
            .resolve(lower.module())
            .contains_key(&probe)
    );
}

#[test]
fn arity_collision_cannot_borrow_an_unemitted_source_body() {
    let mut vbc = compile(
        "module alpha { public fn choose(value: Int) -> Int { value + 1 } } module beta { public fn choose(left: Int, right: Int) -> Int { left + right } } fn probe(value: Int) -> Int { alpha.choose(value) + beta.choose(value, 7) }",
    );
    let one = id(&vbc, "alpha.choose");
    let two = id(&vbc, "beta.choose");
    let name = vbc.strings.intern("choose");
    for fd in &mut vbc.functions {
        if fd.id.0 == one || fd.id.0 == two {
            fd.name = name;
        }
    }
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("arity").with_debug_info(false),
    );
    lower.lower_module(&vbc).unwrap();
    let facts = lower.native_calls.resolve(lower.module());
    let calls = facts.get(&id(&vbc, "probe")).unwrap();
    assert!(
        calls.is_empty(),
        "an arity collision with no surviving source body is opaque"
    );
    // T1598: this is an existing dispatch defect, not successful execution.
    let verification = lower
        .module()
        .verify()
        .expect_err("existing malformed arity collision");
    assert!(
        verification
            .to_str()
            .unwrap()
            .contains("Incorrect number of arguments")
    );
}

#[test]
fn an_early_runtime_replacement_has_no_source_body_receipt() {
    let vbc = compile(
        "type Boxed is { value: Int }; implement Boxed { fn hash_value(&self) -> Int { 37 } } fn probe(value: &Boxed) -> Int { value.hash_value() }",
    );
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("opaque").with_debug_info(false),
    );
    lower.lower_module(&vbc).unwrap();
    let body = lower
        .module()
        .get_function("probe")
        .unwrap()
        .print_to_string();
    assert!(
        body.to_str().unwrap().contains("verum_generic_hash"),
        "control must exercise existing replacement: {body}"
    );
    let facts = lower.native_calls.resolve(lower.module());
    assert!(facts.get(&id(&vbc, "probe")).unwrap().is_empty());
}

#[test]
fn generic_calls_record_the_selected_body_without_discarding_the_inline_witness() {
    let vbc = compile(
        "fn identity<T>(value: T) -> T { value } fn probe(value: Int) -> Int { identity<Int>(value) }",
    );
    let probe = id(&vbc, "probe");
    let descriptor = vbc
        .get_function(verum_vbc::module::FunctionId(probe))
        .unwrap();
    assert!(
        descriptor
            .instructions
            .as_ref()
            .unwrap()
            .iter()
            .any(|instruction| matches!(
                instruction,
                verum_vbc::instruction::Instruction::CallG { .. }
            ))
    );
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("generic").with_debug_info(false),
    );
    lower.lower_module(&vbc).unwrap();
    let facts = lower.native_calls.resolve(lower.module());
    let calls = facts.get(&probe).unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, id(&vbc, "identity"));
    assert_eq!(calls[0].1.arguments.len(), 1);
    assert_eq!(calls[0].1.result_view, ResultView::Word);
}

#[test]
fn static_method_receipt_does_not_invent_a_receiver_parameter() {
    let vbc = compile(
        "type Factory is { padding: Int }; implement Factory { fn make(value: Int) -> Int { value + 7 } } fn probe(factory: &Factory, value: Int) -> Int { factory.make(value) }",
    );
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("static").with_debug_info(false),
    );
    lower.lower_module(&vbc).unwrap();
    let facts = lower.native_calls.resolve(lower.module());
    let calls = facts.get(&id(&vbc, "probe")).unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, id(&vbc, "Factory.make"));
    assert_eq!(calls[0].1.arguments.len(), 1);
    assert_eq!(calls[0].1.call.count_arguments(), 1);
    let fd = vbc
        .get_function(verum_vbc::module::FunctionId(id(&vbc, "probe")))
        .unwrap();
    assert!(matches!(
        fd.instructions.as_ref().unwrap()[calls[0].1.instruction],
        verum_vbc::instruction::Instruction::CallM { .. }
    ));
}

#[test]
fn mutually_recursive_calls_have_finite_exact_edges_without_a_return_classification() {
    let vbc = compile(
        "fn alpha(value: Int) -> Int { beta(value) } fn beta(value: Int) -> Int { alpha(value) }",
    );
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("cycle").with_debug_info(false),
    );
    lower.lower_module(&vbc).unwrap();
    let facts = lower.native_calls.resolve(lower.module());
    assert_eq!(
        facts.get(&id(&vbc, "alpha")).unwrap()[0].0,
        id(&vbc, "beta")
    );
    assert_eq!(
        facts.get(&id(&vbc, "beta")).unwrap()[0].0,
        id(&vbc, "alpha")
    );
    // Receipts are edges only: no recursion=true address classification exists.
}

#[test]
fn late_abi_attribute_change_invalidates_source_evidence() {
    let vbc =
        compile("fn selected(p: &Int) -> Int { *p } fn probe(p: &Int) -> Int { selected(p) }");
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("abi").with_debug_info(false),
    );
    lower.lower_module(&vbc).unwrap();
    let probe = id(&vbc, "probe");
    assert_eq!(
        lower
            .native_calls
            .resolve(lower.module())
            .get(&probe)
            .unwrap()
            .len(),
        1
    );
    let selected = lower.module().get_function("selected").unwrap();
    let attr = context.create_enum_attribute(
        verum_llvm::attributes::Attribute::get_named_enum_kind_id("inreg"),
        0,
    );
    selected.add_attribute(verum_llvm::attributes::AttributeLoc::Param(0), attr);
    assert!(
        lower
            .native_calls
            .resolve(lower.module())
            .get(&probe)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn native_argument_conversions_never_reuse_an_unchanged_register_fact() {
    for (source, expected_instruction, float_result) in [
        (
            "fn selected(value: Float32) -> Float32 { value } fn probe(value: Float32) -> Float32 { selected(value) }",
            "fptrunc",
            true,
        ),
        (
            "fn selected<T>(value: T) -> T { value } fn probe(value: Float32) -> Float32 { selected<Float32>(value) }",
            "bitcast",
            false,
        ),
        (
            "type Factory is { padding: Int }; implement Factory { fn selected(value: Float32) -> Float32 { value } } fn probe(factory: &Factory, value: Float32) -> Float32 { factory.selected(value) }",
            "fptrunc",
            true,
        ),
    ] {
        let vbc = compile(source);
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("narrow").with_debug_info(false),
        );
        lower.lower_module(&vbc).unwrap();
        let facts = lower.native_calls.resolve(lower.module());
        let calls = facts.get(&id(&vbc, "probe")).unwrap();
        assert_eq!(calls.len(), 1, "{source}");
        assert!(
            matches!(calls[0].1.arguments[0].view, ArgumentView::Adjusted(_)),
            "{:?}",
            calls[0].1.arguments[0]
        );
        if float_result {
            assert_eq!(calls[0].1.result_view, ResultView::Opaque);
        }
        assert!(
            calls[0].1.arguments[0]
                .value
                .print_to_string()
                .to_str()
                .unwrap()
                .contains(expected_instruction)
        );
    }
}

#[test]
fn retained_abi_attributes_do_not_claim_a_transparent_parameter_or_result_adapter() {
    use verum_llvm::attributes::{Attribute, AttributeLoc};
    let vbc = compile(
        "fn selected(value: Int) -> Int { value } fn probe(value: Int) -> Int { selected(value) }",
    );
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("abi_adapter").with_debug_info(false),
    );
    lower.lower_module(&vbc).unwrap();
    let selected = lower.module().get_function("selected").unwrap();
    let attr = context.create_enum_attribute(Attribute::get_named_enum_kind_id("inreg"), 0);
    selected.add_attribute(AttributeLoc::Param(0), attr);
    selected.add_attribute(AttributeLoc::Return, attr);
    // Model an emitter that carries this ABI from declaration onward: recapture
    // its source body with the attributes present, rather than a stale seal.
    lower
        .native_calls
        .capture(id(&vbc, "selected"), selected, verum_common::List::new());
    let facts = lower.native_calls.resolve(lower.module());
    let calls = facts.get(&id(&vbc, "probe")).unwrap();
    assert_eq!(calls.len(), 1);
    assert!(matches!(
        calls[0].1.arguments[0].view,
        ArgumentView::Adjusted(_)
    ));
    assert_eq!(calls[0].1.result_view, ResultView::Opaque);
}
