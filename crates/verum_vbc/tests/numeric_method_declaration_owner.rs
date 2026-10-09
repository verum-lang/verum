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
    public fn to_be_bytes(self) -> [Byte; 8] { [17; 8] }
}
implement USize {
    public fn to_be_bytes(self) -> List<Byte> { [37, 41] }
}
implement Int64 {
    public fn to_be_bytes(self) -> [Byte; 8] { [29; 8] }
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
                .map(Into::into),
            Instruction::CallM { method_id, .. } => module.get_string(verum_vbc::StringId(*method_id)).map(Into::into),
            _ => None,
        })
        .collect()
}

fn check(source: &str, expected: i64) {
    let module = compile(source);
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
