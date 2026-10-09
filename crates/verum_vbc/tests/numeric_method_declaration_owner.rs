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
