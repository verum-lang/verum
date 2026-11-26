//! T2840: the graph-theory layer owns the Graph implementation, while the
//! collection exports borrowing methods and keeps its representation private.
//! This is source admission, not a claim that every graph algorithm is proved.

use std::path::Path;
use verum_ast::{
    ItemKind, Module,
    cfg::TargetConfig,
    decl::{ImplItemKind, ImplKind},
};
use verum_common::{List, Set, Shared, Text};
use verum_compiler::phases::{
    CompilationPhase, ExecutionTier, LanguageProfile, OptimizationLevel, PhaseContext, PhaseData,
    PhaseInput, VerifyMode, semantic_analysis::SemanticAnalysisPhase,
};
use verum_compiler::{CompilerOptions, Session, core_compiler::StdlibModuleResolver};
use verum_diagnostics::Diagnostic;
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleInfo, ModuleLoader, ModulePath, ModuleRegistry, ModuleSource,
    extract_exports_from_module, resolve_glob_reexports, resolve_specific_reexport_kinds,
};

const COLLECTION: &str = "core.collections.adjacency_list";
const THEORY: &str = "core.math.graph";

fn source_graph() -> (Session, ModuleRegistry) {
    let session = Session::new(CompilerOptions::default());
    let root = std::env::var_os("VERUM_TEST_SOURCE_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core"));
    let target = TargetConfig::host();
    let mut resolver = StdlibModuleResolver::with_target(&root, target.clone());
    resolver.discover().unwrap();
    let loader = ModuleLoader::with_config(&root, target);
    let mut registry = ModuleRegistry::new();
    let mut pending = List::new();
    for group in resolver.modules_in_order() {
        for path in &group.source_files {
            let source: Text = std::fs::read_to_string(path).unwrap().into();
            let file = session.load_source_string(&source, path.clone()).unwrap();
            let ast = FastParser::new().parse_module_str(&source, file).unwrap();
            let owner =
                verum_compiler::module_utils::source_file_module_path(&group.name, path, &ast);
            let info = loader
                .parse_module(
                    &ModuleSource::new(path.clone(), source, file),
                    registry.allocate_id(),
                    ModulePath::from_str(&owner),
                )
                .unwrap();
            pending.push(info);
        }
    }
    // Inline sources retain their real owner and source span. No ambient type
    // declarations or test-only Graph/collection definitions are supplied.
    while let Some(mut info) = pending.pop() {
        for item in &info.ast.items {
            if let ItemKind::Module(child) = &item.kind
                && let Some(items) = &child.items
            {
                let ast = Module {
                    items: items.clone(),
                    attributes: item.attributes.clone(),
                    file_id: info.file_id,
                    span: item.span,
                };
                pending.push(ModuleInfo::new(
                    registry.allocate_id(),
                    info.path.join(child.name.as_str()),
                    ast,
                    info.file_id,
                    info.source.clone(),
                ));
            }
        }
        assert!(
            registry.get_by_path(&info.path.to_string()).is_none(),
            "duplicate source owner {}",
            info.path
        );
        info.exports = extract_exports_from_module(&info.ast, info.id, &info.path).unwrap();
        registry.register(info);
    }
    resolve_specific_reexport_kinds(&mut registry).unwrap();
    resolve_glob_reexports(&mut registry).unwrap();
    (session, registry)
}

fn check(registry: &ModuleRegistry, module: Module) -> Result<(), List<Diagnostic>> {
    SemanticAnalysisPhase::new()
        .with_module_registry(Shared::new(parking_lot::RwLock::new(registry.clone())))
        .execute(PhaseInput {
            data: PhaseData::AstModules(List::from_iter([module])),
            context: PhaseContext {
                profile: LanguageProfile::Application,
                target_tier: ExecutionTier::Interpreter,
                verify_mode: VerifyMode::None,
                opt_level: OptimizationLevel::O0,
            },
        })
        .map(|_| ())
}

fn client(session: &Session, source: &str) -> Module {
    let source = format!("module application.graph_client;\n{source}");
    let file = session
        .load_source_string(&source, "graph_protocol_boundary.vr".into())
        .unwrap();
    FastParser::new().parse_module_str(&source, file).unwrap()
}

fn graph_impl(item: &verum_ast::Item) -> bool {
    matches!(&item.kind, ItemKind::Impl(declaration)
        if matches!(&declaration.kind, ImplKind::Protocol { protocol, .. }
            if protocol.as_ident().is_some_and(|name| name.as_str() == "Graph")))
}

#[test]
fn actual_adjacency_list_has_no_reverse_theory_dependency() {
    let (session, registry) = source_graph();
    let collection = registry.get_by_path(COLLECTION).unwrap();
    assert!(!collection.ast.items.iter().any(graph_impl));
    for item in &collection.ast.items {
        if let ItemKind::Mount(mount) = &item.kind {
            let path = match &mount.tree.kind {
                verum_ast::decl::MountTreeKind::Path(path)
                | verum_ast::decl::MountTreeKind::Glob(path)
                | verum_ast::decl::MountTreeKind::Nested { prefix: path, .. } => path,
                verum_ast::decl::MountTreeKind::File { .. } => panic!("unexpected file dependency"),
            };
            assert!(
                !path.segments.iter().any(|segment| matches!(segment,
                verum_ast::ty::PathSegment::Name(name) if name.as_str() == "math")),
                "the collection regained a graph-theory dependency: {path}"
            );
        }
    }
    check(&registry, collection.ast.clone())
        .unwrap_or_else(|errors| panic!("complete AdjacencyList source: {errors:#?}"));
    let imports = "mount core.collections.adjacency_list.AdjacencyList;
        mount core.base.protocols.{Eq, Hash, Clone};";
    let positive = format!("{imports}
        fn build() -> AdjacencyList<Int, Int> {{
            let mut graph: AdjacencyList<Int, Int> = AdjacencyList.empty();
            let key = 2;
            graph.add_vertex(key);
            graph.add_edge(key, 4, 17);
            graph
        }}
        fn extend<V: Eq + Hash + Clone>(graph: &mut AdjacencyList<V, Int>, key: &V) {{
            graph.add_edge(key, key, 3);
        }}");
    check(&registry, client(&session, &positive))
        .unwrap_or_else(|errors| panic!("graph insertion: {errors:#?}"));
    // A user-defined Clone can borrow a field of its receiver. A graph
    // that survives this call may not retain a borrow of the local key.
    let escape = format!("{imports}
        fn escape<V: Eq + Hash + Clone>(graph: &mut AdjacencyList<V, Int>, key: V) {{
            graph.add_vertex(&key);
        }}");
    let errors = check(&registry, client(&session, &escape)).expect_err("local vertex escaped");
    assert!(errors.iter().any(|error| error.code() == Some("E312")
        && error.message().contains("key")), "{errors:#?}");
}

#[test]
fn actual_graph_adapter_uses_public_accessors_and_keeps_private_fields_closed() {
    let (session, registry) = source_graph();
    let original = registry.get_by_path(THEORY).unwrap();
    let implementations: List<_> = original
        .ast
        .items
        .iter()
        .filter(|item| graph_impl(item))
        .collect();
    assert_eq!(
        implementations.len(),
        1,
        "the protocol owner must contain exactly one Graph adapter"
    );
    let ItemKind::Impl(adapter) = &implementations[0].kind else {
        unreachable!()
    };
    let methods: Set<_> = adapter
        .items
        .iter()
        .filter_map(|item| match &item.kind {
            ImplItemKind::Function(function) => Some(function.name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        methods,
        Set::from_iter(["vertices", "edges", "out_edges", "in_edges", "degree"])
    );
    assert_eq!(
        adapter.items.len(),
        6,
        "retain all methods and the original axiom witness"
    );
    assert!(adapter.items.iter().any(|item| matches!(&item.kind,
        ImplItemKind::Proof { axiom_name, .. } if axiom_name.as_str() == "truncation")));

    // Check the complete, unchanged protocol and adapter bodies together. The
    // other graph algorithms have independent dynamic-protocol/coherence gates;
    // neither their signatures nor the adapter's obligations are rewritten here.
    let mut selected = original.ast.clone();
    selected.items.retain(|item| match &item.kind {
        ItemKind::Module(_) | ItemKind::Mount(_) => true,
        ItemKind::Type(declaration) => declaration.name.as_str() == "Graph",
        _ => graph_impl(item),
    });
    assert_eq!(
        selected
            .items
            .iter()
            .filter(|item| graph_impl(item))
            .count(),
        1
    );
    let mut scoped = ModuleRegistry::new();
    for (_, info) in registry.all_modules() {
        let mut info = (**info).clone();
        if info.path.to_string() == THEORY {
            info.ast = selected.clone();
            info.exports = extract_exports_from_module(&info.ast, info.id, &info.path).unwrap();
        }
        scoped.register(info);
    }
    check(&scoped, selected)
        .unwrap_or_else(|errors| panic!("actual Graph protocol and adapter bodies: {errors:#?}"));

    let imports = "mount core.collections.adjacency_list.AdjacencyList;\n\
        mount core.math.graph.Graph;";
    check(&scoped, client(&session, &format!(r#"
        {imports}
        fn vertices(value: &AdjacencyList<Int, Int>) -> &List<Int> {{ value.vertices_ref() }}
        fn degree(value: &AdjacencyList<Int, Int>, vertex: &Int) -> Int {{ value.degree(vertex) }}
        fn use_protocol<G: Graph<Int, Int>>(value: &G, vertex: &Int) -> Int {{ value.degree(vertex) }}
    "#))).unwrap_or_else(|errors| panic!("public graph boundary: {errors:#?}"));

    for field in ["vertex_list", "out_edges"] {
        let errors = check(&scoped, client(&session, &format!(
            "{imports} fn probe(value: &AdjacencyList<Int, Int>) {{ let hidden = value.{field}; }}"
        ))).expect_err("protocol ownership exposed private collection storage");
        assert!(
            errors
                .iter()
                .any(|error| error.code() == Some("E202") && error.message().contains(field)),
            "private {field}: {errors:#?}"
        );
    }
    let errors = check(
        &scoped,
        client(&session, "fn probe<G: Graph<Int, Int>>(value: &G) {}"),
    )
    .expect_err("a protocol in the registry became an ambient declaration");
    assert!(
        errors.iter().any(|error| error.message().contains("Graph")),
        "{errors:#?}"
    );
}
