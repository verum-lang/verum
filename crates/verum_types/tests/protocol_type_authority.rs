//! Universal associated contracts must hold for every permitted family argument.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
use verum_types::{Type, protocol::{CoherenceMode, ProtocolImpl}};

fn errors(source: &str) -> List<Text> {
    errors_with_owner(source, Some("families"))
}

fn errors_with_owner(source: &str, owner: Option<&str>) -> List<Text> {
    let module = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    if let Some(owner) = owner {
        checker.set_current_module_path(owner);
    }
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&module)
        .iter()
        .map(|error| format!("{:?}", error.to_diagnostic()).into())
        .collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{:?}", error.to_diagnostic()).into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(|error| format!("{:?}", error.to_diagnostic()).into()),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|error| format!("{error:?}").into()),
    );
    errors
}

fn accept(source: &str) {
    let errors = errors(source);
    assert!(errors.is_empty(), "{source}\n{errors:#?}");
}
fn reject(source: &str) {
    let errors = errors(source);
    assert!(!errors.is_empty(), "unproved contract accepted: {source}");
}

fn heads(extra: &str, first: &str, second: Option<(&str, &str)>, query: &str) -> Text {
    let second = second
        .map(|(head, item)| format!("implement Stream for Cell<{head}>{{type Item={item};}}"))
        .unwrap_or_default();
    format!(
        r#"module families;
        type Stream is protocol{{type Item;}};
        type Factory is protocol{{type Rows:Stream<Item=Int>;}};
        type Cell<T> is{{value:T}};
        {extra}
        implement Stream for Cell<{first}>{{type Item=Int;}}
        {second}
        type Db is{{}}; implement Factory for Db{{type Rows=Cell<{query}>;}}
    "#
    )
    .into()
}
#[test]
fn distinct_sized_builtin_is_disjoint_from_int() {
    accept(&heads("", "Int", Some(("Int8", "Bool")), "Int"));
}

const BORROWED_RECEIVERS: &str = r#"module families;
    type Mark is protocol {};
    type Allowed is protocol {};
    type Bag<T> is { value: T };
    type Need<T: Mark> is { value: T };
    implement Allowed for Int {}
"#;

#[test]
fn borrowed_and_owned_implementations_prove_their_own_receivers() {
    for tier in ["&", "&checked ", "&unsafe "] {
        let declarations = format!(
            "{BORROWED_RECEIVERS}
            implement Mark for Float {{}}
            implement<T: Allowed> Mark for {tier}Bag<T> {{}}"
        );
        for receiver in ["Float".to_owned(), format!("{tier}Bag<Int>")] {
            accept(&format!("{declarations} fn probe(value: Need<{receiver}>) {{}}"));
        }
        for receiver in ["Bool".to_owned(), "Bag<Int>".to_owned(), format!("{tier}Bag<Bool>")] {
            reject(&format!("{declarations} fn probe(value: Need<{receiver}>) {{}}"));
        }
    }
}

#[test]
fn reference_aliases_preserve_the_actual_implementation_receiver() {
    let declarations = format!(
        "{BORROWED_RECEIVERS}
        type Borrow<T> is &Bag<T>;
        type Scalar is Float;
        implement<T> Mark for Borrow<T> {{}}"
    );
    accept(&format!(
        "{declarations} implement Mark for Float {{}}
        fn probe(scalar: Need<Scalar>, borrowed: Need<&Bag<Int>>) {{}}"
    ));
    reject(&format!("{declarations} fn probe(value: Need<Bag<Int>>) {{}}"));
    reject(&format!("{declarations} fn probe<T>(value: Need<T>) {{}}"));
    reject(&format!("{declarations} implement<T> Mark for &Bag<T> {{}}"));
}

#[test]
fn reference_tier_and_mutability_select_the_implementation() {
    let constraints = format!(
        "{BORROWED_RECEIVERS}
        implement Mark for Float {{}}
        implement Mark for &Float {{}}
        implement Mark for &checked mut Float {{}}"
    );
    accept(&format!(
        "{constraints} fn probe(a: Need<Float>, b: Need<&Float>, c: Need<&checked mut Float>) {{}}"
    ));
    for missing in ["&mut Float", "&checked Float", "&unsafe Float"] {
        reject(&format!("{constraints} fn probe(value: Need<{missing}>) {{}}"));
    }
}
#[test]
fn same_identity_int64_alias_uses_the_existing_implementation() {
    for (head, alias) in [
        ("Int", "Int64"),
        ("Float", "Float64"),
        ("Byte", "UInt8"),
        ("UInt64", "USize"),
    ] {
        accept(&heads("", head, None, alias));
    }
}
#[test]
fn duplicate_alias_heads_are_rejected_at_declaration_registration() {
    for (left, right) in [
        ("Int", "Int64"),
        ("UInt64", "USize"),
        ("Float", "Float64"),
        ("Byte", "UInt8"),
    ] {
        reject(&format!(
            "module families; type Stream is protocol{{}}; implement Stream for {left}{{}} implement Stream for {right}{{}}"
        ));
    }
}
#[test]
fn conflicting_int64_alias_implementation_is_not_disjoint() {
    reject(&heads("", "Int", Some(("Int64", "Bool")), "Int"));
}
#[test]
fn transparent_source_alias_cannot_hide_conflicting_equality() {
    for head in ["Int", "UInt64"] {
        reject(&heads(
            &format!("type Alias is {head};"),
            head,
            Some(("Alias", "Bool")),
            head,
        ));
    }
}
#[test]
fn declared_record_with_builtin_spelling_keeps_its_owner() {
    accept(&heads(
        "type Int8 is { value:Bool };",
        "Int",
        Some(("Int8", "Bool")),
        "Int",
    ));
}
#[test]
fn ordinary_declared_record_is_disjoint_from_int() {
    accept(&heads(
        "type Token is { value:Bool };",
        "Int",
        Some(("Token", "Bool")),
        "Int",
    ));
}

#[test]
fn declared_nominal_head_preserves_every_generic_and_explicit_premise() {
    let source = r#"module families;
        type Stream is protocol { type Item; };
        type Marker is protocol {};
        type Factory is protocol { type Rows: Stream; };
        type Wrap<T> is { value: T };
        type Db is {};
        implement Stream for Int { type Item = Int; }
        implement<T: Stream> Stream for Wrap<T> where T: Marker { type Item = T; }
        implement Factory for Db { type Rows = Wrap<Int>; }
    "#;
    reject(source);
    let with_witness = source.replace(
        "type Marker is protocol {};",
        "type Marker is protocol {}; implement Marker for Int {}",
    );
    accept(&with_witness);
    reject(&with_witness.replace("implement Stream for Int { type Item = Int; }", ""));
}

#[test]
fn a_bare_source_homonym_cannot_borrow_builtin_evidence() {
    let source = r#"
        type Int64 is { marker: Bool };
        type P is protocol {};
        type Factory is protocol { type Rows: P; };
        implement P for Int {}
        type Db is {};
        implement Factory for Db { type Rows = Int64; }
    "#;
    for owner in [None, Some("owner")] {
        assert!(
            !errors_with_owner(source, owner).is_empty(),
            "nominal homonym borrowed builtin evidence: {owner:?}"
        );
        let valid = source.replace("type Rows = Int64", "type Rows = Int");
        assert!(
            errors_with_owner(&valid, owner).is_empty(),
            "valid builtin: {owner:?}"
        );
    }
}

const ALIAS_HEAD: &str = r#"module families;
    type P is protocol {};
    type Factory is protocol { type Rows: P; };
    type Cell<T> is { value: T };
    type Alias<T> is Cell<T>;
    type Db is {};
"#;

#[test]
fn applied_alias_head_uses_the_declared_implementation() {
    accept(&format!(
        "{ALIAS_HEAD} implement P for Cell<Int> {{}} implement Factory for Db {{ type Rows = Alias<Int>; }}"
    ));
    accept(&format!(
        "{ALIAS_HEAD} implement P for Alias<Int> {{}} implement Factory for Db {{ type Rows = Cell<Int>; }}"
    ));
    reject(&format!(
        "{ALIAS_HEAD} implement P for Cell<Int> {{}} implement Factory for Db {{ type Rows = Alias<Bool>; }}"
    ));
}

#[test]
fn applied_alias_duplicates_refuse_in_both_orders() {
    let concrete = "implement P for Cell<Int> {}";
    let alias = "implement P for Alias<Int> {}";
    reject(&format!("{ALIAS_HEAD} {concrete} {alias}"));
    reject(&format!("{ALIAS_HEAD} {alias} {concrete}"));
}

#[test]
fn mixed_alias_telescope_preserves_value_identity_and_type_role() {
    let header = r#"module families;
        type P is protocol {};
        type Factory is protocol { type Rows: P; };
        type Sized<T, N: meta Int> is { value: T };
        type Alias<T, M: meta Int> is Sized<T, M>;
        type Db is {};
        implement P for Sized<Bool, 1> {}
    "#;
    accept(&format!(
        "{header} implement Factory for Db {{ type Rows = Alias<Bool, 1>; }}"
    ));
    reject(&format!(
        "{header} implement Factory for Db {{ type Rows = Alias<Bool, 2>; }}"
    ));
    reject(&format!(
        "{header} implement Factory for Db {{ type Rows = Alias<Int, 1>; }}"
    ));
    reject(&format!(
        "{header} implement Factory for Db {{ type Rows = Alias<Bool, Int>; }}"
    ));
    reject(&format!(
        "{header} implement Factory for Db {{ type Rows = Alias<Bool>; }}"
    ));
}

#[test]
fn a_bare_alias_to_a_record_does_not_create_a_new_nominal_head() {
    let header = "module families; type Original is { value: Int }; type Alias is Original; type P is protocol {};";
    for pair in [
        "implement P for Original {} implement P for Alias {}",
        "implement P for Alias {} implement P for Original {}",
    ] {
        reject(&format!("{header} {pair}"));
    }
    accept(&format!(
        "{header} type Factory is protocol {{ type Rows: P; }}; type Db is {{}}; implement P for Original {{}} implement Factory for Db {{ type Rows = Alias; }}"
    ));
}

#[test]
fn nested_generic_alias_pattern_uses_the_owned_telescope() {
    let source = r#"module families;
        type P is protocol {};
        type Factory is protocol { type Rows: P; };
        type Alias<T> is T;
        type Cell<T> is { value: T };
        type Db is {};
        implement<T> P for Cell<Alias<T>> {}
        implement Factory for Db { type Rows = Cell<Int>; }
    "#;
    accept(source);
}

#[test]
fn receiver_lookup_refuses_even_identical_malformed_alias_applications() {
    use verum_ast::{Ident, Path, Span};
    let source = "type Mark is protocol {}; type Cell<T> is { value: T }; type Alias<T> is Cell<T>; type Id<T> is T;";
    let ast = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("families");
    assert!(checker.prepare_module_declarations(&ast).is_empty());
    let mut registry = checker.protocol_checker.write();
    let owner = |name: &str| Path::single(Ident::new(name, Span::default()));
    let receiver = |name: &str, args: List<Type>| Type::Named { path: owner(name), args };
    let valid = receiver("families.Alias", [Type::Int].into_iter().collect());
    let invalid = receiver("families.Alias", List::new());
    let implementation = |for_type| ProtocolImpl {
        parameters: List::new(),
        protocol: owner("families.Mark"),
        protocol_args: List::new(),
        for_type,
        where_clauses: List::new(),
        methods: Default::default(),
        associated_types: Default::default(),
        associated_consts: Default::default(),
        specialization: None,
        impl_crate: None,
        span: Span::default(),
        type_param_fn_bounds: Default::default(),
    };
    // Isolate lookup from coherence: an index entry is not an applicability proof.
    registry.set_coherence_mode(CoherenceMode::Off);
    registry.register_impl(implementation(valid.clone())).unwrap();
    registry.register_impl(implementation(invalid.clone())).unwrap();
    let id = |inner| receiver("families.Id", [inner].into_iter().collect());
    for query in [id(id(valid.clone())), valid, receiver("families.Cell", [Type::Int].into_iter().collect())] {
        assert!(registry.find_impl_for_instantiation(&query, &owner("families.Mark"), &[]).is_some());
    }
    assert!(registry.find_impl_for_instantiation(&invalid, &owner("families.Mark"), &[]).is_none());
}


#[test]
fn nominal_tuple_heads_are_distinct_from_scalar_implementations() {
    for (declaration, head) in [
        ("type Token is (UInt64);", "Token"),
        ("type Token<T> is (T);", "Token<UInt64>"),
        ("type Token<T> is (T, T);", "Token<UInt64>"),
    ] {
        for scalar in ["Int", "Text"] {
            accept(&heads(declaration, scalar, Some((head, "Bool")), scalar));
            reject(&heads(declaration, scalar, Some((head, "Bool")), head));
        }
    }
}
