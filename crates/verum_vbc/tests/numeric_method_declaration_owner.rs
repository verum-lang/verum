//! T1701: a numeric storage width does not identify an inherent method declaration.
#![cfg(feature = "codegen")]

use verum_common::{List, Shared, Text};
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::Instruction;
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::VbcModule;

// Distinct source bodies make wrong-owner selection observable before any
// byte conversion intrinsic or array-to-List coercion is involved.
const DECLARATIONS: &str = r#"
implement UInt64 {
    public fn to_be_bytes(self) -> [Byte; 8] { let bytes: [Byte; 8] = [17; 8]; bytes }
}
implement USize {
    public fn to_be_bytes(self) -> List<Byte> { [37, 41] }
}
implement Int64 {
    public fn to_be_bytes(self) -> [Byte; 8] { let bytes: [Byte; 8] = [29; 8]; bytes }
}
implement ISize {
    public fn to_be_bytes(self) -> List<Byte> { [43, 47] }
}
"#;

fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source)
        .parse_module()
        .expect("numeric owner syntax");
    VbcCodegen::with_config(CodegenConfig::new("numeric_owner"))
        .compile_module(&ast)
        .expect("numeric owner compilation")
}

fn roundtrip(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).expect("serialize source module"),
    )
    .expect("deserialize source module")
}

fn calls(module: &VbcModule, entry: verum_vbc::module::FunctionId) -> List<Text> {
    let function = module.get_function(entry).expect("entry descriptor");
    let start = function.bytecode_offset as usize;
    let end = start + function.bytecode_length as usize;
    verum_vbc::bytecode::decode_instructions(&module.bytecode[start..end])
        .expect("decode source instructions")
        .iter()
        .filter_map(|instruction| match instruction {
            Instruction::Call { func_id, .. } | Instruction::CallG { func_id, .. } => module
                .get_function(verum_vbc::module::FunctionId(*func_id))
                .and_then(|f| module.get_string(f.name))
                .map(|name| format!("Call#{func_id} {name}").into()),
            Instruction::CallM { method_id, .. } => module
                .get_string(verum_vbc::StringId(*method_id))
                .map(|name| format!("CallM {name}").into()),
            _ => None,
        })
        .collect()
}

fn check(source: &str, expected: i64) {
    let module = compile(source);
    for function in &module.functions {
        if function.bytecode_length > 0
            && let Some(name) = module.get_string(function.name)
            && name.ends_with(".to_be_bytes")
        {
            eprintln!(
                "declared body {name}: {:?}",
                function.params.first().map(|p| &p.type_ref)
            );
        }
    }
    let mut failures = List::<Text>::new();
    for (phase, module) in [("source", module.clone()), ("wire", roundtrip(&module))] {
        let entry = module
            .functions
            .iter()
            .find(|function| {
                module
                    .get_string(function.name)
                    .is_some_and(|name| name == "probe" || name == "numeric_owner.probe")
            })
            .expect("exact probe declaration")
            .id;
        let selected = calls(&module, entry);
        eprintln!("{phase} selected calls: {selected:?}");
        match Interpreter::new(Shared::new(module).into_arc()).execute_function(entry) {
            Ok(value) if value.is_int() && value.as_i64() == expected => {}
            result => failures.push(
                format!("{phase}: expected {expected}, got {result:?}; calls={selected:?}").into(),
            ),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn unsigned_probe(receiver: &str, declarations: &str) -> Text {
    format!(
        r#"{DECLARATIONS}
{declarations}
fn probe() -> Int {{
    let value: USize = 1;
    let mut bytes = {receiver}.to_be_bytes();
    bytes.push(97);
    (bytes[0] as Int) * 100 + bytes.len()
}}
"#
    )
    .into()
}

#[test]
fn static_calls_use_the_declared_numeric_bodies() {
    check(
        &format!(
            r#"{DECLARATIONS}
fn probe() -> Int {{
    let unsigned = UInt64.to_be_bytes(1);
    let signed = Int64.to_be_bytes(-1);
    let mut size = USize.to_be_bytes(1);
    let mut ssize = ISize.to_be_bytes(-1);
    size.push(97);
    ssize.push(101);
    (unsigned[0] as Int) + (signed[0] as Int) + (size[0] as Int)
        + (ssize[0] as Int) + size.len() + ssize.len()
}}
"#
        ),
        132,
    );
}

#[test]
fn unsigned_instance_retains_usize_list_declaration() {
    check(&unsigned_probe("value", ""), 3703);
}

#[test]
fn unsigned_cast_retains_usize_list_declaration() {
    check(&unsigned_probe("(1 as USize)", ""), 3703);
}

#[test]
fn unsigned_alias_retains_its_declared_target() {
    check(
        &unsigned_probe("(1 as SizeAlias)", "type SizeAlias is USize;"),
        3703,
    );
}

#[test]
fn signed_instance_retains_isize_list_declaration() {
    check(
        &format!(
            r#"{DECLARATIONS}
fn probe() -> Int {{
    let value: ISize = -1;
    let mut bytes = value.to_be_bytes();
    bytes.push(97);
    (bytes[0] as Int) * 100 + bytes.len()
}}
"#
        ),
        4303,
    );
}

#[test]
fn fixed_width_owners_keep_their_own_declared_bodies() {
    check(
        &format!(
            r#"{DECLARATIONS}
fn probe() -> Int {{
    let unsigned: UInt64 = 1;
    let signed: Int64 = -1;
    let a = unsigned.to_be_bytes();
    let b = signed.to_be_bytes();
    (a[0] as Int) * 100 + (b[0] as Int)
}}
"#
        ),
        1729,
    );
}

#[test]
fn typed_fixed_array_and_list_return_carriers_are_independently_valid() {
    check(
        r#"
fn fixed() -> [Byte; 8] { let bytes: [Byte; 8] = [17; 8]; bytes }
fn flexible() -> List<Byte> { [37, 41] }
fn probe() -> Int {
    let a = fixed();
    let mut b = flexible();
    b.push(97);
    (a[0] as Int) * 10000 + (b[0] as Int) * 100 + b.len()
}
"#,
        173703,
    );
}

#[test]
fn signed_alias_and_parentheses_keep_the_declared_target() {
    check(
        &format!(
            r#"{DECLARATIONS}
type SignedSize is ISize;
type SignedAlias is SignedSize;
fn probe() -> Int {{
    let value: SignedAlias = -1;
    let mut bytes = (value).to_be_bytes();
    bytes.push(97);
    (bytes[0] as Int) * 100 + bytes.len()
}}
"#,
        ),
        4303,
    );
}

#[test]
fn reversed_declaration_order_keeps_same_width_owners_separate() {
    check(
        r#"
implement ISize { public fn to_be_bytes(self) -> List<Byte> { [43, 47] } }
implement Int64 {
    public fn to_be_bytes(self) -> [Byte; 8] { let bytes: [Byte; 8] = [29; 8]; bytes }
}
implement USize { public fn to_be_bytes(self) -> List<Byte> { [37, 41] } }
implement UInt64 {
    public fn to_be_bytes(self) -> [Byte; 8] { let bytes: [Byte; 8] = [17; 8]; bytes }
}
fn probe() -> Int {
    let unsigned: UInt64 = 1;
    let signed: Int64 = -1;
    let size: USize = 1;
    let ssize: ISize = -1;
    let a = unsigned.to_be_bytes();
    let b = signed.to_be_bytes();
    let mut c = size.to_be_bytes();
    let mut d = ssize.to_be_bytes();
    c.push(97);
    d.push(101);
    (a[0] as Int) + (b[0] as Int) + (c[0] as Int)
        + (d[0] as Int) + c.len() + d.len()
}
"#,
        132,
    );
}

#[test]
fn borrowed_value_can_call_its_declared_by_value_method() {
    check(
        &format!(
            r#"{DECLARATIONS}
fn read(value: &USize) -> Int {{
    let bytes = value.to_be_bytes();
    (bytes[0] as Int) * 100 + bytes.len()
}}
fn probe() -> Int {{ let value: USize = 29; read(&value) }}
"#,
        ),
        3702,
    );
}

#[test]
fn declared_reference_receiver_keeps_its_reference_carrier() {
    check(
        r#"
implement USize { fn owner_read(&self) -> Int { (*self as Int) + 13 } }
fn read(value: &USize) -> Int { value.owner_read() }
fn probe() -> Int { let value: USize = 29; read(&value) }
"#,
        42,
    );
}

#[test]
fn unrelated_qualified_suffix_cannot_replace_a_builtin_declaration() {
    check(
        r#"
module foreign {
    type USize is { marker: Int };
    implement USize { public fn to_be_bytes(self) -> List<Byte> { [37, 41] } }
}
type SizeAlias is USize;
fn probe() -> Int { (1 as SizeAlias).to_be_bytes().len() }
"#,
        8,
    );
}

#[test]
fn float_declarations_keep_their_own_width_and_body() {
    check(
        r#"
implement Float32 {
    public fn to_be_bytes(self) -> [Byte; 4] { let bytes: [Byte; 4] = [53; 4]; bytes }
}
implement Float64 {
    public fn to_be_bytes(self) -> [Byte; 8] { let bytes: [Byte; 8] = [61; 8]; bytes }
}
fn probe() -> Int {
    let narrow: Float32 = 1.0 as Float32;
    let wide: Float64 = 1.0;
    let a = narrow.to_be_bytes();
    let b = wide.to_be_bytes();
    (a[0] as Int) * 10000 + (b[0] as Int) * 100 + a.len() * 10 + b.len()
}
"#,
        536148,
    );
}

#[test]
fn borrowed_by_value_numeric_method_receives_the_value_not_the_reference_tag() {
    check(
        r#"
implement USize { fn owner_copy(self) -> Int { (self as Int) + 13 } }
fn read(value: &USize) -> Int { value.owner_copy() }
fn probe() -> Int { let value: USize = 29; read(&value) }
"#,
        42,
    );
}

#[test]
fn bodyless_numeric_declaration_keeps_existing_builtin_fallback() {
    check(
        r#"
implement USize { public fn to_be_bytes(self) -> List<Byte>; }
fn probe() -> Int {
    let value: USize = 1;
    value.to_be_bytes().len()
}
"#,
        8,
    );
}

#[test]
fn bodyless_numeric_alias_keeps_existing_builtin_fallback() {
    check(
        r#"
implement USize { public fn to_be_bytes(self) -> List<Byte>; }
type SizeAlias is USize;
fn probe() -> Int { (1 as SizeAlias).to_be_bytes().len() }
"#,
        8,
    );
}

#[test]
fn empty_source_body_is_still_an_executable_declaration() {
    check(
        r#"
implement USize { public fn to_be_bytes(self) -> Unit {} }
fn probe() -> Int {
    let value: USize = 1;
    if value.to_be_bytes() == () { 42 } else { -1 }
}
"#,
        42,
    );
}

fn derivative_probe(declarations: &str, binding: &str, expression: &str) -> Text {
    format!(
        r#"{declarations}
fn probe() -> Int {{
    {binding}
    @vbc(GRAD_BEGIN, value);
    let primal: Float = {expression};
    let tape: Int = @vbc(GRAD_END, primal);
    let derivative: Float = @vbc(GRAD_BACKWARD, tape, 1.0);
    (derivative * 1000.0) as Int
}}
"#,
    )
    .into()
}

#[test]
fn builtin_float_sine_retains_its_primitive_derivative() {
    check(
        &derivative_probe("", "let value: Float = 0.0;", "value.sin()"),
        1000,
    );
}

#[test]
fn ordinary_function_body_transfers_argument_and_return_tape_nodes() {
    check(
        &derivative_probe(
            "fn squared(value: Float) -> Float { value * value }",
            "let value: Float = 3.0;",
            "squared(value)",
        ),
        6000,
    );
}

#[test]
fn declared_float_body_owns_its_derivative_instead_of_the_method_name() {
    check(
        &derivative_probe(
            "implement Float { fn sin(self) -> Float { self * self } }",
            "let value: Float = 3.0;",
            "value.sin()",
        ),
        6000,
    );
}

#[test]
fn declared_float_alias_retains_its_body_derivative() {
    check(
        &derivative_probe(
            "type Scalar is Float; implement Float { fn sin(self) -> Float { self * self } }",
            "let value: Scalar = 3.0;",
            "value.sin()",
        ),
        6000,
    );
}

#[test]
fn declared_float_body_reads_the_referent_tape_node_through_cbgr() {
    check(
        &derivative_probe(
            "implement Float { fn sin(self) -> Float { self * self } }",
            "let value: Float = 3.0; let borrowed = &value;",
            "borrowed.sin()",
        ),
        6000,
    );
}

#[test]
fn declared_numeric_body_transfers_explicit_argument_tape_nodes() {
    check(
        &derivative_probe(
            "implement Float { fn scale(self, factor: Float) -> Float { self * factor } }",
            "let value: Float = 3.0; let receiver: Float = 2.0;",
            "receiver.scale(value)",
        ),
        2000,
    );
}

const RETURNED_SIZE_REFERENCE: &str = r#"
implement USize { fn to_be_bytes(self) -> List<Byte> { [37, 41] } }
fn borrowed() -> &USize { let value: USize = 29; &value }
"#;

#[test]
fn returned_numeric_reference_is_a_retained_thinref() {
    let source = compile(RETURNED_SIZE_REFERENCE);
    for (phase, module) in [("source", source.clone()), ("wire", roundtrip(&source))] {
        let entry = module.functions.iter()
            .find(|f| module.get_string(f.name) == Some("numeric_owner.borrowed"))
            .expect("exact borrowed producer").id;
        let mut interpreter = Interpreter::new(Shared::new(module).into_arc());
        let value = interpreter.execute_function(entry).expect("returned numeric reference");
        assert!(value.is_thin_ref(), "{phase}: {value:?}");
        let reference = value.as_thin_ref();
        assert!(interpreter.state.escape_cells.iter()
            .any(|cell| cell.get().cast::<u8>() == reference.ptr),
            "{phase}: returned reference must be owned by the escape-cell roster");
        // SAFETY: the live interpreter owns the matching initialized Value cell.
        let referent = unsafe { *reference.ptr.cast::<verum_vbc::value::Value>() };
        assert!(referent.is_int(), "{phase}: {referent:?}");
        assert_eq!(referent.as_i64(), 29, "{phase}");
    }
}

#[test]
fn returned_thinref_keeps_the_declared_numeric_method() {
    let source = format!(r#"{RETURNED_SIZE_REFERENCE}
fn probe() -> Int {{
    let value: &USize = borrowed();
    value.to_be_bytes().len()
}}
"#);
    let module = compile(&source);
    let entry = module.functions.iter()
        .find(|f| module.get_string(f.name) == Some("numeric_owner.probe"))
        .expect("exact probe");
    let start = entry.bytecode_offset as usize;
    let end = start + entry.bytecode_length as usize;
    let instructions = verum_vbc::bytecode::decode_instructions(&module.bytecode[start..end])
        .expect("probe instructions");
    assert!(instructions.iter().any(|i| matches!(i, Instruction::CallM { method_id, .. }
        if module.get_string(verum_vbc::StringId(*method_id)) == Some("USize.to_be_bytes"))),
        "{instructions:?}");
    assert!(!instructions.iter().any(|i| matches!(i, Instruction::Deref { .. })
        || matches!(i, Instruction::MemExtended { sub_op, .. }
            if *sub_op == verum_vbc::instruction::MemSubOpcode::DerefValue as u8)),
        "the implicit receiver must reach CallM without an inserted dereference: {instructions:?}");
    check(&source, 2);
}

#[test]
fn explicitly_dereferenced_returned_numeric_reference_keeps_its_body() {
    check(&format!(r#"{RETURNED_SIZE_REFERENCE}
fn probe() -> Int {{
    let value: &USize = borrowed();
    (*value).to_be_bytes().len()
}}
"#), 2);
}

#[test]
fn returned_interior_numeric_reference_keeps_the_declared_method() {
    check(r#"
type Holder is { value: USize };
implement USize { fn to_be_bytes(self) -> List<Byte> { [37, 41] } }
fn borrowed(owner: &Holder) -> &USize { &owner.value }
fn probe() -> Int {
    let owner = Holder { value: 29 };
    let value: &USize = borrowed(&owner);
    value.to_be_bytes().len()
}
"#, 2);
}

#[test]
fn returned_thinref_preserves_a_declared_reference_self_carrier() {
    check(r#"
implement USize { fn to_be_bytes(&self) -> List<Byte> { [(*self as Byte), 41] } }
fn borrowed() -> &USize { let value: USize = 29; &value }
fn probe() -> Int {
    let value: &USize = borrowed();
    let bytes = value.to_be_bytes();
    (bytes[0] as Int) * 100 + bytes.len()
}
"#, 2902);
}

#[test]
fn a_returned_float_reference_refuses_unproved_gradient_provenance_after_slot_reuse() {
    let source = compile(r#"
implement Float { fn sin(self) -> Float { self * self } }
fn borrowed(value: Float) -> &Float { &value }
fn probe() -> Int {
    let value: Float = 3.0;
    @vbc(GRAD_BEGIN, value);
    let reference: &Float = borrowed(value);
    let previous: Float = value.sin();
    let primal: Float = reference.sin();
    let tape: Int = @vbc(GRAD_END, primal);
    let derivative: Float = @vbc(GRAD_BACKWARD, tape, 1.0);
    (derivative * 1000.0) as Int
}
"#);
    let entry = source.functions.iter()
        .find(|f| source.get_string(f.name) == Some("numeric_owner.probe"))
        .expect("exact gradient probe").id;
    assert_eq!(calls(&source, entry).iter()
        .filter(|call| call.as_str() == "CallM Float.sin").count(), 2,
        "both same-value method calls must remain in the emitted probe");
    check_receiver_gradient_refusal(source);
}

fn check_receiver_gradient_refusal(source: VbcModule) {
    let mut failures = List::<Text>::new();
    for (phase, module) in [("source", source.clone()), ("wire", roundtrip(&source))] {
        let entry = module.functions.iter()
            .find(|f| module.get_string(f.name) == Some("numeric_owner.probe"))
            .expect("exact gradient probe").id;
        let result = Interpreter::new(Shared::new(module).into_arc()).execute_function(entry);
        if !matches!(&result, Err(error) if error.to_string().contains("autodiff:")
            && error.to_string().contains("receiver provenance"))
        {
            failures.push(format!("{phase}: expected explicit unsupported receiver provenance, got {result:?}").into());
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn returned_float_reference_runs_its_declared_body_outside_gradient_scopes() {
    for (parameter, body) in [("self", "self * self"), ("&self", "(*self) * (*self)")] {
        check(&format!(r#"
implement Float {{ fn sin({parameter}) -> Float {{ {body} }} }}
fn borrowed(value: Float) -> &Float {{ &value }}
fn probe() -> Int {{
    let reference: &Float = borrowed(3.0);
    (reference.sin() * 1000.0) as Int
}}
"#), 9000);
    }
}

#[test]
fn borrowed_float_self_refuses_an_unproved_dereference_tape_transfer() {
    check_receiver_gradient_refusal(compile(&derivative_probe(
        "implement Float { fn sin(&self) -> Float { (*self) * (*self) } }",
        "let value: Float = 3.0; let borrowed = &value;",
        "borrowed.sin()",
    )));
}
