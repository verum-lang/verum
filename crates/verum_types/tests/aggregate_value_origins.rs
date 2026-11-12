use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn check(function: &str) -> List<Text> {
    let source = format!(r#"
        module loans;
        type Number is {{ value: Int }};
        implement Number {{
            fn read(&self) -> Int {{ self.value }}
            fn borrow(&self) -> &Int {{ &self.value }}
        }}
        type Pair is {{ data: &Int, offset: Int }};
        type Packet is Empty | Read {{ data: &Int, offset: Int }};
        type Slot<T> is Empty | Some(T);
        fn number(value: &Number) -> Int {{ value.read() }}
        fn borrowed(value: &Number) -> &Int {{ value.borrow() }}
        fn identity<T>(value: T) -> T {{ value }}
        {function}
    "#);
    let module = FastParser::new().parse_module_str(&source, FileId::new(0)).unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("loans");
    checker.set_current_module_path("loans");
    let mut errors: List<Text> = checker.prepare_module_declarations(&module)
        .iter().map(|error| error.to_string().into()).collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) { errors.push(error.to_string().into()); }
    }
    errors.extend(checker.take_deferred_errors().iter().map(|error| error.to_string().into()));
    errors.extend(checker.diagnostics().iter().filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| format!("{diagnostic:?}").into()));
    errors
}

#[test]
fn scalar_method_results_do_not_retain_the_receiver_in_variants() {
    for value in ["local.read()", "number(&local)", "identity(local.read())"] {
        let errors = check(&format!(
            "fn wrap(data: &Int, local: Number) -> Packet {{ Packet.Read {{ data, offset: {value} }} }}"
        ));
        assert!(errors.is_empty(), "{value}: {errors:#?}");
    }
}

#[test]
fn scalar_calls_in_nested_aggregate_values_do_not_escape() {
    for (ty, value) in [
        ("Pair", "Pair { data, offset: local.read() }"),
        ("Slot<(&Int, Int)>", "Slot.Some((data, local.read()))"),
        ("(&Int, Int)", "(data, number(&local))"),
    ] {
        let errors = check(&format!("fn wrap(data: &Int, local: Number) -> {ty} {{ {value} }}"));
        assert!(errors.is_empty(), "{ty}: {errors:#?}");
    }
}

#[test]
fn reference_calls_and_aliases_still_retain_local_storage() {
    for reference in ["&local.value", "local.borrow()", "borrowed(&local)", "identity(&local.value)"] {
        for (ty, value) in [
            ("Pair", format!("Pair {{ data: {reference}, offset: local.read() }}")),
            ("Packet", format!("Packet.Read {{ data: {reference}, offset: local.read() }}")),
            ("Slot<(&Int, Int)>", format!("Slot.Some(({reference}, local.read()))")),
        ] {
            let errors = check(&format!("fn escape(local: Number) -> {ty} {{ {value} }}"));
            assert!(errors.iter().any(|error| error.contains("does not live long enough")),
                "{value}: {errors:#?}");
        }
    }
}

#[test]
fn a_real_reference_in_an_alias_cannot_hide_behind_a_scalar_field() {
    for signature in ["-> Packet", ""] {
        for output in ["Packet.Read { data: alias, offset: 0 }", "Packet.Read { data: alias, offset: local.read() }"] {
            let errors = check(&format!("fn escape(local: Number) {signature} {{ let alias = local.borrow(); {output} }}"));
            assert!(errors.iter().any(|error| error.contains("does not live long enough")), "{output}: {errors:#?}");
        }
    }
}
