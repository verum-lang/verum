//! Text length and emptiness depend only on the native record's stored byte count.
use verum_codegen::llvm::{context::FunctionContext, instruction::lower_instruction};
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    targets::{InitializationConfig, Target},
};
use verum_vbc::instruction::{Instruction, TextSubOpcode};

#[test]
fn native_text_queries_respect_embedded_nul_and_the_empty_carrier() {
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let context = Context::create();
    let module = context.create_module("text_queries");
    let word = context.i64_type();
    for (name, op) in [
        ("length", TextSubOpcode::ByteLen),
        ("empty", TextSubOpcode::IsEmpty),
    ] {
        let function = module.add_function(name, word.fn_type(&[word.into()], false), None);
        let mut ctx = FunctionContext::new(&context, &module, function, name);
        ctx.builder()
            .position_at_end(context.append_basic_block(function, "entry"));
        ctx.set_register(130, function.get_first_param().unwrap());
        lower_instruction(
            &mut ctx,
            &Instruction::TextExtended {
                sub_op: op as u8,
                // Destination 129 and source 130 each take two wire bytes.
                operands: vec![0x80, 129, 0x80, 130],
            },
        )
        .unwrap();
        let result = ctx.get_register(129).unwrap();
        ctx.builder().build_return(Some(&result)).unwrap();
    }
    module.verify().unwrap();
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    // SAFETY: both generated functions have the declared u64 -> u64 ABI. A
    // nonzero argument points to a live three-word native Text record.
    let (length, empty) = unsafe {
        (
            engine
                .get_function::<unsafe extern "C" fn(u64) -> u64>("length")
                .unwrap(),
            engine
                .get_function::<unsafe extern "C" fn(u64) -> u64>("empty")
                .unwrap(),
        )
    };
    for text in ["", "\0é", "Верум 🧮"] {
        let record = [text.as_ptr() as u64, text.len() as u64, text.len() as u64];
        let address = record.as_ptr() as u64;
        // SAFETY: the record and its byte buffer outlive both calls.
        unsafe {
            assert_eq!(length.call(address), text.len() as u64);
            assert_eq!(empty.call(address), u64::from(text.is_empty()));
        }
    }
    // SAFETY: the native Text ABI treats the null carrier as empty.
    unsafe {
        assert_eq!(length.call(0), 0);
        assert_eq!(empty.call(0), 1);
    }
}
