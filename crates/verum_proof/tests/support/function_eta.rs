use verum_proof::proof_checker::Term;

/// A : U(u), B : A → U(v), c : A, g : A → (x : A) → B x.
/// The first argument of g may mention x before beta reduction.
pub fn cases() -> Vec<(&'static str, Term, Term, bool)> {
    let binders = [
        Term::universe_var("u"),
        Term::pi(Term::var(0), Term::universe_var("v")),
        Term::var(1),
        Term::pi(
            Term::var(2),
            Term::pi(Term::var(3), Term::app(Term::var(3), Term::var(0))),
        ),
    ];
    let close = |body: Term, bind: fn(Term, Term) -> Term| {
        binders
            .iter()
            .rev()
            .cloned()
            .fold(body, |body, domain| bind(domain, body))
    };
    let at_c = Term::app(Term::var(0), Term::var(1));
    let family = Term::pi(Term::var(3), Term::app(Term::var(3), Term::var(0)));
    let erased = Term::app(Term::lam(Term::var(4), Term::var(3)), Term::var(0));
    [
        ("eta_erased_dependency", erased.clone(), false, true),
        ("eta_erased_dependency_symmetric", erased, true, true),
        ("eta_live_dependency", Term::var(0), false, false),
        ("eta_live_dependency_symmetric", Term::var(0), true, false),
    ]
    .into_iter()
    .map(|(id, argument, reverse, accepted)| {
        let expanded = Term::lam(
            Term::var(3),
            Term::app(Term::app(Term::var(1), argument), Term::var(0)),
        );
        let (left, right) = if reverse {
            (expanded, at_c.clone())
        } else {
            (at_c.clone(), expanded)
        };
        let term = close(Term::refl(left.clone()), Term::lam);
        let claim = close(Term::id_ty(family.clone(), left, right), Term::pi);
        (id, term, claim, accepted)
    })
    .collect()
}
