//! Temporary observation borrows and stored payload loans have different ends.
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let module = FastParser::new()
        .parse_module_str(source, FileId::new(1))
        .unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("call_loans");
    let mut found = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            found.push(error);
        }
    }
    found.extend(checker.take_deferred_errors());
    found
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|error| error.to_string().into())
        .collect()
}

#[test]
fn a_discriminant_test_does_not_retain_its_unused_reference_payload() {
    let found = errors(
        r#"
        type Choice<T> is Empty | Full(T);
        type Store<T> is { public item:T };
        implement<T> Store<T> {
            fn inspect(&self)->Choice<&T> { Choice.Full(&self.item) }
            fn replace(&mut self,value:T) { self.item=value; }
        }
        fn change<T>(store:&mut Store<T>,value:T) {
            if store.inspect() is Choice.Empty { store.replace(value); }
        }
    "#,
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_generic_self_result_does_not_establish_input_independence() {
    let found = errors(
        r#"
        type Duplicate is protocol { fn duplicate(&self)->Self; };
        type Store<T> is { public item:T };
        implement<T> Store<T> { fn replace(&mut self,value:T) { self.item=value; } }
        type Holder<T> is { public store:Store<T> };
        fn change<T:Duplicate>(holder:&mut Holder<T>,value:T) {
            holder.store.replace(value.duplicate());
        }
    "#,
    );
    // SELF_BORROWING_RESULT below is a permitted implementation of this exact
    // contract. The generic caller cannot assume that the result is detached
    // from its local, by-value input merely because its declared result is Self.
    assert!(
        found
            .iter()
            .any(|error| error.contains("value") && (error.contains("borrow")
                || error.contains("lifetime") || error.contains("does not live long enough"))),
        "generic independence requires evidence: {found:?}"
    );
}

#[test]
fn a_reference_result_remains_borrowed_during_a_sibling_mutation() {
    let found = errors(
        r#"
        type Store is { public item:Int };
        implement Store {
            fn inspect(&self)->&Int { &self.item }
            fn replace(&mut self,value:&Int) { self.item=*value; }
        }
        fn invalid(store:&mut Store) { store.replace(store.inspect()); }
    "#,
    );
    assert!(
        found
            .iter()
            .any(|error| error.contains("borrow") && error.contains("store")),
        "{found:?}"
    );
}

const SELF_BORROWING_RESULT: &str = r#"
    type Duplicate is protocol { fn duplicate(&self)->Self; };
    type Link is End | Next(&Node);
    type Node is { public link:Link };
    implement Duplicate for Node {
        fn duplicate(&self)->Self { Node{link:Link.Next(self)} }
    }
"#;

#[test]
fn an_owned_self_result_can_retain_a_shared_receiver_borrow() {
    let found = errors(SELF_BORROWING_RESULT);
    assert!(
        found.is_empty(),
        "the actual implementation must be admitted: {found:?}"
    );
}

#[test]
fn a_self_borrowing_owned_result_cannot_escape_its_local_receiver() {
    let found = errors(&format!(
        "{SELF_BORROWING_RESULT}
        fn invalid()->Node {{
            let local=Node{{link:Link.End}};
            local.duplicate()
        }}"
    ));
    assert!(
        found
            .iter()
            .any(|error| error.contains("does not live long enough")
                || error.contains("cannot establish the lifetime")
                || (error.contains("local") && error.contains("borrow"))),
        "{found:?}"
    );
}

#[test]
fn a_selected_independent_result_does_not_borrow_its_receiver() {
    let found = errors(
        r#"
        type Choice<T> is Empty | Full(T);
        type Packet is { public link:Choice<&mut Int> };
        implement Packet {
            fn detached(&self)->Self { Packet{link:Choice.Empty} }
        }
        type Store<T> is { public item:T };
        implement<T> Store<T> { fn replace(&mut self,value:T) { self.item=value; } }
        fn change(store:&mut Store<Packet>,source:&Packet) {
            store.replace(source.detached());
        }
    "#,
    );
    assert!(
        found.is_empty(),
        "the selected result has no input payloads: {found:?}"
    );
}

#[test]
fn a_direct_field_transfer_retains_its_actual_input_loan() {
    let found = errors(
        r#"
        type Store<T> is { public item:T };
        implement<T> Store<T> { fn replace(&mut self,value:T) { self.item=value; } }
        fn invalid(input:&Int)->Store<&Int> {
            let mut store=Store{item:input};
            { let local=3; store.replace(&local); }
            store
        }
    "#,
    );
    assert!(
        found
            .iter()
            .any(|error| error.contains("does not live long enough")
                || error.contains("cannot establish the lifetime")),
        "{found:?}"
    );
}
