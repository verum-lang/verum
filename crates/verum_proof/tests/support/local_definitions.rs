use verum_proof::proof_checker::{Certificate, Term};

fn define(value: Term, body: Term) -> Term {
    Term::Let {
        value: Box::new(value),
        body: Box::new(body),
    }
}

// A : U, x : A, pick := λ(a : A)(b : A). a, d₀ := x.
// Each dᵢ₊₁ := pick dᵢ dᵢ has type A. The checked source grows linearly.
fn shared_choice(steps: usize, mut body: Term) -> Term {
    for index in (0..steps).rev() {
        let pick = Term::var(index + 1);
        body = define(Term::app(Term::app(pick, Term::var(0)), Term::var(0)), body);
    }
    let pick = Term::lam(Term::var(1), Term::lam(Term::var(2), Term::var(1)));
    define(pick, define(Term::var(1), body))
}

fn close(body: Term) -> Certificate {
    Certificate {
        term: Term::lam(Term::universe(0), Term::lam(Term::var(0), body)),
        claimed_type: Term::pi(Term::universe(0), Term::pi(Term::var(0), Term::var(1))),
        metadata: Default::default(),
    }
}

pub fn cases() -> [(&'static str, Certificate, bool); 4] {
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
    ]
}
