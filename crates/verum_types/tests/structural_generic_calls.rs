//! Public checker accepts the structural argument parsed at the call site.
use verum_ast::ItemKind;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    for item in &ast.items {
        if let ItemKind::Type(decl) = &item.kind {
            checker
                .register_type_declaration(decl)
                .expect("type declaration");
        }
    }
    for item in &ast.items {
        if let ItemKind::Impl(decl) = &item.kind {
            checker.register_impl_block(decl).expect("impl declaration");
        }
    }
    for item in &ast.items {
        if let ItemKind::Function(function) = &item.kind {
            checker
                .register_function_signature(function)
                .expect("signature");
        }
    }
    let mut errors: List<_> = ast
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|e| Text::from(format!("{e:?}")))
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors
}

#[test]
fn structural_layout_calls_typecheck_from_source() {
    for ty in [
        "&Byte",
        "&mut Byte",
        "&checked Byte",
        "&unsafe Byte",
        "Byte",
    ] {
        let source =
            format!("fn size<T>() -> Int {{ T.size }} fn probe() -> Int {{ size<{ty}>() }}");
        assert!(
            errors(&source).is_empty(),
            "{source}: {:?}",
            errors(&source)
        );
    }
}

#[test]
fn structural_argument_rejects_an_unrelated_value() {
    for ty in ["Bool", "&Byte"] {
        let source = format!(
            "fn accept<T>(value: T) -> Int {{ 1 }} fn probe() -> Int {{ accept<{ty}>(\"wrong\") }}"
        );
        assert!(!errors(&source).is_empty(), "{source}");
    }
}

#[test]
fn array_type_slots_constrain_actual_argument_values() {
    for ty in ["[Byte; 3]", "[[Byte; 2]; 3]", "[Byte; 0]", "[Byte]"] {
        let source =
            format!("fn size<T>() -> Int {{ T.size }} fn probe() -> Int {{ size<{ty}>() }}");
        assert!(
            errors(&source).is_empty(),
            "{source}: {:?}",
            errors(&source)
        );
        let source = format!(
            "fn accept<T>(value: T) -> Int {{ 1 }} fn probe() -> Int {{ accept<{ty}>(\"wrong\") }}"
        );
        assert!(
            !errors(&source).is_empty(),
            "type slot must constrain array parameter: {source}"
        );
    }
}

#[test]
fn method_owner_shadow_and_non_type_slots_keep_declaration_order() {
    let source = r#"
type Factory<T> is { value: T };
implement<T> Factory<T> {
 fn choose<T>(self, value: T) -> T { value }
 fn original(self) -> T { self.value }
}
fn probe() -> Int {
 let factory: Factory<Int> = Factory { value: 7 };
 let result = factory.choose<[Byte; 3]>([1 as Byte, 2 as Byte, 3 as Byte]);
 factory.original()
}
"#;
    assert!(errors(source).is_empty(), "{:?}", errors(source));
    for declaration in ["const Shape: [Int], T", "Shape: meta [Int], T"] {
        let source = format!(
            "const N: Int = 2; const M: Int = 3; fn keep<{declaration}>(value: T) -> T {{value}} fn probe()->Bool {{keep<[N; M], Bool>(true)}}"
        );
        assert!(
            errors(&source).is_empty(),
            "{source}: {:?}",
            errors(&source)
        );
    }
}

#[test]
fn explicit_array_length_must_be_checked() {
    for argument in [
        "[Byte; -1]",
        "fn([Byte; -1]) -> Int",
        "fn() -> [Byte; 18446744073709551616]",
        "[Byte; 18446744073709551616]",
        "[Byte; 1 / 0]",
        "[3; 4]",
        "[3, 4]",
    ] {
        let source =
            format!("fn size<T>() -> Int {{T.size}} fn probe()->Int {{size<{argument}>()}}");
        assert!(
            !errors(&source).is_empty(),
            "invalid type argument accepted: {source}"
        );
    }
}

#[test]
fn inline_module_calls_bind_selected_declaration_slots() {
    for module in ["alpha", "alpha.beta"] {
        let declaration = if module == "alpha" {
            "module alpha { public fn choose<const Shape: [Int], T>(value: T)->T {value} }"
        } else {
            "module alpha { public module beta { public fn choose<const Shape: [Int], T>(value: T)->T {value} } }"
        };
        let good =
            format!("{declaration} fn probe()->Bool {{{module}.choose<[2; 3], Bool>(true)}}");
        assert!(errors(&good).is_empty(), "{good}: {:?}", errors(&good));
        let bad = good.replace(", Bool>", ", [Byte; 3]>");
        assert!(!errors(&bad).is_empty(), "wrong argument accepted: {bad}");
    }
}

#[test]
fn implicit_lifetime_and_partial_arguments_do_not_shift_type_slots() {
    for (declaration, arguments) in [
        ("'a, T", "'a, Bool"),
        ("{Unused}, T", "Bool"),
        ("const Shape: [Int], T, U", "[2; 3], Bool"),
    ] {
        let extra_parameter = if declaration.ends_with(", U") {
            ", extra: U"
        } else {
            ""
        };
        let extra_value = if declaration.ends_with(", U") {
            ", 7"
        } else {
            ""
        };
        let source = format!(
            "fn choose<{declaration}>(value:T{extra_parameter})->T {{value}} fn probe()->Bool {{choose<{arguments}>(true{extra_value})}}"
        );
        assert!(
            errors(&source).is_empty(),
            "{source}: {:?}",
            errors(&source)
        );
        let bad = source.replace("Bool>(true", "[Byte; 3]>(true");
        assert!(!errors(&bad).is_empty(), "wrong type slot accepted: {bad}");
    }
}

#[test]
fn named_concrete_lengths_and_element_aliases_are_checked_from_source() {
    let source = "const N: Int = 2; const M: Int = 3; type Element is Byte; fn size<T>()->Int {T.size} fn probe()->Int {size<[Element; N + M]>()}";
    assert!(errors(source).is_empty(), "{:?}", errors(source));
}
