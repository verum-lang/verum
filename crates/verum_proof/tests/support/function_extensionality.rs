use verum_proof::proof_checker::{Certificate, Level, Term};

pub fn general() -> Certificate {
    // A : U(u), B : A -> U(v), f g : (x:A) -> B(x),
    // p : (x:A) -> Id(B(x), f(x), g(x)). No finite domain assumption.
    let domains = [
        Term::universe_var("u"),
        Term::pi(Term::var(0), Term::universe_var("v")),
        Term::pi(Term::var(1), Term::app(Term::var(1), Term::var(0))),
        Term::pi(Term::var(2), Term::app(Term::var(2), Term::var(0))),
        Term::pi(
            Term::var(3),
            Term::id_ty(
                Term::app(Term::var(3), Term::var(0)),
                Term::app(Term::var(2), Term::var(0)),
                Term::app(Term::var(1), Term::var(0)),
            ),
        ),
    ];
    let function = Term::pi(Term::var(4), Term::app(Term::var(4), Term::var(0)));
    let term = Term::fun_ext(function.clone(), Term::var(2), Term::var(1), Term::var(0));
    let claim = Term::id_ty(function, Term::var(2), Term::var(1));
    Certificate {
        term: domains
            .iter()
            .rev()
            .cloned()
            .fold(term, |b, a| Term::lam(a, b)),
        claimed_type: domains
            .iter()
            .rev()
            .cloned()
            .fold(claim, |b, a| Term::pi(a, b)),
        metadata: Default::default(),
    }
}

pub fn body(term: &mut Term) -> &mut Term {
    match term {
        Term::Lam(_, next) => body(next),
        other => other,
    }
}

pub fn battery() -> Vec<(&'static str, Certificate, bool)> {
    let cert = general();
    let mut rows = vec![("dependent-universe-polymorphic", cert.clone(), true)];
    let mut reversed = cert.clone();
    if let Term::FunExt { lhs, rhs, .. } = body(&mut reversed.term) {
        std::mem::swap(lhs, rhs);
    }
    rows.push(("reversed-without-symmetry", reversed, false));
    let mut no_evidence = cert.clone();
    if let Term::FunExt { pointwise, .. } = body(&mut no_evidence.term) {
        **pointwise = Term::refl(Term::var(2));
    }
    rows.push(("reflexivity-is-not-pointwise-evidence", no_evidence, false));
    let mut wrong_family = cert.clone();
    if let Term::FunExt { function_type, .. } = body(&mut wrong_family.term) {
        **function_type = Term::pi(Term::var(4), Term::var(5));
    }
    rows.push(("different-codomain-family", wrong_family, false));
    let mut malformed = cert.clone();
    if let Term::FunExt { function_type, .. } = body(&mut malformed.term) {
        **function_type = Term::universe(0);
    }
    rows.push(("carrier-is-not-a-function", malformed, false));
    let mut overflow = cert.clone();
    if let Term::FunExt { function_type, .. } = body(&mut overflow.term) {
        **function_type = Term::pi(Term::Universe(Level::Concrete(u32::MAX)), Term::universe(0));
    }
    rows.push(("compound-universe-overflow", overflow, false));
    for (name, slot) in [
        ("free-function-type", 0),
        ("free-lhs", 1),
        ("free-rhs", 2),
        ("free-pointwise", 3),
    ] {
        let mut open = cert.clone();
        if let Term::FunExt {
            function_type,
            lhs,
            rhs,
            pointwise,
        } = body(&mut open.term)
        {
            **[function_type, lhs, rhs, pointwise][slot] = Term::var(5);
        }
        rows.push((name, open, false));
    }
    let mut beta = cert.clone();
    beta.term = Term::app(
        Term::lam(cert.claimed_type.clone(), Term::var(0)),
        cert.term.clone(),
    );
    rows.push(("checked-application", beta, true));
    let mut local = cert.clone();
    local.term = Term::Let {
        value: Box::new(cert.term.clone()),
        body: Box::new(Term::var(0)),
    };
    rows.push(("checked-local-definition", local, true));
    // Even unused evidence must be checked before let reduction erases it.
    rows.push((
        "unused-foundation-proof",
        Certificate {
            term: Term::Let {
                value: Box::new(cert.term),
                body: Box::new(Term::universe(0)),
            },
            claimed_type: Term::universe(1),
            metadata: Default::default(),
        },
        true,
    ));
    rows
}
