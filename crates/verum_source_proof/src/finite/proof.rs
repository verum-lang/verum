//! Proof-producing finite induction, independent of source elaboration.
//!
//! The supplied predicate is the actual closed, curried contract computation.
//! This module never evaluates it to decide whether the contract holds. Each
//! constructor case ends in reflexivity at `true`; the native checkers must
//! establish that the corresponding predicate application reduces to `true`.
use crate::scoped::Error;
use verum_proof::proof_checker::{self as kernel, Certificate, Context, Term};

const MAX_LEAVES: usize = 4096;
const MAX_PARAMETERS: usize = 64;

fn exhausted(message: &str) -> Error {
    Error::Inconclusive {
        code: "finite_case_limit",
        message: message.into(),
    }
}

fn constructors(domain: &Term) -> Result<usize, Error> {
    match domain {
        Term::Data {
            indices,
            constructors,
            ..
        } if indices.is_empty()
            && !constructors.is_empty()
            && constructors
                .iter()
                .all(|c| c.fields.is_empty() && c.indices.is_empty()) =>
        {
            Ok(constructors.len())
        }
        _ => Err(Error::Unsupported(
            "finite induction requires nonempty, unindexed data types with nullary constructors"
                .into(),
        )),
    }
}

fn value(domain: &Term, constructor: usize) -> Term {
    Term::Construct {
        data: Box::new(domain.clone()),
        constructor,
        args: Vec::new(),
    }
}

/// The predicate is shared in an outer `Let`. `depth` counts the intervening
/// term binders, while the new Pi telescope introduces `domains.len()` more.
fn goal(domains: &[Term], prefix: &[Term], boolean: &Term, depth: usize) -> Term {
    let mut application = Term::var(depth + domains.len());
    for argument in prefix {
        application = Term::app(application, argument.clone());
    }
    for index in (0..domains.len()).rev() {
        application = Term::app(application, Term::var(index));
    }
    domains.iter().rev().fold(
        Term::id_ty(boolean.clone(), application, value(boolean, 1)),
        |body, domain| Term::pi(domain.clone(), body),
    )
}

fn induction(
    domains: &[Term],
    arities: &[usize],
    prefix: &mut Vec<Term>,
    boolean: &Term,
    depth: usize,
) -> Term {
    let Some((domain, tail)) = domains.split_first() else {
        return Term::refl(value(boolean, 1));
    };
    // The motive replaces the currently scrutinized parameter with its own
    // binder. Converting Pi to Lam preserves exactly that binder's scope.
    let Term::Pi(motive_domain, motive_body) = goal(domains, prefix, boolean, depth + 1) else {
        unreachable!("nonempty domain telescope always has a Pi head")
    };
    let cases = (0..arities[0])
        .map(|constructor| {
            prefix.push(value(domain, constructor));
            let case = induction(tail, &arities[1..], prefix, boolean, depth + 1);
            prefix.pop();
            case
        })
        .collect();
    Term::lam(
        domain.clone(),
        Term::Induct {
            scrutinee: Box::new(Term::var(0)),
            motive: Box::new(Term::Lam(motive_domain, motive_body)),
            cases,
        },
    )
}

/// Construct `∀ inputs. predicate(inputs) = true` by complete finite induction.
///
/// Domains must be closed, nonempty, fieldless data declarations. The Boolean
/// carrier uses constructor zero for false and constructor one for true. The
/// returned certificate is a candidate, not a verification verdict: a false
/// contract produces a candidate which the checkers must reject.
pub fn certificate(
    domains: &[Term],
    predicate: &Term,
    boolean: &Term,
) -> Result<Certificate, Error> {
    if domains.len() > MAX_PARAMETERS {
        return Err(exhausted("finite induction exceeds its parameter budget"));
    }
    if constructors(boolean)? != 2 {
        return Err(Error::Unsupported(
            "the Boolean carrier must have exactly false and true constructors".into(),
        ));
    }
    let mut leaves = 1usize;
    let mut arities = Vec::with_capacity(domains.len());
    for domain in domains {
        let count = constructors(domain)?;
        leaves = leaves
            .checked_mul(count)
            .filter(|n| *n <= MAX_LEAVES)
            .ok_or_else(|| exhausted("finite induction exceeds 4096 constructor leaves"))?;
        arities.push(count);
    }
    // Check input typing and closure with the existing kernel, not a second
    // scope/type checker. This does not check that the predicate is true.
    let predicate_type = domains.iter().rev().fold(boolean.clone(), |body, domain| {
        Term::pi(domain.clone(), body)
    });
    kernel::check(&Context::new(), predicate, &predicate_type)?;

    // Share the exact predicate once in each artifact. Keeping its definition
    // outside the generated binders also prevents accidental variable capture.
    let shared = |body| Term::Let {
        value: Box::new(predicate.clone()),
        body: Box::new(body),
    };
    Ok(Certificate {
        term: shared(induction(domains, &arities, &mut Vec::new(), boolean, 0)),
        claimed_type: shared(goal(domains, &[], boolean, 0)),
        metadata: Default::default(),
    })
}
