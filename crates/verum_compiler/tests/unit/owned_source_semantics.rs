use super::*;
use std::sync::Arc;
use verum_ast::FileId;
use verum_fast_parser::Parser;

fn check(
    owner: &str,
    visibility: &str,
    metadata: bool,
    body: &str,
) -> Result<(), List<Diagnostic>> {
    let provider = Parser::new(&format!(
        "module realm.vault; public type Packet is {{ {visibility} value: Int }};"
    ))
    .parse_module()
    .unwrap();
    let source = if owner == "realm.vault" {
        format!("module {owner}; public type Packet is {{ {visibility} value: Int }}; {body}")
    } else {
        format!("module {owner}; mount realm.vault.Packet; {body}")
    };
    let consumer = verum_fast_parser::FastParser::new()
        .parse_module_str(&source, FileId::new(1))
        .unwrap();
    let registry = if owner == "realm.vault" {
        source_registry([(owner, "vault.vr", &consumer)]).unwrap()
    } else {
        source_registry([
            ("realm.vault", "vault.vr", &provider),
            (owner, "consumer.vr", &consumer),
        ])
        .unwrap()
    };
    let mut checker = if metadata {
        let module = verum_vbc::codegen::VbcCodegen::new()
            .compile_module(&provider)
            .unwrap();
        let mut builder = verum_vbc::archive::ArchiveBuilder::new();
        builder.add_module("realm.vault", &module, &[]).unwrap();
        let mut bytes = List::new();
        verum_vbc::archive::write_archive(&builder.finish(), &mut bytes).unwrap();
        let archive =
            verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
        TypeChecker::new_with_core(Arc::new(verum_compiler_metadata(&archive)))
    } else {
        TypeChecker::with_minimal_context()
    };
    checker.register_builtins();
    check_source_module(&mut checker, owner, &consumer, &registry)
}
fn verum_compiler_metadata(
    archive: &verum_vbc::archive::VbcArchive,
) -> verum_types::core_metadata::CoreMetadata {
    crate::archive_metadata::archive_to_core_metadata(archive).unwrap()
}
const BODIES: [&str; 3] = [
    "fn probe(packet: &Packet) -> Int { packet.value }",
    "fn probe(packet: &mut Packet) { packet.value = 42; }",
    "fn probe() -> Packet { Packet { value: 42 } }",
];
#[test]
fn owned_field_matrix_source_and_archive() {
    for (visibility, owner, allowed) in [
        ("private", "realm.vault", true),
        ("private", "realm.vault.child", false),
        ("public(in realm.vault)", "realm.vault.child", true),
        ("public(in realm.vault)", "realm.outside", false),
        ("public", "realm.outside", true),
    ] {
        for metadata in [false, true] {
            for body in BODIES {
                let result = check(owner, visibility, metadata, body);
                if allowed {
                    result.unwrap_or_else(|errors| panic!("{owner} {metadata} {body}: {errors:?}"));
                } else {
                    let errors = result.expect_err("private field accepted");
                    assert!(
                        errors
                            .iter()
                            .any(|e| e.message().contains("value") && e.code() == Some("E202")),
                        "wrong refusal {owner} {metadata}: {errors:?}"
                    );
                }
            }
        }
    }
}
#[test]
fn unused_missing_module_member_and_glob_are_errors() {
    for mount in [
        "mount missing.Packet;",
        "mount realm.vault.Missing;",
        "mount missing.*;",
    ] {
        let provider =
            Parser::new("module realm.vault; public type Packet is { public value: Int };")
                .parse_module()
                .unwrap();
        let consumer = Parser::new(&format!(
            "module realm.client; {mount} fn probe() -> Int {{ 42 }}"
        ))
        .parse_module()
        .unwrap();
        let registry = source_registry([
            ("realm.vault", "vault.vr", &provider),
            ("realm.client", "client.vr", &consumer),
        ])
        .unwrap();
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_builtins();
        let errors = check_source_module(&mut checker, "realm.client", &consumer, &registry)
            .expect_err("unused missing import admitted");
        assert!(
            errors
                .iter()
                .any(|e| e.message().contains("missing") || e.message().contains("Missing")),
            "{errors:?}"
        );
    }
}
#[test]
fn inactive_imports_and_competing_types_do_not_enter_the_scope() {
    let source = "module realm.scope; @cfg(target_os = \"windows\") mount absent.Packet; @cfg(target_os = \"windows\") type Choice is Bool; @cfg(not(target_os = \"windows\")) type Choice is Int; fn probe() -> Choice { 42 }";
    let module = Parser::new(source).parse_module().unwrap();
    let target = TargetConfig::for_target("aarch64-apple-darwin");
    let module = select_source_module("realm.scope", &module, &target)
        .unwrap()
        .unwrap();
    assert!(
        !module
            .items
            .iter()
            .any(|item| matches!(item.kind, ItemKind::Mount(_)))
    );
    let registry = source_registry([("realm.scope", "scope.vr", &module)]).unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    check_source_module(&mut checker, "realm.scope", &module, &registry).unwrap();
}

#[test]
fn distinct_source_parts_cannot_silently_share_a_registry_owner() {
    let first = Parser::new("module realm.owner; public type First is Int;")
        .parse_module()
        .unwrap();
    let second = Parser::new("module realm.owner; public type Second is Bool;")
        .parse_module()
        .unwrap();
    for reverse in [false, true] {
        let mut files = [
            ("realm.owner", "first.vr", &first),
            ("realm.owner", "second.vr", &second),
        ];
        if reverse {
            files.reverse();
        }
        let error = source_registry(files)
            .err()
            .expect("duplicate owner accepted")
            .to_string();
        assert!(
            error.contains("first.vr")
                && error.contains("second.vr")
                && error.contains("realm.owner"),
            "{error}"
        );
    }
    source_registry([
        ("realm.owner", "first.vr", &first),
        ("realm.owner", "first.vr", &first),
    ])
    .unwrap();
}

#[test]
fn original_staged_body_must_satisfy_its_local_annotation() {
    let module =
        Parser::new("module realm.stage; fn probe() -> Int { @const { let x: Bool = 42; x } }")
            .parse_module()
            .unwrap();
    let registry = source_registry([("realm.stage", "stage.vr", &module)]).unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    let errors = check_source_module(&mut checker, "realm.stage", &module, &registry)
        .expect_err("staged Bool annotation erased before admission");
    assert!(
        errors.iter().any(|e| e.message().contains("Bool")),
        "wrong refusal: {errors:?}"
    );
}
#[test]
fn inactive_impl_is_selected_before_type_registration() {
    let module = Parser::new("module realm.platform; @cfg(target_os = \"windows\") implement Missing { fn poison() -> Missing { unavailable } } fn probe() -> Int { 42 }").parse_module().unwrap();
    let selected = select_source_module(
        "realm.platform",
        &module,
        &TargetConfig::for_target("aarch64-apple-darwin"),
    )
    .unwrap()
    .unwrap();
    assert!(
        !selected
            .items
            .iter()
            .any(|item| matches!(item.kind, ItemKind::Impl(_)))
    );
    let registry = source_registry([("realm.platform", "platform.vr", &selected)]).unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    check_source_module(&mut checker, "realm.platform", &selected, &registry).unwrap();
    let active = select_source_module(
        "realm.platform",
        &module,
        &TargetConfig::for_target("x86_64-pc-windows-msvc"),
    )
    .unwrap()
    .unwrap();
    assert!(
        active
            .items
            .iter()
            .any(|item| matches!(item.kind, ItemKind::Impl(_)))
    );
}
#[test]
fn inactive_parent_never_reenables_a_source_child() {
    let parent =
        Parser::new("@cfg(target_os = \"windows\") module realm.parent; public module child;")
            .parse_module()
            .unwrap();
    let child =
        Parser::new("@cfg(target_os = \"macos\") module realm.parent.child; mount absent.Poison;")
            .parse_module()
            .unwrap();
    let selected = select_source_roster(
        &[
            ("realm.parent", "parent.vr", &parent),
            ("realm.parent.child", "child.vr", &child),
        ],
        &TargetConfig::for_target("aarch64-apple-darwin"),
    )
    .unwrap();
    assert!(selected.is_empty());
}
#[test]
fn malformed_cfg_after_false_is_never_hidden_by_selection() {
    for body in [
        "@cfg(target_os = \"windows\") @cfg(not()) fn probe() -> Int { 42 }",
        "type Packet is { @cfg(target_os = \"windows\") @cfg(not()) value: Int };",
        "type Packet is (); implement Packet { @cfg(target_os = \"windows\") @cfg(not()) fn probe() -> Int { 42 } }",
        "type Rules is protocol { @cfg(target_os = \"windows\") @cfg(not()) fn probe() -> Int; };",
    ] {
        let module = Parser::new(&format!("module realm.cfg; {body}"))
            .parse_module()
            .unwrap();
        assert!(
            select_source_module(
                "realm.cfg",
                &module,
                &TargetConfig::for_target("aarch64-apple-darwin")
            )
            .is_err(),
            "malformed condition hidden: {body}"
        );
    }
}
#[test]
fn namespace_alias_group_glob_and_public_reexport_resolve_before_bodies() {
    let provider = Parser::new("module realm.vault; public type Packet is { public value: Int }; public fn make() -> Packet { Packet { value: 42 } }").parse_module().unwrap();
    let facade = Parser::new("module realm.facade; public mount realm.vault.{Packet, make};")
        .parse_module()
        .unwrap();
    for (mount, expression) in [
        ("mount realm.vault;", "vault.make()"),
        ("mount realm.vault as cabinet;", "cabinet.make()"),
        ("mount realm.vault.{make as fabricate};", "fabricate()"),
        ("mount realm.vault.*;", "make()"),
        ("mount realm.facade.{make};", "make()"),
    ] {
        let module = Parser::new(&format!("module realm.consumer; {mount} fn probe() -> Int {{ let packet = {expression}; packet.value }}")).parse_module().unwrap();
        let registry = source_registry([
            ("realm.vault", "vault.vr", &provider),
            ("realm.facade", "facade.vr", &facade),
            ("realm.consumer", "consumer.vr", &module),
        ])
        .unwrap();
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_builtins();
        check_source_module(&mut checker, "realm.consumer", &module, &registry)
            .unwrap_or_else(|errors| panic!("{mount}: {errors:?}"));
    }
}
#[test]
fn supplied_registry_retains_actual_core_declarations_and_aliases() {
    let root = std::env::var_os("VERUM_TEST_SOURCE_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let path = root.join("core/math/logic.vr");
    let source = std::fs::read_to_string(&path).unwrap();
    let core = Parser::new(&source).parse_module().unwrap();
    let registry = source_registry([("core.math.logic", path.to_str().unwrap(), &core)]).unwrap();
    registry
        .write()
        .register_path_alias("std.math.logic", "core.math.logic");
    for (mount, body, allowed) in [
        (
            "core.math.logic.Prop",
            "fn preserve(value: Prop) -> Prop { value }",
            true,
        ),
        (
            "std.math.logic.Prop",
            "fn preserve(value: Prop) -> Prop { value }",
            true,
        ),
        (
            "core.math.logic.Proof",
            "fn expose(value: &Proof<Int>) -> Int { value.witness }",
            false,
        ),
    ] {
        let module = Parser::new(&format!("module example.client; mount {mount}; {body}"))
            .parse_module()
            .unwrap();
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_builtins();
        let result = check_source_module(&mut checker, "example.client", &module, &registry);
        if allowed {
            result.unwrap_or_else(|errors| panic!("{mount}: {errors:?}"));
        } else {
            assert!(
                result.unwrap_err().iter().any(|e| e.code() == Some("E202")),
                "private actual core field allowed"
            );
        }
    }
    assert!(
        registry
            .read()
            .get_by_path_aliased("std.math.logic")
            .is_some()
    );
}
#[test]
fn completing_an_existing_placeholder_preserves_export_owner_ids() {
    let registry = Shared::new(parking_lot::RwLock::new(ModuleRegistry::new()));
    let id = registry.read().allocate_id();
    let file = FileId::new(50);
    registry.write().register(ModuleInfo::new(
        id,
        ModulePath::from_str("realm.vault"),
        Module::empty(file),
        file,
        Text::new(),
    ));
    let provider = Parser::new("module realm.vault; public type Packet is { public value: Int }; public fn make() -> Packet { Packet { value: 42 } }").parse_module().unwrap();
    register_sources(&registry, [("realm.vault", "vault.vr", &provider)]).unwrap();
    {
        let graph = registry.read();
        let complete = graph.get_by_path("realm.vault").unwrap();
        assert_eq!(complete.id, id);
        for (_, export) in complete.exports.all_exports() {
            assert_eq!(
                export.source_module, id,
                "export has a dangling declaration owner"
            );
            assert!(graph.get(export.source_module).is_some());
        }
    }
    let consumer = Parser::new("module realm.client; mount realm.vault.make; fn probe() -> Int { let packet = make(); packet.value }").parse_module().unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    check_source_module(&mut checker, "realm.client", &consumer, &registry).unwrap();
}

#[test]
fn original_source_graph_includes_inline_prelude_and_its_declared_exports() {
    for inline in [false, true] {
        let (owner, prelude) = if inline {
            (
                "core",
                "module core; public module prelude { public mount realm.seed.Pivot; }",
            )
        } else {
            (
                "core.prelude",
                "module core.prelude; public mount realm.seed.Pivot;",
            )
        };
        let sources = [
            (owner, prelude),
            (
                "realm.seed",
                "module realm.seed; public type Pivot is { public code: Int };",
            ),
            (
                "realm.provider",
                "module realm.provider; public type Packet is { public value: Pivot };",
            ),
            (
                "realm.client",
                "module realm.client; mount realm.provider.Packet; fn read(packet: &Packet)->Int { packet.value.code }",
            ),
        ];
        let modules: Vec<_> = sources
            .iter()
            .enumerate()
            .map(|(i, (_, source))| {
                verum_fast_parser::FastParser::new()
                    .parse_module_str(source, FileId::new(i as u32))
                    .unwrap()
            })
            .collect();
        let registry = source_registry(
            sources
                .iter()
                .zip(&modules)
                .map(|((owner, _), module)| (*owner, *owner, module)),
        )
        .unwrap();
        assert!(registry.read().get_by_path("core.prelude").is_some());
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_builtins();
        check_source_module(&mut checker, "realm.client", &modules[3], &registry)
            .unwrap_or_else(|errors| panic!("inline={inline}: {errors:?}"));
    }
}

#[test]
fn inline_and_file_sources_cannot_claim_conflicting_declarations() {
    let outer = Parser::new("module realm; public module child { public type Token is (); }")
        .parse_module()
        .unwrap();
    let file = Parser::new("module realm.child; public type Token is { public value: Int };")
        .parse_module()
        .unwrap();
    let error = source_registry([
        ("realm", "root.vr", &outer),
        ("realm.child", "child.vr", &file),
    ])
    .expect_err("a source file cannot replace the inline declaration");
    assert!(error.to_string().contains("realm.child"), "{error}");
}

// Original-source admission shares the ordinary implicit prelude policy.
fn check_prelude_scope(provider: &str, consumer: &str) -> Vec<String> {
    let sources = [("core.prelude",provider),("core.consumer",consumer)];
    let asts: Vec<_> = sources.iter().enumerate().map(|(i,(_,source))|
        verum_fast_parser::FastParser::new().parse_module_str(source,FileId::new(i as u32)).unwrap()).collect();
    let graph = super::source_registry(sources.iter().zip(&asts).map(|((owner,_),ast)|(*owner,*owner,ast))).unwrap();
    let mut checker=verum_types::TypeChecker::with_minimal_context();checker.register_builtins();
    match super::check_source_module(&mut checker,"core.consumer",&asts[1],&graph){
        Ok(())=>vec![],Err(errors)=>errors.iter().map(|e|e.message().to_string()).collect()
    }
}
#[test] fn implicit_prelude_is_in_scope_for_local_declarations(){
    let errors=check_prelude_scope("module core.prelude; public type Ambient is {public value:Int};", "module core.consumer; type Packet is {value:Ambient}; fn read(packet:&Packet)->Int{packet.value.value}");
    assert!(errors.is_empty(),"{errors:?}");
}
#[test] fn opt_out_keeps_the_name_unavailable(){
    let errors=check_prelude_scope("module core.prelude; public type Ambient is {public value:Int};", "@no_implicit_prelude module core.consumer; type Packet is {value:Ambient};");
    assert!(errors.iter().any(|e|e.contains("Ambient")),"{errors:?}");
}
#[test] fn private_prelude_type_stays_private(){
    let errors=check_prelude_scope("module core.prelude; private type Ambient is {public value:Int};", "module core.consumer; type Packet is {value:Ambient};");
    assert!(errors.iter().any(|e|e.contains("Ambient")),"{errors:?}");
}
#[test] fn local_declaration_has_priority(){
    let errors=check_prelude_scope("module core.prelude; public type Ambient is {public wrong:Bool};", "module core.consumer; type Ambient is {value:Int}; fn read(value:&Ambient)->Int{value.value}");
    assert!(errors.is_empty(),"{errors:?}");
}
#[test] fn explicit_prelude_mount_is_not_duplicated(){
    let errors=check_prelude_scope("module core.prelude; public type Ambient is {public value:Int};", "module core.consumer; mount core.prelude.Ambient; fn read(value:&Ambient)->Int{value.value}");
    assert!(errors.is_empty(),"{errors:?}");
}
#[test] fn minimal_core_can_be_checked_without_a_prelude_declaration(){
    let source=verum_fast_parser::FastParser::new().parse_module_str("module core; public fn probe()->Int{42}",FileId::new(0)).unwrap();
    let graph=super::source_registry([("core","mod.vr",&source)]).unwrap();
    let mut checker=verum_types::TypeChecker::with_minimal_context();checker.register_builtins();
    let result=super::check_source_module(&mut checker,"core",&source,&graph);
    assert!(result.is_ok(),"{result:?}");
}
