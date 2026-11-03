use std::sync::Arc;
use verum_ast::{
    ItemKind,
    ty::{GenericParamKind, TypeKind},
};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const OPS: &[(&str, &str)] = &[
    ("Add", "add"),
    ("Sub", "sub"),
    ("Mul", "mul"),
    ("Div", "div"),
    ("Rem", "rem"),
    ("BitAnd", "bitand"),
    ("BitOr", "bitor"),
    ("BitXor", "bitxor"),
    ("Shl", "shl"),
    ("Shr", "shr"),
    ("AddAssign", "add_assign"),
    ("SubAssign", "sub_assign"),
    ("MulAssign", "mul_assign"),
    ("DivAssign", "div_assign"),
    ("RemAssign", "rem_assign"),
    ("BitAndAssign", "bitand_assign"),
    ("BitOrAssign", "bitor_assign"),
    ("BitXorAssign", "bitxor_assign"),
    ("ShlAssign", "shl_assign"),
    ("ShrAssign", "shr_assign"),
];

// Unit-level qualification of the actual changed declarations. The original
// AST items and canonical source owner are retained; no protocol is restated.
// This is deliberately not a claim that the complete stdlib compiles.
#[test]
fn actual_operator_defaults_and_explicit_rhs_execute() {
    let source = std::fs::read_to_string(format!("{ROOT}/core/base/protocols.vr")).unwrap();
    let original = Parser::new(&source).parse_module().unwrap();
    for &(protocol, method) in OPS {
        let item = original.items.iter().find(|item| matches!(&item.kind, ItemKind::Type(decl) if decl.name.name.as_str() == protocol)).unwrap();
        let ItemKind::Type(decl) = &item.kind else {
            unreachable!()
        };
        assert_eq!(decl.generics.len(), 1, "{protocol}");
        let GenericParamKind::Type {
            name,
            default: Some(default),
            ..
        } = &decl.generics[0].kind
        else {
            panic!("missing actual default {protocol}")
        };
        assert_eq!(name.name.as_str(), "Rhs");
        assert!(
            matches!(&default.kind, TypeKind::Path(path) if matches!(path.segments.as_slice(), [verum_ast::PathSegment::SelfValue])),
            "{protocol}: {default:?}"
        );
        for explicit in [false, true] {
            let mut provider = original.clone();
            provider
                .items
                .retain(|candidate| matches!(candidate.kind, ItemKind::Module(_)));
            provider.items.push(item.clone());
            let target = if explicit {
                format!("{protocol}<Int>")
            } else {
                protocol.into()
            };
            let rhs = if explicit { "Int" } else { "Cell" };
            let value = if explicit { "rhs" } else { "rhs.value" };
            let argument = if explicit { "31" } else { "Cell { value: 31 }" };
            let body = if protocol.ends_with("Assign") {
                format!("fn {method}(&mut self, rhs: {rhs}) {{ self.value = {value}; }}")
            } else {
                format!(
                    "type Output = Cell; fn {method}(self, rhs: {rhs}) -> Cell {{ Cell {{ value: {value} }} }}"
                )
            };
            let run = if protocol.ends_with("Assign") {
                format!("let mut cell = Cell {{ value: 7 }}; cell.{method}({argument}); cell.value")
            } else {
                format!("Cell {{ value: 7 }}.{method}({argument}).value")
            };
            let client = format!(
                "module operator_probe; mount core.base.protocols.{protocol}; type Cell is {{ value: Int }}; implement {target} for Cell {{ {body} }} fn run() -> Int {{ {run} }}"
            );
            let client = Parser::new(&client).parse_module().unwrap();
            let mut generator =
                VbcCodegen::with_config(CodegenConfig::new("main").with_validation());
            generator
                .collect_unit_declarations(&[&provider, &client])
                .unwrap_or_else(|error| panic!("{target}: {error:?}"));
            generator
                .compile_unit_items(&[&provider, &client], ItemFailurePolicy::Strict)
                .unwrap_or_else(|error| panic!("{target}: {error:?}"));
            let module = generator.finalize_module_from_state().unwrap();
            let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
            for module in [
                module,
                verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
            ] {
                let run = module.find_function_by_name("operator_probe.run").unwrap();
                assert_eq!(
                    Interpreter::new(Arc::new(module))
                        .execute_function(run)
                        .unwrap()
                        .try_as_i64(),
                    Some(31),
                    "{target}"
                );
            }
        }
    }
}
