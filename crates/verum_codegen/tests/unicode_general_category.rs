//! Unicode classification retains the raw scalar ABI and the source enum owner.
use std::sync::Arc;
use verum_ast::decl::{ImplItemKind, ItemKind};
use verum_common::Text;
use verum_fast_parser::Parser;
use verum_vbc::{
    Value, VbcModule,
    codegen::{ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module_validated,
    interpreter::Interpreter,
    serialize::serialize_module,
};

#[path = "support/native_library.rs"]
mod native_library;

const CASES: &[(char, &str, i64)] = &[
    ('A', "Lu", 0),
    ('ß', "Ll", 1),
    ('ǅ', "Lt", 2),
    ('ʰ', "Lm", 3),
    ('中', "Lo", 4),
    ('\u{301}', "Mn", 5),
    ('\u{903}', "Mc", 6),
    ('\u{20dd}', "Me", 7),
    ('\u{660}', "Nd", 8),
    ('Ⅻ', "Nl", 9),
    ('²', "No", 10),
    ('_', "Pc", 11),
    ('—', "Pd", 12),
    ('(', "Ps", 13),
    (')', "Pe", 14),
    ('«', "Pi", 15),
    ('»', "Pf", 16),
    ('!', "Po", 17),
    ('+', "Sm", 18),
    ('€', "Sc", 19),
    ('^', "Sk", 20),
    ('😀', "So", 21),
    (' ', "Zs", 22),
    ('\u{2028}', "Zl", 23),
    ('\u{2029}', "Zp", 24),
    ('\0', "Cc", 25),
    ('\u{200d}', "Cf", 26),
    ('\u{f0000}', "Co", 28),
    ('\u{378}', "Cn", 29),
];

// Unicode 17 additions, plane boundaries, and noncharacters exercise the page index.
const BOUNDARIES: &[(char, &str, i64)] = &[
    ('\u{16ea0}', "Lu", 0),
    ('\u{1e6c0}', "Lo", 4),
    ('\u{1faea}', "So", 21),
    ('\u{ffff}', "Cn", 29),
    ('\u{10fffd}', "Co", 28),
    ('\u{10ffff}', "Cn", 29),
];

fn images() -> [VbcModule; 2] {
    let parse = |file, source: &str| {
        verum_fast_parser::FastParser::new()
            .parse_module_str(source, verum_ast::FileId::new(file))
            .unwrap()
    };
    let mut primitive = parse(1, include_str!("../../../core/intrinsics/runtime/text.vr"));
    primitive.items.retain(|item| match &item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Function(f) => f.name.as_str() == "char_general_category",
        _ => false,
    });
    let mut public = parse(2, include_str!("../../../core/text/char.vr"));
    public.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Type(ty) => ty.name.name.as_str() == "GeneralCategory",
        ItemKind::Impl(block) => {
            block.items.retain(|it| {
                matches!(&it.kind,
                ImplItemKind::Function(fun) if fun.name.name.as_str() == "general_category")
            });
            !block.items.is_empty()
        }
        ItemKind::Module(_) | ItemKind::Mount(_) => true,
        _ => false,
    });
    let decoy = parse(
        3,
        "module unrelated; public type GeneralCategory is Cn | Lu | Ll;",
    );
    let mut source = Text::from(
        "module client; mount core.text.char.GeneralCategory;
         mount core.intrinsics.runtime.text.char_general_category;
         fn scalar(c:Char)->Int {char_general_category(c)}
         fn raw(cp:Int)->Int {scalar(cp as Char)}
         fn indirect(cp:Int)->Int {let classify=char_general_category; classify(cp as Char)}
         fn nominal(cp:Int)->Int {let c:Char=cp as Char; match c.general_category() {",
    );
    for &(_, variant, code) in CASES {
        source.push_str(&format!("GeneralCategory.{variant} => {code}, "));
    }
    source.push_str("GeneralCategory.Cs => 27, _ => -1}}");
    let client = parse(4, &source);
    let units = [
        ("unrelated", &decoy),
        ("core.intrinsics.runtime.text", &primitive),
        ("core.text.char", &public),
        ("client", &client),
    ];
    let mut compiler = VbcCodegen::new();
    compiler.initialize();
    compiler.collect_owned_unit_declarations(&units).unwrap();
    compiler.resolve_pending_imports();
    compiler
        .compile_owned_unit_items(&units, ItemFailurePolicy::Strict)
        .unwrap();
    let source = compiler.finalize_module().unwrap();
    let mut wire = deserialize_module_validated(&serialize_module(&source).unwrap()).unwrap();
    for function in &mut wire.functions {
        let start = function.bytecode_offset as usize;
        let mut body = verum_vbc::bytecode::decode_instructions(
            &wire.bytecode[start..start + function.bytecode_length as usize],
        )
        .unwrap();
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut body);
        function.instructions = Some(body);
    }
    [source, wire]
}

#[test]
fn category_codes_and_public_variants_agree_across_source_and_wire() {
    let entries = [
        ("client.raw", "invoke_raw"),
        ("client.indirect", "invoke_indirect"),
        ("client.nominal", "invoke_nominal"),
    ];
    for module in images() {
        let category = module
            .types
            .iter()
            .find(|ty| {
                module
                    .get_string(ty.name)
                    .is_some_and(|name| name.ends_with("GeneralCategory"))
                    && ty.variants.len() == 30
            })
            .expect("public declaration");
        let decoy = module
            .types
            .iter()
            .find(|ty| {
                module
                    .get_string(ty.name)
                    .is_some_and(|name| name.ends_with("GeneralCategory"))
                    && ty.variants.len() == 3
            })
            .expect("unrelated declaration");
        assert_ne!(category.id, decoy.id);
        let constructors: verum_common::List<_> = module
            .functions
            .iter()
            .filter_map(|f| f.instructions.as_ref())
            .flatten()
            .filter_map(|op| match op {
                verum_vbc::Instruction::MakeVariantTyped { type_id, tag, .. } => {
                    Some((*type_id, *tag))
                }
                _ => None,
            })
            .collect();
        for tag in 0..30 {
            assert!(
                constructors.contains(&(category.id.0, tag)),
                "missing public category {tag}"
            );
        }
        let mut vm = Interpreter::new(Arc::new(module.clone()));
        for &(entry, _) in &entries {
            let id = module.find_function_by_name(entry).unwrap();
            for &(ch, name, code) in CASES.iter().chain(BOUNDARIES) {
                let result = vm
                    .execute_function_with_args(id, &[Value::from_char(ch)])
                    .unwrap();
                assert_eq!(result.try_as_i64(), Some(code), "{entry}: {ch:?} {name}");
            }
        }
        let raw = module.find_function_by_name("client.scalar").unwrap();
        for invalid in [
            Value::from_i64(-1),
            Value::from_i64(0xd800),
            Value::from_i64(0xdfff),
            Value::from_i64(0x110000),
            Value::from_bool(true),
            Value::from_f64(0.0),
        ] {
            let error = vm.execute_function_with_args(raw, &[invalid]).unwrap_err();
            assert!(
                error.to_string().contains("requires a valid Char"),
                "{error}"
            );
        }
        native_library::with_library(&module, &entries, |library| {
            for &(_, entry) in &entries {
                // SAFETY: the generated wrapper forwards the declared Int -> Int signature.
                let classify =
                    unsafe { library.get::<unsafe extern "C" fn(i64) -> i64>(entry.as_bytes()) }
                        .unwrap();
                for &(ch, name, code) in CASES.iter().chain(BOUNDARIES) {
                    assert_eq!(
                        unsafe { classify(ch as i64) },
                        code,
                        "{entry}: {ch:?} {name}"
                    );
                }
                if entry == "invoke_raw" {
                    for ch in (0..=0x10ffff).filter_map(char::from_u32) {
                        // SAFETY: each Unicode scalar fits the exported Int parameter.
                        let native = unsafe { classify(i64::from(ch as u32)) };
                        assert_eq!(
                            native,
                            i64::from(verum_vbc::unicode_category::code(ch)),
                            "{ch:?}"
                        );
                    }
                }
            }
        });
    }
}

#[test]
fn conflicting_source_and_wire_declarations_are_refused() {
    for declaration in [
        "fn wrong(c:Char)->GeneralCategory;",
        "fn wrong(c:Int)->Int;",
        "fn wrong()->Int;",
    ] {
        let source = format!(
            r#"type GeneralCategory is Lu | Ll; @intrinsic("char_general_category") {declaration}"#
        );
        let error = VbcCodegen::new()
            .compile_module(&Parser::new(&source).parse_module().unwrap())
            .unwrap_err();
        assert!(
            error.to_string().contains("requires (Char) -> Int"),
            "{error}"
        );
    }
    let [_, mut wire] = images();
    let id = wire
        .find_function_by_name("core.intrinsics.runtime.text.char_general_category")
        .unwrap();
    let declaration = wire.functions.iter_mut().find(|f| f.id == id).unwrap();
    declaration.return_type = verum_vbc::TypeRef::Concrete(verum_vbc::TypeId::BOOL);
    let caller = wire.find_function_by_name("client.indirect").unwrap();
    let error = Interpreter::new(Arc::new(wire.clone()))
        .execute_function_with_args(caller, &[Value::from_i64(65)])
        .unwrap_err();
    assert!(
        error.to_string().contains("requires (Char) -> Int"),
        "{error}"
    );
    let context = verum_llvm::context::Context::create();
    let mut lowering = verum_codegen::llvm::VbcToLlvmLowering::new(
        &context,
        verum_codegen::llvm::LoweringConfig::debug("invalid_category_signature")
            .with_debug_info(false),
    );
    let error = lowering.lower_module(&wire).unwrap_err();
    assert!(
        error.to_string().contains("requires (Char) -> Int"),
        "{error}"
    );
}

#[test]
fn native_rejects_surrogates_and_out_of_range_scalars() {
    const CHILD: &str = "VERUM_TEST_INVALID_CATEGORY_SCALAR";
    if let Ok(value) = std::env::var(CHILD) {
        let invalid: i64 = value.parse().unwrap();
        let [_, mut wire] = images();
        // Supply a malformed scalar directly at the Char opcode boundary.
        // A regular Int-to-Char conversion has its own validation contract.
        let raw = wire.find_function_by_name("client.raw").unwrap();
        let function = wire
            .functions
            .iter_mut()
            .find(|function| function.id == raw)
            .unwrap();
        let body = function.instructions.as_mut().unwrap();
        let mut casts = 0;
        for instruction in body.iter_mut() {
            if let verum_vbc::Instruction::CvtIC { dst, src } = *instruction {
                *instruction = verum_vbc::Instruction::Mov { dst, src };
                casts += 1;
            }
        }
        assert_eq!(casts, 1);
        let mut bytes = Default::default();
        verum_vbc::bytecode::encode_instructions_with_fixup(body, &mut bytes);
        let start = function.bytecode_offset as usize;
        assert_eq!(bytes.len(), function.bytecode_length as usize);
        wire.bytecode[start..start + bytes.len()].copy_from_slice(&bytes);
        native_library::with_library(&wire, &[("client.raw", "invalid_scalar")], |library| {
            // SAFETY: the generated wrapper receives an Int and returns an Int.
            unsafe {
                library
                    .get::<unsafe extern "C" fn(i64) -> i64>(b"invalid_scalar")
                    .unwrap()(invalid)
            };
        });
        panic!("invalid Unicode scalar returned");
    }
    for invalid in [0xd800, 0x110000] {
        let outcome = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "native_rejects_surrogates_and_out_of_range_scalars",
                "--nocapture",
            ])
            .env(CHILD, invalid.to_string())
            .output()
            .unwrap();
        let error = Text::from_utf8_lossy(&outcome.stderr);
        assert!(!outcome.status.success(), "{invalid:#x}");
        assert!(
            error.contains("Unicode general category requires a valid Char"),
            "{invalid:#x} {error}"
        );
    }
}

#[test]
fn native_refuses_a_floating_point_register_as_a_character() {
    let ast = Parser::new("fn forged(value:Float)->Int {0}")
        .parse_module()
        .unwrap();
    let mut image = VbcCodegen::new().compile_module(&ast).unwrap();
    let id = image.find_function_by_name("forged").unwrap();
    let function = image.functions.iter_mut().find(|f| f.id == id).unwrap();
    // Inject malformed VBC whose scalar instruction reads the Float parameter.
    let body = vec![
        verum_vbc::unicode_category::instruction(verum_vbc::Reg(1), verum_vbc::Reg(0)),
        verum_vbc::Instruction::Ret {
            value: verum_vbc::Reg(1),
        },
    ];
    function.register_count = 2;
    function.bytecode_offset = image.bytecode.len() as u32;
    function.bytecode_length =
        verum_vbc::bytecode::encode_instructions(&body, &mut image.bytecode) as u32;
    function.instructions = Some(body);
    let context = verum_llvm::context::Context::create();
    let mut lowering = verum_codegen::llvm::VbcToLlvmLowering::new(
        &context,
        verum_codegen::llvm::LoweringConfig::debug("invalid_category_carrier")
            .with_debug_info(false),
    );
    let error = lowering.lower_module(&image).unwrap_err();
    assert!(
        error.to_string().contains("requires a Char scalar carrier"),
        "{error}"
    );
}
