//! Native calls need the actual selected body convention; callable cleanup refuses.
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_fast_parser::Parser;
use verum_llvm::{context::Context, values::AnyValue};
use verum_vbc::{
    codegen::VbcCodegen, deserialize::deserialize_module, serialize::serialize_module,
};
#[path = "support/native_library.rs"]
mod native_library;

fn compile(source: &str) -> verum_vbc::VbcModule {
    let module = VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().unwrap())
        .unwrap();
    let mut wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for function in &mut wire.functions {
        let start = function.bytecode_offset as usize;
        let mut instructions = verum_vbc::bytecode::decode_instructions(
            &wire.bytecode[start..start + function.bytecode_length as usize],
        )
        .unwrap();
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    wire
}
#[test]
fn native_mixed_parameter_convention_is_refused_before_emission_of_the_call() {
    let mut module = compile("fn take(value: Int) -> Int { value } fn main() -> Int { take(73) }");
    let take = module.find_function_by_name("take").unwrap();
    let main = module.find_function_by_name("main").unwrap();
    let function = module
        .functions
        .iter_mut()
        .find(|function| function.id == take)
        .unwrap();
    function.owned_value_parameters = false;
    function.value_uses = None;
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("mixed_convention").with_debug_info(false),
    );
    let error = lowering.lower_module(&module).unwrap_err();
    assert!(
        error.to_string().contains(&format!(
            "incompatible owned parameter calling conventions: main [#{}] calls take [#{}]", main.0, take.0
        )),
        "{error}"
    );
}

#[test]
fn annotated_source_bodies_own_their_parameters_on_both_sides_of_a_call() {
    let module = compile(r#"
        type Cell is { value: Int };
        implement Cell {
            @intrinsic("annotated_static") fn fresh(value: Int) -> Self { Cell { value } }
            @intrinsic("annotated_method") fn read(&self) -> Int { self.value }
        }
        @intrinsic("annotated_source") fn adjust(mut cell: Cell) -> Int {
            cell.value = cell.value + 1;
            cell.read()
        }
        fn probe() -> Int {
            let cell = Cell.fresh(31);
            let adjusted = adjust(cell);
            cell.value * 100 + adjusted
        }
    "#);
    let entry = module.find_function_by_name("probe").unwrap();
    let result = verum_vbc::interpreter::Interpreter::new(module.clone().into())
        .execute_function(entry).unwrap();
    assert_eq!(result.try_as_i64(), Some(3132));
    native_library::with_library(&module, &[("probe", "invoke_probe")], |library| {
        // SAFETY: the generated adapter exports the nullary Int signature under this name.
        let result = unsafe { library.get::<unsafe extern "C" fn() -> i64>(b"invoke_probe").unwrap()() };
        assert_eq!(result, 3132);
    });
}

#[test]
fn native_eager_async_calls_share_the_emitted_parameter_convention() {
    for source in [
        "async fn answer(value:Int)->Int {value} fn main()->Int {answer(73)}",
        "fn answer(value:Int)->Int {value} async fn main()->Int {answer(73)}",
        "async fn answer()->Int {73} fn main()->Int {async {answer().await}}",
        "type Cell is {value:Int}; async fn alter(mut value:Cell) {value.value=0;} fn main()->Int {let original=Cell{value:73}; alter(original); original.value}",
    ] {
        let module = compile(source);
        let context = Context::create();
        let mut lowering = VbcToLlvmLowering::new(
            &context, LoweringConfig::debug("eager_convention").with_debug_info(false),
        );
        lowering.lower_module(&module).unwrap();
        lowering.module().verify().unwrap();
    }
}

#[test]
fn native_callable_cleanup_has_an_explicit_executable_refusal() {
    let module = compile(
        "fn known() -> Int { 73 } fn dispose(selected: fn() -> Int) {} fn main() -> Int { dispose(known); 73 }",
    );
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("callable_cleanup").with_debug_info(false),
    );
    lowering.lower_module(&module).unwrap();
    lowering.module().verify().unwrap();
    let ir = lowering.module().print_to_string().to_string();
    assert!(
        ir.contains(
            "native callable cleanup requires a producer-issued environment ownership receipt"
        ),
        "{ir}"
    );
    let body = lowering
        .module()
        .get_function("dispose")
        .unwrap()
        .print_to_string()
        .to_string();
    assert!(body.contains("unreachable"), "{body}");
}

fn overwritten_callable() -> verum_vbc::VbcModule {
    let mut module = compile(
        "fn known()->Int {73} fn probe()->Int {let callback:fn()->Int=known; callback()}",
    );
    let entry = module.find_function_by_name("probe").unwrap();
    let function = module.functions.iter_mut().find(|function| function.id == entry).unwrap();
    let instructions = function.instructions.as_mut().unwrap();
    let (index, register) = instructions.iter().enumerate().find_map(|(index, instruction)| {
        if let verum_vbc::Instruction::CallClosure { closure, .. } = instruction {
            Some((index, *closure))
        } else { None }
    }).expect("ordinary source callable call");
    instructions.insert(index, verum_vbc::Instruction::LoadUnit { dst: register });
    module
}

#[test]
fn a_known_callable_producer_does_not_authorize_an_overwritten_register() {
    let module = overwritten_callable();
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context, LoweringConfig::debug("overwritten_callable").with_debug_info(false),
    );
    let error = lowering.lower_module(&module).unwrap_err();
    assert!(error.to_string().contains("producer-issued parameter convention"), "{error}");

}

#[test]
fn a_null_legacy_closure_refuses_before_reading_its_environment() {
    const CHILD: &str = "VERUM_TEST_NULL_LEGACY_CLOSURE";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "a_null_legacy_closure_refuses_before_reading_its_environment", "--nocapture"])
            .env(CHILD, "1").output().unwrap();
        assert!(!output.status.success());
        let stderr = verum_common::Text::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("native closure call received a null carrier"), "{stderr}");
        return;
    }
    let mut module = overwritten_callable();
    for function in &mut module.functions {
        function.owned_value_parameters = false;
        function.value_uses = None;
    }
    native_library::with_library(&module, &[("probe", "invoke_probe")], |library| {
        // SAFETY: the adapter exports the declared probe() -> Int ABI.
        let probe = unsafe { library.get::<unsafe extern "C" fn() -> i64>(b"invoke_probe").unwrap() };
        let returned = unsafe { probe() };
        panic!("invalid callback unexpectedly returned {returned}");
    });
}

#[test]
fn callable_parameters_and_control_flow_joins_require_their_own_producer_proof() {
    for source in [
        "fn invoke(callback:fn()->Int)->Int {callback()} fn known()->Int {73} fn main()->Int {invoke(known)}",
        "fn known()->Int {73} fn other()->Int {19} fn choose(flag:Bool)->Int {let mut callback:fn()->Int=known; if flag {callback=other;} callback()} fn main()->Int {choose(false)}",
    ] {
        let module = compile(source);
        let context = Context::create();
        let mut lowering = VbcToLlvmLowering::new(
            &context, LoweringConfig::debug("unproved_callable").with_debug_info(false),
        );
        let error = lowering.lower_module(&module).unwrap_err();
        assert!(error.to_string().contains("producer-issued parameter convention"), "{error}");
    }
}

#[test]
fn a_selected_callable_cannot_acquire_a_different_body_convention() {
    let mut module = compile(
        "fn known()->Int {73} fn main()->Int {let callback:fn()->Int=known; callback()}",
    );
    let selected = module.find_function_by_name("known").unwrap();
    let descriptor = module.functions.iter_mut().find(|function| function.id == selected).unwrap();
    descriptor.owned_value_parameters = false;
    descriptor.value_uses = None;
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context, LoweringConfig::debug("mixed_callable").with_debug_info(false),
    );
    let error = lowering.lower_module(&module).unwrap_err();
    assert!(error.to_string().contains("incompatible owned parameter"), "{error}");
}

#[test]
fn a_selected_callable_keeps_its_declared_argument_count() {
    let mut module = compile(
        "fn known(value:Int)->Int {value} fn main()->Int {let callback:fn(Int)->Int=known; callback(73)}",
    );
    let entry = module.find_function_by_name("main").unwrap();
    let function = module.functions.iter_mut().find(|function| function.id == entry).unwrap();
    for instruction in function.instructions.as_mut().unwrap() {
        if let verum_vbc::Instruction::CallClosure { args, .. } = instruction {
            args.count = 0;
        }
    }
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context, LoweringConfig::debug("callable_arity").with_debug_info(false),
    );
    let error = lowering.lower_module(&module).unwrap_err();
    assert!(error.to_string().contains("argument count differs from its selected producer"), "{error}");
}

#[test]
fn an_erased_callable_signature_cannot_authorize_a_native_copy() {
    let module = compile("fn duplicate(selected:fn()->Int)->Int {let copied=selected;copied()} fn known()->Int {73} fn main()->Int {duplicate(known)}");
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context, LoweringConfig::debug("erased_callable_copy").with_debug_info(false),
    );
    let error = lowering.lower_module(&module).unwrap_err();
    assert!(error.to_string().contains("native callable copy requires its exact environment producer"), "{error}");
}

#[test]
fn native_callable_copy_checks_the_producers_semantic_signature() {
    let mut module = compile("fn known()->Int {73} fn main()->Int {let selected:fn()->Int=known;let copied=selected;copied()}");
    let mut changed = false;
    for function in &mut module.functions {
        for instruction in function.instructions.iter_mut().flatten() {
            if let verum_vbc::Instruction::CloneTyped { type_ref: verum_vbc::TypeRef::Function { return_type, .. }, .. } = instruction {
                **return_type = verum_vbc::TypeRef::Concrete(verum_vbc::TypeId::BOOL);
                changed = true;
            }
        }
    }
    assert!(changed, "actual source callable copy");
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context, LoweringConfig::debug("mismatched_callable_copy").with_debug_info(false),
    );
    let error = lowering.lower_module(&module).unwrap_err();
    assert!(error.to_string().contains("copy producer does not match the declared signature"), "{error}");
}
