use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str, owner: Option<&str>) -> Vec<String> {
    let module = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    if let Some(owner) = owner {
        checker.set_current_module_path(owner);
    }
    let mut errors: Vec<_> = checker
        .prepare_module_declarations(&module)
        .iter()
        .map(ToString::to_string)
        .collect();
    for item in &module.items {
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
fn accepted(source: &str) {
    for owner in [None, Some("demo")] {
        let errors = errors(source, owner);
        assert!(errors.is_empty(), "{owner:?}: {source}\n{errors:#?}");
    }
}
fn refused(source: &str) {
    for owner in [None, Some("demo")] {
        let errors = errors(source, owner);
        assert!(
            errors.iter().any(|e| e.contains("E405")
                || e.contains("no implementation")
                || e.contains("protocol")
                || e.contains("Type mismatch")
                || e.contains("cannot establish disjointness")),
            "{owner:?}: accepted {source}\n{errors:#?}"
        );
    }
}
const DECL: &str =
    "type P is protocol {}; implement P for Int {} type Requires<T:P> is { value:T };";

#[test]
fn structural_implementation_receivers_use_their_own_premises() {
    let head = "type PairWith<R> is protocol { type Output; };";
    for receiver in ["&Requires<T>", "&mut Requires<T>", "(&Requires<T>, T)"] {
        let body = format!("PairWith<{receiver}> for {receiver} {{ type Output = Requires<T>; }}");
        accepted(&format!("{DECL} {head} implement<T: P> {body}"));
        refused(&format!("{DECL} {head} implement<T> {body}"));
    }
}
#[test]
fn concrete_field_argument_requires_its_bound() {
    refused(&format!("{DECL} type Token is {{value: Requires<Bool>}};"));
}
#[test]
fn concrete_field_argument_uses_existing_implementation() {
    accepted(&format!("{DECL} type Token is {{value: Requires<Int>}};"));
}
#[test]
fn recursive_field_is_not_its_own_evidence() {
    for name in ["Token", "Int64"] {
        refused(&format!(
            "{DECL} type {name} is {{value: Requires<{name}>}};"
        ));
    }
}
#[test]
fn owner_generic_premise_is_preserved() {
    accepted(&format!(
        "{DECL} type Outer<T:P> is {{ value: Requires<T> }};"
    ));
}
#[test]
fn generic_without_premise_is_rejected() {
    refused(&format!(
        "{DECL} type Outer<T> is {{ value: Requires<T> }};"
    ));
}
#[test]
fn alias_preserves_the_application_domain() {
    accepted(&format!(
        "{DECL} type Alias<T:P> is Requires<T>; type Good is {{ value:Alias<Int> }};"
    ));
    refused(&format!(
        "{DECL} type Alias<T:P> is Requires<T>; type Bad is {{ value:Alias<Bool> }};"
    ));
}
#[test]
fn function_annotation_checks_application() {
    refused(&format!(
        "{DECL} fn wrong(value:Requires<Bool>)->Int {{ 42 }}"
    ));
    accepted(&format!(
        "{DECL} fn right<T:P>(value:Requires<T>)->Int {{ 42 }}"
    ));
}
#[test]
fn own_where_premise_is_preserved() {
    accepted(&format!(
        "{DECL} type Outer<T> where type T:P is {{value:Requires<T>}};"
    ));
}
#[test]
fn callable_nominal_domain_is_preserved() {
    accepted(
        "type Runner<F: fn(Int)->Int> is {apply:F}; type Good is {runner:Runner<fn(Int)->Int>};",
    );
    accepted(
        "type Runner<F: fn(Int)->Int> is {apply:F}; type Wrap<F:fn(Int)->Int> is {runner:Runner<F>};",
    );
    refused(
        "type Runner<F: fn(Int)->Int> is {apply:F}; type Bad is {runner:Runner<fn(Bool)->Int>};",
    );
}
#[test]
fn callable_premise_can_prove_conditional_protocol() {
    accepted(
        "type P is protocol {}; implement<F:fn(Int)->Int> P for F {} type Requires<T:P> is {value:T}; type Wrap<F:fn(Int)->Int> is {value:Requires<F>};",
    );
    refused(
        "type P is protocol {}; implement<F:fn(Int)->Int> P for F {} type Requires<T:P> is {value:T}; type Wrap<F> is {value:Requires<F>};",
    );
}

#[test]
fn method_receiver_proves_its_declared_protocol_in_a_result_type() {
    accepted(
        "type Cursor is protocol { fn wrap(self)->View<Self>; }; type View<T:Cursor> is {value:T};",
    );
    for owner in [None, Some("demo")] {
        let found = errors(
            "type Cursor is protocol {}; type Other is protocol { fn wrap(self)->View<Self>; }; type View<T:Cursor> is {value:T};",
            owner,
        );
        assert!(
            found
                .iter()
                .any(|e| e.contains("no implementation proves") && e.contains("Cursor")),
            "{owner:?}: {found:#?}"
        );
    }
}

#[test]
fn method_callable_premise_cannot_escape_its_signature() {
    accepted(
        "type Handler<F:fn(Int)->Bool> is {f:F}; type Factory is protocol { fn handler<F:fn(Int)->Bool>(self,f:F)->Handler<F>; };",
    );
    for owner in [None, Some("demo")] {
        let found = errors(
            "type Handler<F:fn(Int)->Bool> is {f:F}; type Factory is protocol { fn first<F:fn(Int)->Bool>(self,f:F)->Handler<F>; fn second<F>(self,f:F)->Handler<F>; };",
            owner,
        );
        assert!(
            found
                .iter()
                .any(|e| e.contains("nominal callable argument lacks its declared contract")),
            "{owner:?}: {found:#?}"
        );
    }
}

#[test]
fn implementation_receiver_must_establish_the_nominal_key_domain() {
    const DECLS: &str = "type Key is protocol {}; type Hashable is protocol {};
        implement Key for Int {} implement Hashable for Int {}
        type Table<K:Key + Hashable,V> is {value:V};
        type ReadAt<K> is protocol {type Output; fn read(&self, key:K)->&Self.Output;};";
    const MEMBERS: &str = "type Output=V; fn read(&self, key:K)->&V {&self.value}";
    for header in [
        "<K:Key + Hashable,V> ReadAt<K> for Table<K,V>",
        "<K,V> ReadAt<K> for Table<K,V> where K:Key + Hashable",
    ] {
        accepted(&format!("{DECLS} implement{header} {{{MEMBERS}}}"));
    }
    refused(&format!(
        "{DECLS} implement<K:Key,V> ReadAt<K> for Table<K,V> {{{MEMBERS}}}"
    ));
}

#[test]
fn bounded_associated_output_checks_its_receiver_under_the_same_premises() {
    const DECLS: &str = "type Key is protocol {}; implement Key for Int {}
        type Table<K:Key> is {value:K}; type Read is protocol {type Output:Key;};";
    accepted(&format!("{DECLS} implement<K:Key> Read for Table<K> {{type Output=K;}}"));
    refused(&format!("{DECLS} implement<K:Key,V> Read for Table<K> {{type Output=V;}}"));
}

fn mounted(source: &str, provider: &str, foreign: Option<&str>) -> Vec<String> {
    use verum_ast::FileId;
    use verum_fast_parser::FastParser;
    use verum_modules::{
        ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
    };
    let mut registry = ModuleRegistry::new();
    let mut main = None;
    for (index, (owner, source)) in [("demo.lib", provider), ("demo.main", source)]
        .into_iter()
        .chain(foreign.map(|source| ("demo.other", source)))
        .enumerate()
    {
        let ast = FastParser::new()
            .parse_module_str(source, FileId::new(index as u32 + 1))
            .unwrap();
        let id = ModuleId::new(index as u32 + 1);
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, source.into());
        info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
        registry.register(info);
        if owner == "demo.main" {
            main = Some(ast);
        }
    }
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("demo.main");
    checker.set_current_cog("demo");
    checker.set_module_registry_direct(registry);
    check_module(&mut checker, &main.unwrap())
}
fn check_module(checker: &mut TypeChecker, module: &verum_ast::Module) -> Vec<String> {
    let mut errors: Vec<_> = checker
        .prepare_module_declarations(module)
        .iter()
        .map(ToString::to_string)
        .collect();
    for item in &module.items {
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
const PROVIDER: &str = "public type P is protocol {}; implement P for Int {} public type Requires<T:P> is {value:T}; public type Alias<T:P> is Requires<T>;";
#[test]
fn mounted_alias_keeps_defining_bound_and_implementation() {
    for name in ["Requires", "Alias"] {
        let positive = mounted(
            &format!("mount demo.lib.{name} as Imported; type Good is {{ value:Imported<Int> }};"),
            PROVIDER,
            None,
        );
        assert!(positive.is_empty(), "{name}: {positive:#?}");
        let negative = mounted(
            &format!("mount demo.lib.{name} as Imported; type Bad is {{ value:Imported<Bool> }};"),
            PROVIDER,
            None,
        );
        assert!(
            negative.iter().any(|e| e.contains("no implementation")),
            "{name}: {negative:#?}"
        );
    }
}
#[test]
fn sibling_protocol_cannot_prove_an_imported_domain() {
    let source = "mount demo.lib.Requires; mount demo.other.P; implement P for Bool {} type Bad is {value:Requires<Bool>};";
    let found = mounted(source, PROVIDER, Some("public type P is protocol {};"));
    assert!(
        found.iter().any(|e| e.contains("no implementation")),
        "{found:#?}"
    );
}

#[test]
fn imported_receiver_keeps_its_bounds_and_rejects_foreign_evidence() {
    let provider = "public type Required is protocol {}; public type Table<K:Required> is {key:K};";
    let consumer = "type Read is protocol {}; implement<K:Required> Read for Table<K> {}";
    let valid = mounted(
        &format!("mount demo.lib.{{Table,Required}}; {consumer}"),
        provider,
        None,
    );
    assert!(valid.is_empty(), "{valid:#?}");
    let invalid = mounted(
        &format!("mount demo.lib.Table; mount demo.other.Required; {consumer}"),
        provider,
        Some("public type Required is protocol {};"),
    );
    assert!(
        invalid
            .iter()
            .any(|e| e.contains("no implementation proves") && e.contains("Required")),
        "{invalid:#?}"
    );
}
#[test]
fn mounted_generic_uses_only_its_declared_owner_premises() {
    let good = mounted(
        "mount demo.lib.{Requires,P}; type Good<T:P> is {value:Requires<T>};",
        PROVIDER,
        None,
    );
    assert!(good.is_empty(), "{good:#?}");
    let bad = mounted(
        "mount demo.lib.Requires; mount demo.other.P; type Bad<T:P> is {value:Requires<T>};",
        PROVIDER,
        Some("public type P is protocol {};"),
    );
    assert!(
        bad.iter().any(|e| e.contains("no implementation")),
        "{bad:#?}"
    );
}

#[test]
fn callable_premise_retains_its_own_parameter_identity() {
    accepted(
        "type Element is Int; type Runner<A,F:fn(A)->Int> is {apply:F}; type Wrap<Element,F:fn(Element)->Int> is {runner:Runner<Element,F>};",
    );
    refused(
        "type Element is Int; type Runner<F:fn(Int)->Int> is {apply:F}; type Wrap<Element,F:fn(Element)->Int> is {runner:Runner<F>};",
    );
    accepted(
        "type Element is Int; type Runner<A,F:fn(A)->Int> is {apply:F}; fn wrap<Element,F:fn(Element)->Int>(input:Runner<Element,F>)->Int{42}",
    );
    refused(
        "type Element is Int; type Runner<F:fn(Int)->Int> is {apply:F}; fn wrap<Element,F:fn(Element)->Int>(input:Runner<F>)->Int{42}",
    );
}
