//! A current sidecar must not turn absent archive call obligations into none.
use verum_ast::Module;
use verum_common::{List, Shared, Text};
use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, core_metadata::CoreMetadata};
use verum_vbc::{
    VbcModule,
    archive::{ArchiveBuilder, read_archive, write_archive},
    codegen::{CodegenConfig, VbcCodegen},
};

const OWNER: &str = "archive.equalities";
const SOURCE: &str = r#"
    module archive.equalities;
    public type Outcome<T> is Idle | Selected(T);
    public type Family is protocol { type Item; };
    public fn choose<T>(item:T)->Int where T=Outcome<Int> { 0 }
    public fn projected<F:Family>(item:F)->Int where F.Item=Int { 0 }
"#;

fn parse(source: &str) -> Module {
    Parser::new(source).parse_module().expect("source grammar")
}

fn errors(checker: &mut TypeChecker, source: &str) -> List<Text> {
    let ast = parse(source);
    let mut failures = checker.prepare_module_declarations(&ast);
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            failures.push(error);
        }
    }
    failures.extend(checker.take_deferred_errors());
    failures
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|error| format!("{error:?}").into())
        .collect()
}

fn produce(source: &str, interface: bool) -> VbcModule {
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path(OWNER);
    let failures = errors(&mut checker, source);
    assert!(failures.is_empty(), "source checking: {failures:#?}");
    let ast = parse(source);
    let mut producer = VbcCodegen::with_config(CodegenConfig::new(OWNER));
    if interface {
        producer.collect_declaration_interface(&[&ast])
    } else {
        producer.compile_module(&ast)
    }
    .expect("ordinary source producer")
}

fn publish(module: &VbcModule) -> Result<CoreMetadata, Text> {
    let mut builder = ArchiveBuilder::new();
    builder
        .add_module(OWNER, module, &[])
        .map_err(|error| Text::from(error.to_string()))?;
    let mut bytes = List::new();
    write_archive(&builder.finish(), &mut bytes).map_err(|error| Text::from(error.to_string()))?;
    let archive = read_archive(std::io::Cursor::new(bytes.as_slice()))
        .map_err(|error| Text::from(error.to_string()))?;
    let metadata = archive_to_core_metadata(&archive)?;
    verum_types::core_metadata::decode_metadata(&verum_types::core_metadata::encode_metadata(
        &metadata,
    )?)
}

fn consumer_errors(metadata: &CoreMetadata, source: &str, eager: bool) -> List<Text> {
    let metadata = Shared::new(metadata.clone()).into_arc();
    let mut checker = if eager {
        TypeChecker::with_minimal_context()
    } else {
        TypeChecker::new_with_core(metadata.clone())
    };
    checker.register_primitives();
    let mut registry = verum_modules::ModuleRegistry::new();
    registry.register(verum_modules::ModuleInfo::new(
        verum_modules::ModuleId::new(0),
        verum_modules::ModulePath::from_str(OWNER),
        parse(""),
        verum_ast::FileId::new(0),
        Text::new(),
    ));
    checker.set_module_registry_direct(registry);
    // Install dependencies before eager admission captures their revision.
    if eager {
        checker.load_core_eager(metadata);
    }
    checker.set_current_module_path("archive.client");
    let module = parse(source);
    checker.register_stdlib_types_for_module(&module);
    errors(&mut checker, source)
}

#[test]
fn actual_archive_and_sidecar_keep_generic_and_projected_call_obligations() {
    for interface in [false, true] {
        let module = produce(SOURCE, interface);
        for function in &module.functions {
            assert!(function.source_signature.is_some());
            assert!(function.declaration_identity.is_some());
        }
        let metadata = publish(&module).expect("complete source declarations");
        for eager in [false, true] {
            for source in [
                "mount archive.equalities.{Outcome,choose}; fn run()->Int { choose(Outcome.Selected(37)) }",
                "mount archive.equalities.{Family,projected}; type Number is (); implement Family for Number {type Item=Int;} fn run(value:Number)->Int { projected(value) }",
            ] {
                let failures = consumer_errors(&metadata, source, eager);
                assert!(
                    failures.is_empty(),
                    "interface={interface} eager={eager}: {failures:#?}"
                );
            }
            for source in [
                "mount archive.equalities.{choose}; fn run()->Int { choose(true) }",
                "mount archive.equalities.{Family,projected}; type Flag is (); implement Family for Flag {type Item=Bool;} fn run(value:Flag)->Int { projected(value) }",
            ] {
                let failures = consumer_errors(&metadata, source, eager);
                assert!(
                    failures
                        .iter()
                        .any(|error| error.contains("equality") || error.contains("Mismatch")),
                    "interface={interface} eager={eager}: {failures:#?}"
                );
            }
        }
    }
}

#[test]
fn partial_or_absent_source_authority_cannot_be_stamped_as_current_metadata() {
    let original = produce(SOURCE, false);
    for missing in 0..3 {
        let mut module = original.clone();
        let function = module
            .functions
            .iter_mut()
            .find(|function| {
                function
                    .source_signature
                    .as_ref()
                    .is_some_and(|source| source.name.name == "choose")
            })
            .unwrap();
        if missing != 1 {
            function.source_signature = None;
        }
        if missing != 0 {
            function.declaration_identity = None;
        }
        let error = publish(&module).unwrap_err();
        assert!(
            error.contains("source signature") || error.contains("declaration identity"),
            "{error}"
        );
    }
}

#[test]
fn abi_flags_and_intrinsic_spelling_cannot_authorize_an_erased_source_header() {
    let original = produce(SOURCE, false);
    let operation = verum_vbc::intrinsics::registry::INTRINSIC_REGISTRY
        .lookup("SIZE_OF")
        .expect("actual registry operation");
    for intrinsic in [false, true] {
        let mut module = original.clone();
        let intrinsic_name = module.intern_string(operation.name);
        let function = module
            .functions
            .iter_mut()
            .find(|function| {
                function
                    .source_signature
                    .as_ref()
                    .is_some_and(|source| source.name.name == "choose")
            })
            .unwrap();
        function.source_signature = None;
        function.declaration_identity = None;
        // Keep the actual emitted-body/owned-parameter/source-definition
        // evidence consistent. It cannot reconstruct a missing call header.
        if intrinsic {
            function.intrinsic_name = Some(intrinsic_name);
        }
        let error = publish(&module).unwrap_err();
        assert!(
            error.contains("declaration-owned source signature") && error.contains("rebuild"),
            "{error}"
        );
    }
}

#[test]
fn generated_closure_bodies_remain_executable_entries_without_source_exports() {
    let module = produce(
        r#"
        module archive.equalities;
        public const ANSWER:Int=42;
        public type Token is {public value:Int};
        public fn make()->Token {Token {value:42}}
        public fn run()->Int {let callback:fn(Int)->Int=|value:Int| value+1; callback(41)}
    "#,
        false,
    );
    let closure = module
        .functions
        .iter()
        .find(|function| function.closure_capture_count.is_some())
        .expect("actual closure producer");
    assert!(closure.source_signature.is_none() && closure.declaration_identity.is_none());
    assert!(!closure.params.is_empty());
    let closure_name = module.get_string(closure.name).unwrap();
    let metadata = publish(&module).unwrap();
    assert!(
        metadata
            .functions
            .values()
            .all(|function| function.name.as_str() != closure_name)
    );
    assert!(
        metadata
            .functions
            .get(&Text::from("archive.equalities.ANSWER"))
            .expect("declared constant key")
            .is_const
    );
    assert!(
        metadata
            .types
            .get(&Text::from("archive.equalities.Token"))
            .unwrap()
            .source_declaration
            .is_some()
    );
    for eager in [false, true] {
        let failures = consumer_errors(
            &metadata,
            "mount archive.equalities.{ANSWER,Token}; fn read()->Int {ANSWER} fn build()->Token {Token {value:42}}",
            eager,
        );
        assert!(failures.is_empty(), "eager={eager}: {failures:#?}");
    }
    let mut missing_role = module.clone();
    missing_role
        .functions
        .iter_mut()
        .find(|function| function.id == closure.id)
        .unwrap()
        .closure_capture_count = None;
    assert!(
        publish(&missing_role)
            .unwrap_err()
            .contains("declaration-owned source signature")
    );
    // Publication does not mutate or discard the implementation body.
    assert!(
        module
            .functions
            .iter()
            .any(|function| function.id == closure.id && function.bytecode_length > 0)
    );
}

#[test]
fn non_declaration_roles_never_publish_an_unconstrained_callable() {
    let mut module = produce(SOURCE, false);
    let function = module
        .functions
        .iter_mut()
        .find(|function| {
            function
                .source_signature
                .as_ref()
                .is_some_and(|source| source.name.name == "choose")
        })
        .unwrap();
    function.source_signature = None;
    function.declaration_identity = None;
    function.is_const = true;
    assert!(
        publish(&module)
            .unwrap_err()
            .contains("declaration-owned source signature")
    );
}
