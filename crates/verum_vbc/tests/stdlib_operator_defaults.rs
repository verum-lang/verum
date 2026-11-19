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

#[test]
fn the_semiring_identity_defaults_call_their_numeric_owners_zero_and_one() {
    use verum_ast::decl::{ImplKind, ProtocolItemKind};

    let parse = |source: &str| Parser::new(source).parse_module().unwrap();
    let mut protocols = parse(include_str!("../../../core/base/protocols.vr"));
    protocols.items.retain(|item| match &item.kind {
        ItemKind::Module(module) => module.items.is_none(),
        ItemKind::Type(declaration) => {
            matches!(declaration.name.as_str(), "Add" | "Mul" | "Zero" | "One")
        }
        _ => false,
    });
    let mut algebra = parse(include_str!("../../../core/math/algebra.vr"));
    algebra.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(module) => module.items.is_none(),
        ItemKind::Type(declaration) if declaration.name.as_str() == "Semiring" => {
            let verum_ast::TypeDeclBody::Protocol(body) = &mut declaration.body else {
                panic!("Semiring")
            };
            body.items.retain(|item| {
                matches!(&item.kind, ProtocolItemKind::Function { decl, .. }
                if matches!(decl.name.as_str(), "add_identity" | "mul_identity"))
            });
            assert_eq!(body.items.len(), 2);
            true
        }
        ItemKind::Impl(implementation) => matches!(&implementation.kind,
            ImplKind::Protocol { protocol, for_type, .. } if protocol.to_string() == "Semiring"
                && verum_ast::pretty::format_type(for_type) == "Int"),
        _ => false,
    });
    let mut primitives = parse(include_str!("../../../core/base/primitives.vr"));
    primitives.items.retain(|item| match &item.kind {
        ItemKind::Module(module) => module.items.is_none(),
        ItemKind::Impl(implementation) => matches!(&implementation.kind,
            ImplKind::Protocol { protocol, for_type, .. }
                if matches!(protocol.to_string().as_str(), "Zero" | "One")
                && verum_ast::pretty::format_type(for_type) == "Int"),
        _ => false,
    });
    let imports = "mount core.base.protocols.{Add, Mul, Zero, One};";
    primitives.items.extend(parse(imports).items);
    algebra.items.extend(parse(imports).items);
    let consumer = parse(
        r#"
        module consumer;
        mount core.math.algebra.Semiring;
        type Decoy is ();
        implement Decoy { fn zero() -> Int { 79 } fn one() -> Int { 83 } }
        type Impostor is protocol { fn zero() -> Self; };
        implement Impostor for Int { fn zero() -> Int { 79 } }
        fn run() -> Int {
            let n: Int = 7;
            if n.add_identity() && n.mul_identity() { 73 } else { -1 }
        }
    "#,
    );
    for (omit_zero, omit_method) in [(false, false), (true, false), (false, true)] {
        let mut numbers = primitives.clone();
        if omit_zero {
            numbers.items.retain(|item| !matches!(&item.kind, ItemKind::Impl(implementation)
                if matches!(&implementation.kind, ImplKind::Protocol { protocol, .. } if protocol.to_string() == "Zero")));
        } else if omit_method {
            for item in &mut numbers.items {
                if let ItemKind::Impl(implementation) = &mut item.kind
                    && matches!(&implementation.kind, ImplKind::Protocol { protocol, .. } if protocol.to_string() == "Zero")
                {
                    implementation.items.retain(|item| !matches!(&item.kind,
                        verum_ast::decl::ImplItemKind::Function(function) if function.name.as_str() == "zero"));
                }
            }
        }
        let files = [&protocols, &numbers, &algebra, &consumer];
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        cg.collect_unit_declarations(&files).unwrap();
        cg.resolve_pending_imports();
        let compiled = cg.compile_unit_items(&files, ItemFailurePolicy::Strict);
        if omit_zero || omit_method {
            let error = compiled.unwrap_err();
            assert!(error.to_string().contains("zero"), "{error}");
            continue;
        }
        compiled.unwrap();
        let module = cg.finalize_module_from_state().unwrap();
        let encoded = verum_vbc::serialize::serialize_module(&module).unwrap();
        for module in [
            module,
            verum_vbc::deserialize::deserialize_module(&encoded).unwrap(),
        ] {
            let run = module.find_function_by_name("consumer.run").unwrap();
            let value = Interpreter::new(Arc::new(module)).execute_function(run).unwrap();
            assert_eq!(value.try_as_i64(), Some(73));
        }
    }
}

#[test]
fn self_associated_calls_follow_applied_parents_and_declared_defaults() {
    let source = r#"
        module chooser;
        type ReadAs<A> is protocol { fn read() -> Int; };
        type Select<A> is protocol extends ReadAs<A> {
            fn selected(self) -> Int { Self.read() }
        };
        type Local is protocol {
            fn seed() -> Int { 41 }
            fn seeded(self) -> Int { Self.seed() + 1 }
        };
        type Cell is { value: Int };
        implement ReadAs<Int> for Cell { fn read() -> Int { 79 } }
        implement ReadAs<Bool> for Cell { fn read() -> Int { 31 } }
        implement Select<Bool> for Cell {}
        implement Local for Cell {}
        fn run() -> Int { Cell { value: 0 }.selected() + Cell { value: 0 }.seeded() }
    "#;
    for omit_match in [false, true] {
        let source = if omit_match {
            source.replace("implement ReadAs<Bool> for Cell { fn read() -> Int { 31 } }", "")
        } else {
            source.to_owned()
        };
        let unit = Parser::new(&source).parse_module().unwrap();
        let mut generator = VbcCodegen::new();
        let result = generator.compile_module(&unit);
        if omit_match {
            assert!(result.unwrap_err().to_string().contains("read"));
            continue;
        }
        let module = result.unwrap();
        let encoded = verum_vbc::serialize::serialize_module(&module).unwrap();
        for module in [
            module,
            verum_vbc::deserialize::deserialize_module(&encoded).unwrap(),
        ] {
            let run = module.find_function_by_name("chooser.run").unwrap();
            let value = Interpreter::new(Arc::new(module)).execute_function(run).unwrap();
            assert_eq!(value.try_as_i64(), Some(73));
        }
    }
}
