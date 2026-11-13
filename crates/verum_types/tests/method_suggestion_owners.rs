use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::{Type, TypeChecker};

fn errors(source: &str) -> List<Text> {
    checked(TypeChecker::new(), source)
}

fn checked(mut checker: TypeChecker, source: &str) -> List<Text> {
    let module = Parser::new(source).parse_module().expect("valid source");
    checker.register_primitives();
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    let mut messages: List<Text> = errors
        .iter()
        .map(|error| error.to_diagnostic().message().into())
        .collect();
    messages.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|error| error.is_error())
            .map(|error| error.message().into()),
    );
    messages
}

fn assert_hint(source: &str, typed: &str, wanted: Option<&str>) {
    assert_suggestion(errors(source), typed, wanted);
}

fn assert_suggestion(messages: List<Text>, typed: &str, wanted: Option<&str>) {
    assert!(!messages.is_empty(), "missing method was accepted");
    for message in messages {
        assert!(
            message.contains(&format!("no method named `{typed}`")),
            "{message}"
        );
        let suggestion = message.as_str().lines().find_map(|line| {
            line.trim()
                .strip_prefix("help: did you mean `")?
                .strip_suffix("`?")
        });
        assert_eq!(suggestion, wanted, "{message}");
    }
}

#[test]
fn a_receiver_name_contained_in_another_owner_cannot_supply_a_method() {
    let declarations = "type Store is { value:Int };
        type StoreAdapter is { value:Int };
        implement StoreAdapter { fn count(&self)->Int { self.value } }";
    assert_hint(
        &format!("{declarations} fn probe(s:Store)->Int {{ s.coun() }}"),
        "coun",
        None,
    );
    assert_hint(
        &format!("{declarations} fn probe(s:StoreAdapter)->Int {{ s.coun() }}"),
        "coun",
        Some("count"),
    );
    assert!(
        errors(&format!(
            "{declarations} fn probe(s:StoreAdapter)->Int {{ s.count() }}"
        ))
        .is_empty()
    );
}

#[test]
fn equal_distance_names_are_stable_in_both_registration_orders() {
    let a = "fn cat(&self)->Int { self.value }";
    let b = "fn cut(&self)->Int { self.value }";
    for (first, second) in [(a, b), (b, a)] {
        let source = format!(
            "type Store is {{ value:Int }};
            implement Store {{ {first} {second} }}
            fn probe(s:Store)->Int {{ s.cot() }}"
        );
        assert_hint(&source, "cot", Some("cat"));
    }
}

#[test]
fn a_static_factory_is_not_a_correction_for_an_instance_call() {
    let declarations = "type Store is { value:Int };
        implement Store { fn fresh()->Int { 1 } }";
    assert_hint(
        &format!("{declarations} fn probe(s:Store)->Int {{ s.fres() }}"),
        "fres",
        None,
    );
    assert!(
        errors(&format!(
            "{declarations} fn probe()->Int {{ Store.fresh() }}"
        ))
        .is_empty()
    );
}

#[test]
fn instance_suggestions_respect_specialized_receiver_arguments() {
    let declarations = "type Store<T> is { value:T };
        implement Store<Int> { fn count(&self)->Int { 1 } }";
    assert_hint(
        &format!("{declarations} fn probe(s:Store<Bool>)->Int {{ s.coun() }}"),
        "coun",
        None,
    );
    assert_hint(
        &format!("{declarations} fn probe(s:Store<Int>)->Int {{ s.coun() }}"),
        "coun",
        Some("count"),
    );
    assert!(
        errors(&format!(
            "{declarations} fn probe(s:Store<Int>)->Int {{ s.count() }}"
        ))
        .is_empty()
    );
}

#[test]
fn candidate_distance_counts_unicode_characters() {
    let source = "type Meter is { value:Int };
        implement Meter { fn измерить(&self)->Int { self.value } }
        fn probe(m:Meter)->Int { m.измерит() }";
    assert_hint(source, "измерит", Some("измерить"));
}

#[test]
fn a_private_imported_member_cannot_hide_a_visible_correction() {
    let owner = "realm.storage";
    let source = "public type Store is { public value:Int };
        implement Store {
            private fn cat(&self)->Int { self.value }
            public fn cut(&self)->Int { self.value }
        }";
    let ast = Parser::new(source)
        .parse_module()
        .expect("provider grammar");
    let id = ModuleId::new(0);
    let path = ModulePath::from_str(owner);
    let exports = extract_exports_from_module(&ast, id, &path).expect("provider exports");
    let mut module = ModuleInfo::new(id, path, ast, FileId::new(0), source.into());
    module.exports = exports;
    let mut registry = ModuleRegistry::new();
    registry.register(module);
    let mut checker = TypeChecker::new();
    checker.set_module_registry_direct(registry);
    checker.set_current_module_path("realm.consumer");
    let messages = checked(
        checker,
        "mount realm.storage.Store; fn probe(s:Store)->Int { s.cot() }",
    );
    assert_suggestion(messages, "cot", Some("cut"));
}

#[test]
fn registry_signatures_supply_only_the_receivers_instance_names() {
    use verum_types::core_integration::ProtocolCheckerExt;
    use verum_types::protocol::{MethodSignature, ProtocolChecker};
    let mut protocol = ProtocolChecker::new();
    protocol.register_method_public(
        "IntOther",
        MethodSignature::immutable("len", List::new(), Type::Int),
    );
    protocol.register_method_public(
        "Int",
        MethodSignature::static_method("lent", List::new(), Type::Int),
    );
    protocol.register_method_public(
        "Int",
        MethodSignature::immutable("length", List::new(), Type::Int),
    );
    let checker = TypeChecker::with_shared_methods_and_protocols(
        verum_common::Shared::new(parking_lot::RwLock::default()),
        verum_common::Shared::new(parking_lot::RwLock::new(protocol)),
    );
    let messages = checked(checker, "fn measure(item:Int)->Int { item.leng() }");
    assert_suggestion(messages, "leng", Some("length"));
}
