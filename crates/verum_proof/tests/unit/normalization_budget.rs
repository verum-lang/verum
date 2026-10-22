//! A resource pause must preserve the term, including binder annotations.
use super::*;
use proptest::prelude::*;
proptest! {
    #[test]
    fn exhausted_head_budget_preserves_typed_application(level in 0u32..10000) {
        let term=Term::app(Term::lam(Term::universe(level+1),Term::var(0)),Term::universe(level));
        let expected=infer(&Context::new(),&term).unwrap();
        let (paused,remaining)=whnf_fuel(term.clone(),2);
        prop_assert_eq!(remaining,0);
        prop_assert_eq!(&paused,&term);
        prop_assert_eq!(infer(&Context::new(),&paused).unwrap(),expected.clone());
        let (reduced,_)=whnf_fuel(term,8);
        prop_assert_eq!(reduced,Term::universe(level));
        prop_assert_eq!(infer(&Context::new(),&Term::universe(level)).unwrap(),expected);
    }
}
