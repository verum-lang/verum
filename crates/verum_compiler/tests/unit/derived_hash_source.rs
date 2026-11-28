use super::*;
use crate::phases::semantic_analysis::SemanticAnalysisPhase;
use verum_ast::decl::{ImplKind, ItemKind};

#[test]
fn structural_hash_release_keyid() {
    let session = crate::Session::new(crate::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    let source = "module key_client; mount core.security.tuf.types.{KeyId, RoleName}; mount core.cog.resolve.PackageName; mount core.base.protocols.{Hash};
        fn require_hash<T: Hash>(k: &T) -> Int { 17 }
        fn check_key(k: &KeyId) -> Int { require_hash(k) }
        fn check_role(k: &RoleName) -> Int { require_hash(k) }
        fn check_package(k: &PackageName) -> Int { require_hash(k) }";
    let client = Parser::new(source).parse_module().unwrap();
    check_source_module(
        &mut phase.configured_checker(),
        "key_client",
        &client,
        &registry,
    )
    .unwrap_or_else(|errors| panic!("release KeyId Hash: {errors:?}"));
    // Isolate the real declarations and their generated bodies from TUF's
    // independent Map prerequisite in the same release declaration graph.
    for (owner, names) in [
        ("core.security.tuf.types", &["KeyId", "RoleName"][..]),
        ("core.cog.resolve", &["PackageName"][..]),
    ] {
        let mut declaring = registry.read().get_by_path(owner).unwrap().ast.clone();
        declaring.items.retain(|item| match &item.kind {
            ItemKind::Module(_) | ItemKind::Mount(_) => true,
            ItemKind::Type(td) => names.contains(&td.name.as_str()),
            ItemKind::Impl(implementation) => matches!(&implementation.kind,
                ImplKind::Protocol {protocol,for_type,..} if protocol.last_segment_name() == "Hash"
                && names.contains(&verum_ast::pretty::format_type(for_type).as_str())),
            _ => false,
        });
        let count = declaring
            .items
            .iter()
            .filter(|item| matches!(item.kind, ItemKind::Impl(_)))
            .count();
        assert_eq!(count, names.len());
        check_source_module(
            &mut phase.configured_checker(),
            owner,
            &declaring,
            &registry,
        )
        .unwrap_or_else(|errors| panic!("{owner} derived bodies: {errors:?}"));
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core");
    let owners = [
        "core.security.tuf.types",
        "core.security.tuf.role_verify",
        "core.security.tuf.client",
        "core.security.tuf",
    ]
    .map(Text::from);
    let mut session = crate::Session::new(crate::CompilerOptions {
        num_threads: 2,
        ..Default::default()
    });
    let admission =
        crate::CompilationPipeline::new_core(&mut session, crate::CoreConfig::new(root))
            .check_core_sources(&owners);
    if let Err(error) = admission {
        let diagnostic = error.to_string();
        assert!(
            diagnostic.contains("semantic checking failed"),
            "{diagnostic}"
        );
        assert!(
            !diagnostic.contains("KeyId: core::base::protocols::Hash"),
            "{diagnostic}"
        );
    }
}

#[test]
fn structural_hash_requires_each_field() {
    let parser = verum_fast_parser::FastParser::new();
    let owner = "core.base.protocols";
    let protocol = parser
        .parse_module_str(
            "module core.base.protocols; public type Hasher is { public state:Int };
        public type Hash is protocol {fn hash(&self, hasher:&mut Hasher);};",
            FileId::new(0),
        )
        .unwrap();
    let declaration = parser
        .parse_module_str(
            "module app; mount core.base.protocols.{Hasher}; type Opaque is { value:Int };
        implement Opaque { fn hash(&self, hasher: &mut Hasher) {} }
        @derive(Hash) type Wrapper is { payload:Opaque };",
            FileId::new(1),
        )
        .unwrap();
    let target = TargetConfig::host();
    let selected = prepare_source_roster(
        &[
            (owner, "protocols.vr", &protocol),
            ("app", "app.vr", &declaration),
        ],
        &target,
        true,
    )
    .unwrap();
    let registry = source_registry(
        selected
            .iter()
            .map(|(owner, path, module)| (owner.as_str(), path.as_str(), module)),
    )
    .unwrap();
    let phase = SemanticAnalysisPhase::new();
    let result = check_source_module(
        &mut phase.configured_checker(),
        "app",
        &selected[1].2,
        &registry,
    );
    let errors = result.expect_err("opaque storage cannot be hashed");
    assert!(
        errors.iter().any(|error| {
            error.code() == Some("E405")
                && error.message().contains("Opaque")
                && error.message().contains("core::base::protocols::Hash")
        }),
        "{errors:?}"
    );
}

#[test]
fn structural_hash_executes_the_canonical_text_body() {
    use verum_vbc::interpreter::Interpreter;
    let source = r#"@no_implicit_prelude module core.base.protocols;
        public type Hasher is { public state:Int };
        public type Hash is protocol { fn hash(&self, hasher:&mut Hasher); };
        public type Other is protocol { fn hash(&self, hasher:&mut Hasher); };
        implement Other for Text { fn hash(&self, hasher:&mut Hasher) {
            fn hash_field() -> Int { 99 }
            hasher.state = hash_field();
        } }
        implement Hash for Text { fn hash(&self, hasher:&mut Hasher) {
            hasher.state = hasher.state * 100 + (if *self == "alpha" { 71 } else { 13 });
        } }
        @derive(Hash) public type Wrap<T> is (T);
        @derive(Hash) public type Pair is { left: Wrap<Text>, right: Text };
        fn probe() -> Int {
            let mut sink = Hasher {state:0};
            let both = Pair { left: Wrap("alpha"), right: "beta" };
            both.hash(&mut sink); sink.state
        }"#;
    let mut session = crate::Session::new(crate::CompilerOptions::default());
    let file = session
        .load_source_string(source, std::path::PathBuf::from("protocols.vr"))
        .unwrap();
    let mut pipeline = crate::pipeline::CompilationPipeline::new_interpreter(&mut session);
    let module = pipeline.phase_parse(file).unwrap();
    let vbc = pipeline
        .compile_test_unit_to_vbc(module)
        .expect("ordinary checked lowering");
    let wire = verum_vbc::serialize::serialize_module(&vbc).unwrap();
    let vbc = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    verum_vbc::validate::validate_module(&vbc).unwrap();
    let entry = vbc
        .find_function_by_name("core.base.protocols.probe")
        .unwrap();
    assert_eq!(
        Interpreter::new(vbc.into())
            .execute_function_with_args(entry, &[])
            .unwrap()
            .as_i64(),
        7113
    );
}

#[test]
fn structural_hash_respects_selected_fields_inline_owners_and_feature_policy() {
    let source = r#"module cog; module inner {
        @derive(Hash, Clone) type Key is { visible: Int, @cfg(windows) hidden: Int };
    }"#;
    let module = Parser::new(source).parse_module().unwrap();
    let target = TargetConfig::for_target("x86_64-unknown-linux-gnu");
    let inputs = [("cog", "mod.vr", &module)];
    let disabled = prepare_source_roster(&inputs, &target, false).unwrap_err();
    assert!(disabled.to_string().contains("[meta] derive"), "{disabled}");
    let selected = prepare_source_roster(&inputs, &target, true).unwrap();
    let prepared = &selected[0].2;
    let repeated = prepare_source_roster(&[("cog", "mod.vr", prepared)], &target, true).unwrap();
    assert_eq!(repeated, selected);
    let graph = source_registry([("cog", "mod.vr", prepared)]).unwrap();
    let inner = graph.read().get_by_path("cog.inner").unwrap();
    let implementation = inner
        .ast
        .items
        .iter()
        .find(|item| matches!(item.kind, ItemKind::Impl(_)))
        .unwrap();
    let body = verum_ast::pretty::format_item(implementation);
    assert!(
        body.contains("visible") && !body.contains("hidden"),
        "{body}"
    );
    assert!(inner.ast.items.iter().filter_map(|item| match &item.kind { ItemKind::Type(td) => Some(td), _ => None })
        .any(|td| td.attributes.iter().any(|attr| attr.args.as_ref().is_some_and(|args| args.iter()
            .any(|arg| matches!(&arg.kind, verum_ast::ExprKind::Path(p) if p.last_segment_name() == "Clone"))))));
}
