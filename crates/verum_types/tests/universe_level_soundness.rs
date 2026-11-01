use verum_common::{List, Map};
use verum_types::{
    ty::{Type, UniverseLevel as Level},
    universe_level::MAX_LEVEL_NODES,
};

fn assignment(a: u64, b: u64) -> Map<u32, u64> {
    [(0, a), (1000, b)].into_iter().collect()
}

#[test]
fn proposition_formation_agrees_with_the_type_hierarchy() {
    let mut context = verum_types::context::UniverseContext::new();
    assert_eq!(context.universe_of(&Type::Prop).unwrap(), Level::TYPE1);
    assert_eq!(Type::Prop.type_of(), Type::universe(Level::TYPE1));
    context.check_universe(&Type::Prop, Level::TYPE).unwrap();
    assert!(context.solve().is_err());
}

#[test]
fn successor_preserves_variable_identity_and_concrete_overflow() {
    let zero = Level::Variable(0);
    let unrelated = Level::Variable(1);
    assert_ne!(zero.succ().succ(), unrelated.succ());
    assert_eq!(zero.succ().succ().variables().as_slice(), &[0]);
    assert_eq!(zero.succ().succ().evaluate(&assignment(7, 5)), Some(9));
    let top = Level::Concrete(u32::MAX);
    assert_ne!(top.succ(), top);
    assert!(top.is_less_than(&top.succ()));
    assert_eq!(
        top.succ().succ().evaluate(&Map::new()),
        Some(u64::from(u32::MAX) + 2)
    );
    assert_eq!(
        Level::from_value(u64::from(u32::MAX) + 2)
            .unwrap()
            .evaluate(&Map::new()),
        Some(u64::from(u32::MAX) + 2)
    );
    assert!(Level::from_value(u64::MAX).is_none());
}

#[test]
fn max_preserves_all_symbolic_operands_and_never_reinterprets_ids() {
    let a = Level::Variable(0);
    let b = Level::Variable(1000);
    let c = Level::Variable(u32::MAX);
    let expression = a.max(&b.succ()).max(&c.succ().succ());
    assert_eq!(expression.variables().as_slice(), &[0, 1000, u32::MAX]);
    assert_eq!(expression.lower_bound(), 2);
    let mut values = assignment(5, 2);
    values.insert(u32::MAX, 11);
    assert_eq!(expression.evaluate(&values), Some(13));
    assert!(!a.is_less_or_equal(&b));
    assert!(!a.succ().is_less_or_equal(&b.succ()));
    assert!(!b.is_less_or_equal(&Level::Concrete(1000)));
    assert!(a.is_less_than(&a.succ()));
}

#[test]
fn max_and_successor_laws_hold_for_symbolic_levels() {
    let a = Level::Variable(0);
    let b = Level::Variable(1000);
    let c = Level::Concrete(3);
    assert!(a.max(&b).equivalent(&b.max(&a)));
    assert!(a.max(&b).max(&c).equivalent(&a.max(&b.max(&c))));
    assert!(a.max(&a).equivalent(&a));
    assert!(a.max(&b).succ().equivalent(&a.succ().max(&b.succ())));
    // A zero-valued variable plus one already guarantees a minimum of one.
    assert!(Level::TYPE1.is_less_or_equal(&a.succ()));
    assert!(!Level::TYPE2.is_less_or_equal(&a.succ()));
}

#[test]
fn every_small_accepted_order_holds_in_independent_natural_models() {
    let leaves = [
        Level::TYPE,
        Level::TYPE1,
        Level::Variable(0),
        Level::Variable(1000),
    ];
    let mut expressions: List<_> = leaves.iter().cloned().collect();
    for a in &leaves {
        expressions.push(a.succ());
        for b in &leaves {
            expressions.push(Level::Max(Box::new(a.clone()), Box::new(b.clone())));
            expressions.push(a.max(b).succ());
        }
    }
    // The independent evaluator reads the original tree, not its normal form.
    fn eval(level: &Level, a: u64, b: u64) -> u64 {
        match level {
            Level::Concrete(n) => u64::from(*n),
            Level::Variable(0) => a,
            Level::Variable(1000) => b,
            Level::Variable(_) => panic!("unknown fixture variable"),
            Level::Succ(inner) => eval(inner, a, b) + 1,
            Level::Max(left, right) => eval(left, a, b).max(eval(right, a, b)),
        }
    }
    for left in &expressions {
        for right in &expressions {
            let le = left.is_less_or_equal(right);
            let lt = left.is_less_than(right);
            for a in 0..=4 {
                for b in 0..=4 {
                    let l = eval(left, a, b);
                    let r = eval(right, a, b);
                    assert_eq!(left.evaluate(&assignment(a, b)), Some(l));
                    assert!(!le || l <= r, "{left:?} <= {right:?} at ({a},{b})");
                    assert!(!lt || l < r, "{left:?} < {right:?} at ({a},{b})");
                }
            }
        }
    }
}

#[test]
fn missing_assignments_overflow_and_budget_exhaustion_do_not_prove_a_level() {
    let a = Level::Variable(0);
    assert_eq!(a.evaluate(&Map::new()), None);
    assert_eq!(a.succ().evaluate(&assignment(u64::MAX, 0)), None);
    let mut too_deep = Level::TYPE;
    for _ in 0..MAX_LEVEL_NODES {
        too_deep = Level::Succ(Box::new(too_deep));
    }
    assert!(too_deep.normalize().is_none());
    assert!(!too_deep.is_less_or_equal(&too_deep));
    assert!(!too_deep.is_less_than(&too_deep));
}

#[test]
fn subtype_and_unification_cannot_invent_level_relationships() {
    use verum_ast::Span;
    use verum_types::{subtype::Subtyping, unify::Unifier};
    let variable = Type::universe(Level::Variable(0));
    let distinct = Type::universe(Level::Variable(1000));
    let concrete = Type::universe(Level::TYPE1);
    let subtyping = Subtyping::new();
    assert!(!subtyping.is_subtype(&variable, &distinct));
    assert!(!subtyping.is_subtype(&variable, &concrete));
    assert!(!subtyping.is_subtype(&Type::Prop, &variable));
    assert!(!subtyping.is_subtype(
        &Type::Prop,
        &Type::universe(Level::Variable(0).max(&Level::Variable(1000)))
    ));
    assert!(
        Unifier::new()
            .unify(&variable, &distinct, Span::dummy())
            .is_err()
    );
    assert!(
        Unifier::new()
            .unify(&variable, &concrete, Span::dummy())
            .is_err()
    );
    let first = Type::universe(Level::Variable(0).max(&Level::Variable(1000)));
    let second = Type::universe(Level::Variable(1000).max(&Level::Variable(0)));
    assert!(Unifier::new().unify(&first, &second, Span::dummy()).is_ok());
}
