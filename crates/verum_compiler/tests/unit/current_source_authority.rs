//! Current source imports, rather than stale registry files, own constructor policy.
use super::*;

fn check_current(source: &str, registered: Option<&str>) -> Result<(), List<Diagnostic>> {
    let parser = verum_fast_parser::FastParser::new();
    let alpha = parser.parse_module_str(
        "module alpha; public type Envelope<T> is Empty | Ready(T);",
        FileId::new(1),
    ).unwrap();
    let beta = parser.parse_module_str(
        "module beta; public type Envelope<T> is Empty | Ready { payload:T };",
        FileId::new(2),
    ).unwrap();
    let current = parser.parse_module_str(source, FileId::new(3)).unwrap();
    let registered = registered.map(|source| parser.parse_module_str(source, FileId::new(4)).unwrap());
    let mut entries = List::from_iter([
        ("alpha", "alpha.vr", &alpha), ("beta", "beta.vr", &beta),
    ]);
    if let Some(registered) = &registered { entries.push(("consumer", "consumer.vr", registered)); }
    let registry = source_registry(entries).unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    check_source_module(&mut checker, "consumer", &current, &registry)
}

const PUBLIC_CONSUMER: &str = "module consumer; mount alpha.Envelope;
    fn forward<T>(value:T)->T {value}
    fn make()->Envelope<Int> {forward(Envelope.Ready(73))}";
const PRIVATE_CONSUMER: &str = "module consumer; mount beta.Envelope;
    fn forward<T>(value:T)->T {value}
    fn make()->Envelope<Int> {forward(Envelope.Ready(73))}";

#[test]
fn imported_contextual_constructor_works_without_registering_the_consumer() {
    check_current(PUBLIC_CONSUMER, None).unwrap_or_else(|errors| panic!("{errors:?}"));
}

#[test]
fn the_current_mount_replaces_a_stale_homonymous_owner() {
    check_current(PUBLIC_CONSUMER, Some(PRIVATE_CONSUMER))
        .unwrap_or_else(|errors| panic!("{errors:?}"));
    let errors = check_current(PRIVATE_CONSUMER, Some(PUBLIC_CONSUMER))
        .expect_err("stale public payload authority replaced the current private owner");
    assert!(errors.iter().any(|error| error.code() == Some("E202")), "{errors:?}");
}

#[test]
fn a_removed_mount_is_not_recovered_from_the_registry() {
    let current = "module consumer; fn make()->Envelope<Int> {Envelope.Ready(73)}";
    let errors = check_current(current, Some(PUBLIC_CONSUMER))
        .expect_err("a stale file reintroduced its removed import");
    assert!(errors.iter().any(|error| error.code() == Some("E101")), "{errors:?}");
}

#[test]
fn a_local_type_cannot_borrow_a_stale_sum_constructor() {
    let current = "module consumer; type Envelope<T> is { payload:T };
        fn make()->Envelope<Int> {Envelope.Ready(73)}";
    assert!(check_current(current, Some(PUBLIC_CONSUMER)).is_err());
}
