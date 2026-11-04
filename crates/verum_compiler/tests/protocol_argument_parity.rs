//! Protocol applications retain declared kinds in the source checker and VBC.
use std::sync::Arc;
use verum_ast::FileId;
use verum_common::Text;
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::Interpreter,
};

fn source_errors(source: &str) -> Vec<Text> {
    let file_id = FileId::new(0);
    let ast = FastParser::new().parse_module_str(source, file_id).unwrap();
    let id = ModuleId::new(0);
    let owner = ModulePath::from_str("applications");
    let mut module = ModuleInfo::new(id, owner.clone(), ast.clone(), file_id, source.into());
    module.exports = extract_exports_from_module(&ast, id, &owner).unwrap();
    let mut registry = ModuleRegistry::new();
    registry.register(module);
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("applications");
    checker.set_current_module_path("applications");
    checker.set_module_registry_direct(registry);
    let mut errors: Vec<Text> = checker
        .prepare_module_types(&ast)
        .into_iter()
        .map(|e| format!("{e:?}").into())
        .collect();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|e| e.is_error())
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors
}

#[test]
fn closed_integer_application_survives_source_wire_and_execution() {
    let source = r#"module applications;
        type Dimension<N:meta Int> is protocol { fn dimension(&self)->Int {N} };
        type Point is {value:Int};
        implement Dimension<(6 * 7)> for Point {}
        fn main()->Int { let point = Point {value:0}; point.dimension() }
    "#;
    let errors = source_errors(source);
    assert!(errors.is_empty(), "{errors:?}");
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(0))
        .unwrap();
    let compiled = VbcCodegen::with_config(CodegenConfig::new("applications"))
        .compile_module(&ast)
        .unwrap();
    let restored = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&compiled).unwrap(),
    )
    .unwrap();
    for module in [compiled, restored] {
        let entry = module.find_function_by_name("applications.main").unwrap();
        let result = Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap();
        assert_eq!(result.try_as_i64(), Some(42));
    }
}

#[test]
fn invalid_value_applications_refuse_in_both_source_consumers() {
    for arguments in [
        "<Int>",
        "<true>",
        "<1i32>",
        "<(1 / 0)>",
        "<(9223372036854775807 + 1)>",
        "",
        "<1,2>",
    ] {
        let source = format!(
            r#"module applications;
            type Dimension<N:meta Int> is protocol {{}};
            type Point is {{value:Int}};
            implement Dimension{arguments} for Point {{}}
        "#
        );
        let errors = source_errors(&source);
        assert!(!errors.is_empty(), "source accepted {arguments}");
        let ast = FastParser::new()
            .parse_module_str(&source, FileId::new(0))
            .unwrap();
        let result =
            VbcCodegen::with_config(CodegenConfig::new("applications")).compile_module(&ast);
        assert!(result.is_err(), "VBC accepted {arguments}");
    }
}

#[test]
fn nested_value_identity_survives_application_wire_and_vm() {
    for (constructor, value, required) in [
        ("Buffer", 3, 3),
        ("Buffer", 3, 4),
        ("Sized", 3, 3),
        ("Sized", 3, 4),
    ] {
        let source = format!(
            r#"module applications;
            type Buffer<N:meta Int> is {{value:Int}};
            type Sized<N:meta Int> is Buffer<N>;
            type Carrier<T> is protocol {{ fn echo(&self,value:T)->T {{value}} }};
            type Point is {{value:Int}};
            implement Carrier<{constructor}<{value}>> for Point {{}}
            fn need<T:Carrier<Buffer<{required}>>>(point:T)->Int {{7}}
            fn main()->Int {{let point=Point{{value:0}}; need(point)}}
        "#
        );
        let errors = source_errors(&source);
        if value != required {
            assert!(
                errors.iter().any(|error| error.contains("Carrier")),
                "{errors:?}"
            );
            continue; // Ordinary compilation must stop at this obligation.
        }
        assert!(errors.is_empty(), "{errors:?}");
        let ast = FastParser::new()
            .parse_module_str(&source, FileId::new(0))
            .unwrap();
        let compiled = VbcCodegen::with_config(CodegenConfig::new("applications"))
            .compile_module(&ast)
            .unwrap();
        let restored = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&compiled).unwrap(),
        )
        .unwrap();
        for module in [compiled, restored] {
            let echo = module
                .functions
                .iter()
                .find(|function| {
                    module
                        .get_string(function.name)
                        .is_some_and(|name| name.ends_with("Point.echo"))
                })
                .unwrap();
            let buffer = module
                .types
                .iter()
                .find(|ty| module.get_string(ty.name) == Some("Buffer"))
                .unwrap()
                .id;
            let expected = verum_vbc::types::TypeRef::Instantiated {
                base: buffer,
                args: vec![verum_vbc::types::TypeRef::ConstValue(value)],
            };
            let written = module
                .types
                .iter()
                .find(|ty| module.get_string(ty.name) == Some(constructor))
                .unwrap();
            let arguments = vec![verum_vbc::types::TypeRef::ConstValue(value)];
            let declared = verum_vbc::types::TypeRef::Instantiated {
                base: written.id,
                args: arguments.clone(),
            };
            assert_eq!(echo.semantic_return.as_ref(), Some(&declared));
            assert_eq!(
                echo.semantic_params.as_ref().unwrap()[1].as_ref(),
                Some(&declared)
            );
            let expanded = if written.kind == verum_vbc::types::TypeKind::Alias {
                assert_eq!(written.type_params.len(), arguments.len());
                verum_vbc::mono::TypeSubstitution::new(&written.type_params, &arguments)
                    .apply(
                        written
                            .alias_target
                            .as_ref()
                            .expect("alias target authority"),
                    )
                    .unwrap()
            } else {
                declared
            };
            assert_eq!(expanded, expected);
            let entry = module.find_function_by_name("applications.main").unwrap();
            assert_eq!(
                Interpreter::new(Arc::new(module))
                    .execute_function(entry)
                    .unwrap()
                    .try_as_i64(),
                Some(7)
            );
        }
    }
}
