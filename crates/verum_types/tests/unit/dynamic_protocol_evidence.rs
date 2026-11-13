use super::*;
use crate::TypeChecker;
use verum_ast::{FileId, Ident, Path, PathSegment, Span};
use verum_fast_parser::FastParser;

fn path(name: &str) -> Path {
    Path::new(
        name.split('.')
            .map(|name| PathSegment::Name(Ident::new(name, Span::default())))
            .collect(),
        Span::default(),
    )
}

fn object(name: &str) -> Type {
    Type::DynProtocol {
        bounds: List::from_iter([name.into()]),
        bindings: Default::default(),
    }
}

fn registry(source: &str) -> ProtocolChecker {
    let module = FastParser::new()
        .parse_module_str(source, FileId::new(3))
        .unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("fixture");
    checker.set_current_module_path("fixture.owner");
    assert!(checker.prepare_module_declarations(&module).is_empty());
    for item in &module.items {
        checker.check_item(item).unwrap();
    }
    assert!(checker.take_deferred_errors().is_empty());
    assert!(checker.diagnostic_sources().is_empty());
    checker.protocol_checker.read().clone()
}

fn diamond(levels: usize) -> ProtocolChecker {
    let mut source = "type Base is protocol {};".to_owned();
    let mut parents = "Base".to_owned();
    for level in 0..levels {
        source.push_str(&format!(
            "type Left{level} is protocol extends {parents} {{}};
             type Right{level} is protocol extends {parents} {{}};"
        ));
        parents = format!("Left{level} + Right{level}");
    }
    source.push_str(&format!(
        "type Top is protocol extends {parents} {{}};
         fn require<T:Base>(value:&T) {{}}
         fn probe(value:&dyn Top) {{require(value);}}"
    ));
    registry(&source)
}

#[test]
fn diamond_checks_each_exact_completed_application_once() {
    let levels = 8;
    let checker = diamond(levels);
    let expected = 2 * levels + 2;
    let mut remaining = expected;
    let bounds = checker
        .dynamic_protocol_premises(&object("fixture.owner.Top"), &mut remaining)
        .unwrap();
    assert_eq!(bounds.len(), expected);
    assert_eq!(remaining, 0);
}

#[test]
fn dynamic_evidence_spends_the_callers_existing_obligation_budget() {
    let checker = diamond(3);
    let callables = Default::default();
    let mut obligations = Obligations::new(&checker, ConstraintEnvironment::new(&[], &callables));
    let receiver = object("fixture.owner.Top");
    let goal = ProtocolBound::simple(path("fixture.owner.Top"));
    obligations.remaining = 7;
    let error = obligations.assumption(&receiver, &goal).unwrap_err();
    assert!(error.contains("budget"), "{error}");
    assert_eq!(obligations.remaining, 0);
    let mut fresh = Obligations::new(&checker, ConstraintEnvironment::new(&[], &callables));
    fresh.remaining = 8;
    assert!(fresh.assumption(&receiver, &goal).unwrap());
    assert_eq!(fresh.remaining, 0);
}

#[test]
fn exact_application_memo_preserves_distinct_arguments_and_equalities() {
    let checker = registry(
        "type Parent<T> is protocol {type Item;};
         type Left is protocol extends Parent<Int, Item = Int> {};
         type Right is protocol extends Parent<Bool, Item = Bool> {};
         type Top is protocol extends Left + Right {};",
    );
    let receiver = object("fixture.owner.Top");
    let mut remaining = 5;
    let bounds = checker
        .dynamic_protocol_premises(&receiver, &mut remaining)
        .unwrap();
    assert_eq!(remaining, 0);
    assert_eq!(bounds.len(), 5);
    for argument in [Type::Int, Type::Bool] {
        let member = checker
            .resolve_associated_type_for_instantiation(
                &receiver,
                &path("fixture.owner.Parent"),
                std::slice::from_ref(&argument),
                &"Item".into(),
            )
            .unwrap();
        assert_eq!(member, argument);
    }
    assert!(
        checker
            .resolve_associated_type_for_instantiation(
                &receiver,
                &path("fixture.owner.Parent"),
                &[Type::Text],
                &"Item".into(),
            )
            .is_err()
    );
}

#[test]
fn active_cycle_is_refused_before_reusing_a_completed_branch() {
    let mut checker = registry(
        "type Base is protocol {}; type Valid is protocol extends Base {};
         type First is protocol {}; type Second is protocol extends First {};",
    );
    // Deliberately corrupt the already admitted registry to exercise this
    // proof boundary independently of declaration-registration rejection.
    checker
        .protocols
        .get_mut(&Text::from("fixture.owner.First"))
        .unwrap()
        .super_protocols
        .push(ProtocolBound::simple(path("fixture.owner.Second")));
    let receiver = Type::DynProtocol {
        bounds: List::from_iter(["fixture.owner.First".into(), "fixture.owner.Valid".into()]),
        bindings: Default::default(),
    };
    let mut remaining = PROTOCOL_OBLIGATION_BUDGET;
    let error = checker
        .dynamic_protocol_premises(&receiver, &mut remaining)
        .unwrap_err();
    assert!(error.contains("cycle"), "{error}");
    assert!(remaining > 0, "cycle detection consumed the entire budget");
    assert!(
        checker
            .check_universal_bounds(
                &receiver,
                &[ProtocolBound::simple(path("fixture.owner.Base"))],
                &[],
            )
            .is_err()
    );
}
