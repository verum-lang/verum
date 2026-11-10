//! Resolve an awaited protocol operation's residual with the actual standard-library declarations.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_ast::{FileId, ItemKind, Module, decl::ImplKind};
use verum_fast_parser::FastParser;
use verum_vbc::{
    Instruction,
    codegen::{CodegenConfig, CodegenResult, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
    module::VbcModule,
    types::{TypeId, TypeRef},
};

fn source(file: u32, text: &str) -> Module {
    FastParser::new()
        .parse_module_str(text, FileId::new(file))
        .unwrap()
}

fn selected_stdlib(
    file: u32,
    text: &str,
    types: &[&str],
    target: &str,
    implementations: &[&str],
) -> Module {
    let mut module = source(file, text);
    module.items.retain(|item| match &item.kind {
        ItemKind::Type(declaration) => types.contains(&declaration.name.as_str()),
        ItemKind::Protocol(declaration) => types.contains(&declaration.name.as_str()),
        ItemKind::Impl(declaration) => match &declaration.kind {
            ImplKind::Protocol {
                protocol, for_type, ..
            } => {
                let receiver = verum_ast::pretty::format_type(for_type);
                receiver.as_str().split('<').next() == Some(target)
                    && implementations.contains(&protocol.to_string().as_str())
            }
            _ => false,
        },
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Mount(_) => true,
        _ => false,
    });
    module
}

fn compile(result_implementations: &[&str]) -> CodegenResult<VbcModule> {
    let ops = selected_stdlib(
        1,
        include_str!("../../../core/base/ops.vr"),
        &["Never", "ControlFlow", "Try", "FromResidual"],
        "",
        &[],
    );
    let protocols = selected_stdlib(
        2,
        include_str!("../../../core/base/protocols.vr"),
        &["From", "Default"],
        "T",
        &["From"],
    );
    let maybe = selected_stdlib(
        3,
        include_str!("../../../core/base/maybe.vr"),
        &["Maybe"],
        "Maybe",
        &["Try", "FromResidual"],
    );
    let result = selected_stdlib(
        4,
        include_str!("../../../core/base/result.vr"),
        &["Result"],
        "Result",
        result_implementations,
    );
    let caller = source(
        5,
        r#"
module core.service;
mount core.base.result.{Result, Ok, Err};
mount core.base.protocols.{From, Default};
mount core.base.maybe.Maybe;
public type Failure is { public code: Int };
public type Converted is { public code: Int };
public type Store is protocol {
    async fn init(&mut self) -> Result<(), Failure>;
};
public type Backend is { fail: Bool };
implement Store for Backend {
    async fn init(&mut self) -> Result<(), Failure> {
        if self.fail { Err(Failure { code: 31 }) } else { Ok(()) }
    }
}
implement From<Failure> for Converted {
    fn from(error: Failure) -> Converted { Converted { code: 100 + error.code } }
}
implement Default for Converted { fn default() -> Converted { Converted { code: 79 } } }
fn optional(value: Maybe<Int>) -> Result<Int, Converted> { Ok(value?) }
async fn call_store(s: &mut dyn Store) -> Result<Int, Failure> { s.init().await?; Ok(92) }
async fn convert_store(s: &mut dyn Store) -> Result<Int, Converted> { s.init().await?; Ok(93) }
public async fn probe() -> Int {
    let mut s = Backend { fail: false };
    let ok = match call_store(&mut s).await { Ok(v) => v, Err(_) => 0 };
    let other_ok = match convert_store(&mut s).await { Ok(v) => v, Err(_) => 0 };
    s.fail = true;
    let error = match call_store(&mut s).await { Err(e) => e.code, Ok(_) => 0 };
    let converted = match convert_store(&mut s).await { Err(e) => e.code, Ok(_) => 0 };
    let absent = match optional(Maybe.None) { Err(e) => e.code, Ok(_) => 0 };
    if ok == 92 && other_ok == 93 && error == 31 && converted == 131 && absent == 79 { 1 } else { 0 }
}
    "#,
    );
    let files = [&ops, &protocols, &maybe, &result, &caller];
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("core"));
    codegen.collect_unit_declarations(&files)?;
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(&files, ItemFailurePolicy::Strict)?;
    codegen.finalize_module_from_state()
}

#[test]
fn missing_residual_implementation_refuses_the_conversion() {
    let error = compile(&["Try"]).unwrap_err().to_string();
    assert!(error.contains("no declared FromResidual"), "{error}");
}

#[test]
fn awaited_dynamic_result_preserves_its_error_and_converts_a_distinct_error() {
    let module = compile(&["Try", "FromResidual"]).unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    for artifact in [
        module,
        verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
    ] {
        verum_vbc::validate::validate_module(&artifact).unwrap();
        let find = |name| {
            artifact
                .get_function(artifact.find_function_by_name(name).unwrap())
                .unwrap()
        };
        let caller = find("core.service.convert_store");
        let unwrap_error = |name| match &find(name).return_type {
            TypeRef::Instantiated { args, .. } => args[1].clone(),
            other => panic!("missing Result declaration: {other:?}"),
        };
        let expected = [
            TypeRef::Concrete(TypeId::INT),
            unwrap_error("core.service.convert_store"),
            unwrap_error("core.service.call_store"),
        ];
        let start = caller.bytecode_offset as usize;
        let ops = verum_vbc::bytecode::decode_instructions(
            &artifact.bytecode[start..start + caller.bytecode_length as usize],
        )
        .unwrap();
        let converted_by = ops
            .iter()
            .find_map(|op| match op {
                Instruction::CallG {
                    func_id, type_args, ..
                } if type_args.as_slice() == expected => {
                    artifact.get_function(verum_vbc::module::FunctionId(*func_id))
                }
                _ => None,
            })
            .expect("declared conversion with the output and both error witnesses");
        assert!(
            artifact
                .get_string(converted_by.name)
                .unwrap()
                .starts_with("core.base.result.Result.from_residual")
        );
        assert!(matches!(
            converted_by.semantic_parameter_types().unwrap(),
            [Some(TypeRef::Instantiated {
                base: TypeId::RESULT,
                ..
            })]
        ));
        let id = find("core.service.probe").id;
        let value = Interpreter::new(Arc::new(artifact))
            .execute_function(id)
            .unwrap();
        assert_eq!(value.try_as_i64(), Some(1));
    }
}
