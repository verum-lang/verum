//! Async callables preserve eager execution and their bare output across archives.
use std::sync::Arc;
use verum_common::{List, Text};
use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::Parser;
use verum_types::{Type, TypeChecker};
use verum_vbc::{
    archive::{ArchiveBuilder, read_archive, write_archive},
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    module::VbcModule,
    serialize::serialize_module,
    types::{TypeId, TypeParamId, TypeRef},
};

fn checked_module(source: &str, owner: &str) -> VbcModule {
    let parsed = Parser::new(source)
        .parse_module()
        .expect("actual source parses");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path(owner);
    let errors = checker.prepare_module_declarations(&parsed);
    assert!(errors.is_empty(), "{errors:?}");
    for item in &parsed.items {
        checker.check_item(item).expect("source body typechecks");
    }
    let deferred = checker.take_deferred_errors();
    assert!(
        deferred.is_empty(),
        "deferred source failures: {deferred:?}"
    );
    assert!(
        checker
            .diagnostics()
            .iter()
            .all(|diagnostic| !diagnostic.is_error()),
        "{:?}",
        checker.diagnostics()
    );
    VbcCodegen::with_config(CodegenConfig::new(owner))
        .compile_module(&parsed)
        .expect("the same parsed source compiles")
}

fn eager_callback_source(callback_prefix: &str) -> String {
    format!(
        r#"module eager;
{callback_prefix}fn bump(counter: &mut Int) -> Int {{
    *counter = *counter + 1;
    *counter + 40
}}
async fn invoke(callback: async fn(&mut Int) -> Int, counter: &mut Int) -> Int {{
    callback(counter)
}}
async fn probe() -> Int {{
    let mut counter = 0;
    let returned = invoke(bump, &mut counter);
    counter * 100 + returned
}}
"#
    )
}

fn assert_eager_execution(source: &str) {
    let module = checked_module(source, "eager");
    let wire = serialize_module(&module).expect("serialize checked source");
    let restored = deserialize_module(&wire).expect("restore checked source");
    for module in [module, restored] {
        let entry = module
            .find_function_by_name("eager.probe")
            .expect("actual entry point");
        let result = Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .expect("run entry");
        assert_eq!(
            result.try_as_i64(),
            Some(141),
            "the callback mutates the reference before returning its bare Int output"
        );
    }
}

#[test]
fn an_async_callback_runs_before_its_caller_observes_the_mutable_argument() {
    assert_eager_execution(&eager_callback_source("async "));
}

#[test]
fn a_sync_callback_can_satisfy_the_async_callable_contract() {
    assert_eager_execution(&eager_callback_source(""));
}

#[test]
fn async_blocks_run_inline_and_preserve_the_declared_result() {
    assert_eager_execution(
        r#"module eager;
async fn answer() -> Int { 41 }
fn probe() -> Int {
    let mut counter = 0;
    let returned: Int = async {
        counter = counter + 1;
        answer().await
    };
    counter * 100 + returned
}
"#,
    );
}

#[test]
fn an_async_closure_preserves_the_same_eager_mutation_contract() {
    assert_eager_execution(
        r#"module eager;
async fn invoke(callback: async fn(&mut Int) -> Int, counter: &mut Int) -> Int {
    callback(counter)
}
async fn probe() -> Int {
    let bump = async |counter: &mut Int| -> Int {
        *counter = *counter + 1;
        *counter + 40
    };
    let mut counter = 0;
    let returned = invoke(bump, &mut counter);
    counter * 100 + returned
}
"#,
    );
}

const METADATA_SOURCE: &str = r#"module core.eager;
public fn retain(callback: async fn(Int) -> Int) -> async fn(Int) -> Int { callback }
public fn retain_poly(callback: async fn<T>(T) -> T) -> async fn<T>(T) -> T { callback }
"#;

fn assert_wire_callable(ty: &TypeRef, polymorphic: bool) {
    match (ty, polymorphic) {
        (
            TypeRef::Function {
                is_async,
                params,
                return_type,
                ..
            },
            false,
        ) => {
            assert!(*is_async, "callable marker is preserved");
            assert_eq!(params, &[TypeRef::Concrete(TypeId::INT)]);
            assert_eq!(return_type.as_ref(), &TypeRef::Concrete(TypeId::INT));
        }
        (
            TypeRef::Rank2Function {
                is_async,
                type_param_count,
                params,
                return_type,
                ..
            },
            true,
        ) => {
            assert!(*is_async, "rank-two callable marker is preserved");
            assert_eq!(*type_param_count, 1);
            assert_eq!(params, &[TypeRef::Generic(TypeParamId(0))]);
            assert_eq!(return_type.as_ref(), &TypeRef::Generic(TypeParamId(0)));
        }
        _ => panic!("wrong callable carrier: {ty:?}"),
    }
}

fn assert_replayed_callable(ty: &Type, polymorphic: bool) {
    let (function, variable) = match (ty, polymorphic) {
        (Type::Forall { vars, body }, true) => {
            assert_eq!(vars.len(), 1);
            (body.as_ref(), Some(vars[0]))
        }
        (_, false) => (ty, None),
        _ => panic!("rank-two metadata lost its local binder: {ty:?}"),
    };
    let Type::Function {
        params,
        return_type,
        properties,
        property_requirement,
        ..
    } = function
    else {
        panic!("replayed callable is not a function: {function:?}");
    };
    assert_eq!(
        property_requirement,
        &verum_types::computational_properties::PropertyRequirement::source_callable(true),
        "the metadata must preserve its declared async callable contract"
    );
    assert!(
        properties.is_none(),
        "a callable annotation must not fabricate an inferred body footprint: {function:?}"
    );
    let expected = variable.map_or(Type::Int, Type::Var);
    assert_eq!(params.as_slice(), &[expected.clone()]);
    assert_eq!(
        return_type.as_ref(),
        &expected,
        "metadata must not invent a Future wrapper"
    );
}

#[test]
fn callable_and_rank_two_metadata_preserve_async_and_exact_output() {
    let module = checked_module(METADATA_SOURCE, "core.eager");
    let wire = serialize_module(&module).unwrap();
    let restored = deserialize_module(&wire).unwrap();
    for module in [&module, &restored] {
        for (name, polymorphic) in [("retain", false), ("retain_poly", true)] {
            let id = module
                .find_function_by_name(&format!("core.eager.{name}"))
                .expect("source function descriptor");
            let function = module.get_function(id).expect("exact function id");
            assert_wire_callable(&function.params[0].type_ref, polymorphic);
            assert_wire_callable(&function.return_type, polymorphic);
        }
    }
    let mut builder = ArchiveBuilder::new();
    builder.add_module("core.eager", &restored, &[]).unwrap();
    let mut bytes = List::new();
    write_archive(&builder.finish(), &mut bytes).unwrap();
    let archive = read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
    let metadata = archive_to_core_metadata(&archive).expect("publish actual archive metadata");
    for name in ["retain", "retain_poly"] {
        let function = metadata
            .functions
            .get(&Text::from(format!("core.eager.{name}")))
            .expect("exact declaration owner");
        if name == "retain_poly" {
            // Rank-two local kinds and obligations belong to the parsed
            // declaration, not to a guessed textual binder recovered from IDs.
            let source = Parser::new(METADATA_SOURCE).parse_module().unwrap();
            let mut expected = source
                .items
                .iter()
                .find_map(|item| match &item.kind {
                    verum_ast::ItemKind::Function(function)
                        if function.name.name.as_str() == name =>
                    {
                        Some(function.clone())
                    }
                    _ => None,
                })
                .unwrap();
            expected.body = None;
            assert_eq!(function.source_signature.as_ref(), Some(&expected));
        } else {
            assert!(
                function.params[0].ty.starts_with("async fn"),
                "{}",
                function.params[0].ty
            );
            assert!(
                function.return_type.starts_with("async fn"),
                "{}",
                function.return_type
            );
        }
    }
    let encoded = verum_types::core_metadata::encode_metadata(&metadata).unwrap();
    let decoded = verum_types::core_metadata::decode_metadata(&encoded).unwrap();
    let mut stale = encoded.clone();
    stale[8..12]
        .copy_from_slice(&(verum_types::core_metadata::CORE_METADATA_VERSION - 1).to_le_bytes());
    let error = verum_types::core_metadata::decode_metadata(&stale).unwrap_err();
    assert!(error.contains("unsupported metadata schema"), "{error}");
    let mut checker = TypeChecker::new_with_core_eager(Arc::new(decoded));
    assert!(
        checker.take_deferred_errors().is_empty(),
        "metadata replay failed"
    );
    assert!(
        checker
            .diagnostics()
            .iter()
            .all(|diagnostic| !diagnostic.is_error()),
        "{:?}",
        checker.diagnostics()
    );
    for (name, polymorphic) in [("retain", false), ("retain_poly", true)] {
        let expression = Parser::new(&format!("core.eager.{name}"))
            .parse_expr()
            .expect("actual qualified source expression");
        let result = checker
            .synth_expr(&expression)
            .expect("resolve actual archived function");
        let Type::Function {
            params,
            return_type,
            ..
        } = &result.ty
        else {
            panic!("replayed exported function: {:?}", result.ty);
        };
        assert_replayed_callable(&params[0], polymorphic);
        assert_replayed_callable(return_type, polymorphic);
        let deferred = checker.take_deferred_errors();
        assert!(
            deferred.is_empty(),
            "qualified replay failures: {deferred:?}"
        );
        assert!(
            checker
                .diagnostics()
                .iter()
                .all(|diagnostic| !diagnostic.is_error()),
            "{:?}",
            checker.diagnostics()
        );
    }
}

#[test]
fn ordinary_archive_metadata_does_not_invent_a_pure_body() {
    use verum_types::computational_properties::{PropertyRequirement, PropertySet};

    // Source compilation now always carries its declaration signature. Build
    // the ABI-only input explicitly so this separate replay boundary remains
    // covered without stripping evidence from a source-produced archive.
    let mut module = VbcModule::new("core.eager".into());
    let mut function = verum_vbc::module::FunctionDescriptor::new(module.intern_string("pulse"));
    function.return_type = TypeRef::Concrete(TypeId::INT);
    function.properties = verum_vbc::types::PropertySet::ASYNC;
    module.add_function(function);
    let wire = serialize_module(&module).unwrap();
    let restored = deserialize_module(&wire).unwrap();
    let mut builder = ArchiveBuilder::new();
    builder.add_module("core.eager", &restored, &[]).unwrap();
    let mut bytes = List::new();
    write_archive(&builder.finish(), &mut bytes).unwrap();
    let archive = read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
    let metadata = archive_to_core_metadata(&archive).unwrap();
    let declaration = metadata
        .functions
        .get(&Text::from("core.eager.pulse"))
        .expect("the actual exported declaration");
    assert!(declaration.is_async);
    assert!(
        declaration.source_signature.is_none(),
        "this regression must exercise ordinary ABI metadata replay"
    );
    let encoded = verum_types::core_metadata::encode_metadata(&metadata).unwrap();
    let decoded = verum_types::core_metadata::decode_metadata(&encoded).unwrap();
    let mut checker = TypeChecker::new_with_core_eager(Arc::new(decoded));
    let expression = Parser::new("core.eager.pulse").parse_expr().unwrap();
    let result = checker.synth_expr(&expression).unwrap();
    let Type::Function {
        properties,
        property_requirement,
        ..
    } = result.ty
    else {
        panic!("the exported declaration must resolve to a function");
    };
    assert!(
        properties.is_none(),
        "signature-only metadata must not fabricate an inferred body: {properties:?}"
    );
    for required in [
        PropertyRequirement::AtMost(PropertySet::pure()),
        PropertyRequirement::source_callable(false),
    ] {
        assert!(
            !required.satisfied_by(&property_requirement, properties.as_ref()),
            "an async declaration cannot establish {required:?} from its ABI signature"
        );
    }
    let call = Parser::new("core.eager.pulse()").parse_expr().unwrap();
    let result = checker
        .synth_expr(&call)
        .expect("ordinary eager call remains valid");
    assert_eq!(result.ty, Type::Int, "the actual result stays unwrapped");
    assert!(checker.take_deferred_errors().is_empty());
    assert!(
        checker
            .diagnostics()
            .iter()
            .all(|diagnostic| !diagnostic.is_error()),
        "{:?}",
        checker.diagnostics()
    );
}
