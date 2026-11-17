//! Raw argument ownership follows the selected source declaration across bootstrap.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
    module::FunctionId,
};

fn wire(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).unwrap(),
    ).unwrap()
}

#[test]
fn forward_raw_parameters_keep_the_exact_declared_slot() {
    for pointer in ["*const T", "*mut T", "*volatile T"] {
        let provider_source = format!(r#"
            module core.provider;
            public fn inspect<T>(marker: (), pointer: {pointer}) -> Bool {{
                @intrinsic("ptr_is_null", pointer)
            }}
        "#);
        let provider = Parser::new(&provider_source).parse_module().unwrap();
        let consumer_source = format!(r#"
            module core.consumer;
            mount core.provider.inspect;
            fn probe() -> Int {{
                if inspect<Int>((), 0 as {}) {{ 73 }} else {{ 0 }}
            }}
        "#, pointer.replace('T', "Int"));
        let consumer = Parser::new(&consumer_source).parse_module().unwrap();
        let mut declarations = VbcCodegen::with_config(CodegenConfig::new("core.provider"));
        let interface = declarations.collect_declaration_interface(&[&provider]).unwrap();
        for interface in [interface.clone(), wire(&interface)] {
            let mut forward = declarations.export_functions()["inspect"].clone();
            forward.id = FunctionId(verum_vbc::stub_ranges::STAGE3_BASE);
            forward.callable_signature = None;
            forward.return_type = None;
            // A forward name registry has no parameter spellings. The checked
            // source interface, including the preceding unit slot, is complete.
            forward.param_type_names.clear();
            let functions = [("core.provider.inspect".to_owned(), forward)].into_iter().collect();
            let mut compiler = VbcCodegen::with_config(CodegenConfig::new("core.consumer"));
            compiler.import_functions(&functions);
            compiler.import_bootstrap_nominal_dependencies(&[&consumer], &[&interface]).unwrap();
            compiler.collect_unit_declarations(&[&consumer]).unwrap();
            compiler.compile_unit_items(&[&consumer], ItemFailurePolicy::Strict)
                .unwrap_or_else(|error| panic!("{pointer}: {error}"));
            compiler.compile_unit_items(&[&provider], ItemFailurePolicy::Strict).unwrap();
            let module = compiler.finalize_module_from_state().unwrap();
            for module in [module.clone(), wire(&module)] {
                verum_vbc::validate::validate_module(&module).unwrap();
                let entry = module.find_function_by_name("core.consumer.probe").unwrap();
                let value = Interpreter::new(module.into()).execute_function(entry)
                    .unwrap_or_else(|error| panic!("{pointer}: {error}"));
                assert_eq!(value.try_as_i64(), Some(73), "{pointer}");
            }
        }
    }
}
