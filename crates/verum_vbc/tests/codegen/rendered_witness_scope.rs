//! Display text can select a declared binder, but cannot invent one.
use super::VbcCodegen;
use crate::{codegen::CodegenConfig, types::{TypeParamId, TypeRef}};
use verum_fast_parser::Parser;

fn codegen() -> VbcCodegen {
    let ast = Parser::new("type Carrier<T> is { marker: Int }; type Pair<A, B> is { marker: Int };")
        .parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("witness"));
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    codegen
}

#[test]
fn an_unknown_argument_does_not_acquire_a_positional_binder() {
    let codegen = codegen();
    for name in ["Carrier<Absent>", "Pair<Int, Absent>", "Pair<Carrier<Absent>, Int>"] {
        assert_eq!(codegen.type_name_to_type_ref_mono(name), None, "{name}");
    }
}

#[test]
fn a_declared_argument_uses_its_actual_scope_id() {
    let mut codegen = codegen();
    codegen.ctx.generic_type_params.insert("T".into());
    codegen.ctx.generic_type_params_ordered.push("T".into());
    codegen.ctx.current_generic_param_ids.insert("T".into(), 7);
    let carrier = codegen.nominal_type_id("Carrier").unwrap();
    assert_eq!(codegen.type_name_to_type_ref_mono("Carrier<T>"), Some(TypeRef::Instantiated {
        base: carrier, args: vec![TypeRef::Generic(TypeParamId(7))],
    }));
}

#[test]
fn an_unscoped_display_name_does_not_bind_to_a_callers_unrelated_slot() {
    let mut codegen = codegen();
    codegen.ctx.generic_type_params.insert("Actual".into());
    codegen.ctx.generic_type_params_ordered.push("Actual".into());
    codegen.ctx.current_generic_param_ids.insert("Actual".into(), 0);
    assert_eq!(codegen.type_name_to_type_ref_mono("Pair<Actual, Absent>"), None);
}
