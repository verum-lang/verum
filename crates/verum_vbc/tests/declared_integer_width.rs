//! Fixed-width arithmetic through source declarations and their inherent methods.
#![cfg(feature = "codegen")]

use verum_ast::decl::{ImplItemKind, ImplKind};
use verum_ast::{ItemKind, ty::TypeKind};
use verum_fast_parser::Parser;
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};
use verum_vbc::deserialize::deserialize_module;
use verum_vbc::interpreter::Interpreter;
use verum_vbc::serialize::serialize_module;

#[test]
fn primitive_method_owners_preserve_arithmetic_width_after_wire_roundtrip() {
    let mut primitives = Parser::new(include_str!("../../../core/base/primitives.vr"))
        .parse_module()
        .unwrap();
    primitives.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Impl(implementation) => {
            let ImplKind::Inherent(ty) = &implementation.kind else {
                return false;
            };
            let owner = match &ty.kind {
                TypeKind::Path(path) => path.last_segment_name(),
                kind => kind.primitive_name().unwrap_or(""),
            };
            if !matches!(owner, "UInt32" | "Int32" | "UInt64") {
                return false;
            }
            implementation.items.retain(
                |item| matches!(&item.kind, ImplItemKind::Function(f) if f.name.as_str() == "min"),
            );
            !implementation.items.is_empty()
        }
        _ => false,
    });
    assert_eq!(
        primitives
            .items
            .iter()
            .filter(|i| matches!(i.kind, ItemKind::Impl(_)))
            .count(),
        3
    );
    let callers = Parser::new(
        r#"
        module width_client;
        fn shift(x: UInt32, count: Int) -> UInt32 { x << count }
        fn probe() -> Int {
            let values: [UInt32; 1] = [0x510e527f_u32];
            let word = values[0];
            if shift(word, 25) as UInt64 != 4261412864_u64 { return 1; }
            let rotated = (word >> 7) | (word << 25);
            if rotated as UInt64 != 4272037028_u64 { return 2; }
            let max = 4294967295_u32;
            if (max + 1_u32 + 1_u32) as UInt64 != 1_u64 { return 3; }
            let signed = 2147483647_i32;
            if (signed + 1_i32) as Int != -2147483648 { return 4; }
            let wide = 4294967296_u64;
            if wide + 1_u64 != 4294967297_u64 { return 5; }
            0
        }
    "#,
    )
    .parse_module()
    .unwrap();
    let units = [&primitives, &callers];
    let mut generator = VbcCodegen::new();
    generator.initialize();
    generator.collect_unit_declarations(&units).unwrap();
    generator.resolve_pending_imports();
    generator
        .compile_unit_items(&units, ItemFailurePolicy::Strict)
        .unwrap();
    let module = generator.finalize_module().unwrap();
    let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for (route, module) in [("source", module), ("serialized", decoded)] {
        let entry = module
            .functions
            .iter()
            .find(|f| module.get_string(f.name) == Some("width_client.probe"))
            .unwrap()
            .id;
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().as_i64(), 0, "{route}");
    }
}

#[test]
fn compound_operations_use_the_destination_type_and_evaluate_an_index_once() {
    let ast = Parser::new(r#"
        module compound;
        type Fields is { magnitude: UInt64, narrow: UInt8, number: Float };
        type Magnitude is UInt64;
        type Coefficient is Float;
        fn aliases(input: Magnitude, factor: Coefficient) -> Bool {
            let mut high = input; let mut number = factor;
            high >>= 63; number *= factor;
            high == 1_u64 && high == input >> 63 && number == 2.25 && number == factor * factor
        }
        fn once(count: &mut Int) -> Int { *count = *count + 1; 0 }
        fn update(value: &mut UInt64) { *value >>= 63; }
        fn probe_locals() -> Int {
            let mut high = 18446744073709551615_u64;
            high >>= 63;
            if high != 1_u64 {return -1;}
            let mut negative = -8_i32; negative >>= 2;
            if negative != -2_i32 {return -2;}
            let mut number = 1.25; number *= 2.0;
            if number != 2.5 {return -3;}
            let mut dividend = 18446744073709551615_u64; dividend /= 10_u64;
            let mut remainder = 18446744073709551615_u64; remainder %= 10_u64;
            if dividend != 1844674407370955161_u64 || remainder != 5_u64 {return -4;}
            let mut byte = 255_u8; byte += 1_u8;
            if byte != 0_u8 {return -5;}
            let mut signed = 127_i8; signed += 1_i8;
            if signed != -128_i8 {return -6;}
            let quotient = signed / -1_i8;
            signed /= -1_i8;
            if signed != -128_i8 || quotient != signed {return -7;}
            if !aliases(18446744073709551615_u64, 1.5) {return -8;}
            1
        }
        fn probe_fields() -> Int {
            let mut holder = Fields { magnitude: 18446744073709551615_u64, narrow: 200_u8, number: -1.25 };
            holder.magnitude >>= 63; holder.narrow *= 2_u8; holder.number *= -2.0;
            if holder.magnitude == 1_u64 && holder.narrow == 144_u8 && holder.number == 2.5 {1} else {-1}
        }
        fn probe_index_once() -> Int {
            let mut calls = 0; let mut values: [UInt64; 2] = [18446744073709551615_u64, 9_u64];
            values[once(&mut calls)] >>= 63;
            if calls == 1 && values[0] == 1_u64 && values[1] == 9_u64 {1} else {-1}
        }
        fn probe_tuple_and_references() -> Int {
            let mut values = (0.125, 18446744073709551615_u64); values.0 *= 4.0; values.1 %= 10_u64;
            let mut high = 9223372036854775808_u64; update(&mut high);
            if values.0 == 0.5 && values.1 == 5_u64 && high == 1_u64 {1} else {-1}
        }
        static mut STATIC_HIGH: UInt64 = 0_u64;
        fn probe_static() -> Int {
            STATIC_HIGH = 18446744073709551615_u64; STATIC_HIGH >>= 63;
            if STATIC_HIGH == 1_u64 {1} else {-1}
        }
        fn probe_destructuring() -> Int {
            let mut real = 2.5; let mut wide = 18446744073709551615_u64;
            (real, wide) /= (2.0,10_u64);
            if real == 1.25 && wide == 1844674407370955161_u64 {1} else {-1}
        }
        fn probe_captured() -> Int {
            let mut wide = 18446744073709551615_u64;
            let reduce = || { wide >>= 63; wide };
            if reduce() != 1_u64 || wide != 1_u64 {return -1;}
            let mut factor = 1.5;
            let both = || { (wide, factor) *= (4_u64, 2.0); };
            both();
            if wide == 4_u64 && factor == 3.0 {1} else {-2}
        }
    "#).parse_module().unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for (lane, module) in [("source", module), ("serialized", decoded)] {
        let cases: verum_common::List<_> = module
            .functions
            .iter()
            .filter_map(|function| {
                let name = module.get_string(function.name)?;
                name.starts_with("compound.probe_")
                    .then(|| (name.to_owned(), function.id))
            })
            .collect();
        assert_eq!(cases.len(), 7, "{lane}");
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        vm.run_global_ctors().unwrap();
        for (name, entry) in cases {
            assert_eq!(
                vm.execute_function(entry).unwrap().as_i64(),
                1,
                "{lane} {name}"
            );
        }
    }
}

#[test]
fn library_integer_power_terminates_for_the_minimum_exponent() {
    let mut primitives = Parser::new(include_str!("../../../core/base/primitives.vr"))
        .parse_module()
        .unwrap();
    primitives.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Impl(implementation) => {
            implementation.items.retain(|item| {
                matches!(
                &item.kind, ImplItemKind::Function(f) if f.name.as_str() == "unsigned_abs")
            });
            !implementation.items.is_empty()
        }
        _ => false,
    });
    let mut libm = Parser::new(include_str!("../../../core/math/libm.vr"))
        .parse_module()
        .unwrap();
    libm.items.retain(|item| {
        matches!(item.kind, ItemKind::Module(_))
            || matches!(&item.kind, ItemKind::Function(f) if f.name.as_str() == "ipow")
    });
    let mut elementary = Parser::new(include_str!("../../../core/math/elementary.vr"))
        .parse_module()
        .unwrap();
    elementary.items.retain(|item| {
        matches!(item.kind, ItemKind::Module(_) | ItemKind::Mount(_))
            || matches!(&item.kind, ItemKind::Function(f) if f.name.as_str() == "powi")
    });
    let client = Parser::new(
        r#"
        module power_client;
        mount core.math.elementary.powi;
        fn probe() -> Bool {
            powi(-1.0, -9223372036854775808) == 1.0 &&
            powi(2.0, -3) == 0.125 && powi(3.0, 2) == 9.0
        }
    "#,
    )
    .parse_module()
    .unwrap();
    let units = [&primitives, &libm, &elementary, &client];
    let mut generator = VbcCodegen::new();
    generator.initialize();
    generator.collect_unit_declarations(&units).unwrap();
    generator.resolve_pending_imports();
    generator
        .compile_unit_items(&units, ItemFailurePolicy::Strict)
        .unwrap();
    let module = generator.finalize_module().unwrap();
    let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for (lane, module) in [("source", module), ("serialized", decoded)] {
        let entry = module
            .functions
            .iter()
            .find(|f| module.get_string(f.name) == Some("power_client.probe"))
            .unwrap()
            .id;
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        vm.state.config.max_instructions = 100_000;
        assert!(vm.execute_function(entry).unwrap().as_bool(), "{lane}");
    }
}

#[test]
fn compound_assignment_rejects_nonnumeric_destinations_and_float_shifts() {
    for (ty, operator) in [("Number", "+="), ("Float", ">>=")] {
        let source = format!(
            r#"
            module invalid;
            type Number is {{ value: Int }};
            fn invalid(mut value: {ty}) {{ value {operator} 1; }}
        "#
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        let mut generator = VbcCodegen::new();
        generator.initialize();
        generator.collect_unit_declarations(&[&ast]).unwrap();
        generator.resolve_pending_imports();
        let error = generator
            .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
            .unwrap_err();
        assert!(
            error.to_string().contains("numeric operation"),
            "{ty} {operator}: {error}"
        );
    }
}

#[test]
fn raw_integer_assignments_preserve_neighbouring_elements() {
    let ast = Parser::new(
        r#"
        module raw;
        fn bump(p: *mut UInt8) { unsafe { *p += 1_u8; } }
        fn halve(p: *mut Int16) { unsafe { *p = -260_i16; *p >>= 1; } }
    "#,
    )
    .parse_module()
    .unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for (lane, module) in [("source", module), ("serialized", decoded)] {
        let bump = module.find_function_by_name("raw.bump").unwrap();
        let halve = module.find_function_by_name("raw.halve").unwrap();
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        let mut bytes = [0xa5_u8; 16];
        bytes[3] = 255;
        let byte_slot = verum_vbc::value::Value::from_ptr(bytes[3..].as_mut_ptr());
        vm.execute_function_with_args(bump, &[byte_slot]).unwrap();
        let mut expected = [0xa5; 16];
        expected[3] = 0;
        assert_eq!(bytes, expected, "{lane} UInt8");
        let mut shorts = [0x45a5_i16; 8];
        let short_slot = verum_vbc::value::Value::from_ptr(shorts[2..].as_mut_ptr());
        vm.execute_function_with_args(halve, &[short_slot]).unwrap();
        let mut expected = [0x45a5; 8];
        expected[2] = -130;
        assert_eq!(shorts, expected, "{lane} Int16");
    }
}
