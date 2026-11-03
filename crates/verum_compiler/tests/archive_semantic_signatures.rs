//! Release signatures preserve declaration owners through renamed imports.
use verum_common::{List, Shared, Text};
use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, core_metadata::CoreMetadata};
use verum_vbc::{
    archive::ArchiveBuilder,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
};

#[test]
fn nominal_universe_declarations_survive_archive_and_sidecar() {
    for declaration_only in [false, true] {
        let mut archive = ArchiveBuilder::new();
        for (owner, level) in [("core.alpha", 1), ("core.beta", 2)] {
            let ast = Parser::new(&format!("module {owner}; public type Token<A> is {{ public value: Type({level}), public extra: A }};"))
                .parse_module().unwrap();
            let mut producer = VbcCodegen::with_config(CodegenConfig::new(owner));
            let module = if declaration_only {
                producer.collect_declaration_interface(&[&ast]).unwrap()
            } else { producer.compile_module(&ast).unwrap() };
            archive.add_module(owner, &module, &[]).unwrap();
        }
        let mut bytes = List::new();
        verum_vbc::archive::write_archive(&archive.finish(), &mut bytes).unwrap();
        let archive = verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let metadata = archive_to_core_metadata(&archive).unwrap();
        let bytes = verum_types::core_metadata::encode_metadata(&metadata).unwrap();
        let metadata = verum_types::core_metadata::decode_metadata(&bytes).unwrap();
        for eager in [false, true] {
            let result = errors(&metadata, "mount core.alpha.{Token as A}; mount core.beta.{Token as B}; fn a(x: A<Int>) -> Type(1) { x.value } fn b(x: B<Bool>) -> Type(2) { x.value } fn extra(x: A<Int>) -> Int { x.extra }", eager);
            assert!(result.is_empty(), "interface={declaration_only}, eager={eager}: {result:?}");
            let result = errors(&metadata, "mount core.beta.{Token}; fn bad(x: Token<Int>) -> Type(1) { x.value }", eager);
            assert!(!result.is_empty(), "archive permitted a downward universe cast");
        }
    }
}

#[test]
fn nominal_private_type_mounts_keep_their_owner_across_archive_and_sidecar() {
    for declaration_only in [false, true] {
        let mut dependencies = List::new();
        for (owner, level) in [("core.alpha", 1), ("core.beta", 2)] {
            let ast = Parser::new(&format!("module {owner}; public type Token is {{ public value: Type({level}) }};"))
                .parse_module().unwrap();
            dependencies.push(VbcCodegen::with_config(CodegenConfig::new(owner)).compile_module(&ast).unwrap());
        }
        let ast = Parser::new("module core.api; mount core.alpha.{Token as Renamed}; public type Carrier<A> is { public level: Type, public payload: Renamed, public extra: A };")
            .parse_module().unwrap();
        let mut producer = VbcCodegen::with_config(CodegenConfig::new("core.api"));
        producer.import_bootstrap_nominal_dependencies(&[&ast], &dependencies.iter().collect::<List<_>>()).unwrap();
        let module = if declaration_only { producer.collect_declaration_interface(&[&ast]).unwrap() }
            else {
                producer.collect_unit_declarations(&[&ast]).unwrap();
                producer.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap();
                producer.finalize_module_from_state().unwrap()
            };
        let mut builder = ArchiveBuilder::new();
        for module in dependencies.iter().chain(std::iter::once(&module)) {
            builder.add_module(&module.name, module, &[]).unwrap();
        }
        let mut bytes = List::new();
        verum_vbc::archive::write_archive(&builder.finish(), &mut bytes).unwrap();
        let archive = verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let metadata = archive_to_core_metadata(&archive).unwrap();
        let bytes = verum_types::core_metadata::encode_metadata(&metadata).unwrap();
        let metadata = verum_types::core_metadata::decode_metadata(&bytes).unwrap();
        assert_eq!(metadata.declaration_type_bindings.get(&Text::from("core.api")).unwrap()
            .get(&Text::from("Renamed")), Some(&Some("core.alpha.Token".into())));
        for eager in [false, true] {
            let result = errors(&metadata, "fn read(x: core.api.Carrier<Int>) -> core.alpha.Token { x.payload }", eager);
            assert!(result.is_empty(), "interface={declaration_only}, eager={eager}: {result:?}");
            let result = errors(&metadata, "fn wrong(x: core.api.Carrier<Int>) -> core.beta.Token { x.payload }", eager);
            assert!(!result.is_empty(), "private mount changed nominal owner");
        }
    }
}

#[test]
fn actual_ntype_construction_and_projection_specialize_the_same_receiver() {
    let mut ast = Parser::new(include_str!("../../../core/math/hott.vr")).parse_module().unwrap();
    ast.items.retain(|item| matches!(&item.kind, verum_ast::ItemKind::Module(_))
        || matches!(&item.kind, verum_ast::ItemKind::Type(decl) if ["IsTrunc", "NType"].contains(&decl.name.name.as_str())));
    let module = VbcCodegen::with_config(CodegenConfig::new("core.math"))
        .collect_declaration_interface(&[&ast]).unwrap();
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    let owner = module.types.iter().find(|descriptor|
        module.strings.get(descriptor.name).is_some_and(|name| name.ends_with("NType"))).unwrap();
    assert!(matches!(&owner.fields[1].declaration_type,
        Some(verum_vbc::TypeRef::Instantiated { args, .. })
        if matches!(args.first(), Some(verum_vbc::TypeRef::RecordFieldProjection { owner: id, ordinal: 0 }) if *id == owner.id)));
    let mut builder = ArchiveBuilder::new();
    builder.add_module("core.math", &module, &[]).unwrap();
    let metadata = archive_to_core_metadata(&builder.finish()).unwrap();
    for eager in [false, true] {
        for (body, accepted) in [
            ("fn carrier(x: &core.math.hott.NType<0>) -> Type { x.carrier }", true),
            ("fn copy(x: &core.math.hott.NType<0>) -> core.math.hott.NType<0> { core.math.hott.NType { carrier: x.carrier, truncation: x.truncation } }", true),
            ("fn mixed(x: &core.math.hott.NType<0>, y: &core.math.hott.NType<0>) -> core.math.hott.NType<0> { core.math.hott.NType { carrier: x.carrier, truncation: y.truncation } }", false),
            ("fn wrong_index(x: &core.math.hott.NType<1>) -> core.math.hott.NType<0> { core.math.hott.NType { carrier: x.carrier, truncation: x.truncation } }", false),
        ] {
            let mut checker = if eager { TypeChecker::new_with_core_eager(std::sync::Arc::new(metadata.clone())) }
                else { TypeChecker::new_with_core(std::sync::Arc::new(metadata.clone())) };
            checker.register_primitives();
            checker.set_current_module_path("core.math.hott");
            let usage = Parser::new(body).parse_module().unwrap();
            let mut failures = checker.prepare_module_declarations(&usage);
            for item in &usage.items { if let Err(error) = checker.check_item(item) { failures.push(error); } }
            failures.extend(checker.take_deferred_errors());
            let mut messages: Vec<_> = failures.iter().map(|e| e.to_string()).collect();
            messages.extend(checker.diagnostics().iter().filter(|d| d.is_error()).map(|d| format!("{d:?}")));
            if accepted {
                assert!(messages.is_empty(), "eager={eager}, {body}: {messages:?}");
            } else {
                assert!(messages.iter().any(|e| e.to_lowercase().contains("mismatch")), "eager={eager}, {body}: {messages:?}");
            }
        }
    }
}

fn metadata(declaration_only: bool, reverse: bool) -> CoreMetadata {
    let mut owners = List::new();
    for owner in ["core.alpha", "core.beta"] {
        let source = format!("module {owner}; public type Token is {{ value: Int }};");
        let ast = Parser::new(&source).parse_module().unwrap();
        owners.push(
            VbcCodegen::with_config(CodegenConfig::new(owner))
                .compile_module(&ast)
                .unwrap(),
        );
    }
    if reverse {
        owners.reverse();
    }
    let ast = Parser::new(
        r#"
        module core.api;
        mount core.alpha.{Token as Instant};
        mount core.beta.{Token as Other};
        public type Envelope<T> is { value: T };
        public fn boxed<T>(value: Envelope<(&Instant, &T)>) -> Envelope<(&Instant, &T)> { value }
        public fn echo(value: &Instant) -> &Instant { value }
        public fn nested<T>(value: (&Instant, &T)) -> (&Instant, &T) { value }
        public fn other(value: &Other) -> &Other { value }
        public fn checked_echo(value: &checked mut Instant) -> &checked mut Instant { value }
        public fn callable<F: fn(Int) -> Int>(value: F) -> F { value }
    "#,
    )
    .parse_module()
    .unwrap();
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("core.api"));
    producer
        .import_bootstrap_nominal_dependencies_with_resolver(
            &[&ast],
            &owners.iter().collect::<List<_>>(),
            |name| Some(name.into()),
        )
        .unwrap();
    let module = if declaration_only {
        producer.collect_declaration_interface(&[&ast]).unwrap()
    } else {
        // The bootstrap compilation path retains imported declarations;
        // compile_module starts a new independent module and resets them.
        producer.collect_unit_declarations(&[&ast]).unwrap();
        producer
            .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
            .unwrap();
        producer.finalize_module_from_state().unwrap()
    };
    let mut archive = ArchiveBuilder::new();
    for module in owners.iter().chain(std::iter::once(&module)) {
        archive.add_module(&module.name, module, &[]).unwrap();
    }
    // Exercise both archive serialization and the release metadata format.
    let mut bytes = List::new();
    verum_vbc::archive::write_archive(&archive.finish(), &mut bytes).unwrap();
    let archive = verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
    let metadata = archive_to_core_metadata(&archive).expect("valid archive metadata");
    bincode::deserialize(&bincode::serialize(&metadata).unwrap()).unwrap()
}

fn errors(metadata: &CoreMetadata, source: &str, eager: bool) -> List<Text> {
    let mut registry = verum_modules::ModuleRegistry::new();
    for (index, owner) in ["core.alpha", "core.beta", "core.api"].iter().enumerate() {
        registry.register(verum_modules::ModuleInfo::new(
            verum_modules::ModuleId::new(index as u32),
            verum_modules::ModulePath::from_str(owner),
            Parser::new("").parse_module().unwrap(),
            verum_ast::FileId::new(index as u32),
            Text::new(),
        ));
    }
    let metadata = Shared::new(metadata.clone()).into_arc();
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry);
    let ast = Parser::new(source).parse_module().unwrap();
    checker.register_stdlib_types_for_module(&ast);
    let mut errors: List<Text> = ast
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|error| format!("{error:?}").into())
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| format!("{e:?}").into()),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| format!("{e:?}").into()),
    );
    errors
}

#[test]
fn source_signatures_keep_owners_reference_tiers_and_generic_slots() {
    for declaration_only in [false, true] {
        for reverse in [false, true] {
            let metadata = metadata(declaration_only, reverse);
            for (name, expected) in [
                ("echo", "&core.alpha.Token"),
                ("other", "&core.beta.Token"),
                ("nested", "(&core.alpha.Token, &__generic_0)"),
                (
                    "boxed",
                    "core.api.Envelope<(&core.alpha.Token, &__generic_0)>",
                ),
                ("checked_echo", "&checked mut core.alpha.Token"),
            ] {
                let function = metadata
                    .functions
                    .get(&Text::from(format!("core.api.{name}")))
                    .unwrap();
                assert_eq!(
                    function.params[0].ty.as_str(),
                    expected,
                    "{name}, interface={declaration_only}"
                );
                assert_eq!(
                    function.return_type.as_str(),
                    expected,
                    "{name}, interface={declaration_only}"
                );
            }
            assert_eq!(
                metadata
                    .functions
                    .get(&Text::from("core.api.callable"))
                    .unwrap()
                    .params[0]
                    .declared_ty
                    .as_str(),
                "F"
            );
        }
    }
}

#[test]
fn release_consumers_accept_only_the_declared_owner_and_callable_bound() {
    for declaration_only in [false, true] {
        for reverse in [false, true] {
            let metadata = metadata(declaration_only, reverse);
            for eager in [false, true] {
                for source in [
                    "fn probe(value: &core.alpha.Token) { let result: &core.alpha.Token = core.api.echo(value); }",
                    "fn probe(value: &core.beta.Token) { let result: &core.beta.Token = core.api.other(value); }",
                    "fn probe(value: (&core.alpha.Token, &Bool)) { let result: (&core.alpha.Token, &Bool) = core.api.nested(value); }",
                    "fn probe(value: core.api.Envelope<(&core.alpha.Token, &Bool)>) { let result: core.api.Envelope<(&core.alpha.Token, &Bool)> = core.api.boxed(value); }",
                    "fn probe(value: fn(Int) -> Int) -> fn(Int) -> Int { core.api.callable(value) }",
                ] {
                    assert!(
                        errors(&metadata, source, eager).is_empty(),
                        "{source}: {:?}",
                        errors(&metadata, source, eager)
                    );
                }
                for source in [
                    "fn probe(value: &core.beta.Token) { let result: &core.alpha.Token = core.api.echo(value); }",
                    "fn probe(value: &core.alpha.Token) { let result: &core.beta.Token = core.api.echo(value); }",
                    "fn probe(value: (&core.beta.Token, &Bool)) { let result: (&core.alpha.Token, &Bool) = core.api.nested(value); }",
                    "fn probe(value: core.api.Envelope<(&core.beta.Token, &Bool)>) { let result: core.api.Envelope<(&core.alpha.Token, &Bool)> = core.api.boxed(value); }",
                    "fn probe(value: fn(Bool) -> Bool) -> fn(Bool) -> Bool { core.api.callable(value) }",
                ] {
                    let errors = errors(&metadata, source, eager);
                    assert!(
                        !errors.is_empty() && errors.iter().all(|e| e.contains("Mismatch")),
                        "{source}: expected type mismatches, got {errors:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn actual_stdlib_datetime_signature_retains_the_imported_declaration() {
    let archive = verum_compiler::embedded_stdlib_vbc::get_runtime_archive()
        .expect("release stdlib archive required");
    let metadata = archive_to_core_metadata(archive).expect("valid archive metadata");
    let function = metadata
        .functions
        .get(&Text::from(
            "core.database.sqlite.native.datetime.iso.render",
        ))
        .expect("public datetime renderer");
    assert_eq!(
        function.params[0].ty.as_str(),
        "&core.database.sqlite.native.datetime.instant.DatetimeInstant"
    );
    assert_eq!(function.return_type.as_str(), "Text");
}

#[test]
#[ignore = "requires the matching freshly baked stdlib embedded in this test binary"]
fn actual_stdlib_nominal_universes_reach_the_compiler_pipeline() {
    use verum_compiler::{CompilationPipeline, CompilerOptions, OutputFormat, Session, VerifyMode};
    let archive = verum_compiler::embedded_stdlib_vbc::get_runtime_archive().expect("fresh stdlib");
    let metadata = archive_to_core_metadata(archive).unwrap();
    let descriptor = metadata.types.get(&Text::from("core.math.hott.NType")).expect("declared NType");
    assert!(descriptor.source_declaration.is_some(), "fresh nominal declaration carrier");
    let source = Parser::new(include_str!("../../../core/math/hott.vr")).parse_module().unwrap();
    let original = source.items.iter().find_map(|item| match &item.kind {
        verum_ast::ItemKind::Type(decl) if decl.name.name.as_str() == "NType" => Some(decl),
        _ => None,
    }).unwrap();
    assert_eq!(descriptor.source_declaration.as_ref(), Some(original), "retain the entire source declaration, including dependent fields");
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("nominal.vr");
    std::fs::write(&input, "mount core.math.hott.{NType}; fn keep(x: &NType<0>) -> &NType<0> { x } fn main() {} ").unwrap();
    let mut session = Session::new(CompilerOptions {
        input, verify_mode: VerifyMode::Runtime, output_format: OutputFormat::Human,
        check_only: true, ..Default::default()
    });
    let result = CompilationPipeline::new(&mut session).run_check_only();
    assert!(result.is_ok(), "{result:?}\n{}", session.format_diagnostics());
}

#[test]
#[ignore = "requires the matching freshly baked stdlib embedded in this test binary"]
fn actual_stdlib_ntype_refuses_index_substitution_and_private_field_access() {
    use verum_compiler::{CompilationPipeline, CompilerOptions, OutputFormat, Session, VerifyMode};
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("dependent.vr");
    for (body, expected) in [
        ("fn read(x: &NType<0>) -> Type { x.carrier }", &["E202", "carrier"][..]),
        ("fn change(x: &NType<0>) -> &NType<1> { x }", &["mismatch"][..]),
    ] {
        std::fs::write(&input, format!("mount core.math.hott.{{NType}}; {body} fn main() {{}}")).unwrap();
        let mut session = Session::new(CompilerOptions {
            input: input.clone(), verify_mode: VerifyMode::Runtime, output_format: OutputFormat::Human,
            check_only: true, ..Default::default()
        });
        let result = CompilationPipeline::new(&mut session).run_check_only();
        assert!(result.is_err(), "accepted invalid stdlib consumer: {body}");
        let diagnostics = session.format_diagnostics();
        assert!(expected.iter().all(|word| diagnostics.contains(word)) && !diagnostics.contains("E101"),
            "wrong refusal for {body}: {result:?}\n{diagnostics}");
    }
}

#[test]
fn archived_protocol_object_safety_uses_declared_receivers_and_binders() {
    let declarations = Parser::new("module core.api;
        public type Reader is protocol { fn read(&self) -> Int; };
        public type Writer is protocol { fn write(&mut self, value: Int); };
        public type Static is protocol { fn read(value: &Int) -> Int; };
        public type Consuming is protocol { fn read(self, value: &Int) -> Int; };
        public type Generic is protocol { fn read<T>(&self) -> Int; };
        public type Factory is protocol { fn make(&self) -> Self; };")
        .parse_module().unwrap();
    for declaration_only in [false, true] {
        let mut producer = VbcCodegen::with_config(CodegenConfig::new("core.api"));
        let module = if declaration_only {
            producer.collect_declaration_interface(&[&declarations]).unwrap()
        } else {
            producer.compile_module(&declarations).unwrap()
        };
        let mut erased = module.clone();
        for ty in &mut erased.types {
            for slot in &mut ty.variants {
                slot.method_dispatch = None;
            }
        }
        let mut incomplete = ArchiveBuilder::new();
        incomplete.add_module("core.api", &erased, &[]).unwrap();
        assert!(archive_to_core_metadata(&incomplete.finish()).unwrap_err()
            .contains("lacks a protocol method declaration"));

        let mut builder = ArchiveBuilder::new();
        builder.add_module("core.api", &module, &[]).unwrap();
        let homonym = Parser::new("module core.beta;
            public type Reader is protocol { fn read(value: &Int) -> Int; };")
            .parse_module().unwrap();
        let other = VbcCodegen::with_config(CodegenConfig::new("core.beta"))
            .compile_module(&homonym).unwrap();
        builder.add_module("core.beta", &other, &[]).unwrap();
        let mut bytes = List::new();
        verum_vbc::archive::write_archive(&builder.finish(), &mut bytes).unwrap();
        let archive = verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let metadata = archive_to_core_metadata(&archive).unwrap();
        let bytes = verum_types::core_metadata::encode_metadata(&metadata).unwrap();
        let metadata = verum_types::core_metadata::decode_metadata(&bytes).unwrap();
        for eager in [false, true] {
            let collision = errors(&metadata, "mount core.api.{Reader as Safe};
                mount core.beta.{Reader as StaticReader};
                fn good(x: &dyn Safe) {} fn bad(x: &dyn StaticReader) {}", eager);
            assert_eq!(collision.len(), 1, "eager={eager}: {collision:?}");
            assert!(collision[0].contains("self parameter"), "{collision:?}");
            for (protocol, expected) in [("Reader", None), ("Writer", None),
                ("Static", Some("self parameter")), ("Consuming", Some("by value")),
                ("Generic", Some("generic")), ("Factory", Some("returns self"))] {
                let source = format!("mount core.api.{{{protocol} as Bound}}; fn accept(x: &dyn Bound) {{}}");
                let result = errors(&metadata, &source, eager);
                let messages = format!("{result:?}").to_lowercase();
                if let Some(expected) = expected {
                    assert!(messages.contains(expected), "{protocol}, eager={eager}: {result:?}");
                } else {
                    assert!(result.is_empty(), "{protocol}, eager={eager}: {result:?}");
                }
            }
        }
    }
}

#[test]
fn nested_dyn_arguments_keep_the_function_declaration_owner() {
    let mut builder = ArchiveBuilder::new();
    let mut memory = Parser::new(include_str!("../../../core/base/memory.vr")).parse_module().unwrap();
    memory.items.retain(|item| matches!(&item.kind, verum_ast::ItemKind::Module(_))
        || matches!(&item.kind, verum_ast::ItemKind::Type(decl) if ["Shared", "SharedInner"].contains(&decl.name.name.as_str())));
    let shared = VbcCodegen::with_config(CodegenConfig::new("core.base.memory"))
        .collect_declaration_interface(&[&memory]).unwrap();
    builder.add_module("core.base.memory", &shared, &[]).unwrap();
    let mut libraries = Vec::new();
    for owner in ["core.alpha", "core.beta"] {
        let source = format!("module {owner};
            public type Store is protocol {{ fn load(&self) -> Int; }};
            public fn consume(store: &Shared<dyn Store>) {{}}");
        let ast = Parser::new(&source).parse_module().unwrap();
        let module = VbcCodegen::with_config(CodegenConfig::new(owner))
            .collect_declaration_interface(&[&ast]).unwrap();
        builder.add_module(owner, &module, &[]).unwrap();
        libraries.push(module);
    }
    let mounted = Parser::new("module core.api; mount core.beta.{Store as Backend};
        public fn read_backend(store: &Shared<dyn Backend>) {}").parse_module().unwrap();
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("core.api"));
    producer.import_bootstrap_nominal_dependencies(&[&mounted], &libraries.iter().collect::<List<_>>()).unwrap();
    let module = producer.collect_declaration_interface(&[&mounted]).unwrap();
    builder.add_module("core.api", &module, &[]).unwrap();
    let metadata = archive_to_core_metadata(&builder.finish()).unwrap();
    let sidecar = verum_types::core_metadata::encode_metadata(&metadata).unwrap();
    let decoded = verum_types::core_metadata::decode_metadata(&sidecar).unwrap();
    for eager in [false, true] {
        let imports = "mount core.api.{read_backend}; mount core.alpha.{Store as A, consume as take_a};
            mount core.beta.{Store as B, consume as take_b};";
        let accepted = errors(&decoded, &format!("{imports}
            fn accepted(a: &Shared<dyn A>, b: &Shared<dyn B>) {{ take_a(a); take_b(b); read_backend(b); }}"), eager);
        assert!(accepted.is_empty(), "eager={eager}: {accepted:?}");
        let rejected = errors(&decoded, &format!("{imports}
            fn rejected(a: &Shared<dyn A>) {{ take_b(a); }}"), eager);
        assert!(rejected.iter().any(|error| error.contains("Mismatch")
            && error.contains("core.alpha.Store") && error.contains("core.beta.Store")),
            "eager={eager}: {rejected:?}");
    }
}

#[test]
fn an_archived_value_receiver_copies_scalars_without_authorizing_writes() {
    let ast = Parser::new(
        r#"
        module core.alpha;
        public type Parcel is { public count: Int };
        implement Int {
            public fn snapshot(self) -> Int { self }
            public fn exclusive(&mut self) -> Int { *self }
        }
        implement Parcel {
            public fn consume(self) -> Int { self.count }
        }
    "#,
    )
    .parse_module()
    .unwrap();
    for declaration_only in [false, true] {
        let mut producer = VbcCodegen::with_config(CodegenConfig::new("core.alpha"));
        let module = if declaration_only {
            producer.collect_declaration_interface(&[&ast]).unwrap()
        } else {
            producer.collect_unit_declarations(&[&ast]).unwrap();
            producer
                .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
                .unwrap();
            producer.finalize_module().unwrap()
        };
        let mut builder = ArchiveBuilder::new();
        builder.add_module(&module.name, &module, &[]).unwrap();
        let mut bytes = List::new();
        verum_vbc::archive::write_archive(&builder.finish(), &mut bytes).unwrap();
        let archive =
            verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let metadata = archive_to_core_metadata(&archive).unwrap();
        let wire = verum_types::core_metadata::encode_metadata(&metadata).unwrap();
        let metadata = verum_types::core_metadata::decode_metadata(&wire).unwrap();
        for (name, spelling) in [("snapshot", "Self"), ("exclusive", "&mut Self")] {
            assert!(
                metadata
                    .functions
                    .values()
                    .any(|f| f.name.as_str().rsplit('.').next() == Some(name)
                        && f.params
                            .first()
                            .is_some_and(|p| p.name == "self" && p.declared_ty == spelling)),
                "missing {name}: {spelling}"
            );
        }
        for eager in [false, true] {
            let test = |source: &str| {
                errors(
                    &metadata,
                    &format!("mount core.alpha.Parcel; {source}"),
                    eager,
                )
            };
            let read = test("fn probe(value: &Parcel) -> Int { value.count.snapshot() }");
            assert!(
                read.is_empty(),
                "interface={declaration_only} eager={eager}: {read:?}"
            );
            for bad in [
                "fn probe(value: &Parcel) -> Int { value.count.exclusive() }",
                "fn probe(value: &Parcel) -> Int { value.consume() }",
            ] {
                let bad = test(bad);
                assert!(
                    bad.iter()
                        .any(|error| error.contains("SharedReferenceWrite")
                            || error.contains("shared reference")),
                    "{bad:?}"
                );
            }
        }
        let mut erased = metadata;
        for function in erased
            .functions
            .values_mut()
            .filter(|f| f.name.as_str().rsplit('.').next() == Some("snapshot"))
        {
            function.params[0].declared_ty = Text::new();
        }
        let denied = errors(
            &erased,
            "mount core.alpha.Parcel; fn probe(value: &Parcel) -> Int { value.count.snapshot() }",
            false,
        );
        assert!(
            denied
                .iter()
                .any(|error| error.contains("SharedReferenceWrite")
                    || error.contains("shared reference")),
            "erased receiver: {denied:?}"
        );
    }
}

#[test]
fn actual_integer_formatting_copies_a_shared_record_field() {
    let primitives = Parser::new(include_str!("../../../core/base/primitives.vr"))
        .parse_module()
        .unwrap();
    let parcel = Parser::new(
        "module core.alpha; public type Stamp is { public published_at: Int };",
    )
    .parse_module()
    .unwrap();
    let mut builder = ArchiveBuilder::new();
    for (owner, ast) in [("core.base", &primitives), ("core.alpha", &parcel)] {
        let module = VbcCodegen::with_config(CodegenConfig::new(owner))
            .collect_declaration_interface(&[ast])
            .unwrap();
        builder.add_module(owner, &module, &[]).unwrap();
    }
    let metadata = archive_to_core_metadata(&builder.finish()).unwrap();
    for eager in [false, true] {
        let result = errors(
            &metadata,
            "mount core.alpha.Stamp; fn format_stamp(record: &Stamp) -> Text { record.published_at.to_text() }",
            eager,
        );
        assert!(result.is_empty(), "eager={eager}: {result:?}");
    }
}

fn namespace_archive_declarations() -> verum_vbc::VbcModule {
    let namespaces = Parser::new(
        "module core.parent; public module core; public module api; public module decoy;",
    )
    .parse_module()
    .unwrap();
    let payload =
        Parser::new("module core.parent.core; public type Envelope<T> is { public value: T };")
            .parse_module()
            .unwrap();
    let decoy =
        Parser::new("module core.parent.decoy; public type Envelope<T> is { public value: T };")
            .parse_module()
            .unwrap();
    let api = Parser::new("module core.parent.api; mount super.core as selected; public type Carrier<T> is { public cell: selected.Envelope<T>, public witness: Type };")
        .parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("core.parent"))
        .collect_declaration_interface(&[&namespaces, &payload, &decoy, &api])
        .unwrap();
    assert!(module.bytecode.is_empty());
    verum_vbc::deserialize::deserialize_module(&verum_vbc::serialize::serialize_module(&module).unwrap()).unwrap()
}


#[test]
fn archive_and_sidecar_replay_the_generic_record_in_its_selected_namespace() {
    // Only decoded archive data reaches the metadata loader and fresh checkers.
    let module = namespace_archive_declarations();
    let mut builder = ArchiveBuilder::new();
    builder.add_module("core.parent", &module, &[]).unwrap();
    let mut bytes = List::new();
    verum_vbc::archive::write_archive(&builder.finish(), &mut bytes).unwrap();
    let archive = verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
    let metadata = archive_to_core_metadata(&archive).unwrap();
    let metadata = verum_types::core_metadata::decode_metadata(
        &verum_types::core_metadata::encode_metadata(&metadata).unwrap()).unwrap();
    assert_eq!(
        metadata
            .declaration_type_bindings
            .get(&Text::from("core.parent.api"))
            .unwrap()
            .get(&Text::from("selected"))
            .unwrap()
            .as_deref(),
        Some("core.parent.core")
    );
    let source = "fn inspect<T>(x: core.parent.api.Carrier<T>) -> core.parent.core.Envelope<T> { x.cell } fn value(x: core.parent.api.Carrier<Int>) -> Int { x.cell.value }";
    for eager in [false, true] {
        let good = errors(&metadata, source, eager);
        assert!(good.is_empty(), "eager={eager}: {good:?}");
        let wrong = errors(
            &metadata,
            "fn wrong(x: core.parent.api.Carrier<Int>) -> core.parent.decoy.Envelope<Int> { x.cell }",
            eager,
        );
        assert!(
            wrong
                .iter()
                .any(|message| message.to_lowercase().contains("mismatch")),
            "eager={eager}: {wrong:?}"
        );
        let mut missing = metadata.clone();
        missing
            .declaration_type_bindings
            .get_mut(&Text::from("core.parent.api"))
            .unwrap()
            .remove(&Text::from("selected"));
        let refused = errors(&missing, source, eager);
        assert!(
            refused.iter().any(|message| message.contains("selected")),
            "missing binding, eager={eager}: {refused:?}"
        );
    }
}

#[test]
fn fresh_vbc_consumer_executes_the_same_imported_generic_record() {
    let archive = namespace_archive_declarations();
    let consumer = Parser::new(
        r#"
        module core.parent.consumer;
        mount core.parent.api.Carrier;
        mount core.parent.core.Envelope as Expected;
        type Envelope<T> is { wrong: T };
        fn inspect<T>(value: &Carrier<T>) -> T { value.cell.value }
        fn main() -> Int {
            let carrier: Carrier<Int> = Carrier { cell: Expected { value: 41 }, witness: Int };
            inspect<Int>(&carrier)
        }
    "#,
    )
    .parse_module()
    .unwrap();
    let mut importer = VbcCodegen::with_config(CodegenConfig::new("core.parent.consumer"));
    importer.import_archive_module_types(&archive);
    importer.collect_unit_declarations(&[&consumer]).unwrap();
    importer.resolve_pending_imports();
    importer
        .compile_default_methods_with_policy(ItemFailurePolicy::Strict)
        .unwrap();
    importer
        .compile_unit_items(&[&consumer], ItemFailurePolicy::Strict)
        .unwrap();
    let module = importer.finalize_module_from_state().unwrap();
    let replay = verum_vbc::deserialize::deserialize_module(&verum_vbc::serialize::serialize_module(&module).unwrap()).unwrap();
    for module in [module, replay] {
        let entry = module
            .find_function_by_name("core.parent.consumer.main")
            .unwrap();
        assert_eq!(
            verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
                .execute_function(entry)
                .unwrap()
                .try_as_i64(),
            Some(41)
        );
    }
}
