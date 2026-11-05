//! Repeated imports must preserve one declaration contract after pool relocation.
#![cfg(feature = "codegen")]
use verum_ast::Module;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    module::{Constant, FunctionDescriptor, VbcModule},
};
fn parse(source: &str) -> Module {
    Parser::new(source).parse_module().unwrap()
}
fn interface(source: &str) -> (VbcModule, VbcCodegen) {
    interface_at(source, 0)
}
fn interface_at(source: &str, file: u32) -> (VbcModule, VbcCodegen) {
    let file = verum_ast::FileId::new(file);
    let ast = verum_fast_parser::VerumParser::new()
        .parse_module(verum_lexer::Lexer::new(source, file), file)
        .unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("fixture"));
    let module = codegen
        .collect_owned_declaration_interface(&[("fixture", &ast)])
        .unwrap();
    (module, codegen)
}
fn wire(module: &VbcModule) -> VbcModule {
    verum_vbc::validate::validate_module(module).unwrap();
    let bytes = verum_vbc::serialize::serialize_module(module).unwrap();
    let result = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    verum_vbc::validate::validate_module(&result).unwrap();
    result
}
fn selected(module: &VbcModule) -> &FunctionDescriptor {
    module
        .functions
        .iter()
        .find(|function| {
            let name = module.strings.get(function.name).unwrap();
            let owner = function
                .origin_module
                .and_then(|id| module.strings.get(id))
                .unwrap_or(&module.name);
            verum_vbc::module::qualify_module_name(owner, name) == "fixture.provider.selected"
        })
        .unwrap()
}
const FIRST: &str = "module fixture.provider; @intrinsic(\"contract_alpha\") public fn selected(value: Int = 41)->Int;";
const CALLER: &str =
    "module fixture.consumer; mount fixture.provider.selected; public fn probe()->Int{selected()}";
fn reject_second(first: &str, second: &str) {
    let (first, registered) = interface(first);
    let (second, _) = interface(second);
    for roundtrip in [false, true] {
        let first = if roundtrip {
            wire(&first)
        } else {
            first.clone()
        };
        let second = if roundtrip {
            wire(&second)
        } else {
            second.clone()
        };
        let caller = parse(CALLER);
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("fixture"));
        codegen.initialize();
        codegen.import_functions(&registered.export_functions());
        codegen
            .import_bootstrap_nominal_dependencies(&[&caller], &[&first])
            .unwrap();
        let error = codegen
            .import_bootstrap_nominal_dependencies(&[&caller], &[&second])
            .expect_err(
                "a changed contract must fail at repeated import, before compiling its caller",
            );
        assert!(
            error
                .to_string()
                .contains("conflicting callable declarations"),
            "{error}"
        );
    }
}
fn accept_repeat(first: &str, second: &str) {
    let (first, registered) = interface(first);
    let (second, _) = interface(second);
    for roundtrip in [false, true] {
        let first = if roundtrip {
            wire(&first)
        } else {
            first.clone()
        };
        let second = if roundtrip {
            wire(&second)
        } else {
            second.clone()
        };
        let caller = parse(CALLER);
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("fixture").with_validation());
        codegen.initialize();
        codegen.import_functions(&registered.export_functions());
        codegen
            .import_bootstrap_nominal_dependencies(&[&caller], &[&first])
            .unwrap();
        codegen
            .import_bootstrap_nominal_dependencies(&[&caller], &[&second])
            .unwrap();
        codegen
            .collect_owned_unit_declarations(&[("fixture", &caller)])
            .unwrap();
        codegen.resolve_pending_imports();
        codegen
            .compile_owned_unit_items(&[("fixture", &caller)], ItemFailurePolicy::Strict)
            .unwrap();
        let result = codegen.finalize_module().unwrap();
        for result in [result.clone(), wire(&result)] {
            let function = selected(&result);
            assert!(matches!(
                result.constants[function.params[0].default.unwrap().0 as usize],
                Constant::Int(41)
            ));
            assert_eq!(
                result.strings.get(function.intrinsic_name.unwrap()),
                Some("contract_alpha")
            );
            assert!(!function.has_source_body);
        }
    }
}
#[test]
fn identical_source_and_wire_imports_are_idempotent() {
    accept_repeat(FIRST, FIRST);
}
#[test]
fn relocated_string_constant_and_function_pools_remain_idempotent() {
    accept_repeat(
        FIRST,
        "module fixture.provider; public fn unrelated(p:Float=3.5)->Float{p} @intrinsic(\"contract_alpha\") public fn selected(value: Int = 41)->Int;",
    );
}
#[test]
fn changed_default_is_rejected_at_import() {
    reject_second(
        FIRST,
        "module fixture.provider; @intrinsic(\"contract_alpha\") public fn selected(value: Int = 99)->Int;",
    );
}
#[test]
fn removed_default_is_rejected_at_import() {
    reject_second(
        FIRST,
        "module fixture.provider; @intrinsic(\"contract_alpha\") public fn selected(value: Int)->Int;",
    );
}
#[test]
fn changed_intrinsic_is_rejected_at_import() {
    reject_second(
        FIRST,
        "module fixture.provider; @intrinsic(\"contract_beta\") public fn selected(value: Int = 41)->Int;",
    );
}
#[test]
fn source_body_cannot_be_replaced_with_a_forward_intrinsic() {
    reject_second(
        "module fixture.provider; @intrinsic(\"contract_alpha\") public fn selected(value: Int = 41)->Int{value+1}",
        FIRST,
    );
}
#[test]
fn forward_intrinsic_cannot_gain_source_body_on_repeat() {
    reject_second(
        FIRST,
        "module fixture.provider; @intrinsic(\"contract_alpha\") public fn selected(value: Int = 41)->Int{value+1}",
    );
}
#[test]
fn changed_context_identity_is_rejected_at_import() {
    reject_second(
        "module fixture.provider; context FirstGate{fn check();} @intrinsic(\"contract_alpha\") public fn selected(value:Int=41)->Int using [FirstGate];",
        "module fixture.provider; context SecondGate{fn check();} @intrinsic(\"contract_alpha\") public fn selected(value:Int=41)->Int using [SecondGate];",
    );
}

#[test]
fn changed_async_contract_is_rejected() {
    reject_second(
        FIRST,
        "module fixture.provider; @intrinsic(\"contract_alpha\") public async fn selected(value:Int=41)->Int;",
    );
}
#[test]
fn changed_generator_contract_is_rejected() {
    reject_second(
        FIRST,
        "module fixture.provider; @intrinsic(\"contract_alpha\") public fn* selected(value:Int=41)->Int;",
    );
}
#[test]
fn changed_type_bound_is_rejected() {
    reject_second(
        "module fixture.provider; public type FirstBound is protocol {}; public type SecondBound is protocol {}; @intrinsic(\"contract_alpha\") public fn selected<T: FirstBound>(value:T)->T;",
        "module fixture.provider; public type FirstBound is protocol {}; public type SecondBound is protocol {}; @intrinsic(\"contract_alpha\") public fn selected<T: SecondBound>(value:T)->T;",
    );
}
#[test]
fn changed_type_default_is_rejected() {
    reject_second(
        "module fixture.provider; @intrinsic(\"contract_alpha\") public fn selected<T=Int>(value:T)->T;",
        "module fixture.provider; @intrinsic(\"contract_alpha\") public fn selected<T=Bool>(value:T)->T;",
    );
}
#[test]
fn changed_parameter_name_is_rejected() {
    reject_second(
        FIRST,
        "module fixture.provider; @intrinsic(\"contract_alpha\") public fn selected(different:Int=41)->Int;",
    );
}
#[test]
fn changed_function_bound_is_rejected() {
    reject_second(
        "module fixture.provider; @intrinsic(\"contract_alpha\") public fn selected<T:fn(Int)->Bool>(value:Int=41)->Int;",
        "module fixture.provider; @intrinsic(\"contract_alpha\") public fn selected<T:fn(Bool)->Int>(value:Int=41)->Int;",
    );
}
#[test]
fn changed_source_role_is_rejected() {
    reject_second(
        "module fixture.provider; @intrinsic(\"contract_alpha\") public fn selected<N:meta Int>(value:Int=41)->Int;",
        "module fixture.provider; @intrinsic(\"contract_alpha\") public fn selected<N:meta Bool>(value:Int=41)->Int;",
    );
}

fn import_pair(
    first: &VbcModule,
    second: &VbcModule,
    registered: &VbcCodegen,
) -> Result<(), String> {
    let caller = parse(CALLER);
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("fixture"));
    codegen.initialize();
    codegen.import_functions(&registered.export_functions());
    codegen
        .import_bootstrap_nominal_dependencies(&[&caller], &[first])
        .map_err(|error| format!("first import: {error}"))?;
    codegen
        .import_bootstrap_nominal_dependencies(&[&caller], &[second])
        .map_err(|error| format!("second import: {error}"))?;
    Ok(())
}
#[test]
fn reparsed_signature_with_new_positions_and_file_id_is_idempotent() {
    let source = "module fixture.provider; @intrinsic(\"contract_alpha\") public fn selected<N:meta Int>(value:Int=41)->Int;";
    let (first, registered) = interface_at(source, 4);
    let (second, _) = interface_at(&format!("// source relocated\n\n{source}"), 991);
    assert_ne!(
        selected(&first).source_signature,
        selected(&second).source_signature
    );
    for (first, second) in [
        (first.clone(), second.clone()),
        (wire(&first), wire(&second)),
    ] {
        import_pair(&first, &second, &registered).unwrap();
    }
}
#[test]
fn complete_source_telescope_survives_the_declaration_and_wire_paths() {
    let (module, _) = interface(
        "module fixture.provider; public type Bound is protocol {}; @intrinsic(\"contract_alpha\") public fn selected<T:Bound=Int,N:meta Int>(value:T)->T;",
    );
    for module in [module.clone(), wire(&module)] {
        let source = selected(&module)
            .source_signature
            .as_ref()
            .expect("ordinary generic bounds require a complete declaration carrier");
        assert!(source.body.is_none());
        assert_eq!(source.generics.len(), 2);
        assert!(
            matches!(&source.generics[0].kind,verum_ast::ty::GenericParamKind::Type{bounds,default:Some(_),..} if bounds.len()==1)
        );
        assert!(matches!(
            &source.generics[1].kind,
            verum_ast::ty::GenericParamKind::Meta { .. }
        ));
    }
}
#[test]
fn equal_source_signature_does_not_authorize_changed_descriptor_contract() {
    let (module, registered) = interface(
        "module fixture.provider; @intrinsic(\"contract_alpha\") public fn selected<T>(value:Int=41)->Int;",
    );
    for change in 0..9 {
        let mut changed = module.clone();
        let index = changed
            .functions
            .iter()
            .position(|function| function.id == selected(&module).id)
            .unwrap();
        let function = &mut changed.functions[index];
        match change {
            0 => function.params[0].is_mut = !function.params[0].is_mut,
            1 => {
                function.type_params[0].default = Some(verum_vbc::types::TypeRef::Concrete(
                    verum_vbc::types::TypeId::BOOL,
                ))
            }
            2 => function.type_params[0].variance = verum_vbc::types::Variance::Covariant,
            3 => function.properties |= verum_vbc::types::PropertySet::ASYNC,
            4 => function.calling_convention = verum_vbc::module::CallingConvention::SysV64,
            5 => function.is_gpu_only = !function.is_gpu_only,
            6 => function.is_const = !function.is_const,
            7 => function.type_params[0]
                .type_bounds
                .push(verum_vbc::types::TypeRef::Concrete(
                    verum_vbc::types::TypeId::BOOL,
                )),
            _ => {
                function.yield_type = Some(verum_vbc::types::TypeRef::Concrete(
                    verum_vbc::types::TypeId::BOOL,
                ))
            }
        }
        assert_eq!(
            selected(&module).source_signature,
            selected(&changed).source_signature
        );
        let error = import_pair(&module, &changed, &registered)
            .expect_err("equal source must not authorize a different ABI");
        assert!(
            error.contains("conflicting callable declarations"),
            "case {change}: {error}"
        );
    }
}
#[test]
fn same_interface_and_compiled_body_have_one_declaration_contract() {
    let source = "module fixture.provider; public fn selected<T:fn(Int)->Bool>(mut value:Int=41)->Int{value+1}";
    let (interface, registered) = interface(source);
    let ast = parse(source);
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("fixture").with_validation());
    codegen.initialize();
    codegen
        .collect_owned_unit_declarations(&[("fixture", &ast)])
        .unwrap();
    codegen
        .compile_owned_unit_items(&[("fixture", &ast)], ItemFailurePolicy::Strict)
        .unwrap();
    let compiled = codegen.finalize_module().unwrap();
    assert!(selected(&compiled).bytecode_length > 0);
    assert_eq!(selected(&interface).bytecode_length, 0);
    import_pair(&interface, &compiled, &registered).unwrap();
    import_pair(&wire(&compiled), &wire(&interface), &registered).unwrap();
}
