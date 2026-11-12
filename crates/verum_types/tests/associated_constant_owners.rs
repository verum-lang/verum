use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(body: &str) -> List<Text> {
    errors_with_module(body, None)
}

fn errors_with_module(body: &str, dependency: Option<(&str, &str)>) -> List<Text> {
    let source = format!("module measures;
        type Interval is (UInt64);
        implement Interval {{ public const ZERO: Interval = Interval(0_u64); }}
        {body}");
    let ast = Parser::new(&source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("measures");
    if let Some((owner, dependency)) = dependency {
        use verum_ast::FileId;
        use verum_modules::{ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
        let mut registry = ModuleRegistry::new();
        for (index, (owner, source, module)) in [
            (owner, dependency, Parser::new(dependency).parse_module().unwrap()),
            ("measures", source.as_str(), ast.clone()),
        ].into_iter().enumerate() {
            let id = ModuleId::new(index as u32);
            let path = ModulePath::from_str(owner);
            let mut info = ModuleInfo::new(id, path.clone(), module.clone(), FileId::new(index as u32), source.into());
            info.exports = extract_exports_from_module(&module, id, &path).unwrap();
            registry.register(info);
        }
        checker.set_module_registry_direct(registry);
    }
    let mut errors = checker.prepare_module_declarations(&ast);
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) { errors.push(error); }
    }
    errors.extend(checker.take_deferred_errors());
    let mut result: List<Text> = errors.iter().map(|error| error.to_string().into()).collect();
    result.extend(checker.diagnostics().iter().filter(|d| d.is_error()).map(|d| d.message().into()));
    result
}

#[test]
fn a_newtype_constant_uses_the_declaration_owner() {
    let found = errors("fn zero() -> Interval { Interval.ZERO }");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_constant_does_not_erase_its_declared_result_type() {
    assert!(!errors("fn wrong() -> Bool { Interval.ZERO }").is_empty());
    assert!(!errors("fn missing() -> Interval { Interval.ABSENT }").is_empty());
}

#[test]
fn a_lexical_value_keeps_its_own_fields() {
    let found = errors("type Value is { ZERO: Bool };
        fn read(Interval: Value) -> Bool { Interval.ZERO }");
    assert!(found.is_empty(), "{found:?}");
    assert!(!errors("fn wrong(Interval: Int) -> Interval { Interval.ZERO }").is_empty());
}

#[test]
fn imported_associated_values_keep_visibility_and_renamed_owners() {
    let declarations = "module core.items;
        public type Token is (Int);
        implement Token {
            public const LIMIT: Int = 7;
            const SECRET: Int = 9;
        }
        public fn own() -> Int { Token.SECRET }";
    let check = |body: &str| errors_with_module(
        &format!("mount core.items.{{Token as Imported}}; {body}"),
        Some(("core.items", declarations)),
    );
    let found = check("fn read() -> Int { Imported.LIMIT }");
    assert!(found.is_empty(), "{found:?}");
    let found = check("fn leak() -> Int { Imported.SECRET }");
    assert!(found.iter().any(|error| error.contains("SECRET") && error.contains("visibility")), "{found:?}");
    assert!(!check("fn wrong() -> Bool { Imported.LIMIT }").is_empty());
}

#[test]
fn named_and_glob_imports_obey_restricted_declaration_visibility() {
    for owner in ["measures.vault", "foreign.vault"] {
        let declaration = format!("module {owner};
            public(cog) fn inspect() -> Int {{ 7 }}
            public(cog) const LIMIT: Int = 7;
            fn hidden() -> Int {{ 9 }}");
        for mount in [format!("mount {owner}.{{inspect, LIMIT}};"), format!("mount {owner}.*;")] {
            let found = errors_with_module(
                &format!("{mount} fn read() -> Int {{ inspect() + LIMIT }}"),
                Some((owner, &declaration)),
            );
            assert_eq!(found.is_empty(), owner == "measures.vault", "{mount}: {found:?}");
            assert!(!errors_with_module(
                &format!("{mount} fn leak() -> Int {{ hidden() }}"),
                Some((owner, &declaration)),
            ).is_empty());
        }
    }
}
