//! Exercise the integer ordering methods from core.base.primitives through typed VBC calls.
#![cfg(feature = "codegen")]

use std::fmt::Write;
use std::sync::Arc;
use verum_ast::ItemKind;
use verum_ast::decl::{ImplItemKind, ImplKind};
use verum_ast::ty::TypeKind;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_vbc::codegen::VbcCodegen;
use verum_vbc::interpreter::Interpreter;
use verum_vbc::{Value, VbcModule};

const INTEGERS: &[(&str, u32, bool)] = &[
    ("Int", 64, true),
    ("Int8", 8, true),
    ("UInt8", 8, false),
    ("Int16", 16, true),
    ("UInt16", 16, false),
    ("Int32", 32, true),
    ("UInt32", 32, false),
    ("Int64", 64, true),
    ("UInt64", 64, false),
    ("Int128", 128, true),
    ("UInt128", 128, false),
    ("ISize", usize::BITS, true),
    ("USize", usize::BITS, false),
];

fn source_with_callers() -> VbcModule {
    let mut primitives = Parser::new(include_str!("../../../core/base/primitives.vr"))
        .parse_module()
        .expect("production primitives grammar");
    primitives.items.retain_mut(|item| {
        let ItemKind::Impl(implementation) = &mut item.kind else {
            return matches!(item.kind, ItemKind::Module(_));
        };
        let ImplKind::Inherent(receiver) = &implementation.kind else {
            return false;
        };
        let name = match &receiver.kind {
            TypeKind::Path(path) => path.last_segment_name(),
            kind => kind.primitive_name().unwrap_or(""),
        };
        if !INTEGERS.iter().any(|(ty, _, _)| *ty == name) {
            return false;
        }
        implementation.items.retain(|item| {
            matches!(&item.kind,
            ImplItemKind::Function(f) if matches!(f.name.as_str(), "min" | "max" | "clamp"))
        });
        !implementation.items.is_empty()
    });

    let mut callers = Text::from("module order_client;\n");
    for (ty, _, _) in INTEGERS {
        for method in ["min", "max"] {
            writeln!(
                callers,
                "fn {ty}_{method}(x: {ty}, y: {ty}) -> {ty} {{ x.{method}(y) }}"
            )
            .unwrap();
        }
        writeln!(
            callers,
            "fn {ty}_clamp(x: {ty}, lo: {ty}, hi: {ty}) -> {ty} {{ x.clamp(lo, hi) }}"
        )
        .unwrap();
    }
    let callers = Parser::new(&callers).parse_module().expect("typed callers");
    let mut codegen = VbcCodegen::new();
    codegen
        .compile_module(&primitives)
        .expect("integer source method bodies");
    codegen
        .compile_additional_module(&callers)
        .expect("typed method calls")
}

// Each triple lists distinct, increasing values within the declared integer domain.
fn ordered_triples(bits: u32, signed: bool) -> List<[u128; 3]> {
    let high_bit = 1u128 << (bits - 1);
    let mut cases = List::new();
    if signed {
        let min = !(high_bit - 1);
        let max = high_bit - 1;
        cases.push([min, min + 1, u128::MAX]);
        cases.push([min, 0, max]);
        cases.push([max - 2, max - 1, max]);
    } else {
        let max = u128::MAX >> (128 - bits);
        cases.push([0, high_bit - 1, high_bit]);
        cases.push([high_bit, max - 1, max]);
    }
    if bits >= 64 {
        let first = 1u128 << 53;
        cases.push([first, first + 1, first + 2]);
    }
    if bits == 128 {
        cases.push([1, (1 << 64) + 1, (1 << 96) + 1]);
    }
    cases
}

#[test]
fn integer_min_max_and_clamp_keep_every_operand_bit() {
    let module = Arc::new(source_with_callers());
    let mut vm = Interpreter::new(module.clone());
    vm.run_global_ctors().expect("source initialization");
    for &(ty, bits, signed) in INTEGERS {
        for method in ["min", "max", "clamp"] {
            let name = format!("core.base.primitives.{ty}.{method}");
            let body = module
                .find_function_by_name(&name)
                .and_then(|id| module.get_function(id));
            assert!(
                body.is_some_and(|f| f.bytecode_length > 0),
                "missing {name}"
            );
        }
        for [lo, mid, hi] in ordered_triples(bits, signed) {
            let mut check = |method: &str, operands: &[u128], expected: u128| {
                let id = module
                    .find_function_by_name(&format!("order_client.{ty}_{method}"))
                    .expect("typed caller");
                let args: List<_> = operands
                    .iter()
                    .map(|&raw| {
                        if bits == 128 {
                            Value::from_i128_raw_signed(raw, signed)
                        } else {
                            Value::from_i64(raw as i64)
                        }
                    })
                    .collect();
                let actual = vm.execute_function_with_args(id, &args).unwrap();
                if bits == 128 {
                    assert!(actual.is_boxed_i128(), "{ty}.{method}");
                    assert_eq!(actual.boxed_i128_is_signed(), signed, "{ty}.{method}");
                    assert_eq!(
                        actual.as_i128_raw(),
                        expected,
                        "{ty}.{method}({operands:?})"
                    );
                } else {
                    assert_eq!(
                        actual.try_as_i64(),
                        Some(expected as i64),
                        "{ty}.{method}({operands:?})"
                    );
                }
            };
            for (lower, upper) in [(lo, mid), (mid, hi), (lo, hi)] {
                for pair in [[lower, upper], [upper, lower]] {
                    check("min", &pair, lower);
                    check("max", &pair, upper);
                }
            }
            check("min", &[mid, mid], mid);
            check("max", &[mid, mid], mid);
            check("clamp", &[lo, mid, hi], mid);
            check("clamp", &[hi, lo, mid], mid);
            check("clamp", &[mid, lo, hi], mid);
            check("clamp", &[lo, lo, hi], lo);
            check("clamp", &[hi, lo, hi], hi);
            check("clamp", &[lo, mid, mid], mid);
        }
    }
}
