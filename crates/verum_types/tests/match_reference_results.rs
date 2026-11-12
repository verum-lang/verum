use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn check(body: &str) -> List<Text> {
    let source = format!(
        r#"
        module match_result;
        type Slot<T> is Empty | Occupied(T);
        type Receiver is {{ value: Int }};
        implement Receiver {{ fn recv(&mut self) -> Int {{ self.value }} }}
        type Stream is {{ receiver: Slot<Receiver>, counter: Int }};
        implement Stream {{
            fn clear(&mut self) {{ self.receiver = Slot.Empty; }}
            fn next(&mut self) -> Int {{ {body} }}
        }}
    "#
    );
    check_source(&source)
}

fn check_source(source: &str) -> List<Text> {
    let module = FastParser::new()
        .parse_module_str(&source, FileId::new(0))
        .unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("match_result");
    checker.set_current_module_path("match_result");
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&module)
        .into_iter()
        .map(|error| error.to_string().into())
        .collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error.to_string().into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| error.to_string().into()),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .map(|diagnostic| format!("{diagnostic:?}").into()),
    );
    errors
}

#[test]
fn mutable_variant_payload_outlives_its_pattern_scope() {
    let errors = check(
        r#"
        let receiver = match &mut self.receiver {
            Slot.Occupied(receiver) => receiver,
            Slot.Empty => return 0,
        };
        receiver.recv()
    "#,
    );
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn retained_mutable_payload_excludes_overwriting_the_variant() {
    for access in [
        "self.receiver = Slot.Empty;",
        "self.clear();",
        "let alias = &self.receiver;",
        "let alias = &mut self.receiver;",
    ] {
        let errors = check(
            r#"
        let receiver = match &mut self.receiver {
            Slot.Occupied(receiver) => receiver,
            Slot.Empty => return 0,
        };
        ACCESS
        receiver.recv()
    "#
            .replace("ACCESS", access)
            .as_str(),
        );
        assert!(
            errors.iter().any(|error| error.contains("borrow")),
            "{errors:#?}"
        );
    }
}

#[test]
fn shared_payload_cannot_acquire_mutable_access() {
    let errors = check(
        r#"
        let receiver = match &self.receiver {
            Slot.Occupied(receiver) => receiver,
            Slot.Empty => return 0,
        };
        receiver.recv()
    "#,
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("shared reference")),
        "{errors:#?}"
    );
}

#[test]
fn annotated_match_result_preserves_mutable_payload_access() {
    let errors = check(
        r#"
        let receiver: &mut Receiver = match &mut self.receiver {
            Slot.Occupied(receiver) => receiver,
            Slot.Empty => return 0,
        };
        receiver.recv()
    "#,
    );
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn escaped_payload_borrows_only_its_source_field() {
    let errors = check(
        r#"
        let receiver = match &mut self.receiver {
            Slot.Occupied(receiver) => receiver,
            Slot.Empty => return 0,
        };
        self.counter = 4;
        receiver.recv()
    "#,
    );
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn source_field_becomes_available_after_the_result_scope() {
    let errors = check(
        r#"
        let result = {
            let receiver = match &mut self.receiver {
                Slot.Occupied(receiver) => receiver,
                Slot.Empty => return 0,
            };
            receiver.recv()
        };
        self.receiver = Slot.Empty;
        result
    "#,
    );
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn all_branch_referents_remain_borrowed_until_the_result_last_use() {
    for (left, right) in [
        ("&mut left", "&mut right"),
        ("{ &mut left }", "{ &mut right }"),
    ] {
        for update in ["", "left = 3;", "right = 4;"] {
            let errors = check_source(&format!(
                r#"
            fn choose(flag: Bool) -> Int {{
                let mut left = 1;
                let mut right = 2;
                let chosen = match flag {{ true => {left}, false => {right} }};
                {update}
                *chosen
            }}
        "#
            ));
            if update.is_empty() {
                assert!(errors.is_empty(), "{errors:#?}");
            } else {
                assert!(
                    errors.iter().any(|error| error.contains("borrow")),
                    "{update}: {errors:#?}"
                );
            }
        }
    }
}

#[test]
fn same_referent_in_alternative_arms_does_not_conflict_with_itself() {
    let errors = check_source(
        r#"
        fn choose(flag: Bool) -> Int {
            let mut item = 1;
            let chosen = match flag { true => &mut item, false => &mut item };
            *chosen
        }
    "#,
    );
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn returning_a_branch_selected_local_reference_is_rejected() {
    for branches in [
        "true => input, false => &local",
        "true => &local, false => input",
    ] {
        let errors = check_source(&format!(
            r#"
            fn choose(flag: Bool, input: &Int) -> &Int {{
                let local = 2;
                let chosen = match flag {{ {branches} }};
                chosen
            }}
        "#
        ));
        assert!(errors.iter().any(|error| error.contains("local") &&
            error.contains("does not live long enough")), "{errors:#?}");
    }
}

#[test]
fn incoming_reference_can_escape_a_match_in_either_direction() {
    for body in [
        "match flag { true => left, false => right }",
        "let chosen = match flag { true => left, false => right }; chosen",
        "return match flag { true => left, false => right };",
    ] {
        let errors = check_source(&format!(
            "fn choose(flag: Bool, left: &Int, right: &Int) -> &Int {{ {body} }}"
        ));
        assert!(errors.is_empty(), "{body}: {errors:#?}");
    }
}

#[test]
fn mutable_local_cannot_escape_as_a_branch_selected_reference() {
    for tail in ["chosen", "return chosen;"] {
        let errors = check_source(&format!(
            r#"
            fn choose(flag: Bool, input: &mut Int) -> &mut Int {{
                let mut local = 2;
                let chosen = match flag {{ true => input, false => &mut local }};
                {tail}
            }}
        "#
        ));
        assert!(errors.iter().any(|error| error.contains("local") &&
            error.contains("does not live long enough")), "{errors:#?}");
    }
}

#[test]
fn a_multi_origin_holder_protects_all_sources_until_release() {
    use verum_ast::Span;
    use verum_types::aliasing::BorrowTracker;
    let mut loans = BorrowTracker::new();
    let first = loans.borrow_immut("first", Span::dummy()).unwrap();
    let second = loans.borrow_immut("second", Span::dummy()).unwrap();
    loans.register_holder("chosen", first);
    loans.register_holder("chosen", second);
    assert!(loans.borrow_mut_for_call("first", Span::dummy()).is_err());
    assert!(loans.borrow_mut_for_call("second", Span::dummy()).is_err());
    loans.release_borrow_at_last_use("chosen");
    assert!(loans.borrow_mut("first", Span::dummy()).is_ok());
    assert!(loans.borrow_mut("second", Span::dummy()).is_ok());
}

#[test]
fn leaving_a_nested_reader_preserves_a_held_field_loan() {
    use verum_ast::Span;
    use verum_types::aliasing::BorrowTracker;
    let mut loans = BorrowTracker::new();
    let outer = loans
        .borrow_field_immut("record", "field", Span::dummy())
        .unwrap();
    loans.register_holder("outer", outer);
    loans.enter_scope();
    loans
        .borrow_field_immut("record", "field", Span::dummy())
        .unwrap();
    loans.exit_scope();
    assert!(loans.borrow_mut_for_call("record", Span::dummy()).is_err());
    assert!(
        loans
            .borrow_field_mut("record", "field", Span::dummy())
            .is_err()
    );
    loans.release_borrow_at_last_use("outer");
    assert!(loans.borrow_mut_for_call("record", Span::dummy()).is_ok());
}
