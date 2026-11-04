use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;

fn errors(consumer: &str, providers: &[(&str, &str)]) -> List<Text> {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("demo");
    checker.set_current_module_path("demo.main");
    let mut registry = ModuleRegistry::new();
    for (index, (owner, source)) in providers
        .iter()
        .chain(std::iter::once(&("demo.main", consumer)))
        .enumerate()
    {
        let file_id = FileId::new(index as u32);
        let ast = FastParser::new()
            .parse_module_str(source, file_id)
            .expect("source grammar");
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let mut module = ModuleInfo::new(id, path.clone(), ast.clone(), file_id, (*source).into());
        module.exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        registry.register(module);
    }
    checker.set_module_registry_direct(registry.clone());
    let ast = FastParser::new()
        .parse_module_str(consumer, FileId::new(providers.len() as u32))
        .expect("consumer grammar");
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "demo.main", &registry) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
    for item in &ast.items {
        let result = match &item.kind {
            ItemKind::Type(decl) => checker.register_type_declaration(decl),
            ItemKind::Impl(decl) => checker.register_impl_block(decl),
            ItemKind::Function(decl) => checker.register_function_signature(decl),
            _ => continue,
        };
        if let Err(error) = result {
            errors.push(format!("{error:?}").into());
        }
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|e| e.is_error())
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors
}

fn acceptance(source: &str, expected: bool) {
    let found = errors(source, &[]);
    assert_eq!(found.is_empty(), expected, "{source}\n{found:?}");
}

#[test]
fn exact_value_argument_controls_protocol_satisfaction() {
    for (required, accepted) in [(1, true), (2, false)] {
        acceptance(
            &format!(
                r#"
            type Choice<N: meta Int> is protocol {{}};
            type Item is {{ id: Int }};
            implement Choice<1> for Item {{}}
            fn need<T: Choice<{required}>>(value: T) -> Int {{ 7 }}
            fn probe(value: Item) -> Int {{ need(value) }}
        "#
            ),
            accepted,
        );
    }
}

#[test]
fn mixed_type_and_value_slots_keep_their_positions() {
    for (args, accepted) in [("Int, 1", true), ("Int, 2", false), ("Text, 1", false)] {
        acceptance(
            &format!(
                r#"
            type Choice<T, N: meta Int> is protocol {{}};
            type Item is {{ id: Int }};
            implement Choice<Int, 1> for Item {{}}
            fn need<T: Choice<{args}>>(value: T) -> Int {{ 7 }}
            fn probe(value: Item) -> Int {{ need(value) }}
        "#
            ),
            accepted,
        );
    }
}

#[test]
fn declared_roles_reject_type_value_confusion() {
    for source in [
        "type P<T> is protocol {}; type Item is {id:Int}; implement P<1> for Item {}",
        "type P<N:meta Int> is protocol {}; type Item is {id:Int}; implement P<Int> for Item {}",
        "type P<N:meta Int> is protocol {}; type Item is {id:Int}; implement P<true> for Item {}",
    ] {
        acceptance(source, false);
    }
}

#[test]
fn positional_roster_requires_exact_arity() {
    for arguments in ["", "<1, 2>"] {
        acceptance(
            &format!(
                "type P<N:meta Int> is protocol {{}}; type Item is {{id:Int}}; implement P{arguments} for Item {{}}"
            ),
            false,
        );
    }
}

#[test]
fn type_defaults_are_substituted_in_declaration_order() {
    acceptance(
        r#"
        type P<A, B=A> is protocol {};
        type Item is {id:Int};
        implement P<Int> for Item {}
        fn need<T:P<Int,Int>>(value:T)->Int { 7 }
        fn probe(value:Item)->Int { need(value) }
    "#,
        true,
    );
}

#[test]
fn checked_integer_arguments_preserve_expression_value() {
    for (argument, accepted) in [("(1 + 2)", true), ("(2 + 2)", false)] {
        acceptance(
            &format!(
                r#"
            type P<N:meta Int> is protocol {{}};
            type Item is {{id:Int}};
            implement P<3> for Item {{}}
            fn need<T:P<{argument}>>(value:T)->Int {{7}}
            fn probe(value:Item)->Int {{need(value)}}
        "#
            ),
            accepted,
        );
    }
}

#[test]
fn unevaluated_or_overflowed_values_do_not_become_runtime_types() {
    for argument in ["(1 / 0)", "(9223372036854775807 + 1)", "source()", "1i32"] {
        acceptance(
            &format!(
                r#"
            fn source()->Int {{1}}
            type P<N:meta Int> is protocol {{}};
            type Item is {{id:Int}};
            implement P<{argument}> for Item {{}}
        "#
            ),
            false,
        );
    }
}

#[test]
fn different_value_instances_coexist_and_exact_repetition_conflicts() {
    for (second, accepted) in [(2, true), (1, false)] {
        acceptance(
            &format!(
                r#"
            type Choice<T, N:meta Int> is protocol {{}};
            type Item is {{id:Int}};
            implement Choice<Int,1> for Item {{}}
            implement Choice<Int,{second}> for Item {{}}
            fn need<T:Choice<Int,2>>(value:T)->Int {{7}}
            fn probe(value:Item)->Int {{need(value)}}
        "#
            ),
            accepted,
        );
    }
}

#[test]
fn inherited_self_default_uses_the_constrained_receiver() {
    for (required, accepted) in [("Item", true), ("Int", false)] {
        acceptance(
            &format!(
                r#"
            type P<Rhs=Self> is protocol {{}};
            type Item is {{id:Int}};
            implement P for Item {{}}
            fn need<T:P<{required}>>(value:T)->Int {{7}}
            fn probe(value:Item)->Int {{need(value)}}
        "#
            ),
            accepted,
        );
    }
    acceptance(
        r#"
        type P<Rhs=Self> is protocol {};
        type Item is {id:Int};
        implement P for Item {}
        fn need<T:P>(value:T)->Int {7}
        fn probe(value:Item)->Int {need(value)}
    "#,
        true,
    );
}

#[test]
fn invalid_application_in_a_bound_cannot_be_silently_removed() {
    for clause in ["<T:P<true>>", "<T> where T:P<true>"] {
        // The where form follows the return type in Verum.
        let source = if clause.contains("where") {
            "type P<N:meta Int> is protocol {}; fn f<T>(x:T)->Int where T:P<true> {7}".to_owned()
        } else {
            format!("type P<N:meta Int> is protocol {{}}; fn f{clause}(x:T)->Int {{7}}")
        };
        acceptance(&source, false);
    }
}

#[test]
fn symbolic_or_refined_values_require_their_own_evidence() {
    for source in [
        "type P<N:meta Int> is protocol {}; type Item is {id:Int}; implement<N:meta Int> P<N> for Item {}",
        "type P<N:meta Bool> is protocol {}; type Item is {id:Int}; implement P<true> for Item {}",
        "type P<T> is protocol {}; type Item is {id:Int}; implement<N:meta Int> P<N> for Item {}",
    ] {
        acceptance(source, false);
    }
}

#[test]
fn imported_default_uses_its_owner_not_callers_type_parameter() {
    let provider = "public type Alias is { id:Int }; public type Choice<T=Alias> is protocol {};";
    let source = r#"
        mount demo.provider.Choice;
        mount demo.provider.Alias as Original;
        type Item is {id:Int};
        implement Choice<Original> for Item {}
        fn need<Alias, T:Choice>(other:Alias,value:T)->Int {7}
        fn probe(value:Item)->Int {need(1,value)}
    "#;
    let found = errors(source, &[("demo.provider", provider)]);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn missing_owner_default_cannot_borrow_callers_generic() {
    let provider = "public type Choice<T=Absent> is protocol {};";
    let source = r#"
        mount demo.provider.Choice;
        type Item is {id:Int};
        implement Choice<Int> for Item {}
        fn need<Absent, T:Choice>(other:Absent,value:T)->Int {7}
    "#;
    let found = errors(source, &[("demo.provider", provider)]);
    assert!(
        !found.is_empty(),
        "missing owner default became caller variable"
    );
}

#[test]
fn compound_type_positions_cannot_borrow_a_value_binder() {
    for argument in ["(N,Int)", "&N", "[N]", "fn()->N", "Packet<N>"] {
        acceptance(
            &format!(
                r#"
            type P<T> is protocol {{}};
            type Packet<T> is {{value:T}};
            type Item is {{id:Int}};
            implement<N:meta Int> P<{argument}> for Item {{}}
        "#
            ),
            false,
        );
    }
}

#[test]
fn explicit_lifetime_arguments_require_represented_evidence() {
    for source in [
        "type P is protocol {}; type Item is {id:Int}; implement P<'a> for Item {}",
        "type P<'a> is protocol {}; type Item is {id:Int}; implement P<'a> for Item {}",
    ] {
        acceptance(source, false);
    }
}

#[test]
fn implementation_named_equality_cannot_disappear() {
    acceptance(
        r#"
        type P is protocol {};
        type Item is {id:Int};
        implement P<Ghost=Int> for Item {}
    "#,
        false,
    );
}

#[test]
fn nested_nominal_values_are_types_and_keep_exact_identity() {
    for (required, accepted) in [(3, true), (4, false)] {
        acceptance(
            &format!(
                r#"
            type Buffer<N:meta Int> is {{value:Int}};
            type P<T> is protocol {{}};
            type Item is {{id:Int}};
            implement P<Buffer<3>> for Item {{}}
            fn need<T:P<Buffer<{required}>>>(value:T)->Int {{7}}
            fn probe(value:Item)->Int {{need(value)}}
        "#
            ),
            accepted,
        );
    }
}

#[test]
fn nominal_alias_cannot_hide_a_value_in_an_ordinary_type_slot() {
    let found = errors(
        r#"
        type Packet<T> is {value:T};
        type Bad<N:meta Int> is Packet<N>;
        type P<T> is protocol {};
        type Item is {id:Int};
        implement P<Bad<3>> for Item {}
    "#,
        &[],
    );
    assert!(
        found.iter().any(|error| {
            error.contains("E405") && error.contains("value") && error.contains("type parameter")
        }),
        "{found:?}"
    );
}

#[test]
fn value_carrier_uses_preceding_declared_type_argument() {
    for (carrier, accepted) in [("Int", true), ("Bool", false)] {
        acceptance(
            &format!(
                r#"
            type Choice<T,N:meta T> is protocol {{}};
            type Item is {{id:Int}};
            implement Choice<{carrier},3> for Item {{}}
        "#
            ),
            accepted,
        );
    }
}

#[test]
fn imported_value_binder_cannot_become_a_type_parameter() {
    let provider = r#"
        public type P<T> is protocol {};
        public type Item is {id:Int};
        implement<N:meta Int> P<N> for Item {}
    "#;
    let source = r#"
        mount demo.provider.{P,Item};
        fn need<T:P<Int>>(value:T)->Int {7}
        fn probe(value:Item)->Int {need(value)}
    "#;
    let found = errors(source, &[("demo.provider", provider)]);
    assert!(
        found
            .iter()
            .any(|error| error.contains("type parameter") || error.contains("value")),
        "{found:?}"
    );
}

#[test]
fn function_value_binder_cannot_become_a_type_bound_argument() {
    acceptance(
        r#"
        type P<T> is protocol {};
        fn need<N:meta Int,T:P<N>>(value:T)->Int {7}
    "#,
        false,
    );
}
#[test]
fn mixed_alias_slots_preserve_exact_value_identity() {
    for (required, accepted) in [(3, true), (4, false)] {
        acceptance(
            &format!(
                r#"
            type Buffer<T,N:meta Int> is {{value:T}};
            type Sized<N:meta Int,T> is Buffer<T,N>;
            type P<T> is protocol {{}};
            type Item is {{id:Int}};
            implement P<Sized<3,Int>> for Item {{}}
            fn need<T:P<Buffer<Int,{required}>>>(value:T)->Int {{7}}
            fn probe(value:Item)->Int {{need(value)}}
        "#
            ),
            accepted,
        );
    }
}

#[test]
fn imported_meta_alias_uses_its_own_declared_slots() {
    let provider = r#"
        public type Buffer<T,N:meta Int> is {value:T};
        public type Sized<N:meta Int,T> is Buffer<T,N>;
    "#;
    for (required, accepted) in [(3, true), (4, false)] {
        let source = format!(
            r#"
            mount demo.provider.{{Buffer,Sized}};
            type P<T> is protocol {{}};
            type Item is {{id:Int}};
            implement P<Sized<3,Int>> for Item {{}}
            fn need<T:P<Buffer<Int,{required}>>>(value:T)->Int {{7}}
            fn probe(value:Item)->Int {{need(value)}}
        "#
        );
        let found = errors(&source, &[("demo.provider", provider)]);
        assert_eq!(found.is_empty(), accepted, "{source}\n{found:?}");
    }
}
