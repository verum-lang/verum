//! Reachable native calls retain the same producer checks on every CFG edge.
use std::sync::Arc;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_fast_parser::Parser;
use verum_llvm::{context::Context, values::AnyValue};
use verum_vbc::{
    VbcModule, codegen::VbcCodegen, deserialize::deserialize_module_validated,
    interpreter::Interpreter, serialize::serialize_module,
};
#[path = "support/native_library.rs"]
mod native_library;

fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source).parse_module().unwrap();
    VbcCodegen::new().compile_module(&ast).unwrap()
}

fn images(source: &str) -> [VbcModule; 2] {
    let module = compile(source);
    let wire =
        attach_bodies(deserialize_module_validated(&serialize_module(&module).unwrap()).unwrap());
    [module, wire]
}

fn attach_bodies(mut module: VbcModule) -> VbcModule {
    for function in &mut module.functions {
        let start = function.bytecode_offset as usize;
        let mut body = verum_vbc::bytecode::decode_instructions(
            &module.bytecode[start..start + function.bytecode_length as usize],
        )
        .unwrap();
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut body);
        function.instructions = Some(body);
    }
    module
}

#[test]
fn unreachable_calls_do_not_undo_a_source_ownership_refusal() {
    for discipline in ["affine", "linear"] {
        for mutable in ["", "mut "] {
            for tail in [
                "callback()",
                "if true {callback()} else {0}",
                "loop {break callback();}",
            ] {
                for module in images(&format!(
                    "type {discipline} Token is {{value:Int}}; fn probe()->Int {{let {mutable}token:Token=Token{{value:7}}; let callback=||token.value; {tail}}}"
                )) {
                    let entry = module.find_function_by_name("probe").unwrap();
                    let error = Interpreter::new(Arc::new(module.clone()))
                        .execute_function(entry)
                        .unwrap_err();
                    assert!(
                        matches!(
                            error,
                            verum_vbc::interpreter::InterpreterError::Panic { .. }
                        ),
                        "{error}"
                    );
                    let context = Context::create();
                    let mut lowering = VbcToLlvmLowering::new(
                        &context,
                        LoweringConfig::debug("dead_owned_capture").with_debug_info(false),
                    );
                    lowering.lower_module(&module).unwrap();
                    lowering.module().verify().unwrap();
                    let source_instructions: usize = module
                        .functions
                        .iter()
                        .filter_map(|function| function.instructions.as_ref())
                        .map(|body| body.len())
                        .sum();
                    assert!(
                        lowering.stats().instructions_lowered < source_instructions,
                        "unreachable bytecode must not be counted as lowered instructions"
                    );
                    let ir = lowering
                        .module()
                        .get_function("probe")
                        .unwrap()
                        .print_to_string()
                        .to_string();
                    assert!(ir.contains("call void @verum_panic"), "{ir}");
                    assert!(ir.contains("unreachable"), "{ir}");
                    assert!(!ir.contains("$closure$"), "{ir}");
                }
            }
        }
    }
}

#[test]
fn source_refusal_verifies_with_orphan_sweeping_disabled() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "unreachable_calls_do_not_undo_a_source_ownership_refusal",
            "--nocapture",
        ])
        .env("VERUM_SKIP_ORPHAN_SWEEP", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        verum_common::Text::from_utf8_lossy(&output.stdout),
        verum_common::Text::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn reachable_invalid_callable_is_refused_on_normal_and_recovery_paths() {
    for body in [
        "callback()",
        "if flag {73} else {callback()}",
        "try {panic(\"caught\"); 0} recover |_| {callback()}",
        "let result=try {73} recover |_| {0}; if flag {result} else {callback()}",
        "try {let token=Token{value:7}; let refused=||token.value; refused()} recover |_| {callback()}",
    ] {
        for module in images(&format!(
            "type affine Token is {{value:Int}}; fn probe(callback:fn()->Int, flag:Bool)->Int {{{body}}}"
        )) {
            let context = Context::create();
            let mut lowering = VbcToLlvmLowering::new(
                &context,
                LoweringConfig::debug("reachable_callable_guard").with_debug_info(false),
            );
            let error = lowering.lower_module(&module).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("producer-issued parameter convention"),
                "{error}"
            );
        }
    }
}

#[test]
fn capture_refusal_keeps_its_recovery_handler_reachable() {
    for module in images(
        "type affine Token is {value:Int}; fn probe()->Int {try {let token=Token{value:7}; let refused=||token.value; refused()} recover |_| {73}}",
    ) {
        let entry = module.find_function_by_name("probe").unwrap();
        assert_eq!(
            Interpreter::new(Arc::new(module.clone()))
                .execute_function(entry)
                .unwrap()
                .as_i64(),
            73
        );
        native_library::with_library(&module, &[("probe", "invoke_probe")], |library| {
            // SAFETY: the emitted adapter forwards exactly the source zero-argument Int signature.
            let result = unsafe {
                library
                    .get::<unsafe extern "C" fn() -> i64>(b"invoke_probe")
                    .unwrap()()
            };
            assert_eq!(result, 73);
        });
    }
}

#[test]
fn fused_integer_branches_use_instruction_targets_on_both_edges() {
    use verum_vbc::{Instruction as I, instruction::CompareOp};
    let mut module = compile(
        "fn choose(a:Int,b:Int)->Int {if a<b {73} else {29}} fn probe()->Int {choose(1,2)+choose(3,2)}",
    );
    let selected = module.find_function_by_name("choose").unwrap();
    let old = module.bytecode.clone();
    module.bytecode.clear();
    let mut fused = 0;
    for function in &mut module.functions {
        let start = function.bytecode_offset as usize;
        let mut body = verum_vbc::bytecode::decode_instructions(
            &old[start..start + function.bytecode_length as usize],
        )
        .unwrap();
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut body);
        if function.id == selected {
            for pc in 0..body.len().saturating_sub(1) {
                if let (
                    I::CmpI {
                        op: CompareOp::Lt,
                        dst,
                        a,
                        b,
                    },
                    I::JmpNot { cond, offset },
                ) = (&body[pc], &body[pc + 1])
                    && dst == cond
                {
                    let branch = I::JmpCmp {
                        op: CompareOp::Ge,
                        a: *a,
                        b: *b,
                        offset: offset.checked_add(1).unwrap(),
                    };
                    body[pc] = branch;
                    body[pc + 1] = I::Nop;
                    fused += 1;
                }
            }
        }
        function.bytecode_offset = module.bytecode.len() as u32;
        function.bytecode_length =
            verum_vbc::bytecode::encode_instructions_with_fixup(&body, &mut module.bytecode) as u32;
        function.instructions = Some(body);
    }
    assert_eq!(
        fused, 1,
        "replace the actual selected integer comparison and branch"
    );
    let wire =
        attach_bodies(deserialize_module_validated(&serialize_module(&module).unwrap()).unwrap());
    for module in [module, wire] {
        let entry = module.find_function_by_name("probe").unwrap();
        assert_eq!(
            Interpreter::new(Arc::new(module.clone()))
                .execute_function(entry)
                .unwrap()
                .as_i64(),
            102
        );
        native_library::with_library(&module, &[("probe", "invoke_probe")], |library| {
            // SAFETY: the emitted adapter forwards exactly the source zero-argument Int signature.
            let result = unsafe {
                library
                    .get::<unsafe extern "C" fn() -> i64>(b"invoke_probe")
                    .unwrap()()
            };
            assert_eq!(result, 102);
        });
    }
}

#[test]
fn a_named_callable_keeps_its_declaration_through_aliases_and_loop_joins() {
    let source = r#"
        fn step(n: Int) -> Int { n + 1 }
        fn choose(flag: Bool) -> Int {
            let initial = step;
            let callback: fn(Int) -> Int = if flag { initial } else { step };
            let mut n: Int = 0;
            while n < 2 { n = callback(n); }
            callback(n + 70)
        }
        fn probe() -> Int { choose(true) + choose(false) }
    "#;
    for module in images(source) {
        let entry = module.find_function_by_name("probe").unwrap();
        let result = Interpreter::new(Arc::new(module.clone())).execute_function(entry).unwrap();
        assert_eq!(result.as_i64(), 146);
        native_library::with_library(&module, &[("probe", "invoke_probe")], |library| {
            // SAFETY: the native adapter exports the source nullary Int signature.
            let result = unsafe {
                library.get::<unsafe extern "C" fn() -> i64>(b"invoke_probe").unwrap()()
            };
            assert_eq!(result, 146);
        });
    }
}

#[test]
fn an_indirect_write_or_conflicting_loop_backedge_invalidates_the_callable() {
    for body in [
        "let slot = &mut callback; *slot = other; callback()",
        "let mut n:Int=0; while n<2 { let used=callback(); callback=other; n=n+used; } n",
    ] {
        let source = format!(
            "fn known()->Int {{1}} fn other()->Int {{2}} fn probe()->Int {{let mut callback:fn()->Int=known; {body}}}"
        );
        for module in images(&source) {
            let context = Context::create();
            let mut lowering = VbcToLlvmLowering::new(
                &context,
                LoweringConfig::debug("conflicting_callable").with_debug_info(false),
            );
            let error = lowering.lower_module(&module).unwrap_err();
            assert!(error.to_string().contains("producer-issued parameter convention"), "{error}");
        }
    }
}
