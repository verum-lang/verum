//! T1573: native reference representation survives generic sum projections.
use std::cell::RefCell;
use verum_codegen::llvm::{LoweringConfig, RefTier, VbcToLlvmLowering};
use verum_common::{Heap, List, Text};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    memory_buffer::MemoryBuffer,
    targets::{InitializationConfig, Target},
    values::AnyValue,
};
use verum_vbc::codegen::VbcCodegen;

thread_local! { static ALLOCATIONS: RefCell<List<Heap<[u64]>>> = RefCell::new(List::new()); }
extern "C" fn allocate(size: u64) -> *mut u64 {
    ALLOCATIONS.with(|allocations| {
        let mut storage = List::from_elem(0_u64, size.div_ceil(8) as usize).into_boxed_slice();
        let pointer = storage.as_mut_ptr();
        allocations.borrow_mut().push(storage);
        pointer
    })
}
extern "C" fn unexpected_panic(_message: u64, _len: u64, _file: u64, _line: u32) {
    std::process::abort();
}
extern "C" fn unexpected_exit(_code: u64) {
    std::process::abort();
}
const SOURCE: &str = r#"
    type Parcel<T> is Absent | Present(T);
    type Cell is { padding: Int, value: Int };
    fn borrow_payload<T>(p: &Parcel<T>) -> Parcel<&T> {
        match p { Parcel.Present(ref value) => Parcel.Present(value), Parcel.Absent => Parcel.Absent }
    }
    fn project<T>(p: Parcel<T>) -> T {
        match p { Parcel.Present(value) => value, Parcel.Absent => panic("absent") }
    }
    fn first(p: &Parcel<Cell>) -> &Cell { project(borrow_payload(p)) }
    fn forward(p: &Parcel<Cell>) -> &Cell { first(p) }
    fn scalar_first(p: &Parcel<Int>) -> &Int { project(borrow_payload(p)) }
    fn scalar_forward(p: &Parcel<Int>) -> &Int { scalar_first(p) }
    fn value_first(p: Parcel<Int>) -> Int { project(p) }
    fn value_forward(p: Parcel<Int>) -> Int { value_first(p) }
    fn record_read(p: &Parcel<Cell>) -> Int { forward(p).value }
    fn scalar_read(p: &Parcel<Int>) -> Int { *scalar_forward(p) }
    fn value_read(p: Parcel<Int>) -> Int { value_forward(p) }
    implement Cell { fn read(&self) -> Int { self.value } }
    fn method_read(p: &Parcel<Cell>) -> Int { forward(p).read() }
    fn mut_first(p: &mut Parcel<Int>) -> &mut Int {
        match p { Parcel.Present(ref mut value) => value, Parcel.Absent => panic("absent") }
    }
    fn mut_forward(p: &mut Parcel<Int>) -> &mut Int { mut_first(p) }
    fn original_write(p: &mut Parcel<Int>, replacement: Int) { *mut_forward(p) = replacement; }
    type RefCell is { link: &Int };
    fn held_reference(p: &RefCell) -> &Int { p.link }
    fn held_read(p: &RefCell) -> Int { *held_reference(p) }
    fn mixed(p: &Parcel<Cell>, other: &Cell, choose: Bool) -> &Cell {
        if choose { forward(p) } else { other }
    }
    fn mixed_read(p: &Parcel<Cell>, other: &Cell, choose: Bool) -> Int { mixed(p, other, choose).value }
    fn replace(p: &mut Parcel<&Cell>, replacement: &Cell) {
        match p { Parcel.Present(ref mut value) => *value = replacement, Parcel.Absent => {} }
    }
    fn read_effect_forward(p: &Cell, storage: &mut Parcel<&Cell>, replacement: &Cell) -> &Cell {
        let observed = p.value;
        replace(storage, replacement);
        p
    }
    fn mixed_effect_identity(p: &Parcel<Cell>, storage: &mut Parcel<&Cell>, replacement: &Cell) -> &Cell {
        read_effect_forward(forward(p), storage, replacement)
    }
    fn read_before_write(p: &mut Parcel<&Cell>, replacement: &Cell) -> &Cell {
        let saved = project(p);
        replace(p, replacement);
        saved
    }
    fn read_after_write(p: &mut Parcel<&Cell>, replacement: &Cell) -> &Cell {
        replace(p, replacement);
        project(p)
    }
    fn before_mutation_read(p: &Parcel<Cell>, replacement: &Cell) -> Int {
        let wrapped = borrow_payload(p);
        read_before_write(wrapped, replacement).value
    }
    fn after_mutation_read(p: &Parcel<Cell>, replacement: &Cell) -> Int {
        let wrapped = borrow_payload(p);
        read_after_write(wrapped, replacement).value
    }
    fn record_mut_first(p: &mut Parcel<Cell>) -> &mut Cell {
        match p { Parcel.Present(ref mut value) => value, Parcel.Absent => panic("absent") }
    }
    fn record_mut_forward(p: &mut Parcel<Cell>) -> &mut Cell { record_mut_first(p) }
    fn original_record_write(p: &mut Parcel<Cell>, replacement: Int) { record_mut_forward(p).value = replacement; }
    type Holder is { cell: Cell };
    fn direct_cell_ref(holder: &Holder) -> &Cell { &holder.cell }
    fn already_loaded(holder: &Holder) -> Int { return direct_cell_ref(holder).value; }
    type Factory is ();
    implement Factory { fn make(p: &Parcel<Cell>) -> &Cell { forward(p) } }
    fn static_read(factory: &Factory, p: &Parcel<Cell>) -> Int { factory.make(p).value }
    fn recursive(p: &Parcel<Cell>) -> &Cell { recursive(p) }
    fn recursive_read(p: &Parcel<Cell>) -> Int { recursive(p).value }

"#;

fn with_ir(check: impl FnOnce(&Context, &str)) {
    with_source_ir(SOURCE, check);
}

fn with_source_ir(source: &str, check: impl FnOnce(&Context, &str)) {
    let ast = Parser::new(source).parse_module().expect("source parses");
    let vbc = VbcCodegen::new().compile_module(&ast).expect("source VBC");
    if std::env::var_os("VERUM_REFERENCE_TEST_TRACE").is_some() {
        for f in &vbc.functions {
            eprintln!(
                "{} {:?} {:?}",
                vbc.get_string(f.name).unwrap(),
                f.params.iter().map(|p| &p.type_ref).collect::<List<_>>(),
                f.instructions
            );
        }
    }
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("returned_refs")
            .with_default_tier(RefTier::Tier1)
            .with_debug_info(false),
    );
    lower.lower_module(&vbc).expect("source LLVM");
    let mut ir = Text::from(
        "declare ptr @verum_checked_malloc(i64)\ndeclare void @verum_internal_exit_i64(i64)\ndeclare void @verum_panic(ptr, i64, ptr, i32)\n",
    );
    for function in lower.module().get_functions() {
        if function.count_basic_blocks() == 0
            && ![
                b"verum_checked_malloc".as_slice(),
                b"verum_internal_exit_i64".as_slice(),
                b"verum_panic".as_slice(),
            ]
            .contains(&function.get_name().to_bytes())
        {
            ir.push_str(function.print_to_string().to_str().expect("declaration"));
        }
    }
    ir.push_str(
        lower
            .module()
            .get_function("verum_internal_memset")
            .expect("memset")
            .print_to_string()
            .to_str()
            .expect("memset IR"),
    );
    for global in lower.module().get_globals() {
        ir.push_str(global.print_to_string().to_str().expect("global"));
        ir.push('\n');
    }
    for descriptor in &vbc.functions {
        let name = vbc.get_string(descriptor.name).expect("name");
        let function = lower.module().get_function(name).expect(name);
        assert!(function.verify(true), "{name}");
        ir.push_str(function.print_to_string().to_str().expect("source IR"));
    }
    check(&context, &ir);
}

#[test]
fn source_projection_preserves_the_original_record_cell() {
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    with_ir(|context, ir| {
        // First verify the new normalization before calling the record consumer:
        // an old emitter would read object fields relative to the payload cell.
        let reader = ir
            .split("@record_read(")
            .nth(1)
            .expect("reader")
            .split("\n}")
            .next()
            .unwrap();
        assert!(
            reader.contains("reference_value_load"),
            "missing value view for forwarded slot: {reader}"
        );
        let module = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(),
                "source",
            ))
            .expect("IR parse");
        module.verify().expect("valid IR");
        let engine = module
            .create_jit_execution_engine(OptimizationLevel::None)
            .expect("JIT");
        engine.add_global_mapping(
            &module.get_function("verum_checked_malloc").unwrap(),
            allocate as *const () as usize,
        );
        engine.add_global_mapping(
            &module.get_function("verum_internal_exit_i64").unwrap(),
            unexpected_exit as *const () as usize,
        );
        engine.add_global_mapping(
            &module.get_function("verum_panic").unwrap(),
            unexpected_panic as *const () as usize,
        );
        let mut cell = [0_u64, 0, 0, 11, 73];
        let mut parcel = [0_u64; 8];
        parcel[3] = 1; // tag=Present, payload follows the variant data header.
        parcel[4] = cell.as_mut_ptr() as u64;
        // SAFETY: source functions accept one native slot and return one slot;
        // all record/variant fields and the original cell outlive every call.
        unsafe {
            let forward = engine
                .get_function::<unsafe extern "C" fn(u64) -> u64>("forward")
                .unwrap();
            assert_eq!(
                forward.call(parcel.as_mut_ptr() as u64),
                parcel.as_mut_ptr().add(4) as u64
            );
            let read = engine
                .get_function::<unsafe extern "C" fn(u64) -> u64>("record_read")
                .unwrap();
            assert_eq!(read.call(parcel.as_mut_ptr() as u64), 73);
            let method = engine
                .get_function::<unsafe extern "C" fn(u64) -> u64>("method_read")
                .unwrap();
            assert_eq!(method.call(parcel.as_mut_ptr() as u64), 73);
            let static_read = engine
                .get_function::<unsafe extern "C" fn(u64, u64) -> u64>("static_read")
                .unwrap();
            assert_eq!(static_read.call(0, parcel.as_mut_ptr() as u64), 73);
            let write_record = engine
                .get_function::<unsafe extern "C" fn(u64, u64)>("original_record_write")
                .unwrap();
            write_record.call(parcel.as_mut_ptr() as u64, 79);
            assert_eq!(cell[4], 79);
            assert_eq!(parcel[4], cell.as_mut_ptr() as u64);
            std::ptr::write_volatile(cell.as_mut_ptr().add(4), 73);
            let mut holder = [0_u64, 0, 0, cell.as_mut_ptr() as u64];
            let loaded = engine
                .get_function::<unsafe extern "C" fn(u64) -> u64>("already_loaded")
                .unwrap();
            assert_eq!(loaded.call(holder.as_mut_ptr() as u64), 73);
            let mut replacement = [0_u64, 0, 0, 17, 97];
            let before = engine
                .get_function::<unsafe extern "C" fn(u64, u64) -> u64>("read_before_write")
                .unwrap();
            let after = engine
                .get_function::<unsafe extern "C" fn(u64, u64) -> u64>("read_after_write")
                .unwrap();
            let before_read = engine
                .get_function::<unsafe extern "C" fn(u64, u64) -> u64>("before_mutation_read")
                .unwrap();
            let after_read = engine
                .get_function::<unsafe extern "C" fn(u64, u64) -> u64>("after_mutation_read")
                .unwrap();
            assert_eq!(
                before_read.call(parcel.as_mut_ptr() as u64, replacement.as_mut_ptr() as u64),
                73
            );
            assert_eq!(
                after_read.call(parcel.as_mut_ptr() as u64, replacement.as_mut_ptr() as u64),
                97
            );
            let original_cell = parcel.as_mut_ptr().add(4) as u64;
            let mut packet = [0_u64; 8];
            packet[3] = 1;
            packet[4] = original_cell;
            assert_eq!(
                before.call(packet.as_mut_ptr() as u64, replacement.as_mut_ptr() as u64),
                original_cell
            );
            assert_eq!(packet[4], replacement.as_mut_ptr() as u64);
            packet[4] = original_cell;
            assert_eq!(
                after.call(packet.as_mut_ptr() as u64, replacement.as_mut_ptr() as u64),
                replacement.as_mut_ptr() as u64
            );
            parcel[4] = 73;
            let scalar = engine
                .get_function::<unsafe extern "C" fn(u64) -> u64>("scalar_read")
                .unwrap();
            assert_eq!(scalar.call(parcel.as_mut_ptr() as u64), 73);
            let value = engine
                .get_function::<unsafe extern "C" fn(u64) -> u64>("value_read")
                .unwrap();
            assert_eq!(value.call(parcel.as_mut_ptr() as u64), 73);
            let write = engine
                .get_function::<unsafe extern "C" fn(u64, u64)>("original_write")
                .unwrap();
            write.call(parcel.as_mut_ptr() as u64, 91);
            assert_eq!(parcel[4], 91);
            let mut original = 37_u64;
            let mut holder = [0_u64, 0, 0, (&mut original as *mut u64) as u64];
            let held = engine
                .get_function::<unsafe extern "C" fn(u64) -> u64>("held_read")
                .unwrap();
            assert_eq!(held.call(holder.as_mut_ptr() as u64), 37);
        }
    });
}

#[test]
fn unknown_and_value_results_do_not_gain_speculative_loads() {
    with_ir(|_, ir| {
        for name in [
            "value_read",
            "held_read",
            "mixed_read",
            "mixed_effect_identity",
            "recursive_read",
            "after_mutation_read",
            "already_loaded",
        ] {
            let marker = Text::from(format!("@{name}("));
            let function = ir
                .split(marker.as_str())
                .nth(1)
                .expect(name)
                .split("\n}")
                .next()
                .unwrap();
            assert!(
                !function.contains("reference_value_load"),
                "{name}: {function}"
            );
        }
    });
}

#[test]
fn actual_native_replacement_is_opaque_even_with_a_readable_reference_body() {
    let mut source = Text::from(SOURCE);
    source.push_str(
        r#"    type Trap is { packet: &Parcel<Cell> };
    implement Trap { fn hash_value(&self) -> &Cell { forward(self.packet) } }
    fn intercepted_read(trap: &Trap) -> Int { trap.hash_value().value }
"#,
    );
    with_source_ir(&source, |_, ir| {
        let caller = ir
            .split("@intercepted_read(")
            .nth(1)
            .expect("caller")
            .split("\n}")
            .next()
            .unwrap();
        assert!(
            caller.contains("verum_generic_hash"),
            "negative must exercise the actual existing replacement: {caller}"
        );
        assert!(
            !caller.contains("reference_value_load"),
            "runtime replacement cannot inherit the source reference summary: {caller}"
        );
    });
}

#[test]
fn a_real_eager_adapter_is_not_loaded_again() {
    with_ir(|_, ir| {
        let caller = ir
            .split("@already_loaded(")
            .nth(1)
            .expect("caller")
            .split("\n}")
            .next()
            .unwrap();
        assert!(
            caller.contains("ret_slot_load"),
            "control must exercise the existing emitted load: {caller}"
        );
        assert!(
            !caller.contains("reference_value_load"),
            "already-loaded carrier must not inherit the raw slot fact: {caller}"
        );
    });
}

#[test]
fn release_effects_kill_future_storage_projections_not_saved_words() {
    let mut source = Text::from(SOURCE);
    source.push_str(
        r#"
        fn project_then_release(p: Parcel<&Cell>, disposable: Cell) -> &Cell {
            let saved = project(p);
            drop(disposable);
            saved
        }
        fn release_then_project(p: Parcel<&Cell>, disposable: Cell) -> &Cell {
            drop(disposable);
            project(p)
        }
        fn before_release_read(p: &Parcel<Cell>, disposable: Cell) -> Int {
            let wrapped = borrow_payload(p);
            project_then_release(wrapped, disposable).value
        }
        fn after_release_read(p: &Parcel<Cell>, disposable: Cell) -> Int {
            let wrapped = borrow_payload(p);
            release_then_project(wrapped, disposable).value
        }
    "#,
    );
    with_source_ir(&source, |_, ir| {
        let before = ir
            .split("@before_release_read(")
            .nth(1)
            .unwrap()
            .split("\n}")
            .next()
            .unwrap();
        let after = ir
            .split("@after_release_read(")
            .nth(1)
            .unwrap()
            .split("\n}")
            .next()
            .unwrap();
        assert!(
            before.contains("reference_value_load"),
            "saved word remains a slot: {before}"
        );
        assert!(
            !after.contains("reference_value_load"),
            "DropRef may invalidate future aggregate reads: {after}"
        );
    });
}
