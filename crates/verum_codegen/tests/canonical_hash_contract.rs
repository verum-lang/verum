//! T1587: native primitive hashing follows the declared DefaultHasher byte stream.
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::{List, Set, Text};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    memory_buffer::MemoryBuffer,
    module::Module,
    targets::{InitializationConfig, Target},
    values::AnyValue,
};
use verum_vbc::codegen::VbcCodegen;

fn reachable_ir(module: &Module, roots: &[&str]) -> Text {
    let mut ir = Text::new();
    let full = module.print_to_string();
    for line in full.to_str().expect("IR UTF8").lines() {
        if line.starts_with("target ")
            || line.starts_with("attributes #")
            || (line.starts_with('%') && line.contains(" = type "))
        {
            ir.push_str(line);
            ir.push('\n');
        }
    }
    let mut pending: List<Text> = roots.iter().map(|name| Text::from(*name)).collect();
    let mut seen = Set::new();
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let text = if let Some(function) = module.get_function(&name) {
            function.print_to_string()
        } else if let Some(global) = module.get_global(&name) {
            global.print_to_string()
        } else {
            continue;
        };
        let text = text.to_str().expect("item UTF8");
        ir.push_str(text);
        ir.push('\n');
        for tail in text.split('@').skip(1) {
            let name = tail
                .split(|c: char| !c.is_ascii_alphanumeric() && !"_.$".contains(c))
                .next()
                .expect("split head");
            if module.get_function(name).is_some() || module.get_global(name).is_some() {
                pending.push(Text::from(name));
            }
        }
    }
    ir
}

fn with_source(source: &str, check: impl for<'ctx> FnOnce(&'ctx Context, &Module<'ctx>)) {
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    let ast = Parser::new(source).parse_module().expect("grammar");
    let vbc = VbcCodegen::new().compile_module(&ast).expect("source VBC");
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("hash_contract").with_debug_info(false),
    );
    lowering.lower_module(&vbc).expect("source LLVM");
    check(&context, lowering.module());
}

// This oracle spells the public DefaultHasher contract, independent of the
// emitted LLVM implementation: state0, rotate5/XORbyte/wrapping multiply.
fn fxhash(bytes: &[u8]) -> i64 {
    bytes.iter().fold(0u64, |state, byte| {
        (state.rotate_left(5) ^ u64::from(*byte)).wrapping_mul(0x517cc1b727220a95)
    }) as i64
}

#[test]
fn source_int_hash_matches_canonical_little_endian_bytes() {
    with_source(
        "fn hash_probe(value: Int) -> Int { value.hash_value() }",
        |context, full| {
            let text = reachable_ir(full, &["hash_probe"]);
            let module = context
                .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                    text.as_bytes(),
                    "int_hash",
                ))
                .expect("reachable IR");
            module.verify().expect("verified");
            let engine = module
                .create_jit_execution_engine(OptimizationLevel::None)
                .expect("JIT");
            // SAFETY: source function accepts and returns one signed Int value slot.
            let hash =
                unsafe { engine.get_function::<unsafe extern "C" fn(i64) -> i64>("hash_probe") }
                    .expect("hash probe");
            let values = [7, 0, -1, i64::MIN, i64::MAX, -73, 91];
            let actual: List<_> = values
                .iter()
                .map(|value| unsafe { hash.call(*value) })
                .collect();
            let expected: List<_> = values
                .iter()
                .map(|value| fxhash(&value.to_le_bytes()))
                .collect();
            assert_eq!(actual, expected);
        },
    );
}

#[test]
fn typed_int_hash_does_not_inspect_the_value_as_an_object_address() {
    with_source(
        "fn hash_probe(value: Int) -> Int { value.hash_value() }",
        |_, module| {
            let ir = reachable_ir(module, &["hash_probe"]);
            assert!(
                !ir.contains("@verum_is_text_object("),
                "a carried Int is never a speculative Text address: {ir}"
            );
        },
    );
}

#[test]
fn pointer_shaped_integer_bits_are_hashed_without_loading_memory() {
    with_source(
        "fn hash_probe(value: Int) -> Int { value.hash_value() }",
        |context, full| {
            let text = reachable_ir(full, &["hash_probe"]);
            assert!(
                !text.contains("@verum_is_text_object("),
                "do not execute a speculative load"
            );
            let module = context
                .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                    text.as_bytes(),
                    "pointer_bits",
                ))
                .expect("reachable IR");
            module.verify().expect("verified");
            let engine = module
                .create_jit_execution_engine(OptimizationLevel::None)
                .expect("JIT");
            // SAFETY: the source signature is Int→Int; the structural assertion
            // above refuses to execute legacy address classification on these bits.
            let hash =
                unsafe { engine.get_function::<unsafe extern "C" fn(i64) -> i64>("hash_probe") }
                    .unwrap();
            for value in [0x1000_0000i64, 0x1_0000_0000, 0x7fff_ffff_fff0] {
                assert_eq!(unsafe { hash.call(value) }, fxhash(&value.to_le_bytes()));
            }
        },
    );
}

fn canonical_source() -> Text {
    let protocols = include_str!("../../../core/base/protocols.vr");
    let begin = protocols
        .find("public type Hash is protocol")
        .expect("Hash source");
    let end = protocols.find("// Clone Protocol").expect("next protocol");
    let mut source = Text::from(&protocols[begin..end]);
    let primitives = include_str!("../../../core/base/primitives.vr");
    let begin = primitives
        .find("implement Hash for Int {")
        .expect("Int Hash source");
    let end = begin + primitives[begin..].find("\n}\n").expect("impl end") + 3;
    source.push_str(&primitives[begin..end]);
    // The actual DefaultHasher body calls these two actual primitive bodies;
    // include its source dependencies rather than substitute a test hasher.
    source.push_str(" implement Int { ");
    for signature in [
        "public fn wrapping_mul(self, rhs: Int)",
        "public fn rotate_left(self, n: Int)",
    ] {
        let begin = primitives
            .find(signature)
            .expect("hasher primitive dependency");
        let end = begin + primitives[begin..].find("\n    }").expect("method end") + 6;
        source.push_str(&primitives[begin..end]);
    }
    source.push_str(" }");
    source
}

#[test]
fn real_declared_hash_default_agrees_with_the_byte_contract() {
    let mut source = canonical_source();
    source.push_str(" fn declared_hash(value: Int) -> Int { value.hash_value() }");
    with_source(&source, |context, full| {
        let ir = reachable_ir(full, &["declared_hash"]);
        assert!(!ir.contains("@verum_is_text_object("));
        let module = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(),
                "declared_hash",
            ))
            .expect("reachable IR");
        module.verify().expect("verified");
        let engine = module
            .create_jit_execution_engine(OptimizationLevel::None)
            .expect("JIT");
        // SAFETY: the public source wrapper is Int→Int.
        let hash =
            unsafe { engine.get_function::<unsafe extern "C" fn(i64) -> i64>("declared_hash") }
                .unwrap();
        for value in [7i64, 0, -1, i64::MIN, i64::MAX] {
            assert_eq!(unsafe { hash.call(value) }, fxhash(&value.to_le_bytes()));
        }
    });
}

#[test]
fn erased_integer_and_ascii_text_keys_share_the_canonical_mixer() {
    #[repr(C, align(16))]
    struct NativeText {
        data: u64,
        len: u64,
        capacity: u64,
    }
    with_source(
        "fn hash_probe(value: Int) -> Int { value.hash_value() }",
        |context, full| {
            let ir = reachable_ir(full, &["verum_generic_hash", "verum_hash_byte"]);
            let module = context
                .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                    ir.as_bytes(),
                    "erased_hash",
                ))
                .expect("reachable IR");
            module.verify().expect("verified");
            let engine = module
                .create_jit_execution_engine(OptimizationLevel::None)
                .expect("JIT");
            // SAFETY: exact emitted i64→i64 and (i64,i64)→i64 helper signatures.
            let hash = unsafe {
                engine.get_function::<unsafe extern "C" fn(i64) -> i64>("verum_generic_hash")
            }
            .unwrap();
            let byte = unsafe {
                engine.get_function::<unsafe extern "C" fn(i64, i64) -> i64>("verum_hash_byte")
            }
            .unwrap();
            // These values are outside the legacy erased-key pointer domain.
            for value in [7i64, 0, -1, i64::MIN, i64::MAX] {
                assert_eq!(unsafe { hash.call(value) }, fxhash(&value.to_le_bytes()));
            }
            // A real live flat Text ABI object; no fabricated address/header test.
            // Embedded NUL handling is an existing, separately scoped limitation.
            for bytes in [b"".as_slice(), b"key", b"another-key"] {
                let mut terminated: List<u8> = bytes.iter().copied().collect();
                terminated.push(0);
                let object = NativeText {
                    data: terminated.as_ptr() as u64,
                    len: bytes.len() as u64,
                    capacity: 0,
                };
                let value = &object as *const NativeText as i64;
                assert_eq!(unsafe { hash.call(value) }, fxhash(bytes));
            }
            // Streamed writes preserve seed state, including empty writes.
            let mut state = 0;
            for value in [7i64, -1, i64::MIN] {
                for b in value.to_le_bytes() {
                    state = unsafe { byte.call(state, i64::from(b)) };
                }
            }
            let bytes: List<u8> = [7i64, -1, i64::MIN]
                .into_iter()
                .flat_map(i64::to_le_bytes)
                .collect();
            assert_eq!(state, fxhash(&bytes));
        },
    );
}

#[test]
fn explicit_default_hasher_write_int_agrees_with_native_scalar_hash() {
    let mut source = canonical_source();
    source.push_str(
        r#"
        fn explicit_hash(value: Int) -> Int {
            let mut hasher = DefaultHasher.new();
            hasher.write_int(value);
            hasher.finish()
        }
    "#,
    );
    with_source(&source, |context, full| {
        let ir = reachable_ir(full, &["explicit_hash"]);
        let module = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(),
                "explicit_hash",
            ))
            .expect("reachable IR");
        module.verify().expect("verified");
        let engine = module
            .create_jit_execution_engine(OptimizationLevel::None)
            .expect("JIT");
        // SAFETY: exact public source wrapper accepts and returns one Int.
        let hash =
            unsafe { engine.get_function::<unsafe extern "C" fn(i64) -> i64>("explicit_hash") }
                .unwrap();
        for value in [7i64, 0, -1, i64::MIN, i64::MAX] {
            assert_eq!(unsafe { hash.call(value) }, fxhash(&value.to_le_bytes()));
        }
    });
}

#[test]
fn source_borrowed_int_hash_reads_the_scalar_cell_once() {
    with_source(
        r#"
        fn borrowed_hash(value: &Int) -> Int { value.hash_value() }
        fn borrow_probe(value: Int) -> Int { borrowed_hash(&value) }
    "#,
        |context, full| {
            let ir = reachable_ir(full, &["borrow_probe"]);
            assert!(
                !ir.contains("@verum_is_text_object("),
                "known Int never classifies bits"
            );
            let module = context
                .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                    ir.as_bytes(),
                    "borrowed_hash",
                ))
                .expect("reachable IR");
            module.verify().expect("verified");
            let engine = module
                .create_jit_execution_engine(OptimizationLevel::None)
                .expect("JIT");
            // SAFETY: the source wrapper creates a live local borrow, and its
            // external signature is one Int value slot in and out.
            let hash =
                unsafe { engine.get_function::<unsafe extern "C" fn(i64) -> i64>("borrow_probe") }
                    .unwrap();
            for value in [7i64, 0, -1, 0x1_0000_0000] {
                assert_eq!(unsafe { hash.call(value) }, fxhash(&value.to_le_bytes()));
            }
        },
    );
}

#[test]
fn native_map_key_operations_keep_the_same_canonical_hash() {
    use verum_codegen::llvm::runtime::RuntimeLowering;
    use verum_common::layout::{
        MAP_CAP_OFFSET, MAP_ENTRIES_OFFSET, MAP_LEN_OFFSET, MAP_TOMBSTONES_OFFSET,
        VARIANT_PAYLOAD_OFFSET, VARIANT_TAG_OFFSET,
    };
    use verum_common::well_known_types::{maybe_none_tag, maybe_success_tag};
    with_source(
        "fn hash_probe(value: Int) -> Int { value.hash_value() }",
        |context, full| {
            // Exercise the actual emitted Map replacement bodies on their declared
            // ABI. Construction/resizing and Set source forwarding have separate
            // public differential coverage; this table has spare capacity throughout.
            let i64_type = context.i64_type();
            let builder = context.create_builder();
            for (name, arity) in [
                ("Map.insert", 3),
                ("Map.get", 2),
                ("Map.contains_key", 2),
                ("Map.remove", 2),
            ] {
                let params: List<_> = (0..arity).map(|_| i64_type.into()).collect();
                let function = full.add_function(name, i64_type.fn_type(&params, false), None);
                builder.position_at_end(context.append_basic_block(function, "placeholder"));
                builder.build_return(Some(&i64_type.const_zero())).unwrap();
            }
            RuntimeLowering::new(context)
                .emit_text_ir_functions(full)
                .expect("Map runtime bodies");
            let ir = reachable_ir(
                full,
                &[
                    "Map.insert",
                    "Map.get",
                    "Map.contains_key",
                    "Map.remove",
                    "verum_os_free",
                ],
            );
            let module = context
                .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                    ir.as_bytes(),
                    "map_hash",
                ))
                .expect("reachable IR");
            module.verify().expect("verified");
            let engine = module
                .create_jit_execution_engine(OptimizationLevel::None)
                .expect("JIT");
            // SAFETY: exact i64 slot signatures created above and the runtime's
            // allocation release signature. All table and payload storage is live.
            let insert = unsafe {
                engine.get_function::<unsafe extern "C" fn(i64, i64, i64) -> i64>("Map.insert")
            }
            .unwrap();
            let get =
                unsafe { engine.get_function::<unsafe extern "C" fn(i64, i64) -> i64>("Map.get") }
                    .unwrap();
            let contains = unsafe {
                engine.get_function::<unsafe extern "C" fn(i64, i64) -> i64>("Map.contains_key")
            }
            .unwrap();
            let remove = unsafe {
                engine.get_function::<unsafe extern "C" fn(i64, i64) -> i64>("Map.remove")
            }
            .unwrap();
            let free = unsafe {
                engine.get_function::<unsafe extern "C" fn(*mut u8, u64)>("verum_os_free")
            }
            .unwrap();
            let consume = |value: i64, expected: Option<i64>| {
                assert_ne!(value, 0, "Maybe result is allocated");
                let ptr = value as *mut u8;
                // SAFETY: Map returns a runtime-allocated Maybe, whose tag always
                // exists and whose payload exists only for Some. Read then release
                // through the matching verum_os_alloc/verum_os_free pair.
                unsafe {
                    let tag = ptr.add(VARIANT_TAG_OFFSET as usize).cast::<u32>().read();
                    assert_eq!(
                        tag,
                        if expected.is_some() {
                            maybe_success_tag()
                        } else {
                            maybe_none_tag()
                        }
                    );
                    if let Some(expected) = expected {
                        assert_eq!(
                            ptr.add(VARIANT_PAYLOAD_OFFSET as usize)
                                .cast::<i64>()
                                .read(),
                            expected
                        );
                    }
                    free.call(
                        ptr,
                        VARIANT_PAYLOAD_OFFSET + if expected.is_some() { 8 } else { 0 },
                    );
                }
            };
            let mut entries = [[0i64; 4]; 16];
            let mut map = [0i64; 7];
            map[MAP_CAP_OFFSET as usize / 8] = entries.len() as i64;
            map[MAP_ENTRIES_OFFSET as usize / 8] = entries.as_mut_ptr() as i64;
            let address = map.as_mut_ptr() as i64;
            let keys = [7i64, 0, -1, i64::MIN, i64::MAX];
            for (index, key) in keys.iter().copied().enumerate() {
                consume(
                    unsafe { insert.call(address, key, 100 + index as i64) },
                    None,
                );
                let raw = fxhash(&key.to_le_bytes());
                let positive = raw.wrapping_abs();
                let expected = if positive as u64 <= 1 {
                    positive + 2
                } else {
                    positive
                };
                let slot = entries
                    .iter()
                    .find(|slot| slot[2] != 0 && slot[0] == key)
                    .expect("inserted key");
                assert_eq!(slot[2], expected, "stored hash must use declared bytes");
            }
            assert_eq!(map[MAP_LEN_OFFSET as usize / 8], 5);
            for (index, key) in keys.iter().copied().enumerate() {
                assert_eq!(unsafe { contains.call(address, key) }, 1);
                consume(unsafe { get.call(address, key) }, Some(100 + index as i64));
                consume(
                    unsafe { insert.call(address, key, 200 + index as i64) },
                    Some(100 + index as i64),
                );
                consume(unsafe { get.call(address, key) }, Some(200 + index as i64));
            }
            assert_eq!(
                map[MAP_LEN_OFFSET as usize / 8],
                5,
                "replacement is not insertion"
            );
            assert_eq!(unsafe { contains.call(address, 91) }, 0);
            consume(unsafe { get.call(address, 91) }, None);
            for (index, key) in keys.iter().copied().enumerate() {
                consume(
                    unsafe { remove.call(address, key) },
                    Some(200 + index as i64),
                );
                assert_eq!(unsafe { contains.call(address, key) }, 0);
                consume(unsafe { get.call(address, key) }, None);
            }
            assert_eq!(map[MAP_LEN_OFFSET as usize / 8], 0);
            assert_eq!(map[MAP_TOMBSTONES_OFFSET as usize / 8], 5);
        },
    );
}

#[test]
fn durable_hash_differential_fixture_obeys_the_public_grammar() {
    Parser::new(include_str!(
        "../../../vcs/specs/L0-critical/stdlib-runtime/primitive_hash_parity.vr"
    ))
    .parse_module()
    .expect("public differential fixture grammar");
}
