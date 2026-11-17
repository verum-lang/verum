use verum_ast::Span;
use verum_fast_parser::FastParser;
use verum_types::{TypeChecker, source_files::SourceFileRegistry};

#[test]
fn rejected_fields_point_to_the_member_in_the_correct_source_file() {
    let files = SourceFileRegistry::new();
    for (body, code) in [
        ("fn probe<T>(value: T) -> Int { value.absent }", "E103"),
        ("fn probe(value: Bool) -> Int { value.absent }", "E103"),
        (
            "type Row is { count: Int }; fn probe(value: Row) -> Int { value.absent }",
            "E404",
        ),
    ] {
        let source = format!("// строка 🧮\n{body}");
        let owner = files.register_file(format!("{code}-{body}.vr"), &source);
        let module = FastParser::new().parse_module_str(&source, owner).unwrap();
        let mut checker = TypeChecker::new();
        checker.register_primitives();
        let mut errors = checker.prepare_module_declarations(&module);
        for item in &module.items {
            if let Err(error) = checker.check_item(item) {
                errors.push(error);
            }
        }
        errors.extend(checker.take_deferred_errors());
        let start = source.rfind("absent").unwrap();
        let target = Span::new(start as u32, (start + "absent".len()) as u32, owner);
        let expected = files.span_to_line_col(target);
        assert!(
            errors.iter().any(|error| {
                let diagnostic =
                    error.to_diagnostic_with_span(Some(|span| files.span_to_line_col(span)));
                diagnostic.code() == Some(code) && diagnostic.primary_span() == Some(&expected)
            }),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn private_field_diagnostic_keeps_the_access_and_declaring_identifiers_in_their_files() {
    use verum_ast::ItemKind;
    use verum_modules::{
        ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
    };

    let files = SourceFileRegistry::new();
    let provider =
        "public type Unused is { secret: Bool };\npublic type Packet is { /* 📦 */ secret: Int };";
    let consumer = "mount provider.Packet;\nfn read(p: Packet) -> Int { /* 🧭 */ p.secret }";
    let workspace = tempfile::tempdir().unwrap();
    let register = |name, source| {
        let path = workspace.path().join(name);
        std::fs::write(&path, source).unwrap();
        files.register_file(path.to_string_lossy(), source)
    };
    let provider_id = register("provider.vr", provider);
    let consumer_id = register("consumer.vr", consumer);
    let parser = FastParser::new();
    let dependency = parser.parse_module_str(provider, provider_id).unwrap();
    let module = parser.parse_module_str(consumer, consumer_id).unwrap();
    let owner = ModulePath::from_str("provider");
    let id = ModuleId::new(0);
    let exports = extract_exports_from_module(&dependency, id, &owner).unwrap();
    let mut info = ModuleInfo::new(id, owner, dependency, provider_id, provider.into());
    info.exports = exports;
    let mut registry = ModuleRegistry::new();
    registry.register(info);
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry.clone());
    let ItemKind::Mount(import) = &module.items[0].kind else {
        panic!("mount expected")
    };
    checker
        .process_import(import, "consumer", &registry)
        .unwrap();
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    let diagnostic = errors
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|error| error.to_diagnostic_with_span(Some(|span| files.span_to_line_col(span))))
        .find(|diagnostic| diagnostic.code() == Some("E202"))
        .unwrap_or_else(|| panic!("expected private field refusal: {errors:?}"));
    let expected = |source: &str, file| {
        let start = source.rfind("secret").unwrap() as u32;
        files.span_to_line_col(Span::new(start, start + 6, file))
    };
    assert_eq!(
        diagnostic.primary_span(),
        Some(&expected(consumer, consumer_id))
    );
    let related = diagnostic.secondary_labels();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].span, expected(provider, provider_id));
    let mut human = Vec::new();
    verum_diagnostics::Emitter::new(verum_diagnostics::EmitterConfig::no_color())
        .emit(&diagnostic, &mut human)
        .unwrap();
    let human = String::from_utf8(human).unwrap();
    assert!(human.contains("visibility declared here"), "{human}");
    for source in [provider, consumer] {
        assert!(human.contains(source.lines().nth(1).unwrap()), "{human}");
    }
    let mut encoded = Vec::new();
    let mut emitter = verum_diagnostics::Emitter::new(verum_diagnostics::EmitterConfig::json());
    emitter.emit(&diagnostic, &mut encoded).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    let spans = json["spans"].as_array().unwrap();
    assert_eq!(spans.len(), 2);
    for (index, (source, file, primary)) in [
        (consumer, consumer_id, true),
        (provider, provider_id, false),
    ]
    .into_iter()
    .enumerate()
    {
        let location = expected(source, file);
        let line = source.lines().nth(1).unwrap();
        let column = line.split_once("secret").unwrap().0.chars().count() + 1;
        assert_eq!(spans[index]["is_primary"], primary);
        assert_eq!(
            spans[index]["location"],
            serde_json::json!({
                "file":location.file, "line":2, "column":column, "length":6,
            })
        );
    }
}
