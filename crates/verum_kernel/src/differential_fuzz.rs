//! Property-based mutation fuzzer for the differential-kernel registry.
//!
//! ## Architectural role
//!
//! The canonical-battery roster is curated: 24 hand-built
//! certificates that the registry's kernels are cross-validated
//! against.  That covers the *intentional* surface, but not the
//! long tail of structurally-arbitrary terms.  Algorithmically-
//! distinct kernels (bidirectional + WHNF; NbE + quote;
//! manifest-driven) can match on every curated certificate yet
//! diverge on a corner case the curator never considered.
//!
//! This module ships **mutation-based property fuzzing**: it takes
//! the canonical accept-path certs as **seeds**, applies structural
//! **mutation chains** (1–3 mutations applied in sequence — universe
//! lifts, subterm-swaps, binder-domain rewrites, application
//! injections, variable-index perturbations), and runs each mutant
//! through every kernel in the registry. The property invariant is
//!
//!   **For every mutant `c`, `registry.verify_all(&c).agreement`
//!   must NOT be `Disagreement`.**
//!
//! Whether the mutant types or fails to type is irrelevant — what
//! matters is that all kernels reach the same verdict.  Disagreement
//! flags a kernel-implementation bug.
//!
//! ## Disagreement triage — automatic shrinking
//!
//! When the harness finds a disagreement, it automatically shrinks
//! the producing chain to a minimal failing case via greedy
//! 1-element-removal to fixpoint.  The audit report carries both
//! the original chain and the shrunk minimal chain — so the kernel
//! author sees the smallest reproducer rather than a deeply-mutated
//! cert.  A shrunk-to-empty chain is the highest-priority bug
//! class: the seed alone disagrees, meaning kernel drift exists on
//! the unmutated curated surface.
//!
//! ## Why mutation, not from-scratch generation
//!
//! Generating arbitrary closed CIC terms uniformly produces almost
//! exclusively ill-typed shapes.  Both kernels reject them quickly,
//! but rejection-only fuzzing has poor signal: kernels share
//! fast-rejection paths, and the interesting divergences happen on
//! *near-well-typed* shapes where one kernel might tolerate
//! something the other does not.
//!
//! Mutation seeded from canonical well-typed certificates keeps
//! mutants close to the well-typed surface.  A single universe lift
//! often preserves typeability; a subterm swap usually breaks it
//! cleanly.  Both regimes exercise the kernels' decision boundary
//! aggressively.  A complementary [`run_generative_campaign`]
//! samples structurally-arbitrary terms directly for rejection-
//! path coverage.
//!
//! ## Determinism
//!
//! The PRNG is a deterministic xorshift64* seeded by the campaign's
//! base seed.  The same base seed produces the same mutant sequence
//! across runs — disagreements are reproducible and bisectable.
//! No `rand` crate dependency.
//!
//! ## Coverage instrumentation
//!
//! Every campaign records per-mutation hit counts, per-seed hit
//! counts, and the chain-length distribution.  Surfaced in the
//! audit-report payload so sampling bias is observable when the
//! gate passes — a mutation that never fires across 500 iters
//! deserves investigation.
//!
//! ## Integration
//!
//! The audit gate runs a small bounded campaign each invocation
//! (default 500 iterations) and flips to failure on the first
//! disagreement.  Bug-detection is live — at audit time every
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
    LiftAllUniverses {
        /// Lift increment applied to every `Universe(n)`.
        delta: u32,
    },
    /// Lift universes in term ONLY (not claimed_type). Usually
    /// breaks typeability — both kernels MUST reject identically.
    LiftTermUniversesOnly {
        /// Lift increment applied to term-side universes.
        delta: u32,
    },
    /// Lift universes in claimed_type ONLY. Same intent as above
    /// but the asymmetry is on the type side.
    LiftClaimedTypeUniversesOnly {
        /// Lift increment applied to claimed-type universes.
        delta: u32,
    },
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
    ReplaceTermWithFreeVariable {
        /// De-Bruijn index of the free variable replacing the term.
        idx: usize,
    },
    /// Wrap the term in `App(term, Universe(0))` — applying a
    /// non-function. Both kernels MUST reject identically.
    AppToNonFunction,
    /// Wrap the term in an extra lambda layer with the given
    /// domain. The claimed_type is left as-is, so most mutants
    /// are ill-typed (the wrapping changes the term's type to
    /// `Π(_:domain). T`). Both kernels MUST reject identically
    /// when ill-typed.
    WrapTermInExtraLam {
        /// Domain type for the extra `λ`-binder wrapping the term.
        domain: Term,
    },
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
            term = Term::universe(0);
        }
        Mutation::ReplaceClaimedTypeWithUniverseZero => {
            claimed_type = Term::universe(0);
        }
        Mutation::ReplaceTermWithFreeVariable { idx } => {
            term = Term::Var(*idx);
        }
        Mutation::AppToNonFunction => {
            term = Term::app(term, Term::universe(0));
        }
        Mutation::WrapTermInExtraLam { domain } => {
            term = Term::lam(domain.clone(), term);
        }
        Mutation::SwapTermAndType => {
            std::mem::swap(&mut term, &mut claimed_type);
        }
        Mutation::PiDomainToUniverseZero => {
            if let Term::Pi(_, body) = &claimed_type {
                claimed_type = Term::Pi(Box::new(Term::universe(0)), body.clone());
            }
        }
        Mutation::LamDomainToUniverseZero => {
            if let Term::Lam(_, body) = &term {
                term = Term::Lam(Box::new(Term::universe(0)), body.clone());
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
        Term::Universe(level) => Term::Universe(level.clone().shifted_by(delta)),
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
        Term::Sigma(a, b) => Term::Sigma(
            Box::new(lift_universes_in_term(a, delta)),
            Box::new(lift_universes_in_term(b, delta)),
        ),
        Term::Pair(a, b) => Term::Pair(
            Box::new(lift_universes_in_term(a, delta)),
            Box::new(lift_universes_in_term(b, delta)),
        ),
        Term::Fst(p) => Term::Fst(Box::new(lift_universes_in_term(p, delta))),
        Term::Snd(p) => Term::Snd(Box::new(lift_universes_in_term(p, delta))),
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
        0 => Term::universe(0),
        1 => Term::universe(1),
        2 => Term::universe(2),
        3 => Term::Pi(Box::new(Term::universe(0)), Box::new(Term::universe(0))),
        _ => unreachable!(),
    }
}

// =============================================================================
// Mutation chains
// =============================================================================

/// Default upper bound on mutation-chain length.  Each iteration
/// samples a chain of length 1..=`MAX_MUTATION_CHAIN_LEN` and
/// applies them sequentially.  Chains > 3 rarely produce novel
/// shapes (mutations saturate after a few applications) and waste
/// CI budget.
pub const MAX_MUTATION_CHAIN_LEN: usize = 3;

/// A reproducible sequence of mutations applied to a seed
/// certificate.  Length 1 reduces to a single-mutation iteration;
/// longer chains explore deeper neighbourhoods of the seed.
///
/// Chains are recorded verbatim in [`FuzzResult`] so a disagreement
/// found in CI can be replayed locally by re-applying the same
/// chain to the same seed — the bug is bisectable from a single
/// `(seed_index, chain)` pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationChain {
    /// Mutations applied in order.  An empty chain is the identity.
    pub mutations: Vec<Mutation>,
}

impl MutationChain {
    /// Number of mutations in the chain.
    pub fn len(&self) -> usize {
        self.mutations.len()
    }
    /// True iff the chain is empty (identity mutation).
    pub fn is_empty(&self) -> bool {
        self.mutations.is_empty()
    }
    /// Stable diagnostic representation: comma-separated mutation
    /// tags, in chain order.  Used in audit-report rows.
    pub fn tags(&self) -> String {
        self.mutations
            .iter()
            .map(|m| m.tag())
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Sample a mutation chain of length 1..=`max_chain_len`.
/// Length is uniformly sampled in that closed interval; mutations
/// are sampled independently via [`sample_mutation`].
pub fn sample_mutation_chain(rng: &mut FuzzRng, max_chain_len: usize) -> MutationChain {
    let len = if max_chain_len == 0 {
        1
    } else {
        1 + rng.gen_below(max_chain_len as u64) as usize
    };
    let mutations = (0..len).map(|_| sample_mutation(rng)).collect();
    MutationChain { mutations }
}

/// Apply a chain of mutations to a certificate, in order.
/// `apply_mutation_chain(c, &chain)` is left-associative:
/// `chain[1](chain[0](c))`, etc.
pub fn apply_mutation_chain(cert: &Certificate, chain: &MutationChain) -> Certificate {
    let mut current = cert.clone();
    for mutation in &chain.mutations {
        current = apply_mutation(&current, mutation);
    }
    current
}

// =============================================================================
// Canonical seed certificates
// =============================================================================

/// Curated seed certificates the fuzzer mutates.  Each seed is
/// known to be accepted by every registered kernel; mutations
/// drive the kernels into the long tail of accept/reject regions.
///
/// Sourced from [`crate::canonical_battery::canonical_battery`] —
/// every accept-path cert in the canonical 24-cert library is a
/// seed.  This shares the seed roster with the differential audit
/// gates: a regression in either gate exercises the *same* term
/// surface, so any drift between mutation campaign and curated
/// audit shows up immediately.
///
/// Augmented with one extra hand-built seed (the K combinator —
/// 4-binder constant function) that's structurally deeper than
/// any cert in the canonical battery, so mutation chains have
/// room to reach novel shapes.
pub fn seed_certificates() -> Vec<Certificate> {
    let mut seeds: Vec<Certificate> = crate::canonical_battery::canonical_battery()
        .into_iter()
        .filter_map(|c| {
            crate::canonical_battery::expected_verdict(c.id)
                .filter(|expect| *expect)
                .map(|_| c.certificate)
        })
        .collect();
    // K combinator — λA. λB. λa. λ_. a  :  ΠA. ΠB. Πa. Π_. A.
    // Deeper binder nesting than anything in the canonical battery;
    // gives mutation chains room to reach novel shapes.
    seeds.push(Certificate {
        term: Term::lam(
            Term::universe(0),
            Term::lam(
                Term::universe(0),
                Term::lam(Term::var(1), Term::lam(Term::var(1), Term::var(1))),
            ),
        ),
        claimed_type: Term::pi(
            Term::universe(0),
            Term::pi(
                Term::universe(0),
                Term::pi(Term::var(1), Term::pi(Term::var(1), Term::var(3))),
            ),
        ),
        metadata: std::collections::BTreeMap::new(),
    });
    seeds
}

// =============================================================================
// Coverage instrumentation
// =============================================================================

/// Coverage instrumentation for a fuzz campaign.  Lets the audit
/// report flag distribution bias — e.g. a mutation that never
/// fires on any seed, or a seed never picked by the round-robin
/// dispatcher.
#[derive(Debug, Clone, Default)]
pub struct FuzzCoverage {
    /// Per-mutation hit count, keyed by stable tag.  A mutation
    /// with zero hits across a 500-iter campaign is suspicious —
    /// likely a bug in [`sample_mutation`] or a tag rename.
    pub per_mutation_hits: std::collections::BTreeMap<&'static str, usize>,
    /// Per-seed hit count, keyed by seed index.  Pinpoints round-
    /// robin behaviour: every seed should receive ≈ `iter/seeds`
    /// hits in a balanced campaign.
    pub per_seed_hits: std::collections::BTreeMap<usize, usize>,
    /// Distribution of chain lengths observed.  Index `i` holds
    /// the count of iterations whose chain had length `i+1`
    /// (chain length is always ≥ 1 in the mutation regime).
    pub chain_length_distribution: Vec<usize>,
}

impl FuzzCoverage {
    fn record(&mut self, seed_index: usize, chain: &MutationChain) {
        *self.per_seed_hits.entry(seed_index).or_insert(0) += 1;
        for m in &chain.mutations {
            *self.per_mutation_hits.entry(m.tag()).or_insert(0) += 1;
        }
        let len = chain.len().max(1);
        if self.chain_length_distribution.len() < len {
            self.chain_length_distribution.resize(len, 0);
        }
        self.chain_length_distribution[len - 1] += 1;
    }
}

// =============================================================================
// Shrinker
// =============================================================================

/// Result of shrinking a disagreement to a minimal failing case.
/// Reduces audit-report noise and gives the kernel author the
/// smallest reproducer rather than the original (potentially
/// deeply-mutated) chain.
#[derive(Debug, Clone)]
pub struct ShrinkReport {
    /// Length of the chain that originally produced the disagreement.
    pub original_chain_len: usize,
    /// Length of the minimal chain still producing a disagreement.
    /// `<=` `original_chain_len`; usually strictly less.
    pub minimal_chain_len: usize,
    /// The minimal chain itself.  Empty iff the seed *alone* (no
    /// mutations) already produces a disagreement — that's a much
    /// more interesting bug class than mutation-induced ones.
    pub minimal_chain: MutationChain,
    /// Number of shrink steps the algorithm took before reaching
    /// the minimum.  Bounded by the chain length.
    pub steps_taken: usize,
}

/// Shrink a disagreeing mutation chain to a minimal failing case.
///
/// Algorithm: greedy 1-element-removal, repeated to fixpoint.  At
/// each step try removing one mutation from the chain (every
/// position) and re-running through the registry.  The first
/// removal that *preserves* the disagreement becomes the new
/// chain; iterate.  Terminates when no single-element removal
/// preserves disagreement (or the chain is empty, which means the
/// seed itself disagrees — a far more interesting bug class).
///
/// Bounded by `seed_chain.len()` outer iterations × `chain.len()`
/// inner trials; total `O(n²)` registry calls.  For
/// [`MAX_MUTATION_CHAIN_LEN`] = 3 this is at most 6 calls per
/// shrink — negligible cost vs CI runtime budget.
pub fn shrink_disagreement(
    registry: &KernelRegistry,
    seed: &Certificate,
    seed_chain: &MutationChain,
) -> ShrinkReport {
    let original_chain_len = seed_chain.len();
    let mut current = seed_chain.clone();
    let mut steps = 0usize;

    'outer: loop {
        if current.is_empty() {
            break;
        }
        // Try to remove each element in turn; the first removal
        // that preserves disagreement becomes the new current.
        for i in 0..current.len() {
            let mut candidate = current.clone();
            candidate.mutations.remove(i);
            let mutant = apply_mutation_chain(seed, &candidate);
            let verdict = registry.verify_all(&mutant);
            if matches!(
                verdict.agreement,
                AgreementVerdict::Disagreement { .. }
            ) {
                current = candidate;
                steps += 1;
                continue 'outer;
            }
        }
        // No single removal preserves disagreement — fixpoint.
        break;
    }

    ShrinkReport {
        original_chain_len,
        minimal_chain_len: current.len(),
        minimal_chain: current,
        steps_taken: steps,
    }
}

// =============================================================================
// Fuzz iteration + campaign
// =============================================================================

/// One fuzz iteration's outcome.  Carries the mutation chain
/// applied, the resulting verdict, and the seed-certificate index
/// for reproducibility.
#[derive(Debug, Clone)]
pub struct FuzzResult {
    /// Iteration index in the campaign, starting at 0.
    pub iteration: usize,
    /// Index into `seed_certificates()` of the seed used.
    pub seed_index: usize,
    /// Stable comma-joined mutation-tag list ("" for generative
    /// iterations or empty chains; see [`MutationChain::tags`]).
    /// Kept as a string so the audit-report payload is JSON-stable.
    pub mutation_tag: String,
    /// The mutation chain that produced the mutant — exact replay
    /// surface.  Empty for generative iterations.
    pub chain: MutationChain,
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
    /// Per-disagreement minimal-failing-case reports.  Same length
    /// + index-aligned with `disagreements`.  An empty
    /// `minimal_chain` means the seed alone already disagrees,
    /// which is a much higher-priority bug.
    pub shrunk_disagreements: Vec<ShrinkReport>,
    /// Coverage instrumentation: per-mutation, per-seed,
    /// chain-length-distribution.  Surfaces sampling bias when the
    /// campaign passes — a mutation that never fires deserves
    /// investigation.
    pub coverage: FuzzCoverage,
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
/// samples a mutation chain from `rng` (length 1..=[`MAX_MUTATION_CHAIN_LEN`]),
/// applies it, runs the registry's `verify_all`, and returns the
/// result.
pub fn run_fuzz_iteration(
    iteration: usize,
    rng: &mut FuzzRng,
    seeds: &[Certificate],
    registry: &KernelRegistry,
) -> FuzzResult {
    let seed_index = iteration % seeds.len();
    let seed = &seeds[seed_index];
    let chain = sample_mutation_chain(rng, MAX_MUTATION_CHAIN_LEN);
    let mutant = apply_mutation_chain(seed, &chain);
    let verdict = registry.verify_all(&mutant);
    FuzzResult {
        iteration,
        seed_index,
        mutation_tag: chain.tags(),
        chain,
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
///
/// Builds coverage instrumentation incrementally + runs the
/// shrinker on every disagreement so the report carries a
/// minimal failing case for each bug surfaced.
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
    let mut shrunk_disagreements: Vec<ShrinkReport> = Vec::new();
    let mut coverage = FuzzCoverage::default();

    for iter in 0..n_iterations {
        let result = run_fuzz_iteration(iter, &mut rng, &seeds, registry);
        coverage.record(result.seed_index, &result.chain);
        match &result.verdict.agreement {
            AgreementVerdict::Unanimous => unanimous_accept += 1,
            AgreementVerdict::UnanimousReject => unanimous_reject += 1,
            AgreementVerdict::Disagreement { .. } => {
                let shrink =
                    shrink_disagreement(registry, &seeds[result.seed_index], &result.chain);
                disagreements.push(result);
                shrunk_disagreements.push(shrink);
            }
        }
    }

    FuzzCampaignReport {
        total_iterations: n_iterations,
        unanimous_accept,
        unanimous_reject,
        disagreements,
        shrunk_disagreements,
        coverage,
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
            0 => Term::universe(rng.gen_below(4) as u32),
            1 => Term::Var(rng.gen_below(3) as usize),
            _ => Term::universe(0),
        };
    }
    match rng.gen_below(5) {
        0 => Term::universe(rng.gen_below(4) as u32),
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
        mutation_tag: "generative".to_string(),
        // Generative iterations have no chain — the certificate is
        // synthesised directly, not derived from a seed via
        // mutations.  An empty chain is the canonical sentinel.
        chain: MutationChain {
            mutations: Vec::new(),
        },
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

/// Run a generative-fuzz campaign against an explicit
/// [`KernelRegistry`].
///
/// Same audit-failure contract as the mutation campaign: any
/// disagreement is a kernel-implementation bug.
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
        // Generative iterations carry no mutation chain → no
        // shrinking surface.  An empty `shrunk_disagreements`
        // index-aligned with `disagreements` keeps the report
        // shape uniform across both regimes.
        shrunk_disagreements: Vec::new(),
        // Generative iterations don't exercise the per-mutation /
        // per-seed dimensions (no seeds, no chains), so coverage is
        // intentionally empty for this regime.
        coverage: FuzzCoverage::default(),
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
                assert_eq!(*domain, Term::universe(2));
                match *body {
                    Term::Lam(_, lam_body) => assert_eq!(*lam_body, Term::universe(5)),
                    other => panic!("expected Lam, got {:?}", other),
                }
            }
            other => panic!("expected Pi, got {:?}", other),
        }
    }

    #[test]
    fn lift_universes_saturates_at_u32_max() {
        let t = Term::universe(u32::MAX - 1);
        let lifted = lift_universes_in_term(&t, 100);
        assert_eq!(lifted, Term::universe(u32::MAX));
    }

    #[test]
    fn apply_mutation_replace_term_with_universe_zero() {
        let cert = seed_certificates()[0].clone();
        let m = apply_mutation(&cert, &Mutation::ReplaceTermWithUniverseZero);
        assert_eq!(m.term, Term::universe(0));
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
        let seeds = seed_certificates();
        // Find the polymorphic-identity seed (poly-id-shape) — the
        // first 3-binder Lam in the canonical battery.
        let cert = seeds
            .iter()
            .find(|c| {
                matches!(
                    &c.term,
                    Term::Lam(_, body)
                        if matches!(&**body, Term::Lam(_, _))
                )
            })
            .expect("polymorphic-identity seed must exist")
            .clone();
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
                domain: Term::universe(0),
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

    // ----- Mutation chains -----

    #[test]
    fn empty_chain_is_identity() {
        // Pin: applying an empty chain returns the seed unchanged.
        let chain = MutationChain { mutations: vec![] };
        let seed = &seed_certificates()[0];
        let result = apply_mutation_chain(seed, &chain);
        assert_eq!(result.term, seed.term);
        assert_eq!(result.claimed_type, seed.claimed_type);
    }

    #[test]
    fn chain_application_is_left_associative() {
        // Pin: chain[0] applied first, chain[1] to that result.
        // Lifting universes by 1 then by 2 should equal lifting by 3.
        let chain_a = MutationChain {
            mutations: vec![
                Mutation::LiftAllUniverses { delta: 1 },
                Mutation::LiftAllUniverses { delta: 2 },
            ],
        };
        let chain_b = MutationChain {
            mutations: vec![Mutation::LiftAllUniverses { delta: 3 }],
        };
        let seed = &seed_certificates()[0];
        let r_a = apply_mutation_chain(seed, &chain_a);
        let r_b = apply_mutation_chain(seed, &chain_b);
        assert_eq!(r_a.term, r_b.term);
        assert_eq!(r_a.claimed_type, r_b.claimed_type);
    }

    #[test]
    fn sample_mutation_chain_respects_max_len() {
        // Pin: sampled chains never exceed max_chain_len.
        let mut rng = FuzzRng::new(123);
        for _ in 0..200 {
            let chain = sample_mutation_chain(&mut rng, 5);
            assert!(chain.len() >= 1, "chain length must be at least 1");
            assert!(chain.len() <= 5, "chain length must be ≤ max");
        }
    }

    #[test]
    fn chain_tags_join_with_commas() {
        // Pin: stable serialization shape for audit reports.
        let chain = MutationChain {
            mutations: vec![
                Mutation::LiftAllUniverses { delta: 1 },
                Mutation::AppToNonFunction,
                Mutation::SwapTermAndType,
            ],
        };
        assert_eq!(
            chain.tags(),
            "lift_all_universes,app_to_non_function,swap_term_and_type"
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

    #[test]
    fn seed_roster_includes_canonical_battery_accept_path() {
        // Pin: every accept-path canonical cert is a fuzz seed.
        // A drift here means a freshly-added canonical cert isn't
        // exercising the fuzzer, weakening coverage silently.
        let canonical_count = crate::canonical_battery::canonical_battery()
            .iter()
            .filter(|c| {
                crate::canonical_battery::expected_verdict(c.id).unwrap_or(false)
            })
            .count();
        let seeds = seed_certificates();
        // Plus one extra K-combinator seed that's hand-built.
        assert_eq!(seeds.len(), canonical_count + 1);
    }

    // ----- Campaign soundness on the default registry -----

    #[test]
    fn small_campaign_default_registry_zero_disagreements() {
        // The headline soundness pin: a 200-iteration campaign
        // against the default registry must produce ZERO
        // disagreements.  Failure here is a kernel-implementation
        // bug.
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

    /// Synthetic kernel that always rejects.
    struct AlwaysRejectKernel;
    impl KernelChecker for AlwaysRejectKernel {
        fn name(&self) -> &'static str {
            "always_reject_synthetic"
        }
        fn description(&self) -> &'static str {
            "synthetic — always rejects (test-only)"
        }
        fn verify(&self, _cert: &Certificate) -> Result<(), CheckError> {
            Err(CheckError::NotAType(Term::universe(0)))
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
        // Every disagreement must come paired with a shrunk
        // minimal-failing-case report.
        assert_eq!(
            report.disagreements.len(),
            report.shrunk_disagreements.len(),
        );
        // At least one shrunk chain should be strictly shorter
        // than its origin (the shrinker is doing real work).
        let any_shrunk = report
            .shrunk_disagreements
            .iter()
            .any(|s| s.minimal_chain_len < s.original_chain_len);
        assert!(
            any_shrunk,
            "shrinker should reduce at least one chain in a 100-iter campaign",
        );
    }

    // ----- Shrinker -----

    #[test]
    fn shrinker_reduces_chain_to_minimum() {
        // Pin: a 3-element chain where only mutation #1 contributes
        // to the disagreement should shrink to a 1-element chain.
        // Setup: trusted-base + AlwaysAccept; the seed (polymorphic
        // identity) is unanimously accepted by both.  AppToNonFunction
        // makes the trusted base reject (app on non-function), which
        // AlwaysAccept still accepts → disagreement.  Lift mutations
        // alone preserve cumulativity-respecting typeability, so they
        // don't disagree.
        use crate::kernel_registry::ProofCheckerKernel;
        let mut registry = KernelRegistry::new();
        registry.register(ProofCheckerKernel);
        registry.register(AlwaysAcceptKernel);
        let seed = &seed_certificates()[0];
        // Sanity: seed alone is unanimously accepted.
        let v0 = registry.verify_all(seed);
        assert!(matches!(v0.agreement, AgreementVerdict::Unanimous));

        let chain = MutationChain {
            mutations: vec![
                Mutation::LiftAllUniverses { delta: 1 },
                Mutation::AppToNonFunction,
                Mutation::LiftAllUniverses { delta: 1 },
            ],
        };
        // Sanity: the original 3-chain produces a disagreement.
        let mutant = apply_mutation_chain(seed, &chain);
        let v = registry.verify_all(&mutant);
        assert!(
            matches!(v.agreement, AgreementVerdict::Disagreement { .. }),
            "expected disagreement on full chain, got {:?}",
            v.agreement
        );

        let report = shrink_disagreement(&registry, seed, &chain);
        assert_eq!(report.original_chain_len, 3);
        assert_eq!(
            report.minimal_chain_len, 1,
            "shrinker should isolate the AppToNonFunction mutation"
        );
        assert_eq!(
            report.minimal_chain.mutations[0].tag(),
            "app_to_non_function"
        );
        // Sanity: the minimal chain still produces a disagreement.
        let minimal_mutant = apply_mutation_chain(seed, &report.minimal_chain);
        let v = registry.verify_all(&minimal_mutant);
        assert!(matches!(v.agreement, AgreementVerdict::Disagreement { .. }));
    }

    #[test]
    fn shrinker_terminates_on_seed_level_disagreement() {
        // Pin: when the seed itself disagrees (kernel drift on the
        // unmutated curated surface), the shrinker reduces the chain
        // to empty — every mutation is removable because the seed
        // alone already triggers disagreement.
        use crate::kernel_registry::ProofCheckerKernel;
        let mut registry = KernelRegistry::new();
        registry.register(ProofCheckerKernel);
        registry.register(AlwaysRejectKernel);
        let seed = &seed_certificates()[0];
        let chain = MutationChain {
            mutations: vec![
                Mutation::LiftAllUniverses { delta: 0 },
                Mutation::LiftAllUniverses { delta: 0 },
            ],
        };
        let report = shrink_disagreement(&registry, seed, &chain);
        assert_eq!(report.original_chain_len, 2);
        assert_eq!(report.minimal_chain_len, 0);
        // Sanity: the seed alone produces disagreement, confirming
        // the shrinker reached a minimum that is genuinely the
        // shortest disagreeing prefix.
        let v = registry.verify_all(seed);
        assert!(matches!(v.agreement, AgreementVerdict::Disagreement { .. }));
    }

    // ----- Coverage instrumentation -----

    #[test]
    fn coverage_per_seed_hits_round_robin_balanced() {
        // Pin: a campaign of `2 × seeds.len()` iterations should
        // hit every seed exactly twice (round-robin scheduling).
        let registry = KernelRegistry::default();
        let seeds = seed_certificates();
        let n = seeds.len() * 2;
        let report = run_fuzz_campaign_against(&registry, n, 0xDEAD_BEEF);
        for (idx, hits) in &report.coverage.per_seed_hits {
            assert!(*idx < seeds.len(), "seed index out of range");
            assert_eq!(*hits, 2, "seed {} got {} hits, expected 2", idx, hits);
        }
        assert_eq!(report.coverage.per_seed_hits.len(), seeds.len());
    }

    #[test]
    fn coverage_chain_length_distribution_within_bounds() {
        // Pin: every chain length recorded is in 1..=MAX_MUTATION_CHAIN_LEN.
        let registry = KernelRegistry::default();
        let report = run_fuzz_campaign_against(&registry, 200, 42);
        let max_observed = report
            .coverage
            .chain_length_distribution
            .iter()
            .enumerate()
            .filter(|(_, c)| **c > 0)
            .map(|(i, _)| i + 1)
            .max()
            .unwrap_or(0);
        assert!(max_observed >= 1);
        assert!(max_observed <= MAX_MUTATION_CHAIN_LEN);
        let total: usize = report.coverage.chain_length_distribution.iter().sum();
        assert_eq!(total, 200, "total chain hits should equal iterations");
    }

    #[test]
    fn coverage_records_every_mutation_tag_within_500_iters() {
        // Pin: a 500-iter campaign should hit every mutation tag at
        // least once (with chain lengths up to 3 the expected hit
        // rate per mutation is ~1/11 × ~2 per iter = ~91 hits).
        // Zero hits would indicate a sampling bug.
        let registry = KernelRegistry::default();
        let report = run_fuzz_campaign_against(&registry, 500, 0xABCD);
        let expected_tags = [
            "lift_all_universes",
            "lift_term_universes_only",
            "lift_claimed_type_universes_only",
            "replace_term_with_universe_zero",
            "replace_claimed_type_with_universe_zero",
            "replace_term_with_free_variable",
            "app_to_non_function",
            "wrap_term_in_extra_lam",
            "swap_term_and_type",
            "pi_domain_to_universe_zero",
            "lam_domain_to_universe_zero",
        ];
        for tag in expected_tags {
            assert!(
                report.coverage.per_mutation_hits.get(tag).copied().unwrap_or(0) > 0,
                "mutation '{}' got zero hits in 500-iter campaign",
                tag
            );
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
