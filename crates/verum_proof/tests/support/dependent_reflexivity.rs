use verum_proof::proof_checker::Term;

pub fn cases() -> Vec<(&'static str, Term, Term, bool)> {
    // A : U, x : A, y : A and distinct extra binders. The claimed equality
    // must continue to refer to the original A and x under the whole telescope.
    const EXTRA: usize = 48;
    let mut domains = vec![Term::universe(0), Term::var(0), Term::var(1)];
    domains.extend((0..EXTRA).map(|_| Term::universe(0)));
    let x = Term::var(EXTRA + 1);
    let y = Term::var(EXTRA);
    let carrier = Term::var(EXTRA + 2);
    let close = |body: Term, binder: fn(Term, Term) -> Term| {
        domains
            .iter()
            .rev()
            .fold(body, |body, ty| binder(ty.clone(), body))
    };
    let proof = close(Term::refl(x.clone()), Term::lam);
    [
        ("nested_reflexivity", carrier.clone(), x.clone(), true),
        ("nested_wrong_endpoint", carrier, y, false),
        ("nested_wrong_carrier", Term::universe(0), x.clone(), false),
    ]
    .into_iter()
    .map(|(name, ty, endpoint, accepted)| {
        (
            name,
            proof.clone(),
            close(Term::id_ty(ty, x.clone(), endpoint), Term::pi),
            accepted,
        )
    })
    .collect()
}
