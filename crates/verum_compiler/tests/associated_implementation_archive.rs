//! Associated families retain their lexical telescope through actual archive publication.
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

const OWNER: &str = "core.family_archive";
const SIMPLE: &str = r#"
module core.family_archive;
public type Rel is protocol { type Out<Declared>; fn tag(&self)->Int; };
public type Pair<A, B> is { first: A, second: B };
public type Owner is { value: Bool };
implement Rel for Owner { type Out<Actual> = Pair<Bool, Actual>; fn tag(&self)->Int { 7 } }
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
    checker.set_generic_associated_types_enabled(true);
    checker.register_primitives();
    checker.set_current_module_path(OWNER);
    let errors = diagnostics(&mut checker, source);
    assert!(
        errors.is_empty(),
        "source producer must typecheck: {errors:?}"
    );
    unchecked_module(source)
}

fn unchecked_module(source: &str) -> VbcModule {
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
    consumer_with_family_policy(metadata, source, eager, true)
}

fn consumer_with_family_policy(
    metadata: &CoreMetadata,
    source: &str,
    eager: bool,
    enabled: bool,
) -> Vec<String> {
    let mut checker = archive_consumer(metadata, eager, enabled, Some(source));
    diagnostics(&mut checker, source)
}

fn archive_consumer(
    metadata: &CoreMetadata,
    eager: bool,
    enable_families: bool,
    registered_source: Option<&str>,
) -> TypeChecker {
    let mut registry = verum_modules::ModuleRegistry::new();
    registry.register(verum_modules::ModuleInfo::new(
        verum_modules::ModuleId::new(0),
        verum_modules::ModulePath::from_str(OWNER),
        Parser::new("").parse_module().unwrap(),
        verum_ast::FileId::new(0),
        Text::new(),
    ));
    if let Some(source) = registered_source {
        registry.register(verum_modules::ModuleInfo::new(
            verum_modules::ModuleId::new(1),
            verum_modules::ModulePath::from_str("consumer"),
            Parser::new(source).parse_module().unwrap(),
            verum_ast::FileId::new(1),
            source.into(),
        ));
    }
    let metadata = Arc::new(metadata.clone());
    let mut checker = TypeChecker::with_minimal_context();
    checker.set_module_registry_direct(registry);
    checker.set_current_module_path("consumer");
    checker.set_generic_associated_types_enabled(enable_families);
    if eager {
        checker.load_core_eager(metadata);
    } else {
        checker.register_primitives();
        checker.set_core_metadata(metadata);
    }
    checker
}

#[test]
fn renamed_family_locals_and_fixed_rhs_survive_wire_archive_and_sidecar() {
    let module = source_module(SIMPLE);
    let original = module
        .types
        .iter()
        .flat_map(|t| &t.protocols)
        .find_map(|p| p.source_implementation.clone())
        .expect("actual implementation telescope");
    assert_eq!(original.origin_module(), OWNER);
    let metadata = publish(&module).expect("publish complete source evidence");
    assert!(
        metadata
            .implementations
            .iter()
            .any(|i| i.source_implementation.as_ref() == Some(&original)),
        "the exact declaration-owned carrier must survive all archive boundaries"
    );
    for eager in [false, true] {
        let good = consumer_errors(
            &metadata,
            "mount core.family_archive.{Owner,Pair}; fn probe(value:Owner.Out<Int>)->Pair<Bool,Int>{value}",
            eager,
        );
        assert!(good.is_empty(), "eager={eager}: {good:?}");
        let bad = consumer_errors(
            &metadata,
            "mount core.family_archive.{Owner,Pair}; fn probe(value:Owner.Out<Int>)->Pair<Int,Bool>{value}",
            eager,
        );
        assert!(
            bad.iter().any(|e| e.to_lowercase().contains("mismatch")),
            "the fixed Bool argument cannot be replaced positionally; eager={eager}: {bad:?}"
        );
    }
}

#[test]
fn receiver_substitution_and_family_locals_remain_separate_after_publication() {
    let module = source_module(
        r#"
module core.family_archive;
public type Rel is protocol { type Out<X>; };
public type Owner<A,B> is { left:A, right:B };
implement<A,B> Rel for Owner<B,A> { type Out<Local> = (A,B,Local); }
"#,
    );
    let metadata = publish(&module).unwrap();
    for eager in [false, true] {
        let good = consumer_errors(
            &metadata,
            "mount core.family_archive.Owner; type Concrete is Owner<Bool,Int>; fn probe(value:Concrete.Out<Text>)->(Int,Bool,Text){value}",
            eager,
        );
        assert!(good.is_empty(), "eager={eager}: {good:?}");
        let bad = consumer_errors(
            &metadata,
            "mount core.family_archive.Owner; type Concrete is Owner<Bool,Int>; fn probe(value:Concrete.Out<Text>)->(Bool,Int,Text){value}",
            eager,
        );
        assert!(
            bad.iter().any(|e| e.to_lowercase().contains("mismatch")),
            "eager={eager}: {bad:?}"
        );
    }
}

#[test]
fn erased_family_telescope_cannot_publish_an_owner_scoped_substitute() {
    let mut module = source_module(SIMPLE);
    assert!(
        publish(&module).is_ok(),
        "unmodified evidence is the positive control"
    );
    let mut removed = 0;
    for implementation in module.types.iter_mut().flat_map(|t| &mut t.protocols) {
        if implementation.source_implementation.take().is_some() {
            removed += 1;
        }
    }
    assert_eq!(removed, 1);
    let error = publish(&module)
        .expect_err("erased source telescope must not be guessed from runtime slots");
    assert!(
        error.contains(
            "associated family implementation lacks its declaration-owned source telescope"
        ),
        "{error}"
    );
}

#[test]
fn foreign_receiver_carrier_is_not_authority_for_the_enclosing_owner() {
    let source = format!(
        "{SIMPLE} public type Decoy is {{ value:Int }}; implement Rel for Decoy {{ type Out<Local> = Pair<Int, Local>; fn tag(&self)->Int {{ 9 }} }}"
    );
    let mut module = source_module(&source);
    assert!(
        publish(&module).is_ok(),
        "both real implementations are valid before substitution"
    );
    let decoy = module
        .types
        .iter()
        .find(|t| module.get_string(t.name) == Some("Decoy"))
        .expect("source Decoy descriptor")
        .protocols
        .iter()
        .find_map(|p| p.source_implementation.clone())
        .expect("real Decoy source carrier");
    let owner_id = module
        .types
        .iter()
        .find(|t| module.get_string(t.name) == Some("Owner"))
        .expect("source Owner descriptor")
        .id;
    let implementation = module
        .types
        .iter_mut()
        .find(|t| t.id == owner_id)
        .unwrap()
        .protocols
        .iter_mut()
        .find(|p| p.source_implementation.is_some())
        .unwrap();
    implementation.source_implementation = Some(decoy);
    let error = publish(&module).expect_err("a foreign implementation cannot certify this owner");
    assert!(
        error.to_lowercase().contains("implementation")
            && (error.to_lowercase().contains("receiver")
                || error.to_lowercase().contains("target")
                || error.to_lowercase().contains("owner")),
        "{error}"
    );
}

#[test]
fn family_local_shadowing_does_not_capture_an_archived_owner_parameter() {
    let module = source_module(
        r#"
module core.family_archive;
public type Rel is protocol { type Out<X>; };
public type Pair<A,B> is { left:A, right:B };
public type Owner<A> is { value:A };
implement<A> Rel for Owner<A> { type Out<A> = Pair<Bool,A>; }
"#,
    );
    let metadata = publish(&module).unwrap();
    for eager in [false, true] {
        let good = consumer_errors(
            &metadata,
            "mount core.family_archive.{Owner,Pair}; type Concrete is Owner<Text>; fn probe(value:Concrete.Out<Int>)->Pair<Bool,Int>{value}",
            eager,
        );
        assert!(good.is_empty(), "eager={eager}: {good:?}");
        let bad = consumer_errors(
            &metadata,
            "mount core.family_archive.{Owner,Pair}; type Concrete is Owner<Text>; fn probe(value:Concrete.Out<Int>)->Pair<Bool,Text>{value}",
            eager,
        );
        assert!(
            bad.iter().any(|e| e.to_lowercase().contains("mismatch")),
            "eager={eager}: {bad:?}"
        );
    }
}

const SPECIALIZED: &str = r#"
module core.family_archive;
public type ReadValue is protocol { type Value; fn read_value(&self)->Self.Value; };
public type ReadMore is protocol extends ReadValue {};
public type Tray<A> is { value:A };
implement ReadValue for Tray<Int> { type Value = Int; fn read_value(&self)->Int { self.value } }
implement ReadValue for Tray<Bool> { type Value = Bool; fn read_value(&self)->Bool { self.value } }
"#;

#[test]
fn specialized_members_and_superprotocols_survive_the_sidecar() {
    let metadata = publish(&source_module(SPECIALIZED)).unwrap();
    let child = metadata.protocols.get(&Text::from("ReadMore")).unwrap();
    assert_eq!(child.super_protocols.as_slice(), &[Text::from("ReadValue")]);
    assert!(
        metadata
            .implementations
            .iter()
            .all(|implementation| implementation.target_type == "Tray"
                && implementation.source_implementation.is_some())
    );
    let tray = metadata
        .types
        .get(&Text::from("core.family_archive.Tray"))
        .unwrap();
    assert_eq!(tray.methods.as_slice(), &[Text::from("read_value")]);
    for eager in [false, true] {
        for (receiver, result) in [
            ("Bool", "Bool"),
            ("Int", "Int"),
            ("Bool", "Int"),
            ("Int", "Bool"),
        ] {
            let consumer = format!(
                "mount core.family_archive.{{Tray,ReadValue}}; fn consume(value:Tray<{receiver}>)->{result} {{ value.read_value() }}"
            );
            let errors = consumer_errors(&metadata, &consumer, eager);
            assert_eq!(
                errors.is_empty(),
                receiver == result,
                "eager={eager}: {consumer}: {errors:?}"
            );
        }
    }
}

#[test]
fn a_sibling_member_with_a_forged_signature_cannot_publish() {
    let mut module = source_module(SPECIALIZED);
    let method = module
        .functions
        .iter_mut()
        .find(|function| {
            function.source_signature.as_ref().is_some_and(|signature| {
                signature.name.name == "read_value"
                    && signature
                        .return_type
                        .as_ref()
                        .is_some_and(|ty| verum_ast::pretty::format_type(ty) == "Bool")
            })
        })
        .unwrap();
    let signature = method.source_signature.as_mut().unwrap();
    signature.return_type = None;
    let error = publish(&module).unwrap_err();
    assert!(
        error.contains("implementation header") || error.contains("implementation signature"),
        "{error}"
    );
}

#[test]
fn a_sibling_body_cannot_certify_another_implementation() {
    let mut module = source_module(SPECIALIZED);
    let int_method = module
        .functions
        .iter()
        .find(|function| {
            function.source_signature.as_ref().is_some_and(|signature| {
                signature.name.name == "read_value"
                    && signature
                        .return_type
                        .as_ref()
                        .is_some_and(|ty| verum_ast::pretty::format_type(ty) == "Int")
            })
        })
        .unwrap()
        .id
        .0;
    let other = module
        .types
        .iter_mut()
        .flat_map(|ty| &mut ty.protocols)
        .find(|implementation| {
            implementation.methods.len() == 1 && implementation.methods[0] != int_method
        })
        .expect("concrete Bool implementation has a materialized method");
    other.methods[0] = int_method;
    let error = publish(&module).unwrap_err();
    assert!(error.contains("implementation signature"), "{error}");
}

#[test]
fn removing_a_concrete_telescope_does_not_create_an_inheritance_edge() {
    let mut module = source_module(SPECIALIZED);
    let target = module
        .types
        .iter_mut()
        .find(|ty| ty.kind == verum_vbc::types::TypeKind::Record)
        .unwrap();
    target.protocols[0].source_implementation = None;
    let error = publish(&module).unwrap_err();
    assert!(
        error.contains("declaration-owned source telescope"),
        "{error}"
    );
}

#[test]
fn structural_methods_retain_their_own_catalogue_and_receiver_shape() {
    for (name, target, other) in [
        ("Slice", "[Int]", "[Int; 4]"),
        ("Array", "[Int; 4]", "[Int]"),
    ] {
        let source = format!(
            "module {OWNER}; implement {target} {{ public fn archived_marker(&self)->Bool {{ true }} }}"
        );
        let module = source_module(&source);
        let metadata = publish(&module).unwrap();
        let descriptor = metadata
            .functions
            .get(&Text::from(format!("{name}.archived_marker")))
            .unwrap();
        assert_eq!(descriptor.parent_type.as_deref(), Some(name));
        assert!(
            metadata
                .inherent_implementations
                .contains_key(&Text::from(name))
        );
        if name == "Array" {
            let mut forged = metadata.clone();
            forged
                .functions
                .get_mut(&Text::from("Array.archived_marker"))
                .unwrap()
                .params[0]
                .ty = "&[Int; 5]".into();
            let error = verum_types::core_metadata::encode_metadata(&forged).unwrap_err();
            assert!(
                error.contains("source array extent 4 disagrees with archive extent 5"),
                "{error}"
            );
        }

        let mut wrong = module.clone();
        let other_source = Parser::new(&format!(
            "implement {other} {{ public fn archived_marker(&self)->Bool {{ true }} }}"
        ))
        .parse_module()
        .unwrap();
        let verum_ast::ItemKind::Impl(other_block) = &other_source.items[0].kind else {
            panic!("implementation")
        };
        let owner = wrong
            .types
            .iter_mut()
            .find(|ty| !ty.source_inherent_implementations.is_empty())
            .unwrap();
        let mut block = owner.source_inherent_implementations[0]
            .declaration()
            .unwrap();
        block.kind = other_block.kind.clone();
        owner.source_inherent_implementations[0] =
            verum_ast::source_implementation::AssociatedImplementationSource::new(
                OWNER.into(),
                &block,
            )
            .unwrap();
        assert!(publish(&wrong).is_err(), "{other} was admitted on {name}");
    }
}

#[test]
fn an_inherited_default_retains_the_declaring_protocol_signature() {
    let source = r#"
        module core.family_archive;
        public type Advertise is protocol { fn message(&self)->Int { 9 } };
        public type Detailed is protocol extends Advertise {};
        public type Entry is { id:Int };
        implement Advertise for Entry {}
        implement Detailed for Entry {}
    "#;
    let metadata = publish(&source_module(source)).unwrap();
    for eager in [false, true] {
        let source = "mount core.family_archive.{Entry,Advertise}; fn read(entry:Entry)->Int { entry.message() }";
        assert!(
            consumer_errors(&metadata, source, eager).is_empty(),
            "eager={eager}"
        );
    }
}

#[test]
fn an_explicit_override_and_a_default_select_their_own_receivers() {
    let explicit = "implement Measure for Tray<Bool> { fn measure(&self)->Int { 3 } }";
    let inherited = "implement Measure for Tray<Int> {}";
    for (first, second) in [(explicit, inherited), (inherited, explicit)] {
        let source = format!(r#"
            module {OWNER};
            public type Tray<T> is {{ value:T }};
            public type Measure is protocol {{ fn measure(&self)->Int {{ 7 }} }};
            {first}
            {second}
            public fn probe()->Int {{
                let flag:Tray<Bool> = Tray {{ value:true }};
                let number:Tray<Int> = Tray {{ value:92 }};
                flag.measure() * 10 + number.measure()
            }}
        "#);
        let module = source_module(&source);
        let metadata = publish(&module).unwrap();
        let mut module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        assert!(module.resolve_external_bands().is_empty());
        verum_vbc::validate::validate_module(&module).unwrap();
        let entry = module.find_function_by_name(&format!("{OWNER}.probe")).unwrap();
        let result = verum_vbc::interpreter::Interpreter::new(Arc::new(module)).execute_function(entry).unwrap();
        assert_eq!(result.as_i64(), 37, "{first}");
        for eager in [false, true] {
            for receiver in ["Int", "Bool", "Text"] {
                let consumer = format!("mount {OWNER}.{{Tray,Measure}}; fn member(v:Tray<{receiver}>)->Int {{v.measure()}}");
                let errors = consumer_errors(&metadata, &consumer, eager);
                assert_eq!(errors.is_empty(), receiver != "Text", "{first} {receiver} eager={eager}: {errors:?}");
            }
        }
    }
}

#[test]
fn value_indexed_family_wire_preserves_domain_and_consumer_policy() {
    let source = r#"
        module core.family_archive;
        public type Indexed<N: meta Int> is { value: Int };
        public type Family is protocol { type Cell<N: meta Int{>= 0}>; };
        public type Owner is {};
        implement Family for Owner { type Cell<M: meta Int{>= 0}> = Indexed<M>; }
    "#;
    let module = source_module(source);
    let metadata = publish(&module).expect("publish declared value family through the archive wire");
    let use_site = "mount core.family_archive.{Owner,Indexed,Family}; fn same(value:Owner.Cell<3>) -> Indexed<3> {value}";
    for eager in [false, true] {
        let admitted = consumer_errors(&metadata, use_site, eager);
        assert!(admitted.is_empty(), "eager={eager}: {admitted:?}");
        let disabled = consumer_with_family_policy(&metadata, use_site, eager, false);
        assert!(
            disabled.iter().any(|e| e.contains("generic_associated_types = false")),
            "eager={eager}: {disabled:?}"
        );
        let mismatch = consumer_errors(&metadata, &use_site.replace("Indexed<3>", "Indexed<4>"), eager);
        assert!(
            mismatch.iter().any(|e| e.contains("exact value index")),
            "eager={eager}: {mismatch:?}"
        );
    }

    // Shape-correct VBC from an unchecked frontend still has to prove the
    // declared family contract when publishing its implementation catalogue.
    let narrowed = source.replace("Cell<M: meta Int{>= 0}>", "Cell<M: meta Int{> 0}>");
    let failure = publish(&unchecked_module(&narrowed))
        .expect_err("the producer cannot narrow the protocol's admitted value domain");
    assert!(failure.contains("domain"), "{failure}");
}

#[test]
fn archive_specializations_preserve_the_protocols_universal_associated_result() {
    let source = r#"
        module core.family_archive;
        public type Stream is protocol { type Item; };
        public type Connection is protocol { type Rows<R>: Stream<Item = R>; };
        public type Cursor<T> is { value: T };
        implement<T> Stream for Cursor<T> { type Item = T; }
        public type Db is {};
        implement Connection for Db { type Rows<R> = Cursor<R>; }
        @specialize implement Stream for Cursor<Int> { type Item = Int; }
    "#;
    publish(&source_module(source))
        .expect("matching specialization preserves every Rows<R> result");
    let invalid = source.replace("type Item = Int;", "type Item = Bool;");
    let error = publish(&unchecked_module(&invalid))
        .expect_err("a concrete specialization cannot change a promised item");
    assert!(
        error.contains("Item") && (error.contains("potential") || error.contains("implementation")),
        "{error}"
    );
}

#[test]
fn current_declaration_mounts_select_the_archived_receiver() {
    let metadata = publish(&source_module(SIMPLE)).unwrap();
    let edited = "mount core.family_archive.Owner as Chosen; type Concrete is Chosen; fn probe(value:Concrete.Out<Int>)->core.family_archive.Pair<Bool,Int>{value}";
    for eager in [false, true] {
        for stored in [None, Some("mount core.family_archive.Pair as Chosen;")] {
            let mut checker = archive_consumer(&metadata, eager, true, stored);
            let errors = diagnostics(&mut checker, edited);
            assert!(errors.is_empty(), "eager={eager}, stored={stored:?}: {errors:?}");
        }
    }
}

#[test]
fn a_removed_mount_cannot_supply_an_archived_receiver_to_an_edited_declaration() {
    let metadata = publish(&source_module(SIMPLE)).unwrap();
    let stale = "mount core.family_archive.Owner as Chosen;";
    let edited = "type Concrete is {value:Chosen};";
    for eager in [false, true] {
        let mut checker = archive_consumer(&metadata, eager, true, Some(stale));
        let errors = diagnostics(&mut checker, edited);
        assert!(
            errors.iter().any(|error| error.contains("type not found: Chosen")),
            "eager={eager}: {errors:?}"
        );
    }
}

#[test]
fn object_result_equality_survives_the_published_archive() {
    let provider = "module core.family_archive;
        public type Source is protocol { type Item; fn read(&self) -> Self.Item; };";
    let metadata = publish(&source_module(provider)).expect("publish protocol with projected result");
    for eager in [false, true] {
        for (result, valid) in [("Int", true), ("Bool", false)] {
            let source = format!("mount core.family_archive.Source;
                fn read(source: &dyn Source<Item = Int>) -> {result} {{ source.read() }}");
            let errors = consumer_errors(&metadata, &source, eager);
            assert_eq!(errors.is_empty(), valid, "eager={eager}, {source}: {errors:?}");
        }
    }
}

#[test]
fn static_and_dynamic_associated_results_execute_with_the_same_carrier() {
    let program = r#"type Reading is protocol {
    type Item;
    fn read(&self) -> Self.Item;
};

type Counter is { value: Int };

implement Reading for Counter {
    type Item = Int;
    fn read(&self) -> Int { self.value }
}

fn read_static<R: Reading<Item = Int>>(source: &R) -> Int {
    source.read()
}

fn read_dynamic(source: &dyn Reading<Item = Int>) -> Int {
    source.read()
}

fn main() -> Int {
    let sensor = Counter { value: 21 };
    read_static(&sensor) + read_dynamic(&sensor)
}
"#;
    let module = source_module(program);
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, wire] {
        let main = module.find_function_by_name("core.family_archive.main").unwrap();
        let actual = verum_vbc::interpreter::Interpreter::new(module.into())
            .execute_function(main).expect("execute both concrete and object calls");
        assert_eq!(actual.try_as_i64(), Some(42));
    }
}
