use verum_codegen::llvm::runtime::RuntimeLowering;
use verum_common::layout::{VALUE_SLOT_SIZE, VARIANT_PAYLOAD_OFFSET};
use verum_llvm::context::Context;
use verum_llvm::targets::{InitializationConfig, Target};
use verum_llvm::{AddressSpace, OptimizationLevel};

#[test]
fn a_native_variant_reference_addresses_its_payload_and_writes_through() {
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    let context = Context::create();
    let module = context.create_module("variant_reference");
    let builder = context.create_builder();
    let i64_type = context.i64_type();
    let ptr_type = context.ptr_type(AddressSpace::default());
    let function = module.add_function(
        "write_second",
        i64_type.fn_type(&[ptr_type.into()], false),
        None,
    );
    builder.position_at_end(context.append_basic_block(function, "entry"));
    let variant = function
        .get_first_param()
        .expect("variant")
        .into_pointer_value();
    let runtime = RuntimeLowering::new(&context);
    let field = runtime
        .lower_get_variant_data_addr(&builder, variant, 1)
        .expect("field address");
    builder
        .build_store(field, i64_type.const_int(3, false))
        .expect("write through");
    let value = runtime
        .lower_get_variant_data(&builder, variant, 1)
        .expect("read value");
    builder.build_return(Some(&value)).expect("return");
    module.verify().expect("valid IR");

    let mut storage = [0_u64; 16];
    let first = (VARIANT_PAYLOAD_OFFSET / VALUE_SLOT_SIZE) as usize;
    storage[first] = 41;
    storage[first + 1] = 1;
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::None)
        .expect("JIT");
    // SAFETY: the generated function has this C signature; storage includes
    // the complete header and both payload slots and stays live for the call.
    let got = unsafe {
        engine
            .get_function::<unsafe extern "C" fn(*mut u64) -> u64>("write_second")
            .expect("function")
            .call(storage.as_mut_ptr())
    };
    assert_eq!(got, 3);
    assert_eq!(storage[first], 41, "the adjacent field must remain intact");
    assert_eq!(
        storage[first + 1],
        3,
        "the mutation must reach the original payload"
    );
}
