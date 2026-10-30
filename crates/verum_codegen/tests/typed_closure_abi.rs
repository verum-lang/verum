//! T1563: declared callable types cross the native slot ABI without losing bits.
use std::{cell::RefCell, sync::Arc};
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::{Heap, List, Text};
use verum_fast_parser::Parser;
use verum_llvm::{OptimizationLevel, context::Context, memory_buffer::MemoryBuffer};
use verum_llvm::{
    targets::{InitializationConfig, Target},
    values::AnyValue,
};
use verum_vbc::{codegen::VbcCodegen, interpreter::Interpreter, module::VbcModule};

thread_local! {
    static STORAGE: RefCell<List<Heap<[u64]>>> = RefCell::new(List::new());
}

extern "C" fn allocate(size: u64) -> *mut u64 {
    STORAGE.with(|storage| {
        let mut allocation = List::from_elem(0_u64, size.div_ceil(8) as usize).into_boxed_slice();
        let pointer = allocation.as_mut_ptr();
        storage.borrow_mut().push(allocation);
        pointer
    })
}

extern "C" fn unexpected_exit(_code: u64) {
    std::process::abort();
}

extern "C" fn unexpected_assert(_message: *const u8) -> i32 {
    eprintln!("native source assertion failed");
    std::process::abort();
}

extern "C" fn unexpected_panic(message: *const u8, length: u64, _file: *const u8, _line: u32) {
    // Generated panic calls carry a valid constant message extent.
    let message = unsafe { std::slice::from_raw_parts(message, length as usize) };
    eprintln!("native panic: {}", String::from_utf8_lossy(message));
    std::process::abort();
}

fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source).parse_module().expect("parse source");
    VbcCodegen::new().compile_module(&ast).expect("source VBC")
}

fn with_native(
    module: &VbcModule,
    check: impl FnOnce(&verum_llvm::execution_engine::ExecutionEngine),
) {
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("typed_callbacks").with_debug_info(false),
    );
    lower.lower_module(module).expect("lower source");
    let mut ir = Text::from(
        "declare ptr @verum_checked_malloc(i64)\ndeclare void @verum_internal_exit_i64(i64)\n",
    );
    if lower.module().get_function("verum_panic").is_some() {
        ir.push_str("declare void @verum_panic(ptr, i64, ptr, i32)\n");
    }
    if lower.module().get_function("verum_internal_puts").is_some() {
        ir.push_str("declare i32 @verum_internal_puts(ptr)\n");
    }
    for global in lower.module().get_globals() {
        ir.push_str(global.print_to_string().to_str().unwrap());
        ir.push('\n');
    }
    if let Some(memset) = lower.module().get_function("verum_internal_memset") {
        ir.push_str(memset.print_to_string().to_str().unwrap());
        ir.push('\n');
    }
    for descriptor in &module.functions {
        let name = module.get_string(descriptor.name).expect("function name");
        let function = lower.module().get_function(name).expect(name);
        assert!(
            function.verify(true),
            "invalid source function: {}",
            function.print_to_string()
        );
        ir.push_str(function.print_to_string().to_str().expect("UTF-8 IR"));
        // NewClosure's named-function adapter is a native emission detail.
        if let Some(adapter) = lower.module().get_function(&format!("{name}$trampoline$")) {
            assert!(
                adapter.verify(true),
                "invalid callable adapter: {}",
                adapter.print_to_string()
            );
            ir.push_str(adapter.print_to_string().to_str().expect("UTF-8 IR"));
        }
    }
    if let Some(path) = std::env::var_os("TYPED_IR_DUMP") {
        std::fs::write(path, ir.as_bytes()).unwrap();
    }
    let executable = context
        .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
            ir.as_bytes(),
            "typed_callbacks",
        ))
        .expect("source IR");
    executable.verify().expect("valid source module");
    let engine = executable
        .create_jit_execution_engine(OptimizationLevel::None)
        .expect("JIT");
    engine.add_global_mapping(
        &executable.get_function("verum_checked_malloc").unwrap(),
        allocate as *const () as usize,
    );
    engine.add_global_mapping(
        &executable.get_function("verum_internal_exit_i64").unwrap(),
        unexpected_exit as *const () as usize,
    );
    if let Some(panic) = executable.get_function("verum_panic") {
        engine.add_global_mapping(&panic, unexpected_panic as *const () as usize);
    }
    if let Some(assert) = executable.get_function("verum_internal_puts") {
        engine.add_global_mapping(&assert, unexpected_assert as *const () as usize);
    }
    check(&engine);
    STORAGE.with(|storage| storage.borrow_mut().clear());
}

fn check_float(source: &str, expected: f64) {
    let module = compile(source);
    let function = module.find_function_by_name("probe").expect("probe");
    let interpreted = Interpreter::new(Arc::new(module.clone()))
        .execute_function(function)
        .expect("interpreter")
        .as_f64();
    assert_eq!(interpreted, expected, "interpreter control");
    with_native(&module, |engine| {
        // SAFETY: the source probe is a zero-argument function returning Float.
        let actual = unsafe {
            engine
                .get_function::<unsafe extern "C" fn() -> f64>("probe")
                .unwrap()
                .call()
        };
        assert_eq!(actual, expected, "native callback arithmetic");
    });
}

#[test]
fn direct_float_and_f32_calls_are_controls() {
    for ty in ["Float", "Float32"] {
        check_float(
            &format!(
                "fn half(x: {ty}) -> {ty} {{ x / 2.0 }} fn probe() -> Float {{ half(6.5) + 0.125 }}"
            ),
            3.375,
        );
    }
}

#[test]
fn named_float_function_value_preserves_arguments_and_result() {
    check_float(
        "fn half(x: Float) -> Float { x / 2.0 } fn probe() -> Float { let f: fn(Float) -> Float = half; f(6.5) + 0.125 }",
        3.375,
    );
}

#[test]
fn named_f32_function_value_preserves_arguments_and_result() {
    check_float(
        "fn half(x: Float32) -> Float32 { x / 2.0 } fn probe() -> Float { let f: fn(Float32) -> Float32 = half; f(6.5) + 0.125 }",
        3.375,
    );
}

#[test]
fn noncapturing_float_lambda_preserves_arguments_and_result() {
    check_float(
        "fn probe() -> Float { let f = |x: Float| -> Float { x / 2.0 }; f(6.5) + 0.125 }",
        3.375,
    );
}

#[test]
fn noncapturing_f32_lambda_preserves_arguments_and_result() {
    check_float(
        "fn probe() -> Float { let f = |x: Float32| -> Float32 { x / 2.0 }; f(6.5) + 0.125 }",
        3.375,
    );
}

#[test]
fn capturing_float_lambda_preserves_environment_arguments_and_result() {
    check_float(
        "fn probe() -> Float { let bias = 0.375; let f = |x: Float| -> Float { x / 2.0 + bias }; f(6.5) + 0.125 }",
        3.75,
    );
}

#[test]
fn capturing_f32_lambda_preserves_environment_arguments_and_result() {
    check_float(
        "fn probe() -> Float { let bias = 0.375; let f = |x: Float32| -> Float32 { x / 2.0 + bias }; f(6.5) + 0.125 }",
        3.75,
    );
}

#[test]
fn int_bool_and_unit_callables_keep_their_behavior() {
    let module = compile(
        r#"
        fn negate(x: Bool) -> Bool { !x }
        fn increment(x: Int) -> Int { x + 5 }
        fn empty() { }
        fn probe() -> Int {
            let n: fn(Bool) -> Bool = negate;
            let i: fn(Int) -> Int = increment;
            let u: fn() = empty;
            u();
            if n(false) { i(32) } else { 99 }
        }
    "#,
    );
    let function = module.find_function_by_name("probe").unwrap();
    assert_eq!(
        Interpreter::new(Arc::new(module.clone()))
            .execute_function(function)
            .unwrap()
            .as_i64(),
        37
    );
    with_native(&module, |engine| {
        // SAFETY: exact zero-argument Int source signature.
        assert_eq!(
            unsafe {
                engine
                    .get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .unwrap()
                    .call()
            },
            37
        );
    });
}

#[test]
fn text_callable_preserves_the_nominal_pointer_value() {
    for source in [
        "fn echo(x: Text) -> Text { x } fn probe(text: Text) -> Text { let f: fn(Text) -> Text = echo; f(text) }",
        "fn probe(text: Text) -> Text { let f = |x: Text| -> Text { x }; f(text) }",
    ] {
        let module = compile(source);
        let function = module.find_function_by_name("probe").unwrap();
        let mut interpreter = Interpreter::new(Arc::new(module.clone()));
        let text = interpreter.alloc_string("callback text").expect("text");
        let result = interpreter
            .execute_function_with_args(function, &[text])
            .unwrap();
        assert_eq!(result, text);
        with_native(&module, |engine| {
            let mut storage = [0_u64; 3];
            let text = storage.as_mut_ptr();
            // SAFETY: probe only passes the borrowed opaque Text carrier through
            // the declared callback. No Text fields are read and no owner drops.
            let result = unsafe {
                engine
                    .get_function::<unsafe extern "C" fn(*mut u64) -> *mut u64>("probe")
                    .unwrap()
                    .call(text)
            };
            assert_eq!(result, text);
        });
    }
}

#[test]
fn typed_float_callback_keeps_signature_when_passed_to_another_function() {
    for ty in ["Float", "Float32"] {
        check_float(
            &format!(
                r#"
            fn apply(f: fn({ty}) -> {ty}) -> Float {{ f(6.5) + 0.125 }}
            fn probe() -> Float {{
                let bias = 0.375;
                apply(|x: {ty}| -> {ty} {{ x / 2.0 + bias }})
            }}
        "#
            ),
            3.75,
        );
    }
}

#[test]
fn typed_bool_lambda_keeps_argument_and_result() {
    let module = compile(
        "fn probe() -> Int { let f = |x: Bool| -> Bool { !x }; if f(false) { 37 } else { 99 } }",
    );
    let probe = module.find_function_by_name("probe").unwrap();
    assert_eq!(
        Interpreter::new(Arc::new(module.clone()))
            .execute_function(probe)
            .unwrap()
            .as_i64(),
        37
    );
    with_native(&module, |engine| {
        // SAFETY: exact zero-argument Int source signature.
        assert_eq!(
            unsafe {
                engine
                    .get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .unwrap()
                    .call()
            },
            37
        );
    });
}

#[test]
fn adapter_preserves_target_calling_convention_and_exposes_slot_convention() {
    use verum_codegen::llvm::{context::FunctionContext, instruction::lower_instruction};
    use verum_vbc::{
        instruction::{Instruction, Reg},
        module::FunctionDescriptor,
    };
    for generated_closure in [false, true] {
        let context = Context::create();
        let native = context.create_module("calling_convention");
        let ptr = context.ptr_type(Default::default());
        let i64_type = context.i64_type();
        let name = if generated_closure {
            "owner$closure$0"
        } else {
            "target"
        };
        let target_type = if generated_closure {
            // Shape already matches the slot ABI. Nondefault convention must
            // still receive an adapter instead of taking the shape fast path.
            i64_type.fn_type(&[ptr.into(), i64_type.into()], false)
        } else {
            context
                .f64_type()
                .fn_type(&[context.f64_type().into()], false)
        };
        let target = native.add_function(name, target_type, None);
        target.set_call_conventions(8); // LLVM fastcc, independent of host ABI.
        let caller = native.add_function("make", ptr.fn_type(&[], false), None);
        let mut vbc = VbcModule::new("calling_convention".into());
        let descriptor = FunctionDescriptor::new(vbc.intern_string(name));
        vbc.add_function(descriptor);
        let mut ctx = FunctionContext::with_vbc_module(&context, &native, &vbc, caller, "make");
        ctx.builder()
            .position_at_end(context.append_basic_block(caller, "entry"));
        lower_instruction(
            &mut ctx,
            &Instruction::NewClosure {
                lexical_types: Vec::new(),
                dst: Reg(0),
                func_id: 0,
                captures: Default::default(),
                capture_types: None,
            },
        )
        .expect("emit callable");
        ctx.builder()
            .build_return(Some(&ctx.get_register(0).unwrap()))
            .unwrap();
        let adapter = native
            .get_function(&format!("{name}$trampoline$"))
            .expect("adapter");
        assert_eq!(adapter.get_call_conventions(), 0, "outward value-slot ABI");
        let ir = adapter.print_to_string();
        assert!(ir.to_str().unwrap().contains("call fastcc"), "{ir}");
        native.verify().expect("valid calling-convention IR");
    }
}

#[test]
fn shared_capture_ownership_refuses_before_native_allocation_in_lenient_mode() {
    for discipline in ["affine", "linear"] {
        for mutable in ["", "mut "] {
            let source = format!("type {discipline} Token is {{ value: Int }}; fn probe() -> Int {{ let {mutable}token: Token = Token {{ value: 7 }}; let call = || token.value; call() }}");
            let module = compile(&source);
            let context = Context::create();
            let mut lower = VbcToLlvmLowering::new(&context, LoweringConfig::debug("capture_refusal").with_debug_info(false));
            lower.lower_module(&module).unwrap();
            let probe = lower.module().get_function("probe").unwrap();
            assert!(probe.verify(true));
            let ir = probe.print_to_string().to_string();
            assert!(ir.contains("call void @verum_panic"), "missing executable refusal: {ir}");
            let entry = probe.get_basic_blocks().iter().map(|block| block.get_instructions().map(|i| i.print_to_string().to_string()).collect::<Vec<_>>().join("\n")).find(|block| block.contains("call void @verum_panic")).unwrap();
            assert!(entry.contains("unreachable"), "refusal must terminate before closure allocation: {entry}");
            assert!(!ir.contains("$closure$"), "no callable may be constructed before refusal: {ir}");
        }
    }
}

#[test]
fn generic_creator_uses_specialized_capture_ownership_in_native_code() {
    let module = specialize_closures(compile("fn make<T>(value: T) -> fn() -> T { || value } fn probe() -> Int { let first = make(7); let second = make(13); first() + second() }"));
    with_native(&module, |engine| {
        let actual = unsafe { engine.get_function::<unsafe extern "C" fn() -> i64>("probe").unwrap().call() };
        assert_eq!(actual, 20);
    });
}

fn specialize_closures(mut module: VbcModule) -> VbcModule {
    use verum_vbc::mono::{InstantiationGraph, discover_call_instantiations, monomorphize_minimal};
    let mut graph = InstantiationGraph::new();
    for function in &mut module.functions { function.is_generic = !function.type_params.is_empty(); }
    for function in &module.functions {
        let body = verum_vbc::bytecode::decode_instructions(&module.bytecode[function.bytecode_offset as usize..(function.bytecode_offset + function.bytecode_length) as usize]).unwrap();
        discover_call_instantiations(&module, &body, function.func_id_base, &mut graph).unwrap();
    }
    assert!(!graph.is_empty());
    monomorphize_minimal(module, &graph).unwrap().module
}

// The generic disposal boundary is bytecode; constructors, Drop bodies and
// counter observations are compiled source. This isolates backend cleanup from
// select's separate source integration coverage.
fn typed_cleanup_module(body: &str) -> VbcModule {
    use verum_vbc::{Instruction, bytecode::encode_instruction, instruction::Reg};
    let mut module = compile(&format!(r#"
type Maybe<T> is None | Some(T);
type Guard is {{ drops: &mut Int, marker: Int }};
implement Drop for Guard {{ fn drop(&mut self) {{ *self.drops = *self.drops + 1; }} }}
fn dispose<T>(value: T) {{}}
{body}
"#));
    let id = module.find_function_by_name("dispose").unwrap();
    let ty = module.get_function(id).unwrap().semantic_parameter_types().unwrap()[0].clone().unwrap();
    let start = module.bytecode.len() as u32;
    encode_instruction(&Instruction::DropTyped { src: Reg(0), type_ref: ty }, &mut module.bytecode);
    encode_instruction(&Instruction::RetV, &mut module.bytecode);
    let end = module.bytecode.len() as u32;
    let function = module.functions.iter_mut().find(|function| function.id == id).unwrap();
    function.bytecode_offset = start;
    function.bytecode_length = end - start;
    function.instructions = None;
    let mut module = verum_vbc::deserialize::deserialize_module(&verum_vbc::serialize::serialize_module(&module).unwrap()).unwrap();
    // The production vbc_mono phase materializes archive bodies before LLVM.
    for function in &mut module.functions {
        if function.instructions.is_none() && function.bytecode_length != 0 {
            let start = function.bytecode_offset as usize;
            let end = start + function.bytecode_length as usize;
            function.instructions = Some(verum_vbc::bytecode::decode_instructions(&module.bytecode[start..end]).unwrap().into());
        }
    }
    specialize_closures(module)
}

fn check_cleanup(body: &str, expected: i64) {
    let module = typed_cleanup_module(body);
    let entry = module.find_function_by_name("probe").unwrap();
    assert_eq!(Interpreter::new(Arc::new(module.clone())).execute_function(entry).unwrap().as_i64(), expected);
    with_native(&module, |engine| {
        let actual = unsafe { engine.get_function::<unsafe extern "C" fn() -> i64>("probe").unwrap().call() };
        assert_eq!(actual, expected, "native typed destruction");
    });
}

#[test]
fn typed_cleanup_native_owned_generic_sum_and_record() {
    check_cleanup(r#"
type Packet<T> is Empty | Held(T);
type Envelope<T> is { value: T };
fn probe() -> Int {
    let mut drops = 0;
    dispose<Maybe<Guard>>(Maybe.Some(Guard { drops: &mut drops, marker: 17 }));
    dispose<Packet<Envelope<Guard>>>(Packet.Held(Envelope { value: Guard { drops: &mut drops, marker: 29 } }));
    drops
}
"#, 2);
}

#[test]
fn typed_cleanup_native_borrowed_raw_and_inactive_variants() {
    check_cleanup(r#"
type Packet<T> is Number(Int) | Held(T);
fn probe() -> Int {
    let mut drops = 0;
    let guard = Guard { drops: &mut drops, marker: 41 };
    dispose<Maybe<&Guard>>(Maybe.Some(&guard));
    dispose<Maybe<&unsafe Guard>>(Maybe.Some(&guard as &unsafe Guard));
    dispose<Maybe<Guard>>(Maybe.None);
    dispose<Packet<Guard>>(Packet.Number(27));
    assert(drops == 0);
    guard.marker
}
"#, 41);
}

#[test]
fn typed_cleanup_native_generic_drop_body_is_discovered() {
    check_cleanup(r#"
type Envelope<T> is { drops: &mut Int, value: T };
implement<T> Drop for Envelope<T> { fn drop(&mut self) { *self.drops = *self.drops + 1; } }
fn probe() -> Int {
    let mut drops = 0;
    dispose<Maybe<Envelope<Int>>>(Maybe.Some(Envelope { drops: &mut drops, value: 23 }));
    dispose<Maybe<Envelope<Bool>>>(Maybe.Some(Envelope { drops: &mut drops, value: true }));
    drops
}
"#, 2);
}

#[test]
fn typed_cleanup_native_missing_receipt_cannot_fall_back_to_a_named_body() {
    let mut module = typed_cleanup_module(r#"
type Envelope<T> is { drops: &mut Int, value: T };
implement<T> Drop for Envelope<T> { fn drop(&mut self) { *self.drops = *self.drops + 1; } }
fn probe() -> Int {
    let mut drops = 0;
    dispose<Maybe<Envelope<Int>>>(Maybe.Some(Envelope { drops: &mut drops, value: 23 }));
    drops
}

"#);
    let drop = module.find_function_by_name("Envelope.drop").unwrap();
    let count = module.specializations.len();
    module.specializations.retain(|receipt| receipt.generic_fn != drop);
    assert_eq!(module.specializations.len() + 1, count);
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(&context, LoweringConfig::debug("missing_drop").with_debug_info(false));
    let error = lower.lower_module(&module).expect_err("missing receipt must refuse even with the body present");
    assert!(error.to_string().contains("no exact native destructor specialization"), "{error}");
}

#[test]
fn typed_cleanup_native_missing_payload_declaration_is_a_compile_error() {
    let mut module = typed_cleanup_module(r#"
fn probe() -> Int {
    let mut drops = 0;
    dispose<Maybe<Guard>>(Maybe.Some(Guard { drops: &mut drops, marker: 41 }));
    drops
}
"#);
    let owner = module.types.iter().find(|ty| module.get_string(ty.name) == Some("Maybe")).unwrap().id;
    let payload = module.types.iter_mut().find(|ty| ty.id == owner).unwrap()
        .variants.iter_mut().find(|variant| variant.arity == 1).unwrap();
    payload.fields[0].declaration_type = None;
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(&context, LoweringConfig::debug("missing_drop_field").with_debug_info(false));
    let error = lower.lower_module(&module).expect_err("missing ownership evidence must refuse compilation");
    assert!(error.to_string().contains("field lacks declaration evidence"), "{error}");
}

fn check_native_select_cleanup(body: &str, expected: i64) {
    let module = compile(&format!(r#"
type Poll<T> is Ready(T) | Pending;
type Waker is {{ marker: Int }};
type Context is {{ waker: &Waker }};
implement Context {{ fn from_waker(waker: &Waker) -> Context {{ Context {{ waker: waker }} }} }}
fn noop_waker() -> Waker {{ Waker {{ marker: 0 }} }}
type Thread is ();
implement Thread {{ fn yield_now() {{}} }}
type Future is protocol {{ type Output; fn poll(&mut self, cx: &mut Context) -> Poll<Self.Output>; }};
{body}
"#));
    let entry = module.find_function_by_name("probe").unwrap();
    assert_eq!(Interpreter::new(Arc::new(module.clone())).execute_function(entry).unwrap().as_i64(), expected);
    with_native(&module, |engine| {
        let actual = unsafe { engine.get_function::<unsafe extern "C" fn() -> i64>("probe").unwrap().call() };
        assert_eq!(actual, expected, "native select owned cleanup");
    });
}

#[test]
fn typed_cleanup_native_select_drops_temporary_futures_before_arm_body() {
    check_native_select_cleanup(r#"
type Tracked is { drops: &mut Int, pending: Bool };
implement Drop for Tracked { fn drop(&mut self) { *self.drops = *self.drops + 1; } }
implement Future for Tracked {
    type Output = Int;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Int> {
        if self.pending { Poll.Pending } else { Poll.Ready(5) }
    }
}
async fn probe() -> Int {
    let mut drops = 0;
    select biased {
        a = Tracked { drops: &mut drops, pending: true }.await => 1000,
        b = Tracked { drops: &mut drops, pending: false }.await => drops,
    }
}
"#, 2);
}

#[test]
fn typed_cleanup_native_select_drops_nonmatching_owned_output() {
    check_native_select_cleanup(r#"
type Maybe<T> is None | Some(T);
type Guard is { drops: &mut Int };
implement Drop for Guard { fn drop(&mut self) { *self.drops = *self.drops + 1; } }
type Maker is { drops: &mut Int };
implement Future for Maker {
    type Output = Maybe<Guard>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Maybe<Guard>> {
        Poll.Ready(Maybe.Some(Guard { drops: self.drops }))
    }
}
async fn probe() -> Int {
    let mut drops = 0;
    select biased { Maybe.None = Maker { drops: &mut drops }.await => 1000, else => drops }
}
"#, 1);
}

#[test]
fn escaping_nested_generic_closures_specialize_native_environments() {
    let module = specialize_closures(compile("fn make<T>(value: T) -> fn() -> T { || { let inner = || value; inner() } } fn probe() -> Int { let first = make(17); let second = make(true); if second() { first() } else { 0 } }"));
    with_native(&module, |engine| {
        let actual = unsafe { engine.get_function::<unsafe extern "C" fn() -> i64>("probe").unwrap().call() };
        assert_eq!(actual, 17);
    });
}

#[test]
fn lexical_types_without_value_captures_specialize_native_scalar_abi() {
    let module = specialize_closures(compile("fn make<T>() -> fn(T) -> T { |value: T| -> T { value } } fn probe() -> Float { let f = make<Float>(); f(17.25) }"));
    with_native(&module, |engine| {
        let actual = unsafe { engine.get_function::<unsafe extern "C" fn() -> f64>("probe").unwrap().call() };
        assert_eq!(actual, 17.25);
    });
}

#[test]
fn nested_closure_parameter_capture_keeps_its_declared_native_type() {
    let module = specialize_closures(compile("fn make<T>() -> fn(T) -> T { |value: T| -> T { let inner = || value; inner() } } fn probe() -> Float { let f = make<Float>(); f(17.25) }"));
    let entry = module.find_function_by_name("probe").unwrap();
    assert_eq!(Interpreter::new(Arc::new(module.clone())).execute_function(entry).unwrap().as_f64(), 17.25);
    with_native(&module, |engine| {
        let actual = unsafe { engine.get_function::<unsafe extern "C" fn() -> f64>("probe").unwrap().call() };
        assert_eq!(actual, 17.25);
    });
}

#[test]
fn generic_receiver_field_capture_preserves_its_native_declaration() {
    let module = specialize_closures(compile("type Cell<T> is { value: T }; implement<T> Cell<T> { fn make(&self) -> fn() -> T { let value = self.value; || value } } fn probe() -> Float { let cell: Cell<Float> = Cell { value: 17.25 }; let f = cell.make(); f() }"));
    let entry = module.find_function_by_name("probe").unwrap();
    assert_eq!(Interpreter::new(Arc::new(module.clone())).execute_function(entry).unwrap().as_f64(), 17.25);
    with_native(&module, |engine| {
        let actual = unsafe { engine.get_function::<unsafe extern "C" fn() -> f64>("probe").unwrap().call() };
        assert_eq!(actual, 17.25);
    });
}

#[test]
fn documented_shared_environments_execute_in_both_tiers() {
    for (source, expected) in [
        ("fn probe() -> Int { let value: Int = 7; let f = || value; f() }", 7),
        ("fn probe() -> Int { let mut value: Int = 7; let f = || { value = value + 1; value }; f(); f() }", 9),
    ] {
        let module = compile(source);
        assert_eq!(Interpreter::new(Arc::new(module.clone())).execute_function(module.find_function_by_name("probe").unwrap()).unwrap().as_i64(), expected);
        with_native(&module, |engine| {
            assert_eq!(unsafe { engine.get_function::<unsafe extern "C" fn() -> i64>("probe").unwrap().call() }, expected);
        });
    }
}

#[test]
fn captured_missing_targets_cannot_become_lenient_fallback_bodies() {
    for target in [0x7FFF_FFFF, 0x2000_0000] {
        let mut module = compile("fn probe() -> Int { let value: Int = 7; let f = || value; f() }");
        let original = std::mem::take(&mut module.bytecode);
        for function in &mut module.functions {
            let mut body = verum_vbc::bytecode::decode_instructions(&original[function.bytecode_offset as usize..(function.bytecode_offset + function.bytecode_length) as usize]).unwrap();
            for instruction in &mut body {
                if let verum_vbc::Instruction::NewClosure { func_id, captures, .. } = instruction {
                    if !captures.is_empty() { *func_id = target; }
                }
            }
            verum_vbc::bytecode::fixup_jump_offsets(&mut body);
            function.bytecode_offset = module.bytecode.len() as u32;
            for instruction in &body { verum_vbc::bytecode::encode_instruction(instruction, &mut module.bytecode); }
            function.bytecode_length = module.bytecode.len() as u32 - function.bytecode_offset;
            function.instructions = Some(body);
        }
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(&context, LoweringConfig::debug("missing_capture_target").with_debug_info(false));
        let error = lower.lower_module(&module).unwrap_err();
        assert!(matches!(error, verum_codegen::llvm::LlvmLoweringError::UnprovenCallableEnvironment(_)), "{error}");
    }
}
