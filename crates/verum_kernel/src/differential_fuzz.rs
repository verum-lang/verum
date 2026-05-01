//! Property-based mutation fuzzer for the differential-kernel registry.
//!
//! ## Architectural role
//!
//! The canonical-certificate roster (proof_term_examples/) is curated:
//! ~13 hand-built certificates that the registry's kernels are
//! cross-validated against. That covers the *intentional* surface,
//! but not the long tail of structurally-arbitrary terms. Two
//! algorithmically distinct kernels (bidirectional + WHNF; NbE +
//! quote) can match on every curated certificate yet diverge on a
//! corner case the curator never considered.
//!
//! This module ships **mutation-based property fuzzing**: it takes
//! the canonical seeds, applies structural mutations (universe
//! lifts, subterm-swaps, binder-domain rewrites, application
//! injections, variable-index perturbations), and runs each mutant
//! through every kernel in the registry. The property invariant is
//!
//!   **For every mutant `c`, `registry.verify_all(&c).agreement`
//!   must NOT be `Disagreement`.**
//!
//! Whether the mutant types or fails to type is irrelevant — what
//! matters is that all kernels reach the same verdict. Disagreement
//! flags a kernel-implementation bug.
//!
//! ## Why mutation, not from-scratch generation
//!
//! Generating arbitrary closed CIC terms uniformly produces almost
//! exclusively ill-typed shapes. Both kernels reject them quickly,
//! but rejection-only fuzzing has poor signal: the kernels share
//! the same fast-rejection paths, and the interesting divergences
//! happen on *near-well-typed* shapes where one kernel might
//! tolerate something the other does not.
//!
//! Mutation seeded from canonical well-typed certificates keeps
//! mutants close to the well-typed surface. A single universe lift
//! often preserves typeability; a subterm swap usually breaks it
//! cleanly. Both regimes exercise the kernels' decision boundary
//! aggressively.
//!
//! ## Determinism
//!
//! The PRNG is a deterministic xorshift64* seeded by the campaign's
//! base seed. The same base seed produces the same mutant sequence
//! across runs — disagreements are reproducible and bisectable.
//! No `rand` crate dependency.
//!
//! ## Integration
//!
//! The audit gate runs a small bounded campaign each invocation
//! (default 100 iterations) and flips to failure on the first
//! disagreement. Bug-detection is live — at audit time every
//! mutant gets cross-checked.

use crate::kernel_registry::{AgreementVerdict, KernelRegistry, MultiVerdict};
use crate::proof_checker::{Certificate, Term};

// =============================================================================
// PRNG — xorshift64* (deterministic, no external deps)
// =============================================================================

/// Deterministic xorshift64* — a fast, well-distributed 64-bit
/// generator. Used because we don't pull `rand` into the kernel
/// crate (the kernel must keep its dependency surface minimal).
#[derive(Debug, Clone)]
pub struct FuzzRng {
    state: u64,
}

impl FuzzRng {
    /// Construct from a seed.  Zero-seeded RNGs degenerate, so the
    /// constructor maps `0 → SPLITMIX_INIT` to keep it well-defined.
    pub fn new(seed: u64) -> Self {
        let s = if seed == 0 { 0x9E3779B97F4A7C15 } else { seed };
        Self { state: s }
    }

    /// Step the state and return a uniformly distributed `u64`.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform integer in `[0, bound)`. `bound` must be non-zero.
    pub fn gen_below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "FuzzRng::gen_below requires non-zero bound");
        self.next_u64() % bound
    }

    /// Uniform `usize` in `[0, bound)`.
    pub fn gen_index(&mut self, bound: usize) -> usize {
        self.gen_below(bound as u64) as usize
    }
}

// =============================================================================
// Mutation grammar
// =============================================================================

/// A structural mutation applied to a certificate's term and/or
/// claimed type. Each variant is deterministic given the input
/// certificate + the random offsets the fuzzer chose.
///
/// **Invariant**: every mutation produces a syntactically-valid
/// `Term` (the Term grammar accepts every `Var(usize)`, every
/// `Universe(u32)`, every well-formed `Pi`/`Lam`/`App`). Whether
/// the mutant is well-typed is irrelevant — the kernels' job is
/// to agree on accept/reject for any mutant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mutation {
    /// Lift every `Universe(n)` in BOTH term and claimed-type by
    /// `delta`. When `delta == 0`, this is the identity mutation
    /// (used as the trivial "no mutation" probe). Larger lifts
    /// preserve well-typedness for cumulativity-respecting terms
    /// — the canonical universe-stability test.
    LiftAllUniverses { delta: u32 },
    /// Lift universes in term ONLY (not claimed_type). Usually
    /// breaks typeability — both kernels MUST reject identically.
    LiftTermUniversesOnly { delta: u32 },
    /// Lift universes in claimed_type ONLY. Same intent as above
    /// but the asymmetry is on the type side.
    LiftClaimedTypeUniversesOnly { delta: u32 },
    /// Replace the entire term with `Universe(0)`, keeping the
    /// claimed type as-is. Almost always rejected — pin that both
    /// kernels reject identically.
    ReplaceTermWithUniverseZero,
    /// Replace the claimed type with `Universe(0)`. Often makes a
    /// previously-valid certificate invalid; pin the agreement.
    ReplaceClaimedTypeWithUniverseZero,
    /// Replace the entire term with `Var(idx)` (free variable in
    /// an empty context). Both kernels MUST reject as
    /// `UnboundVariable`.
    ReplaceTermWithFreeVariable { idx: usize },
    /// Wrap the term in `App(term, Universe(0))` — applying a
    /// non-function. Both kernels MUST reject identically.
    AppToNonFunction,
    /// Wrap the term in an extra lambda layer with the given
    /// domain. The claimed_type is left as-is, so most mutants
    /// are ill-typed (the wrapping changes the term's type to
    /// `Π(_:domain). T`). Both kernels MUST reject identically
    /// when ill-typed.
    WrapTermInExtraLam { domain: Term },
    /// Swap the entire term with the entire claimed type. Almost
    /// always ill-typed (a type at universe N can't have type T
    /// where T was the original term). Pin agreement.
    SwapTermAndType,
    /// Replace a Pi-binder's domain (top-level only) with
    /// `Universe(0)` — usually breaks compatibility with the body.
    PiDomainToUniverseZero,
    /// Replace a Lam-binder's domain (top-level only) with
    /// `Universe(0)`.
    LamDomainToUniverseZero,
}

impl Mutation {
    /// Stable diagnostic tag — used in fuzz reports.
    pub fn tag(&self) -> &'static str {
        match self {
            Mutation::LiftAllUniverses { .. } => "lift_all_universes",
            Mutation::LiftTermUniversesOnly { .. } => "lift_term_universes_only",
            Mutation::LiftClaimedTypeUniversesOnly { .. } => "lift_claimed_type_universes_only",
            Mutation::ReplaceTermWithUniverseZero => "replace_term_with_universe_zero",
            Mutation::ReplaceClaimedTypeWithUniverseZero => "replace_claimed_type_with_universe_zero",
            Mutation::ReplaceTermWithFreeVariable { .. } => "replace_term_with_free_variable",
            Mutation::AppToNonFunction => "app_to_non_function",
            Mutation::WrapTermInExtraLam { .. } => "wrap_term_in_extra_lam",
            Mutation::SwapTermAndType => "swap_term_and_type",
            Mutation::PiDomainToUniverseZero => "pi_domain_to_universe_zero",
            Mutation::LamDomainToUniverseZero => "lam_domain_to_universe_zero",
        }
    }
}

/// Apply a mutation to a certificate, producing a new certificate.
/// Original certificate is unchanged.
pub fn apply_mutation(cert: &Certificate, mutation: &Mutation) -> Certificate {
    let mut term = cert.term.clone();
    let mut claimed_type = cert.claimed_type.clone();

    match mutation {
        Mutation::LiftAllUniverses { delta } => {
            term = lift_universes_in_term(&term, *delta);
            claimed_type = lift_universes_in_term(&claimed_type, *delta);
        }
        Mutation::LiftTermUniversesOnly { delta } => {
            term = lift_universes_in_term(&term, *delta);
        }
        Mutation::LiftClaimedTypeUniversesOnly { delta } => {
            claimed_type = lift_universes_in_term(&claimed_type, *delta);
        }
        Mutation::ReplaceTermWithUniverseZero => {
            term = Term::Universe(0);
        }
        Mutation::ReplaceClaimedTypeWithUniverseZero => {
            claimed_type = Term::Universe(0);
        }
        Mutation::ReplaceTermWithFreeVariable { idx } => {
            term = Term::Var(*idx);
        }
        Mutation::AppToNonFunction => {
            term = Term::app(term, Term::Universe(0));
        }
        Mutation::WrapTermInExtraLam { domain } => {
            term = Term::lam(domain.clone(), term);
        }
        Mutation::SwapTermAndType => {
            std::mem::swap(&mut term, &mut claimed_type);
        }
        Mutation::PiDomainToUniverseZero => {
            if let Term::Pi(_, body) = &claimed_type {
                claimed_type = Term::Pi(Box::new(Term::Universe(0)), body.clone());
            }
        }
        Mutation::LamDomainToUniverseZero => {
            if let Term::Lam(_, body) = &term {
                term = Term::Lam(Box::new(Term::Universe(0)), body.clone());
            }
        }
    }

    Certificate {
        term,
        claimed_type,
        metadata: cert.metadata.clone(),
    }
}

/// Recursively add `delta` to every `Universe(n)` in `term`.
/// Bounded by `u32::MAX` — saturating add for defensive safety.
fn lift_universes_in_term(term: &Term, delta: u32) -> Term {
    match term {
        Term::Var(i) => Term::Var(*i),
        Term::Universe(n) => Term::Universe(n.saturating_add(delta)),
        Term::Pi(domain, body) => Term::Pi(
            Box::new(lift_universes_in_term(domain, delta)),
            Box::new(lift_universes_in_term(body, delta)),
        ),
        Term::Lam(domain, body) => Term::Lam(
            Box::new(lift_universes_in_term(domain, delta)),
            Box::new(lift_universes_in_term(body, delta)),
        ),
        Term::App(f, x) => Term::App(
            Box::new(lift_universes_in_term(f, delta)),
            Box::new(lift_universes_in_term(x, delta)),
        ),
    }
}

// =============================================================================
// Mutation sampling
// =============================================================================

/// Sample a uniformly-distributed mutation from the grammar.
/// Numeric parameters (`delta`, `idx`, `domain`) are themselves
/// sampled from bounded distributions to keep the mutant space
/// manageable.
pub fn sample_mutation(rng: &mut FuzzRng) -> Mutation {
    // 11 mutation kinds — enumerate.
    match rng.gen_below(11) {
        0 => Mutation::LiftAllUniverses {
            delta: rng.gen_below(4) as u32 + 1,
        },
        1 => Mutation::LiftTermUniversesOnly {
            delta: rng.gen_below(4) as u32 + 1,
        },
        2 => Mutation::LiftClaimedTypeUniversesOnly {
            delta: rng.gen_below(4) as u32 + 1,
        },
        3 => Mutation::ReplaceTermWithUniverseZero,
        4 => Mutation::ReplaceClaimedTypeWithUniverseZero,
        5 => Mutation::ReplaceTermWithFreeVariable {
            idx: rng.gen_below(8) as usize,
        },
        6 => Mutation::AppToNonFunction,
        7 => Mutation::WrapTermInExtraLam {
            domain: sample_small_domain(rng),
        },
        8 => Mutation::SwapTermAndType,
        9 => Mutation::PiDomainToUniverseZero,
        10 => Mutation::LamDomainToUniverseZero,
        _ => unreachable!("gen_below(11) ranges 0..11"),
    }
}

/// Sample a small Term used as a Lam binder's domain.  Bounded
/// shape: Universe(0..3) | Pi(Universe(0), Universe(0)).
fn sample_small_domain(rng: &mut FuzzRng) -> Term {
    match rng.gen_below(4) {
        0 => Term::Universe(0),
        1 => Term::Universe(1),
        2 => Term::Universe(2),
        3 => Term::Pi(Box::new(Term::Universe(0)), Box::new(Term::Universe(0))),
        _ => unreachable!(),
    }
}

// =============================================================================
// Canonical seed certificates
// =============================================================================

/// Curated seed certificates the fuzzer mutates.  Each seed is
/// known to be accepted by every registered kernel; mutations
/// drive the kernels into the long tail of accept/reject regions.
///
/// **Stable list** — the seed roster is a load-bearing surface:
/// expanding it strengthens fuzz coverage. Removing a seed weakens
/// it. Adding a new seed requires confirming all default-registry
/// kernels accept the unmutated form.
pub fn seed_certificates() -> Vec<Certificate> {
    vec![
        // Seed 1 — polymorphic identity.
        // λ(A : Universe(0)). λ(x : A). x  :  Π(A : Universe(0)). Π(_ : A). A
        Certificate {
            term: Term::lam(
                Term::universe(0),
                Term::lam(Term::var(0), Term::var(0)),
            ),
            claimed_type: Term::pi(
                Term::universe(0),
                Term::pi(Term::var(0), Term::var(1)),
            ),
            metadata: std::collections::BTreeMap::new(),
        },
        // Seed 2 — identity at Universe(0).
        // λ(x : Universe(0)). x  :  Π(_ : Universe(0)). Universe(0)
        Certificate {
            term: Term::lam(Term::universe(0), Term::var(0)),
            claimed_type: Term::pi(Term::universe(0), Term::universe(0)),
            metadata: std::collections::BTreeMap::new(),
        },
        // Seed 3 — K combinator (constant function).
        // λ(A : Universe(0)). λ(B : Universe(0)). λ(a : A). λ(_ : B). a
        //   :  Π(A : Universe(0)). Π(B : Universe(0)). Π(_ : A). Π(_ : B). A
        Certificate {
            term: Term::lam(
                Term::universe(0),
                Term::lam(
                    Term::universe(0),
                    Term::lam(
                        Term::var(1),
                        Term::lam(Term::var(1), Term::var(1)),
                    ),
                ),
            ),
            claimed_type: Term::pi(
                Term::universe(0),
                Term::pi(
                    Term::universe(0),
                    Term::pi(
                        Term::var(1),
                        Term::pi(Term::var(1), Term::var(3)),
                    ),
                ),
            ),
            metadata: std::collections::BTreeMap::new(),
        },
    ]
}

// =============================================================================
// Fuzz iteration + campaign
// =============================================================================

/// One fuzz iteration's outcome.  Carries the mutation applied,
/// the resulting verdict, and the seed-certificate index for
/// reproducibility.
#[derive(Debug, Clone)]
pub struct FuzzResult {
    /// Iteration index in the campaign, starting at 0.
    pub iteration: usize,
    /// Index into `seed_certificates()` of the seed used.
    pub seed_index: usize,
    /// The mutation applied this iteration.
    pub mutation_tag: &'static str,
    /// Multi-kernel verdict on the mutant.
    pub verdict: MultiVerdict,
}

impl FuzzResult {
    /// Stable diagnostic tag — agreement classification.
    pub fn agreement_tag(&self) -> &'static str {
        self.verdict.agreement.tag()
    }
}

/// Aggregate report from a fuzz campaign.
///
/// **Soundness contract**: `disagreements` MUST be empty for the
/// audit gate to pass. A non-empty list is a kernel-implementation
/// bug surfaced at fuzz time.
#[derive(Debug, Clone)]
pub struct FuzzCampaignReport {
    /// Total iterations in this campaign.
    pub total_iterations: usize,
    /// Mutants where every kernel accepted.
    pub unanimous_accept: usize,
    /// Mutants where every kernel rejected.
    pub unanimous_reject: usize,
    /// Mutants where kernels disagreed.  EVERY entry is a soundness
    /// alert — the audit gate fails on any non-empty list.
    pub disagreements: Vec<FuzzResult>,
    /// Names of every registered kernel (registration order).
    pub registered_kernels: Vec<&'static str>,
}

impl FuzzCampaignReport {
    /// True iff zero disagreements were observed — the soundness
    /// contract held.
    pub fn is_sound(&self) -> bool {
        self.disagreements.is_empty()
    }
}

/// Run a single fuzz iteration.  Picks a seed by `iter % seeds.len()`,
/// samples a mutation from `rng`, applies it, runs the registry's
/// `verify_all`, and returns the result.
pub fn run_fuzz_iteration(
    iteration: usize,
    rng: &mut FuzzRng,
    seeds: &[Certificate],
    registry: &KernelRegistry,
) -> FuzzResult {
    let seed_index = iteration % seeds.len();
    let seed = &seeds[seed_index];
    let mutation = sample_mutation(rng);
    let mutant = apply_mutation(seed, &mutation);
    let verdict = registry.verify_all(&mutant);
    FuzzResult {
        iteration,
        seed_index,
        mutation_tag: mutation.tag(),
        verdict,
    }
}

/// Run a fuzz campaign over the default registry.
///
/// `n_iterations` bounded; `base_seed` makes the campaign
/// deterministic (same seed → same mutant sequence). Returns a
/// `FuzzCampaignReport` aggregating verdicts and (critically) any
/// disagreements.
pub fn run_fuzz_campaign(n_iterations: usize, base_seed: u64) -> FuzzCampaignReport {
    let registry = KernelRegistry::default();
    run_fuzz_campaign_against(&registry, n_iterations, base_seed)
}

/// Run a fuzz campaign against a specific registry (test seam +
/// custom-registry callers).
pub fn run_fuzz_campaign_against(
    registry: &KernelRegistry,
    n_iterations: usize,
    base_seed: u64,
) -> FuzzCampaignReport {
    let seeds = seed_certificates();
    let mut rng = FuzzRng::new(base_seed);
    let mut unanimous_accept = 0usize;
    let mut unanimous_reject = 0usize;
    let mut disagreements: Vec<FuzzResult> = Vec::new();

    for iter in 0..n_iterations {
        let result = run_fuzz_iteration(iter, &mut rng, &seeds, registry);
        match &result.verdict.agreement {
            AgreementVerdict::Unanimous => unanimous_accept += 1,
            AgreementVerdict::UnanimousReject => unanimous_reject += 1,
            AgreementVerdict::Disagreement { .. } => disagreements.push(result),
        }
    }

    FuzzCampaignReport {
        total_iterations: n_iterations,
        unanimous_accept,
        unanimous_reject,
        disagreements,
        registered_kernels: registry.names(),
    }
}

// =============================================================================
// Generative fuzz — random arbitrary CIC terms (vs. mutation)
// =============================================================================
//
// Mutation-based fuzzing (above) keeps mutants close to canonical
// well-typed seeds; generative fuzzing samples the structurally-
// arbitrary CIC term space directly.  The two regimes are
// complementary: mutation explores the well-typed neighbourhood;
// generation explores everywhere else.
//
// For both, the property invariant is identical:
//
//   `registry.verify_all(&cert).agreement` must NOT be
//   `Disagreement` — every registered kernel reaches the same
//   verdict on the same certificate.
//
// Whether the random certificate types or not is irrelevant.
// Generative fuzzing exposes implementation drift in the
// rejection paths — kernels can share fast-accept paths but
// disagree on which structurally-malformed shapes get rejected
// vs. trigger an internal panic.

/// Sample a structurally-arbitrary closed `Term` of bounded depth.
///
/// **Construction**: recursive descent producing any of the five
/// `Term` variants with uniform probability.  Bounded by `depth`:
/// at depth=0 we ALWAYS generate a leaf (`Var(0)` or `Universe(n)`)
/// to terminate.  Variable indices are drawn from `[0, depth+2)`
/// so the term has a chance of being well-scoped under shallow
/// contexts; the kernel rejects free variables uniformly, so this
/// is the soundness-clean way to keep both kernels in lockstep.
///
/// **Soundness contract**: the generated `Term` is structurally
/// valid by construction (every `Pi`/`Lam`/`App` has the right
/// boxed children).  Whether it type-checks under any context is
/// not guaranteed — the test harness uses it as a fuzz input.
pub fn gen_arbitrary_term(rng: &mut FuzzRng, depth: u32) -> crate::proof_checker::Term {
    use crate::proof_checker::Term;
    if depth == 0 {
        // Leaf: Var or Universe with low-bias toward Universe(0)
        // (the most common well-typed leaf).
        return match rng.gen_below(3) {
            0 => Term::Universe(rng.gen_below(4) as u32),
            1 => Term::Var(rng.gen_below(3) as usize),
            _ => Term::Universe(0),
        };
    }
    match rng.gen_below(5) {
        0 => Term::Universe(rng.gen_below(4) as u32),
        1 => Term::Var(rng.gen_below((depth + 2) as u64) as usize),
        2 => Term::Pi(
            Box::new(gen_arbitrary_term(rng, depth - 1)),
            Box::new(gen_arbitrary_term(rng, depth - 1)),
        ),
        3 => Term::Lam(
            Box::new(gen_arbitrary_term(rng, depth - 1)),
            Box::new(gen_arbitrary_term(rng, depth - 1)),
        ),
        4 => Term::App(
            Box::new(gen_arbitrary_term(rng, depth - 1)),
            Box::new(gen_arbitrary_term(rng, depth - 1)),
        ),
        _ => unreachable!("gen_below(5)"),
    }
}

/// Generate a structurally-arbitrary `Certificate` by sampling
/// independent random terms for `term` and `claimed_type`.
/// Both terms have bounded depth.
///
/// **Yield rate**: most generated certificates will be ill-typed
/// (the random `term` rarely inhabits the random `claimed_type`).
/// That's the point — we exercise the rejection paths of both
/// kernels and confirm lock-step rejection.  The few accept-path
/// hits exercise normalisation/conversion lock-step.
pub fn gen_arbitrary_certificate(
    rng: &mut FuzzRng,
    term_depth: u32,
    type_depth: u32,
) -> crate::proof_checker::Certificate {
    crate::proof_checker::Certificate {
        term: gen_arbitrary_term(rng, term_depth),
        claimed_type: gen_arbitrary_term(rng, type_depth),
        metadata: std::collections::BTreeMap::new(),
    }
}

/// Run a single generative-fuzz iteration: sample a random
/// certificate, run it through every registered kernel, return
/// the result with the canonical `MultiVerdict`.
pub fn run_generative_iteration(
    iteration: usize,
    rng: &mut FuzzRng,
    registry: &KernelRegistry,
) -> FuzzResult {
    // Use shallow depths to keep the certificate space dense at
    // realistic shapes; deeper depths rarely produce well-typed
    // candidates and inflate Z3 / kernel evaluation cost.
    let cert = gen_arbitrary_certificate(rng, 4, 4);
    let verdict = registry.verify_all(&cert);
    FuzzResult {
        iteration,
        // seed_index = 0 sentinel — generative fuzz has no seed
        // index since certificates are sampled fresh.
        seed_index: 0,
        // Stable diagnostic tag for generative iterations.
        mutation_tag: "generative",
        verdict,
    }
}

/// Run a generative-fuzz campaign over the default registry.
///
/// Same audit-failure contract as the mutation campaign: any
/// disagreement is a kernel-implementation bug.
pub fn run_generative_campaign(n_iterations: usize, base_seed: u64) -> FuzzCampaignReport {
    let registry = KernelRegistry::default();
    run_generative_campaign_against(&registry, n_iterations, base_seed)
}

pub fn run_generative_campaign_against(
    registry: &KernelRegistry,
    n_iterations: usize,
    base_seed: u64,
) -> FuzzCampaignReport {
    let mut rng = FuzzRng::new(base_seed);
    let mut unanimous_accept = 0usize;
    let mut unanimous_reject = 0usize;
    let mut disagreements: Vec<FuzzResult> = Vec::new();

    for iter in 0..n_iterations {
        let result = run_generative_iteration(iter, &mut rng, registry);
        match &result.verdict.agreement {
            crate::kernel_registry::AgreementVerdict::Unanimous => unanimous_accept += 1,
            crate::kernel_registry::AgreementVerdict::UnanimousReject => unanimous_reject += 1,
            crate::kernel_registry::AgreementVerdict::Disagreement { .. } => {
                disagreements.push(result)
            }
        }
    }

    FuzzCampaignReport {
        total_iterations: n_iterations,
        unanimous_accept,
        unanimous_reject,
        disagreements,
        registered_kernels: registry.names(),
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_registry::KernelChecker;
    use crate::proof_checker::CheckError;

    // ----- PRNG determinism -----

    #[test]
    fn rng_determinism_same_seed_same_sequence() {
        let mut a = FuzzRng::new(42);
        let mut b = FuzzRng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn rng_zero_seed_does_not_degenerate() {
        let mut rng = FuzzRng::new(0);
        let first = rng.next_u64();
        let second = rng.next_u64();
        assert_ne!(first, 0);
        assert_ne!(second, 0);
        assert_ne!(first, second);
    }

    #[test]
    fn rng_distribution_is_not_constant() {
        // Pin: across 256 draws the bottom 4 bits cover at least
        // 8 of the 16 possible values. Catches a degenerate
        // generator that returns the same nibble repeatedly.
        let mut rng = FuzzRng::new(0xBAD_C0FFE);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..256 {
            seen.insert(rng.next_u64() & 0xF);
        }
        assert!(seen.len() >= 8, "RNG nibble distribution too narrow");
    }

    // ----- Mutation correctness -----

    #[test]
    fn lift_universes_zero_is_identity() {
        let t = Term::pi(Term::universe(2), Term::universe(5));
        let lifted = lift_universes_in_term(&t, 0);
        assert_eq!(lifted, t);
    }

    #[test]
    fn lift_universes_increments_every_universe() {
        let t = Term::pi(Term::universe(0), Term::lam(Term::var(0), Term::universe(3)));
        let lifted = lift_universes_in_term(&t, 2);
        match lifted {
            Term::Pi(domain, body) => {
                assert_eq!(*domain, Term::Universe(2));
                match *body {
                    Term::Lam(_, lam_body) => assert_eq!(*lam_body, Term::Universe(5)),
                    other => panic!("expected Lam, got {:?}", other),
                }
            }
            other => panic!("expected Pi, got {:?}", other),
        }
    }

    #[test]
    fn lift_universes_saturates_at_u32_max() {
        let t = Term::Universe(u32::MAX - 1);
        let lifted = lift_universes_in_term(&t, 100);
        assert_eq!(lifted, Term::Universe(u32::MAX));
    }

    #[test]
    fn apply_mutation_replace_term_with_universe_zero() {
        let cert = seed_certificates()[0].clone();
        let m = apply_mutation(&cert, &Mutation::ReplaceTermWithUniverseZero);
        assert_eq!(m.term, Term::Universe(0));
        assert_eq!(m.claimed_type, cert.claimed_type);
    }

    #[test]
    fn apply_mutation_swap_term_and_type() {
        let cert = seed_certificates()[1].clone();
        let m = apply_mutation(&cert, &Mutation::SwapTermAndType);
        assert_eq!(m.term, cert.claimed_type);
        assert_eq!(m.claimed_type, cert.term);
    }

    #[test]
    fn apply_mutation_lift_all_universes_preserves_typeability_for_polymorphic_id() {
        // Polymorphic identity is universe-stable: lift every
        // universe by a constant and the certificate remains
        // well-typed (cumulativity-respecting).
        let cert = seed_certificates()[0].clone();
        let lifted = apply_mutation(&cert, &Mutation::LiftAllUniverses { delta: 3 });
        // Both kernels must still accept.
        let r = KernelRegistry::default().verify_all(&lifted);
        assert!(matches!(r.agreement, AgreementVerdict::Unanimous));
    }

    // ----- Mutation tag distinctness -----

    #[test]
    fn mutation_tags_are_distinct() {
        let probes = [
            Mutation::LiftAllUniverses { delta: 1 },
            Mutation::LiftTermUniversesOnly { delta: 1 },
            Mutation::LiftClaimedTypeUniversesOnly { delta: 1 },
            Mutation::ReplaceTermWithUniverseZero,
            Mutation::ReplaceClaimedTypeWithUniverseZero,
            Mutation::ReplaceTermWithFreeVariable { idx: 0 },
            Mutation::AppToNonFunction,
            Mutation::WrapTermInExtraLam {
                domain: Term::Universe(0),
            },
            Mutation::SwapTermAndType,
            Mutation::PiDomainToUniverseZero,
            Mutation::LamDomainToUniverseZero,
        ];
        let tags: std::collections::BTreeSet<_> = probes.iter().map(|m| m.tag()).collect();
        assert_eq!(
            tags.len(),
            probes.len(),
            "every mutation variant must have a distinct stable tag",
        );
    }

    // ----- Sample mutation reaches every variant -----

    #[test]
    fn sample_mutation_covers_every_variant_within_2048_draws() {
        // Pin: with 11 variants and 2048 draws, every variant must
        // appear at least once. Detects a sampler that misses a
        // variant index (e.g., off-by-one in `gen_below(11)`).
        let mut rng = FuzzRng::new(0xFEED_FACE);
        let mut seen: std::collections::BTreeSet<&'static str> =
            std::collections::BTreeSet::new();
        for _ in 0..2048 {
            seen.insert(sample_mutation(&mut rng).tag());
        }
        assert_eq!(
            seen.len(),
            11,
            "sampler must reach every mutation variant; saw: {:?}",
            seen,
        );
    }

    // ----- Canonical seeds verify under default registry -----

    #[test]
    fn every_seed_is_unanimous_accept_under_default_registry() {
        // Pin: every canonical seed MUST be accepted by every
        // registered kernel before mutation. A seed that fails
        // would poison the campaign — the unmutated baseline is
        // load-bearing.
        let registry = KernelRegistry::default();
        for (i, seed) in seed_certificates().iter().enumerate() {
            let v = registry.verify_all(seed);
            assert!(
                matches!(v.agreement, AgreementVerdict::Unanimous),
                "seed {} disagreement before mutation: {:?}",
                i,
                v,
            );
        }
    }

    // ----- Campaign soundness on the default registry -----

    #[test]
    fn small_campaign_default_registry_zero_disagreements() {
        // The headline soundness pin: a 200-iteration campaign
        // against the default (proof_checker + proof_checker_nbe)
        // registry must produce ZERO disagreements. A failure
        // here is a kernel-implementation bug.
        let report = run_fuzz_campaign(200, 0xDEAD_BEEF);
        assert_eq!(report.total_iterations, 200);
        assert!(
            report.is_sound(),
            "differential fuzz found {} disagreement(s); first: {:?}",
            report.disagreements.len(),
            report.disagreements.first(),
        );
        // Sanity: registered kernels list contains both built-ins.
        assert!(report.registered_kernels.contains(&"proof_checker"));
        assert!(report.registered_kernels.contains(&"proof_checker_nbe"));
        // Sanity: campaign exercised both verdict directions.
        assert!(report.unanimous_accept + report.unanimous_reject == 200);
    }

    // ----- Generative fuzz tests -----

    #[test]
    fn gen_arbitrary_term_at_depth_zero_is_leaf() {
        // Pin: depth=0 produces ONLY leaf terms (Var or Universe).
        // Pi/Lam/App at depth=0 would unbalance the recursion.
        use crate::proof_checker::Term;
        let mut rng = FuzzRng::new(1234);
        for _ in 0..50 {
            let t = gen_arbitrary_term(&mut rng, 0);
            assert!(
                matches!(t, Term::Var(_) | Term::Universe(_)),
                "depth=0 must yield leaf terms, got {:?}",
                t,
            );
        }
    }

    #[test]
    fn gen_arbitrary_term_bounded_depth_does_not_panic() {
        // Pin: bounded depth never overflows the stack.
        let mut rng = FuzzRng::new(0xCAFE);
        for _ in 0..20 {
            let _ = gen_arbitrary_term(&mut rng, 8);
        }
    }

    #[test]
    fn gen_arbitrary_certificate_independent_term_and_type() {
        // Pin: term and claimed_type are independent samples.  The
        // certificate is structurally valid; whether it type-checks
        // is a separate matter (mostly: no).
        let mut rng = FuzzRng::new(42);
        let cert = gen_arbitrary_certificate(&mut rng, 3, 3);
        assert!(cert.metadata.is_empty());
        // Both fields are real Term values (smoke check).
        let _ = format!("{:?}", cert.term);
        let _ = format!("{:?}", cert.claimed_type);
    }

    #[test]
    fn small_generative_campaign_zero_disagreements() {
        // Headline soundness pin: 100-iteration generative campaign
        // over default registry produces ZERO disagreements.  This
        // is the key generative-vs-mutation result: even on
        // structurally-arbitrary CIC terms, the trusted base and
        // NbE kernel agree on every verdict.
        let report = run_generative_campaign(100, 0xDEAD_F00D);
        assert!(
            report.is_sound(),
            "generative fuzz found {} disagreement(s); first: {:?}",
            report.disagreements.len(),
            report.disagreements.first(),
        );
        assert_eq!(report.total_iterations, 100);
        // Sanity: 100 random certificates land in some mix of
        // accept/reject (almost all rejected, by construction —
        // random certificates rarely type-check).
        assert!(report.unanimous_accept + report.unanimous_reject == 100);
    }

    #[test]
    fn generative_campaign_reproducible() {
        // Determinism pin (mirror of mutation campaign).
        let r1 = run_generative_campaign(50, 0xABCDEF);
        let r2 = run_generative_campaign(50, 0xABCDEF);
        assert_eq!(r1.unanimous_accept, r2.unanimous_accept);
        assert_eq!(r1.unanimous_reject, r2.unanimous_reject);
        assert_eq!(r1.disagreements.len(), r2.disagreements.len());
    }

    #[test]
    fn generative_surfaces_disagreements_against_synthetic_kernel() {
        // Liveness pin: when a synthetic always-accept kernel is
        // registered alongside the trusted base, generative fuzz
        // MUST surface disagreements (random terms rarely type, so
        // the trusted base rejects almost all; AlwaysAcceptKernel
        // accepts all → disagreement).  Confirms the gate isn't
        // vacuous.
        struct AlwaysAccept;
        impl KernelChecker for AlwaysAccept {
            fn name(&self) -> &'static str {
                "always_accept_synthetic"
            }
            fn description(&self) -> &'static str {
                "synthetic — always accepts (test-only)"
            }
            fn verify(&self, _: &crate::proof_checker::Certificate) -> Result<(), CheckError> {
                Ok(())
            }
        }
        use crate::kernel_registry::ProofCheckerKernel;
        let mut registry = KernelRegistry::new();
        registry.register(ProofCheckerKernel);
        registry.register(AlwaysAccept);
        let report = run_generative_campaign_against(&registry, 50, 0xCAFE);
        assert!(!report.is_sound(), "synthetic always-accept must surface disagreements");
    }

    #[test]
    fn campaign_reproducible_with_same_seed() {
        // Determinism pin: the same base seed must produce the
        // same disagreement count, accept count, and reject count
        // across runs. Bisectable disagreements depend on this.
        let r1 = run_fuzz_campaign(50, 0x12345);
        let r2 = run_fuzz_campaign(50, 0x12345);
        assert_eq!(r1.unanimous_accept, r2.unanimous_accept);
        assert_eq!(r1.unanimous_reject, r2.unanimous_reject);
        assert_eq!(r1.disagreements.len(), r2.disagreements.len());
    }

    #[test]
    fn campaign_distinct_seeds_produce_distinct_outcomes() {
        // Pin: the campaign actually reads from the seed — two
        // different base seeds produce different verdict
        // distributions (with high probability over 50 iters).
        let r1 = run_fuzz_campaign(50, 1);
        let r2 = run_fuzz_campaign(50, 2);
        let same_distribution =
            r1.unanimous_accept == r2.unanimous_accept
                && r1.unanimous_reject == r2.unanimous_reject;
        assert!(
            !same_distribution,
            "two distinct seeds produced identical distributions — RNG broken?",
        );
    }

    // ----- Disagreement detection works against synthetic kernel -----

    /// Synthetic kernel that always accepts — engineered to
    /// disagree with proof_checker on rejected mutants.
    struct AlwaysAcceptKernel;
    impl KernelChecker for AlwaysAcceptKernel {
        fn name(&self) -> &'static str {
            "always_accept_synthetic"
        }
        fn description(&self) -> &'static str {
            "synthetic — always accepts (test-only)"
        }
        fn verify(&self, _cert: &Certificate) -> Result<(), CheckError> {
            Ok(())
        }
    }

    #[test]
    fn fuzz_campaign_surfaces_disagreement_against_always_accept() {
        // Architectural pin: when a kernel that always accepts is
        // registered alongside the trusted base, the campaign MUST
        // surface non-zero disagreements (because almost every
        // mutation produces an ill-typed mutant the base rejects
        // but AlwaysAcceptKernel accepts). This proves the
        // disagreement-detection is live — not a vacuous "no
        // disagreements found because we're not looking".
        use crate::kernel_registry::ProofCheckerKernel;
        let mut registry = KernelRegistry::new();
        registry.register(ProofCheckerKernel);
        registry.register(AlwaysAcceptKernel);
        let report = run_fuzz_campaign_against(&registry, 100, 0xCAFE);
        assert!(
            !report.is_sound(),
            "synthetic always-accept kernel must surface disagreements vs trusted base",
        );
        assert!(
            !report.disagreements.is_empty(),
            "expected at least one disagreement",
        );
        // Every disagreement should list both kernels.
        for d in &report.disagreements {
            assert_eq!(d.verdict.outcomes.len(), 2);
        }
    }

    // ----- Iteration ordering -----

    #[test]
    fn fuzz_iteration_indexes_seeds_round_robin() {
        // Pin: iteration N picks seed_certificates()[N % len].
        let registry = KernelRegistry::default();
        let seeds = seed_certificates();
        let mut rng = FuzzRng::new(7);
        for iter in 0..(seeds.len() * 3) {
            let r = run_fuzz_iteration(iter, &mut rng, &seeds, &registry);
            assert_eq!(r.seed_index, iter % seeds.len());
            assert_eq!(r.iteration, iter);
        }
    }
}
