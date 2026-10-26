use std::sync::Arc;
use verum_proof::proof_checker::{Context, Level, Term, infer};
use verum_proof::proof_checker_nbe::{self as nbe, Env, NbeContext, Neutral, Value, eval};

#[test]
fn captures_preserve_each_local_telescope_and_environment_subset() {
    let env: Env = (0..4)
        .map(|n| Arc::new(Value::VUniverse(Level::Concrete(n))))
        .collect();
    for local in 0..4 {
        for subset in 0..16 {
            let mut body = Term::universe(0);
            for index in 0..4 {
                if subset & (1 << index) != 0 {
                    body = Term::pair(body, Term::var(local + index));
                }
            }
            for index in 0..local {
                body = Term::pair(body, Term::var(index));
            }
            // Index domains carry the number of preceding local index binders.
            let mut indices = vec![Term::universe(0); local];
            indices.push(body.clone());
            let data = Term::Data {
                name: "Capture".into(),
                level: Level::Concrete(0),
                indices,
                constructors: vec![],
            };
            let Value::VData { indices, .. } = eval(&data, &env) else {
                panic!()
            };
            let closure = &indices[local];
            assert_eq!(closure.env.len(), (subset as u32).count_ones() as usize);
            let arguments: Env = (0..local)
                .map(|n| Arc::new(Value::VUniverse(Level::Concrete(10 + n as u32))))
                .collect();
            let mut full = env.clone();
            full.extend(arguments.iter().cloned());
            let mut compact = closure.env.clone();
            compact.extend(arguments);
            assert_eq!(eval(&body, &full), eval(&closure.body, &compact));
        }
    }
}

#[test]
fn closure_capture_does_not_bind_out_of_scope_variables() {
    let env = vec![Arc::new(Value::VUniverse(Level::Concrete(0)))];
    let term = Term::lam(Term::universe(0), Term::var(usize::MAX));
    let Value::VLam { body, .. } = eval(&term, &env) else {
        panic!()
    };
    assert!(body.env.is_empty());
    assert_eq!(*body.body, Term::var(usize::MAX));
    assert_eq!(
        body.apply(Value::VUniverse(Level::Concrete(1))),
        Value::VNeutral(Neutral::NVar(usize::MAX))
    );
}

#[test]
fn telescope_inference_preserves_dependent_domain_frames() {
    // A : U, B : A -> U, x : A, y : B x |- y : B x.
    let term = Term::lam(
        Term::universe(0),
        Term::lam(
            Term::pi(Term::var(0), Term::universe(0)),
            Term::lam(
                Term::var(1),
                Term::lam(Term::app(Term::var(1), Term::var(0)), Term::var(0)),
            ),
        ),
    );
    assert_eq!(
        infer(&Context::new(), &term).unwrap(),
        nbe::infer(&NbeContext::new(), &term).unwrap()
    );
    let invalid = Term::lam(
        Term::universe(0),
        Term::lam(Term::var(0), Term::lam(Term::var(0), Term::var(0))),
    );
    assert!(infer(&Context::new(), &invalid).is_err());
    assert!(nbe::infer(&NbeContext::new(), &invalid).is_err());
}
