//! Exhaustion is operational failure, even for a well-typed, terminating term.
use super::*;
use proptest::prelude::*;
proptest! {
    #[test]
    fn exhausted_head_budget_never_returns_a_partial_normal_form(level in 0u32..10000) {
        let term=Term::app(Term::lam(Term::universe(level+1),Term::var(0)),Term::universe(level));
        let context=Context::new();
        let conversion=context.conversion();
        let expected=infer(&context,&term).unwrap();
        for fuel in 0..3 {
            prop_assert_eq!(conversion.whnf_fuel(term.clone(),fuel),Err(CheckError::FuelExhausted));
        }
        let (reduced,remaining)=conversion.whnf_fuel(term,3).unwrap();
        prop_assert_eq!(remaining,0);
        prop_assert_eq!(&reduced,&Term::universe(level));
        prop_assert_eq!(infer(&context,&reduced).unwrap(),expected);
    }
}
