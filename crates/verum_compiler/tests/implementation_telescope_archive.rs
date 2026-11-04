//! Implementation quantifiers survive actual source, VBC, archive and sidecar boundaries.
use std::sync::Arc;
use verum_common::{List, Text};
use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, core_metadata::CoreMetadata};
use verum_vbc::{
    archive::{ArchiveBuilder, read_archive, write_archive},
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    module::VbcModule,
    serialize::serialize_module,
};

const OWNER: &str = "core.impl_telescope";
const GENERIC: &str = r#"
module core.impl_telescope;
public type Evidence is protocol { type Carrier; fn index(&self) -> Int; };
public type Cell<N: meta Int> is { value: () };
implement<N: meta Int{>=0}> Evidence for Cell<N> { type Carrier = Cell<N>; fn index(&self) -> Int { N } }
"#;

fn diagnostics(checker: &mut TypeChecker, source: &str) -> Vec<String> {
    let ast = Parser::new(source)
        .parse_module()
        .expect("actual source grammar");
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = checker
        .prepare_module_declarations(&ast)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error.to_string());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(ToString::to_string),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| format!("{d:?}")),
    );
    errors
}

fn source_module(source: &str) -> VbcModule {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path(OWNER);
    let errors = diagnostics(&mut checker, source);
    assert!(
        errors.is_empty(),
        "source producer must typecheck: {errors:?}"
    );
    let ast = Parser::new(source).parse_module().unwrap();
    VbcCodegen::with_config(CodegenConfig::new(OWNER))
        .compile_module(&ast)
        .expect("actual declarations compile")
}

fn publish(module: &VbcModule) -> Result<CoreMetadata, String> {
    let wire = serialize_module(module).map_err(|e| e.to_string())?;
    let restored = deserialize_module(&wire).map_err(|e| e.to_string())?;
    let mut builder = ArchiveBuilder::new();
    builder
        .add_module(OWNER, &restored, &[])
        .map_err(|e| e.to_string())?;
    let mut bytes = List::new();
    write_archive(&builder.finish(), &mut bytes).map_err(|e| e.to_string())?;
    let archive =
        read_archive(std::io::Cursor::new(bytes.as_slice())).map_err(|e| e.to_string())?;
    let metadata = archive_to_core_metadata(&archive).map_err(|e| e.to_string())?;
    let sidecar =
        verum_types::core_metadata::encode_metadata(&metadata).map_err(|e| e.to_string())?;
    verum_types::core_metadata::decode_metadata(&sidecar).map_err(|e| e.to_string())
}

fn consumer_errors(metadata: &CoreMetadata, source: &str, eager: bool) -> Vec<String> {
    let mut registry = verum_modules::ModuleRegistry::new();
    registry.register(verum_modules::ModuleInfo::new(
        verum_modules::ModuleId::new(0),
        verum_modules::ModulePath::from_str(OWNER),
        Parser::new("").parse_module().unwrap(),
        verum_ast::FileId::new(0),
        Text::new(),
    ));
    let metadata = Arc::new(metadata.clone());
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry);
    diagnostics(&mut checker, source)
}

#[test]
fn value_implementation_telescope_survives_publication_and_both_consumers() {
    let module = source_module(GENERIC);
    let original = module
        .types
        .iter()
        .flat_map(|ty| &ty.protocols)
        .find_map(|implementation| implementation.source_implementation.clone())
        .expect("ordinary generic implementation retains its declaration");
    let metadata = publish(&module).expect("actual generic implementation archive");
    assert!(
        metadata
            .implementations
            .iter()
            .any(|implementation| implementation.source_implementation.as_ref() == Some(&original))
    );
    for eager in [false, true] {
        let good = consumer_errors(
            &metadata,
            "mount core.impl_telescope.{Cell,Evidence}; fn require<T:Evidence>(x:T)->T{x} fn probe(x:Cell<7>)->Cell<7>{require(x)}",
            eager,
        );
        assert!(good.is_empty(), "eager={eager}: {good:?}");
        let bad = consumer_errors(
            &metadata,
            "mount core.impl_telescope.{Cell,Evidence}; fn require<T:Evidence>(x:T)->T{x} fn probe(x:Cell<-1>)->Cell<-1>{require(x)}",
            eager,
        );
        assert!(
            bad.iter().any(|error| error.contains("Evidence")
                && (error.contains("bound")
                    || error.contains("Bound")
                    || error.contains("implement"))),
            "actual domain refusal, eager={eager}: {bad:?}"
        );
        let method = consumer_errors(
            &metadata,
            "mount core.impl_telescope.Cell; fn probe(x:Cell<7>)->Int{x.index()}",
            eager,
        );
        assert!(
            method.is_empty(),
            "owned method consumer, eager={eager}: {method:?}"
        );
        let wrong_method = consumer_errors(
            &metadata,
            "mount core.impl_telescope.Cell; fn probe(x:Cell<7>)->Bool{x.index()}",
            eager,
        );
        assert!(
            wrong_method
                .iter()
                .any(|error| error.contains("Int") && error.contains("Bool")),
            "actual method result mismatch, eager={eager}: {wrong_method:?}"
        );
        let associated = consumer_errors(
            &metadata,
            "mount core.impl_telescope.Cell; type Selected is Cell<7>; fn probe(x:Selected.Carrier)->Cell<7>{x}",
            eager,
        );
        assert!(
            associated.is_empty(),
            "exact associated owner, eager={eager}: {associated:?}"
        );
    }
}

#[test]
fn erased_implementation_telescope_is_rejected_by_archive_and_sidecar_admission() {
    let mut module = source_module(GENERIC);
    let mut metadata = publish(&module).expect("positive archive control");
    for implementation in module.types.iter_mut().flat_map(|ty| &mut ty.protocols) {
        implementation.source_implementation = None;
    }
    let error = publish(&module).expect_err("archive may not guess a lost telescope");
    assert!(
        error.contains("declaration-owned source telescope"),
        "{error}"
    );
    for implementation in &mut metadata.implementations {
        implementation.source_implementation = None;
    }
    let error = verum_types::core_metadata::encode_metadata(&metadata)
        .expect_err("sidecar may not certify erased implementation authority");
    assert!(
        error.contains("declaration-owned source telescope"),
        "{error}"
    );
}

#[test]
fn concrete_source_implementation_also_retains_its_declared_header() {
    let mut module = source_module(
        r#"
        module core.impl_telescope;
        public type Evidence is protocol {};
        public type Owner is { value: Int };
        implement Evidence for Owner {}
    "#,
    );
    publish(&module).expect("actual concrete implementation archive");
    let mut count = 0;
    for implementation in module.types.iter_mut().flat_map(|ty| &mut ty.protocols) {
        if implementation.source_implementation.take().is_some() {
            count += 1;
        }
    }
    assert_eq!(count, 1);
    let error = publish(&module)
        .expect_err("concrete runtime owner is not a replacement for its declaration");
    assert!(
        error.contains("declaration-owned source telescope"),
        "{error}"
    );
}

#[test]
fn declaration_carrier_preserves_constants_and_proof_obligations_without_certifying_them() {
    let module = Parser::new(
        r#"implement<T> Evidence for Owner<T> {
        const count: Int = 7;
        proof law by assumption;
        fn observe(&self)->Int { 7 }
    }"#,
    )
    .parse_module()
    .expect("actual implementation grammar");
    let verum_ast::ItemKind::Impl(original) = &module.items[0].kind else {
        panic!("implementation")
    };
    let carrier = verum_ast::source_implementation::AssociatedImplementationSource::new(
        OWNER.into(),
        original,
    )
    .expect("bounded retained declarations");
    let restored = carrier.declaration().unwrap();
    assert_eq!(restored.generics, original.generics);
    assert_eq!(restored.kind, original.kind);
    assert_eq!(restored.items[0], original.items[0]);
    assert_eq!(restored.items[1], original.items[1]);
    let verum_ast::decl::ImplItemKind::Function(function) = &restored.items[2].kind else {
        panic!("method")
    };
    assert!(
        function.body.is_none(),
        "runtime bodies are not part of declaration authority"
    );
}

#[test]
fn borrowed_receiver_keeps_its_shape_under_the_nominal_attachment_owner() {
    let module = source_module(
        r#"module core.impl_telescope;
        public type Evidence is protocol { type Carrier; };
        public type Owner<T> is { value: T };
        implement<T> Evidence for &Owner<T> { type Carrier = T; }
    "#,
    );
    let metadata =
        publish(&module).expect("borrowed target remains attached to its real nominal owner");
    for eager in [false, true] {
        let good = consumer_errors(
            &metadata,
            "mount core.impl_telescope.{Owner,Evidence}; fn require<T:Evidence>(x:T)->T{x} fn probe(x:&Owner<Int>)->&Owner<Int>{require(x)}",
            eager,
        );
        assert!(good.is_empty(), "borrowed receiver eager={eager}: {good:?}");
        let bad = consumer_errors(
            &metadata,
            "mount core.impl_telescope.{Owner,Evidence}; fn require<T:Evidence>(x:T)->T{x} fn probe(x:Owner<Int>)->Owner<Int>{require(x)}",
            eager,
        );
        assert!(
            bad.iter().any(|error| error.contains("Evidence")
                && (error.contains("bound")
                    || error.contains("Bound")
                    || error.contains("implement"))),
            "the attachment owner alone does not implement Evidence, eager={eager}: {bad:?}"
        );
    }
}
