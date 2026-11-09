use verum_ast::{FileId, ItemKind};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;
const PROVIDER: &str = r#"
public type Stream is protocol { type Item; };
public type Factory is protocol { type Rows: Stream; };
public type Rows<T> is { value: T };
public type Box<T> is { value: T };
implement<T> Stream for Rows<T> { type Item = T; }
implement<T> Factory for Box<T> { type Rows = Rows<T>; }
"#;
fn check(source: &str, provider: &str) -> Vec<String> {
    check_with_extra(source, provider, None)
}
fn check_with_extra(source: &str, provider: &str, extra: Option<&str>) -> Vec<String> {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("demo.main");
    checker.set_current_cog("demo");
    let mut registry = ModuleRegistry::new();
    let ast = FastParser::new()
        .parse_module_str(provider, FileId::new(1))
        .unwrap();
    let id = ModuleId::new(1);
    let path = ModulePath::from_str("demo.lib");
    let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, provider.into());
    info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
    registry.register(info);
    if let Some(extra) = extra {
        let ast = FastParser::new()
            .parse_module_str(extra, FileId::new(3))
            .unwrap();
        let id = ModuleId::new(2);
        let path = ModulePath::from_str("demo.foreign");
        let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, extra.into());
        info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
        registry.register(info);
    }
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(2))
        .unwrap();
    let id = ModuleId::new(3);
    let path = ModulePath::from_str("demo.main");
    let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, source.into());
    info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
    registry.register(info);
    checker.set_module_registry_direct(registry.clone());
    let mut errors = Vec::new();
    for item in &ast.items {
        if let ItemKind::Mount(mount) = &item.kind {
            if let Err(e) = checker.process_import(mount, "demo.main", &registry) {
                errors.push(e.to_string());
            }
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(ToString::to_string),
    );
    errors.extend(checker.diagnostic_sources().iter().map(ToString::to_string));
    errors
}
#[test]
fn provider_associated_result_implementation_is_discovered() {
    let errors = check("mount demo.lib.Box;", PROVIDER);
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn imported_implementation_proves_its_own_bounded_receiver() {
    const PROVIDER: &str = "public type Key is protocol {};
        public type Other<T> is {value:T}; implement<T> Key for Other<T> {}
        public type Table<K:Key,V> is {value:V};
        public type Read<K> is protocol {type Output; fn read(&self,key:K)->&Self.Output;};
        implement<K:Key,V> Read<K> for Table<K,V> {
            type Output=V; fn read(&self,key:K)->&V {&self.value}
        }";
    let consumer = "mount demo.lib.Table;";
    let valid = check(consumer, PROVIDER);
    assert!(valid.is_empty(), "{valid:#?}");
    let missing = check(
        consumer,
        &PROVIDER.replace("implement<K:Key,V>", "implement<K,V>"),
    );
    assert!(
        missing.iter().any(|error| error.contains("Key") && error.contains("Other")),
        "{missing:#?}"
    );
}

#[test]
fn imported_marker_implementation_still_owes_its_receivers_domain() {
    let declarations = "public type Key is protocol {}; public type Table<K:Key> is {value:K};
        public type Mark is protocol {};";
    let consumer = "mount demo.lib.Table;";
    let valid = check(consumer, &format!("{declarations} implement<K:Key> Mark for Table<K> {{}}"));
    assert!(valid.is_empty(), "{valid:#?}");
    let invalid = check(consumer, &format!("{declarations} implement<K> Mark for Table<K> {{}}"));
    assert!(invalid.iter().any(|error| error.contains("Key")), "{invalid:#?}");
}
#[test]
fn explicit_dependency_mount_control() {
    let errors = check("mount demo.lib.Rows; mount demo.lib.Box;", PROVIDER);
    assert!(errors.is_empty(), "{errors:#?}");
}
#[test]
fn absent_dependency_implementation_refuses() {
    let provider = PROVIDER.replace("implement<T> Stream for Rows<T> { type Item = T; }", "");
    let errors = check("mount demo.lib.Rows; mount demo.lib.Box;", &provider);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("no implementation proves")),
        "{errors:#?}"
    );
}

#[test]
fn dependency_target_alone_must_not_emit_unrelated_premature_bound_failure() {
    let errors = check("mount demo.lib.Rows;", PROVIDER);
    assert!(errors.is_empty(), "{errors:#?}");
}
#[test]
fn dependent_target_before_dependency_succeeds() {
    let errors = check("mount demo.lib.Box; mount demo.lib.Rows;", PROVIDER);
    assert!(errors.is_empty(), "{errors:#?}");
}
#[test]
fn protocol_before_targets_succeeds() {
    let errors = check(
        "mount demo.lib.Stream; mount demo.lib.Rows; mount demo.lib.Box;",
        PROVIDER,
    );
    assert!(errors.is_empty(), "{errors:#?}");
}
#[test]
fn aliases_do_not_change_dependency_proof() {
    let errors = check(
        "mount demo.lib.Rows as Entries; mount demo.lib.Box as FactoryBox;",
        PROVIDER,
    );
    assert!(errors.is_empty(), "{errors:#?}");
}
#[test]
fn incompatible_dependency_implementation_refuses() {
    let provider = PROVIDER.replace("Stream for Rows<T>", "Stream for Rows<Int>");
    let errors = check("mount demo.lib.Rows; mount demo.lib.Box;", &provider);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("no implementation proves")
                || error.contains("cannot establish disjointness")),
        "{errors:#?}"
    );
}
#[test]
fn implementation_dependency_order_is_not_authority() {
    let provider = PROVIDER.replace(
        "implement<T> Stream for Rows<T> { type Item = T; }\nimplement<T> Factory for Box<T> { type Rows = Rows<T>; }",
        "implement<T> Factory for Box<T> { type Rows = Rows<T>; }\nimplement<T> Stream for Rows<T> { type Item = T; }",
    );
    let errors = check("mount demo.lib.Rows; mount demo.lib.Box;", &provider);
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn same_leaf_foreign_protocol_does_not_prove_required_owner() {
    let provider = format!(
        "mount demo.foreign.Stream as ForeignStream; {}",
        PROVIDER.replace("Stream for Rows<T>", "ForeignStream for Rows<T>")
    );
    let errors = check_with_extra(
        "mount demo.foreign.Stream as ForeignStream; mount demo.lib.Rows; mount demo.lib.Box;",
        &provider,
        Some("public type Stream is protocol { type Item; };"),
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("no implementation proves")),
        "{errors:#?}"
    );
}
#[test]
fn same_leaf_foreign_nominal_does_not_prove_required_receiver() {
    let provider = PROVIDER.replace("Stream for Rows<T>", "Stream for demo.foreign.Rows<T>");
    let errors = check_with_extra(
        "mount demo.lib.Rows; mount demo.lib.Box;",
        &provider,
        Some("public type Rows<T> is { value:T };"),
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("no implementation proves")),
        "{errors:#?}"
    );
}
#[test]
fn circular_result_obligation_has_no_independent_evidence() {
    let provider = r#"
    public type Stream is protocol { type Item; };
    public type Factory is protocol { type Rows: Stream; };
    public type Rows is { value: Int };
    public type Box is { value: Int };
    implement<T: Stream> Stream for T { type Item = Int; }
    implement Factory for Box { type Rows = Rows; }
    "#;
    let errors = check("mount demo.lib.Box;", provider);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("no implementation proves")
                || error.contains("recursive protocol obligation")),
        "{errors:#?}"
    );
}

#[test]
fn competing_source_implementation_is_not_hidden_by_lazy_loading() {
    let provider = format!("{PROVIDER} implement<T> Stream for Rows<T> {{ type Item = Int; }}");
    let errors = check("mount demo.lib.Box;", &provider);
    assert!(
        !errors.is_empty(),
        "competing overlapping implementation accepted"
    );
}

#[test]
fn decreasing_instantiated_implementation_obligations_keep_progress() {
    for provider in [
        DECREASING_PROVIDER.to_owned(),
        DECREASING_PROVIDER.replace("implement Stream for Leaf { type Item = Int; }", "")
            + "implement Stream for Leaf { type Item = Int; }",
    ] {
        for imports in [
            "mount demo.lib.Box;",
            "mount demo.lib.Wrap; mount demo.lib.Box;",
            "mount demo.lib.Leaf; mount demo.lib.Wrap; mount demo.lib.Box;",
        ] {
            let errors = check(imports, &provider);
            assert!(errors.is_empty(), "{imports}: {errors:#?}");
        }
    }
}

const DECREASING_PROVIDER: &str = r#"
    public type Stream is protocol { type Item; };
    public type Factory is protocol { type Rows: Stream; };
    public type Leaf is { value: Int };
    public type Wrap<T> is { value: T };
    public type Box is { value: Int };
    implement Stream for Leaf { type Item = Int; }
    implement<T: Stream> Stream for Wrap<T> { type Item = Int; }
    implement Factory for Box { type Rows = Wrap<Wrap<Leaf>>; }
    "#;

#[test]
fn decreasing_dependency_without_base_evidence_refuses() {
    let provider =
        DECREASING_PROVIDER.replace("implement Stream for Leaf { type Item = Int; }", "");
    let errors = check("mount demo.lib.Box;", &provider);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("no implementation proves")),
        "{errors:#?}"
    );
}

#[test]
fn declared_foreign_protocol_still_supplies_its_own_bound() {
    let provider = format!(
        "mount demo.foreign.Stream as ForeignStream; {}",
        PROVIDER
            .replace("Stream for Rows<T>", "ForeignStream for Rows<T>")
            .replace("type Rows: Stream;", "type Rows: ForeignStream;")
    );
    let errors = check_with_extra(
        "mount demo.foreign.Stream as ForeignStream; mount demo.lib.Rows; mount demo.lib.Box;",
        &provider,
        Some("public type Stream is protocol { type Item; };"),
    );
    assert!(errors.is_empty(), "{errors:#?}");
}
