use verum_proof::proof_checker::{Certificate, Term};

fn define(value: Term, body: Term) -> Term {
    Term::Let {
        value: Box::new(value),
        body: Box::new(body),
    }
}

// The binary function sits immediately outside the first argument.
// Each dᵢ₊₁ := f dᵢ dᵢ has type A. The checked source grows linearly.
fn repeated_application(steps: usize, mut body: Term) -> Term {
    for index in (0..steps).rev() {
        let function = Term::var(index + 1);
        body = define(
            Term::app(Term::app(function, Term::var(0)), Term::var(0)),
            body,
        );
    }
    body
}

fn shared_choice(steps: usize, body: Term) -> Term {
    let pick = Term::lam(Term::var(1), Term::lam(Term::var(2), Term::var(1)));
    define(
        pick,
        define(Term::var(1), repeated_application(steps, body)),
    )
}

fn close(body: Term) -> Certificate {
    Certificate {
        term: Term::lam(Term::universe(0), Term::lam(Term::var(0), body)),
        claimed_type: Term::pi(Term::universe(0), Term::pi(Term::var(0), Term::var(1))),
        metadata: Default::default(),
    }
}

// A : U, f : A → A → A, x : A. Applying a bound f cannot reduce.
fn bound_function(steps: usize, body: Term) -> Certificate {
    let binary = Term::pi(Term::var(0), Term::pi(Term::var(1), Term::var(2)));
    Certificate {
        term: Term::lam(
            Term::universe(0),
            Term::lam(
                binary.clone(),
                Term::lam(Term::var(1), repeated_application(steps, body)),
            ),
        ),
        claimed_type: Term::pi(
            Term::universe(0),
            Term::pi(binary, Term::pi(Term::var(1), Term::var(2))),
        ),
        metadata: Default::default(),
    }
}

pub fn cases() -> [(&'static str, Certificate, bool); 6] {
    // Projection forces inference of the whole local definition in both directions.
    let pair = shared_choice(26, Term::pair(Term::var(0), Term::universe(0)));
    [
        (
            "shared_local_choice",
            close(shared_choice(26, Term::var(0))),
            true,
        ),
        (
            "shared_local_choice_wrong_result",
            close(shared_choice(26, Term::universe(0))),
            false,
        ),
        (
            "shared_local_choice_first",
            close(Term::fst(pair.clone())),
            true,
        ),
        (
            "shared_local_choice_wrong_second",
            close(Term::snd(pair)),
            false,
        ),
        (
            "shared_neutral_result",
            bound_function(26, Term::var(0)),
            true,
        ),
        (
            "shared_neutral_wrong_result",
            bound_function(26, Term::universe(0)),
            false,
        ),
    ]
}
