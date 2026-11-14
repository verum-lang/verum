use super::*;
use std::sync::Arc;
use verum_ast::FileId;
use verum_diagnostics::Diagnostic;
use verum_fast_parser::Parser;

#[path = "source_protocol_registration.rs"]
mod source_protocol_registration;

fn select_source_module(
    owner: &str,
    module: &Module,
    target: &TargetConfig,
) -> anyhow::Result<Option<Module>> {
    Ok(select_source_roster(&[(owner, "example.vr", module)], target)?
        .pop().map(|(_, _, module)| module))
}

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
    let repeated = [
        ("realm.owner", "first.vr", &first),
        ("realm.owner", "first.vr", &first),
    ];
    source_registry(repeated).unwrap();
    let selected = select_source_roster(&repeated, &TargetConfig::host()).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].2, first);
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
        "type Packet is Visible | @cfg(target_os = \"windows\") @cfg(not()) Hidden;",
        "type Packet is Data { @cfg(target_os = \"windows\") @cfg(not()) hidden: Int } | Empty;",
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
fn variant_layout_uses_all_target_conditions_on_cases_and_payload_fields() {
    use verum_ast::{TypeDeclBody, decl::VariantData};
    let source = r#"module realm.cfg;
        public type Packet is Data { always: Int, @cfg(target_os = "windows") extra: Bool }
            | @cfg(target_arch = "aarch64") @cfg(target_os = "windows") ArmWindows
            | @cfg(target_os = "macos") Darwin;
    "#;
    let module = Parser::new(source).parse_module().unwrap();
    for (target, want_cases, want_fields) in [
        ("aarch64-apple-darwin", &["Data", "Darwin"][..], &["always"][..]),
        ("aarch64-pc-windows-msvc", &["Data", "ArmWindows"][..], &["always", "extra"][..]),
        ("x86_64-pc-windows-msvc", &["Data"][..], &["always", "extra"][..]),
    ] {
        let selected = select_source_module("realm.cfg", &module, &TargetConfig::for_target(target))
            .unwrap().unwrap();
        let ItemKind::Type(decl) = &selected.items[1].kind else { panic!("type declaration") };
        let TypeDeclBody::Variant(cases) = &decl.body else { panic!("sum declaration") };
        let names: List<_> = cases.iter().map(|case| case.name.as_str()).collect();
        assert_eq!(names.as_slice(), want_cases, "{target}");
        let Some(VariantData::Record(fields)) = &cases[0].data else { panic!("record payload") };
        let names: List<_> = fields.iter().map(|field| field.name.as_str()).collect();
        assert_eq!(names.as_slice(), want_fields, "{target}");
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

#[test]
fn catch_unwind_uses_the_declared_callee_and_callback_result() {
    use verum_fast_parser::FastParser;
    let maybe = "module core.base.maybe; public type Maybe<T> is None | Some(T);";
    let result = "module core.base.result; public type Result<T,E> is Ok(T) | Err(E);";
    let control = r#"module core.intrinsics.control;
        mount core.base.result.Result;
        mount core.base.maybe.Maybe;
        public type Location is { line: Int };
        public type IntrinsicPanicInfo is { message: Text, location: Maybe<Location> };
        @intrinsic("catch_unwind")
        public fn catch_unwind<T>(f:fn()->T)->Result<T,IntrinsicPanicInfo> { @intrinsic("catch_unwind", f) }
    "#;
    for (extra, should_pass) in [
        ("fn check()->Result<Int,IntrinsicPanicInfo> { @intrinsic(\"catch_unwind\", || 7) }", true),
        ("fn check()->Int { let r:Result<Int,IntrinsicPanicInfo> = @intrinsic(\"catch_unwind\", || 7); match r { Result.Ok(n)=>n, _=>0 } }", true),
        ("type OtherResult<T,E> is Ok(T) | Err(E); fn check()->Int { let r:OtherResult<Int,IntrinsicPanicInfo> = @intrinsic(\"catch_unwind\", || 7); 0 }", false),
        ("fn check()->Result<Bool,IntrinsicPanicInfo> { @intrinsic(\"catch_unwind\", || 7) }", false),
        ("fn check()->Result<Int,Other> { @intrinsic(\"catch_unwind\", || 7) }", false),
        ("fn check()->Result<Int,IntrinsicPanicInfo> { @intrinsic(\"catch_unwind\", |x:Int| x) }", false),
        ("fn check()->Result<Int,IntrinsicPanicInfo> { @intrinsic(\"catch_unwind\", async || { 7 }) }", false),
        ("@intrinsic(\"catch_unwind\") fn wrapper(f:fn()->Int)->Result<Int,Custom> { @intrinsic(\"catch_unwind\", f) }", true),
        ("@intrinsic(\"catch_unwind\") fn wrapper(f:fn()->Int)->Result<Int,Custom> { let g=|| @intrinsic(\"catch_unwind\", f); g() }", false),
    ] {
        let client = format!(r#"module consumer;
            mount core.base.result.Result;
            mount core.base.maybe.Maybe;
            mount core.intrinsics.control.{{IntrinsicPanicInfo, Location}};
            type Other is {{ value: Int }};
            type Custom is {{ message: Text, location: Maybe<Location> }};
            {extra}
        "#);
        let files = [
            ("core.base.maybe", maybe),
            ("core.base.result", result),
            ("core.intrinsics.control", control),
            ("consumer", client.as_str()),
        ];
        let modules: Vec<_> = files.iter().enumerate().map(|(i, (owner, source))| {
            let ast = FastParser::new().parse_module_str(source, FileId::new(i as u32)).unwrap();
            (*owner, ast)
        }).collect();
        let registry = source_registry(modules.iter().map(|(name, ast)| (*name, *name, ast))).unwrap();
        for (owner, ast) in &modules[2..] {
            let mut checker = TypeChecker::new();
            let checked = check_source_module(&mut checker, owner, ast, &registry);
            assert_eq!(checked.is_ok(), *owner != "consumer" || should_pass, "{extra}: {checked:?}");
        }
    }
}

#[test]
fn qualified_calls_follow_the_owned_source_exports_and_aliases() {
    use verum_fast_parser::FastParser;

    let time = r#"module core.intrinsics.runtime.time;
        public fn count()->Int { 73 }
        private fn secret()->Int { 1 }
        public fn takes(n: Int)->Int { n }
    "#;
    let api = r#"module core.intrinsics;
        public mount .runtime.time.{count as cpu_count, takes};
        mount .runtime.time.secret;
    "#;
    for (body, valid) in [
        ("core.intrinsics.runtime.time.count()", true),
        ("super.intrinsics.cpu_count()", true),
        ("super.intrinsics.takes(9)", true),
        ("super.intrinsics.takes()", false),
        ("super.intrinsics.runtime.time.secret()", false),
        ("core.intrinsics.cpu_count()", true),
        ("intrinsics.runtime.time.count()", true),
        ("intrinsics.runtime.time.secret()", false),
        ("core.sys.api.reexport()", true),
        ("core.intrinsics.takes(9)", true),
        ("core.intrinsics.takes()", false),
        ("core.intrinsics.secret()", false),
        ("core.intrinsics.runtime.time.secret()", false),
        ("core.intrinsics.missing()", false),
        ("{ let core = true; core.intrinsics.cpu_count() }", false),
    ] {
        let client = format!("module core.consumer; fn probe()->Int {{ {body} }}");
        let files = [
            ("core.intrinsics.runtime.time", time),
            ("core.intrinsics", api),
            ("core.sys.api", "module core.sys.api; public mount core.intrinsics.cpu_count as reexport;"),
            ("core.consumer", client.as_str()),
            ("core", "module core; public module intrinsics;"),
        ];
        let modules: Vec<_> = files.iter().enumerate().map(|(i, (owner, source))| {
            let ast = FastParser::new().parse_module_str(source, FileId::new(i as u32)).unwrap();
            (*owner, ast)
        }).collect();
        let registry = source_registry(modules.iter().map(|(owner, ast)| (*owner, *owner, ast))).unwrap();
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_builtins();
        let result = check_source_module(&mut checker, "core.consumer", &modules[3].1, &registry);
        assert_eq!(result.is_ok(), valid, "{body}: {result:?}");
    }
}

#[test]
fn mounted_free_functions_keep_their_declared_protocol_requirements() {
    use verum_fast_parser::FastParser;
    let protocol = "module authority; public type Approved<T> is protocol {};";
    let provider = r#"module provider; mount authority.Approved;
        public fn attach<A: Approved<B>,B>(a:A, b:B)->A { a }
        public fn separated<A,B>(a:A, b:B)->A where A: Approved<B> { a }
    "#;
    for (body, allowed) in [
        ("fn check(n:Card)->Card { call(n, true) }", true),
        ("fn check(n:Card)->Card { call(n, 7) }", false),
        ("fn check<T>(v:T)->T { call(v, true) }", false),
        ("fn check<T:Approved<Bool>>(v:T)->T { call(v, true) }", true),
        ("fn check(n:Card)->Card { separated(n, true) }", true),
        ("fn check(n:Card)->Card { separated(n, 7) }", false),
    ] {
        let receiver = format!("module consumer; mount authority.Approved; mount entry.call; mount provider.separated;
            type Card is {{ number:Int }}; implement Approved<Bool> for Card {{}} {body}");
        let sources = [
            ("authority", protocol), ("provider", provider),
            ("entry", "module entry; public mount provider.attach as call;"),
            ("consumer", receiver.as_str()),
        ];
        let modules: Vec<_> = sources.iter().enumerate().map(|(i, (owner, source))| {
            let ast = FastParser::new().parse_module_str(source, FileId::new(i as u32)).unwrap();
            (*owner, ast)
        }).collect();
        let registry = source_registry(modules.iter().map(|(owner, ast)| (*owner, *owner, ast))).unwrap();
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_builtins();
        let result = check_source_module(&mut checker, "consumer", &modules[3].1, &registry);
        assert_eq!(result.is_ok(), allowed, "{body}: {result:?}");
    }
}

#[test]
fn forward_type_aliases_reach_their_declaring_constructor() {
    let concrete = "public type Concrete is { n:Int };";
    let constructor = "implement Concrete { public fn new()->Concrete { Concrete { n:7 } } }";
    let alias = "type Backend is Concrete;";
    let consumer = "type Driver is { inner:Backend }; implement Driver { fn new()->Driver { Driver { inner:Backend.new() } } }";
    for (declarations, valid) in [
        (format!("{consumer} {alias} {concrete} {constructor}"), true),
        (format!("{concrete} {constructor} {alias} {consumer}"), true),
        (format!("{consumer} type Backend is Missing; {concrete} {constructor}"), false),
        (format!("{consumer} {alias} {concrete}"), false),
    ] {
        let source = format!("module core.driver; {declarations}");
        let ast = Parser::new(&source).parse_module().unwrap();
        let registry = source_registry([("core.driver", "driver.vr", &ast)]).unwrap();
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_builtins();
        let result = check_source_module(&mut checker, "core.driver", &ast, &registry);
        assert_eq!(result.is_ok(), valid, "{source}: {result:?}");
    }
}

#[test]
fn declared_tuple_construction_adds_no_effects_to_its_arguments() {
    for (declarations, allowed) in [
        ("type Wrapped is (Int); pure fn wrap(x:Int)->Wrapped { Wrapped(x) }", true),
        ("type Pair is (Int,Bool); pure fn wrap(x:Int)->Pair { Pair(x,true) }", true),
        ("type Wrapped is (Int); pure fn wrap(f:fn()->Int)->Wrapped { Wrapped(f()) }", false),
        ("pure fn wrap(Wrapped:fn(Int)->Int)->Int { Wrapped(5) }", false),
    ] {
        let source = format!("module code; {declarations}");
        let module = Parser::new(&source).parse_module().unwrap();
        let registry = source_registry([("code", "code.vr", &module)]).unwrap();
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_builtins();
        let result = check_source_module(&mut checker, "code", &module, &registry);
        if allowed {
            result.unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
        } else {
            let errors = result.expect_err("arbitrary callback admitted as pure");
            assert!(errors.iter().any(|error| error.code() == Some("E503")), "{source}: {errors:?}");
        }
    }
}

fn release_stdlib_graph(session: &crate::Session) -> (SourceRegistry, crate::phases::semantic_analysis::SemanticAnalysisPhase) {
    use crate::core_compiler::StdlibModuleResolver;
    use crate::phases::semantic_analysis::SemanticAnalysisPhase;
    use verum_fast_parser::FastParser;

    let root = std::env::var_os("VERUM_TEST_SOURCE_ROOT").map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core"));
    let target = session.cfg_evaluator().config().clone();
    let mut resolver = StdlibModuleResolver::with_target(&root, target.clone());
    resolver.discover().unwrap();
    let mut parsed = List::new();
    for group in resolver.modules_in_order() {
        for path in &group.source_files {
            let source = std::fs::read_to_string(path).unwrap();
            let file = session.load_source_string(&source, path.clone()).unwrap();
            let module = FastParser::new().parse_module_str(&source, file).unwrap();
            let owner = crate::module_utils::source_file_module_path(&group.name, path, &module);
            parsed.push((Text::from(owner), Text::from(path.to_string_lossy().as_ref()), module));
        }
    }
    let entries: List<_> = parsed.iter().map(|(owner, path, ast)|
        (owner.as_str(), path.as_str(), ast)).collect();
    let selected = select_source_roster(&entries, &target).unwrap();
    let registry = source_registry(selected.iter().map(|(owner, path, ast)|
        (owner.as_str(), path.as_str(), ast))).unwrap();
    let phase = SemanticAnalysisPhase::new().with_language_features(session.language_features()).with_target(target);
    (registry, phase)
}

#[test]
fn release_runtime_and_sync_source_admission() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    for owner in ["core.runtime.config", "core.sync"] {
        let module = registry.read().get_by_path(owner).unwrap().ast.clone();
        check_source_types(&mut phase.configured_checker(), owner, &module, &registry,
            |error| error.to_diagnostic_with_span(Some(|span| session.convert_span(span))))
            .unwrap_or_else(|errors| panic!("{owner}: {errors:?}"));
    }

    for (receiver, marker, valid) in [
        ("Map<Int, Int>", "Send", true),
        ("Set<Int>", "Sync", true),
        ("Result<Int, Bool>", "Sync", true),
        ("Map<Int, Local>", "Send", false),
        ("Result<Int, Local>", "Send", false),
        ("Result<Local, Int>", "Sync", false),
    ] {
        let source = format!("module marker_probe; mount core.sync.{{Send, Sync}}; mount core.collections.map.Map; mount core.collections.set.Set; type Local is {{ value: Int }}; fn bound<T: {marker}>(_value: T) {{}} fn probe(value: {receiver}) {{ bound(value); }}");
        let module = Parser::new(&source).parse_module().unwrap();
        let result = check_source_module(&mut phase.configured_checker(), "marker_probe", &module, &registry);
        if valid {
            result.unwrap_or_else(|errors| panic!("{receiver}: {marker}: {errors:?}"));
        } else {
            let errors = result.expect_err("unmarked payload satisfied a transfer bound");
            assert!(errors.iter().any(|error| error.code() == Some("E405") && error.message().contains(marker)),
                "{receiver}: {marker}: {errors:?}");
        }
    }
}

#[path = "stdlib_local_file_readers.rs"]
mod stdlib_local_file_readers;

#[path = "infinity_classifier_contract.rs"]
mod infinity_classifier_contract;

#[path = "reference_method_authority.rs"]
mod reference_method_authority;

#[path = "configuration_owned_payloads.rs"]
mod configuration_owned_payloads;
