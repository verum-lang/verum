use super::*;
use crate::core_metadata::{CoreMetadata, TypeDescriptor, TypeDescriptorKind};
use std::sync::Arc;
use verum_common::ResourceDiscipline;

fn descriptor(name: &str, module: &str, origin: Maybe<Text>) -> TypeDescriptor {
    TypeDescriptor {
        name: name.into(),
        module_path: module.into(),
        origin_module_path: origin,
        generic_params: List::new(),
        kind: TypeDescriptorKind::Record {
            fields: List::new(),
        },
        size: Maybe::None,
        alignment: Maybe::None,
        methods: List::new(),
        implements: List::new(),
        decl_span: Maybe::None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
    }
}

fn scan(metadata: &CoreMetadata, name: &str) -> List<Text> {
    let mut modules: List<Text> = metadata
        .types
        .values()
        .filter(|d| d.name.as_str() == name)
        .map(|d| match &d.origin_module_path {
            Maybe::Some(origin) if !origin.is_empty() => origin.clone(),
            _ => d.module_path.clone(),
        })
        .filter(|m| !m.is_empty())
        .collect();
    modules.sort();
    modules.dedup();
    modules
}

#[test]
fn diagnostic_owners_preserve_origins_aliases_sorting_and_absence() {
    let mut metadata = CoreMetadata::default();
    for (key, module, origin) in [
        ("z.Widget", "bundle", Maybe::Some(Text::from("z.actual"))),
        (
            "alias.Renamed",
            "bundle",
            Maybe::Some(Text::from("z.actual")),
        ),
        ("a.Widget", "a", Maybe::None),
        ("legacy.Widget", "legacy", Maybe::Some(Text::new())),
        ("empty.Widget", "", Maybe::None),
    ] {
        metadata
            .types
            .insert(key.into(), descriptor("Widget", module, origin));
    }
    metadata.types.insert(
        "other.widget".into(),
        descriptor("widget", "other", Maybe::None),
    );
    let metadata = Arc::new(metadata);
    let mut checker = TypeChecker::new();
    checker.set_core_metadata(metadata.clone());
    for name in ["Widget", "widget", "Renamed", "Missing", "Widget"] {
        assert_eq!(
            checker.modules_declaring(name),
            scan(&metadata, name),
            "{name}"
        );
    }
    assert_eq!(
        checker.modules_declaring("Widget"),
        List::from_iter([
            Text::from("a"),
            Text::from("legacy"),
            Text::from("z.actual"),
        ])
    );
    let mut caller_copy = checker.modules_declaring("Widget");
    caller_copy.clear();
    assert_eq!(checker.modules_declaring("Widget").len(), 3);
}

#[test]
fn replacing_metadata_invalidates_positive_and_negative_suggestions() {
    let mut checker = TypeChecker::new();
    assert!(checker.modules_declaring("Widget").is_empty());
    let mut first = CoreMetadata::default();
    first.types.insert(
        "old.Widget".into(),
        descriptor("Widget", "old", Maybe::None),
    );
    checker.set_core_metadata(Arc::new(first));
    assert_eq!(
        checker.modules_declaring("Widget"),
        List::from_iter([Text::from("old")])
    );
    assert!(checker.modules_declaring("Added").is_empty());
    let mut second = CoreMetadata::default();
    second
        .types
        .insert("new.Added".into(), descriptor("Added", "new", Maybe::None));
    checker.set_core_metadata(Arc::new(second));
    assert!(checker.modules_declaring("Widget").is_empty());
    assert_eq!(
        checker.modules_declaring("Added"),
        List::from_iter([Text::from("new")])
    );
}

#[test]
fn repeated_diagnostic_queries_match_full_scan() {
    let mut metadata = CoreMetadata::default();
    for i in 0..4096 {
        let name = format!("Type{i}");
        let module = format!("package.module{}", i % 32);
        metadata.types.insert(
            format!("{module}.{name}").into(),
            descriptor(&name, &module, Maybe::None),
        );
    }
    let metadata = Arc::new(metadata);
    let mut checker = TypeChecker::new();
    checker.set_core_metadata(metadata.clone());
    let queries: List<Text> = (0..1024)
        .map(|i| Text::from(format!("Type{}", i * 5)))
        .collect();
    let started = std::time::Instant::now();
    let expected: List<List<Text>> = queries
        .iter()
        .map(|name| scan(&metadata, name.as_str()))
        .collect();
    let scan_elapsed = started.elapsed();
    let started = std::time::Instant::now();
    let actual: List<List<Text>> = queries
        .iter()
        .map(|name| checker.modules_declaring(name.as_str()))
        .collect();
    let index_elapsed = started.elapsed();
    assert_eq!(actual, expected);
    eprintln!(
        "4096 descriptors / 1024 queries, including first index build: scan={scan_elapsed:?}, index={index_elapsed:?}"
    );
}
