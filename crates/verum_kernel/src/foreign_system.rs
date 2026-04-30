//! Canonical `ForeignSystem` enum — single source of truth for the
//! external proof systems Verum interacts with.
//!
//! Verum talks to upstream proof assistants in five capacities:
//!
//!   1. **Cross-format export** — emit Verum theorems to Coq / Lean
//!     for foreign re-check (`verum_kernel::soundness::corpus_export`).
//!   2. **Kernel-soundness mirror** — emit the meta-circular kernel-
//!      soundness corpus to Coq / Lean
//!      (`verum_kernel::soundness::{coq, lean}`).
//!   3. **Proof-term replay** — lower SMT certificates to Coq / Lean /
//!      Agda / Dedukti / Metamath proof scripts
//!      (`verum_smt::proof_replay`).
//!   4. **Foreign-system import** — extract theorem skeletons from
//!      Coq / Lean / Mizar / Isabelle source
//!      (`verum_verification::foreign_import`).
//!   5. **Re-check runner** — invoke the foreign toolchain (native
//!      or Docker) to verify exported certificates
//!      (`verum_smt::cross_format_runner`).
//!
//! Each capacity historically had its own enumeration of supported
//! systems (string IDs in `proof_replay`, `ExportFormat` in
//! `cross_format_runner`, the 4-variant enum in `foreign_import`).
//! This module supplies the canonical type that every layer
//! references; the per-capacity surfaces stay specialized but agree
//! on **which system** they're dispatching for.
//!
//! The enum lives in `verum_kernel` because that crate is the
//! lowest-level domain crate every other layer depends on.  Putting
//! it in `verum_common` would mix domain concepts into the
//! foundation; placing it in `verum_verification` (which is its
//! historical home) creates a dependency cycle with `verum_kernel`.

use serde::{Deserialize, Serialize};

/// External proof system Verum interacts with.
///
/// Variants are arranged by family: traditional CIC-family
/// assistants (Coq / Lean) first, then non-CIC systems (Mizar,
/// Isabelle), then minimal verifiers (Agda, Dedukti, Metamath).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ForeignSystem {
    /// Coq / Rocq — `.v` files.
    Coq,
    /// Lean 4 / Mathlib4 — `.lean` files.
    Lean4,
    /// Mizar — `.miz` files.
    Mizar,
    /// Isabelle/HOL — `.thy` files.
    Isabelle,
    /// Agda — `.agda` files.
    Agda,
    /// Dedukti / Lambdapi — `.dk` / `.lp` files.
    Dedukti,
    /// Metamath — `.mm` files.
    Metamath,
}

impl ForeignSystem {
    /// Stable diagnostic name (matches the `--from <name>` CLI
    /// argument, the `target_name()` of replay backends, and the
    /// `language` tag emitted by `TargetTactic`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Coq => "coq",
            Self::Lean4 => "lean4",
            Self::Mizar => "mizar",
            Self::Isabelle => "isabelle",
            Self::Agda => "agda",
            Self::Dedukti => "dedukti",
            Self::Metamath => "metamath",
        }
    }

    /// Parse a system tag from its diagnostic name.  Accepts
    /// common aliases.  Returns `None` for unrecognised input.
    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "coq" | "rocq" => Some(Self::Coq),
            "lean4" | "lean" | "mathlib4" | "mathlib" => Some(Self::Lean4),
            "mizar" | "mml" => Some(Self::Mizar),
            "isabelle" | "isabelle/hol" | "hol" => Some(Self::Isabelle),
            "agda" => Some(Self::Agda),
            "dedukti" | "lambdapi" | "dk" => Some(Self::Dedukti),
            "metamath" | "mm" => Some(Self::Metamath),
            _ => None,
        }
    }

    /// Conventional file extension (without leading dot).
    pub fn extension(self) -> &'static str {
        match self {
            Self::Coq => "v",
            Self::Lean4 => "lean",
            Self::Mizar => "miz",
            Self::Isabelle => "thy",
            Self::Agda => "agda",
            Self::Dedukti => "dk",
            Self::Metamath => "mm",
        }
    }

    /// Tag for `@framework(<tag>, "...")` attribution.  Matches the
    /// keys used in `core/verify/kernel_v0/lemmas/` and
    /// `apply_graph::is_foreign_framework_target`.
    pub fn framework_tag(self) -> &'static str {
        match self {
            Self::Coq => "coq",
            Self::Lean4 => "lean_mathlib4",
            Self::Mizar => "mizar_mml",
            Self::Isabelle => "isabelle_hol",
            Self::Agda => "agda_stdlib",
            Self::Dedukti => "dedukti",
            Self::Metamath => "metamath",
        }
    }

    /// Install hint for the system's verifier toolchain.  One short
    /// sentence.  Used by checkers' install_hint() and CLI
    /// diagnostics when a foreign tool is missing.
    pub fn install_hint(self) -> &'static str {
        match self {
            Self::Coq => "install Coq via opam: `opam install coq`",
            Self::Lean4 => "install Lean 4 via elan: `curl https://raw.githubusercontent.com/leanprover/elan/master/elan-init.sh -sSf | sh`",
            Self::Mizar => "install Mizar from https://mizar.uwb.edu.pl/",
            Self::Isabelle => "install Isabelle from https://isabelle.in.tum.de/installation.html",
            Self::Agda => "install Agda via cabal: `cabal install Agda`",
            Self::Dedukti => "install Dedukti via opam: `opam install dedukti`",
            Self::Metamath => "clone https://github.com/metamath/metamath-exe and build with make",
        }
    }

    /// Human-readable display name.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Coq => "Coq",
            Self::Lean4 => "Lean 4",
            Self::Mizar => "Mizar",
            Self::Isabelle => "Isabelle/HOL",
            Self::Agda => "Agda",
            Self::Dedukti => "Dedukti",
            Self::Metamath => "Metamath",
        }
    }

    /// All supported systems.
    pub fn all() -> [ForeignSystem; 7] {
        [
            Self::Coq,
            Self::Lean4,
            Self::Mizar,
            Self::Isabelle,
            Self::Agda,
            Self::Dedukti,
            Self::Metamath,
        ]
    }

    /// Systems that have skeleton-import backends (statement-level
    /// extraction from foreign source).  Excludes the proof-replay-
    /// only targets (Agda / Dedukti / Metamath).
    pub fn with_importer() -> [ForeignSystem; 4] {
        [Self::Coq, Self::Lean4, Self::Mizar, Self::Isabelle]
    }

    /// Systems that have proof-replay backends (lower an SMT
    /// certificate to target proof script).
    pub fn with_proof_replay() -> [ForeignSystem; 5] {
        [Self::Coq, Self::Lean4, Self::Agda, Self::Dedukti, Self::Metamath]
    }

    /// Whether this system has a hermetic re-check Checker (native
    /// or Docker).  Currently only Coq + Lean4.
    pub fn has_checker(self) -> bool {
        matches!(self, Self::Coq | Self::Lean4)
    }
}

/// **Capability trait** — every per-system struct (CoqBackend,
/// LeanBackend, CoqImporter, ...) implements this minimal trait so
/// callers that hold a `&dyn ForeignSystemFacade` can dispatch by
/// the canonical [`ForeignSystem`] tag without reaching for the
/// per-capacity trait it specialises in.
///
/// The full collapse of the 5 capacity traits into one is a
/// downstream refactor; this trait provides the unified handle so
/// the migration can land incrementally.
pub trait ForeignSystemFacade {
    /// Which foreign system this facade represents.
    fn system(&self) -> ForeignSystem;

    /// Whether this facade has a Checker capability.  Default: `false`.
    fn has_checker(&self) -> bool {
        self.system().has_checker()
    }

    /// Whether this facade has an Importer capability.
    fn has_importer(&self) -> bool {
        ForeignSystem::with_importer().contains(&self.system())
    }

    /// Whether this facade has a ProofReplay capability.
    fn has_proof_replay(&self) -> bool {
        ForeignSystem::with_proof_replay().contains(&self.system())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_round_trip_for_every_variant() {
        for s in ForeignSystem::all() {
            assert_eq!(ForeignSystem::from_name(s.name()), Some(s));
        }
    }

    #[test]
    fn all_metadata_distinct() {
        let names: std::collections::BTreeSet<_> = ForeignSystem::all().iter().map(|s| s.name()).collect();
        let exts: std::collections::BTreeSet<_> = ForeignSystem::all().iter().map(|s| s.extension()).collect();
        let tags: std::collections::BTreeSet<_> = ForeignSystem::all().iter().map(|s| s.framework_tag()).collect();
        assert_eq!(names.len(), 7);
        assert_eq!(exts.len(), 7);
        assert_eq!(tags.len(), 7);
    }

    #[test]
    fn aliases_resolve() {
        assert_eq!(ForeignSystem::from_name("rocq"), Some(ForeignSystem::Coq));
        assert_eq!(ForeignSystem::from_name("lean"), Some(ForeignSystem::Lean4));
        assert_eq!(ForeignSystem::from_name("mathlib"), Some(ForeignSystem::Lean4));
        assert_eq!(ForeignSystem::from_name("hol"), Some(ForeignSystem::Isabelle));
        assert_eq!(ForeignSystem::from_name("dk"), Some(ForeignSystem::Dedukti));
        assert_eq!(ForeignSystem::from_name("mm"), Some(ForeignSystem::Metamath));
    }

    #[test]
    fn unknown_name_returns_none() {
        assert!(ForeignSystem::from_name("unknown").is_none());
        assert!(ForeignSystem::from_name("").is_none());
    }

    #[test]
    fn capability_partitions_match() {
        for s in ForeignSystem::with_importer() {
            assert!(matches!(
                s,
                ForeignSystem::Coq | ForeignSystem::Lean4
                    | ForeignSystem::Mizar | ForeignSystem::Isabelle
            ));
        }
        for s in ForeignSystem::with_proof_replay() {
            assert!(matches!(
                s,
                ForeignSystem::Coq | ForeignSystem::Lean4
                    | ForeignSystem::Agda | ForeignSystem::Dedukti
                    | ForeignSystem::Metamath
            ));
        }
    }

    #[test]
    fn install_hints_non_empty() {
        for s in ForeignSystem::all() {
            assert!(!s.install_hint().trim().is_empty());
        }
    }

    /// Pin: a minimal facade gets correct capabilities from defaults.
    struct CoqFacade;
    impl ForeignSystemFacade for CoqFacade {
        fn system(&self) -> ForeignSystem {
            ForeignSystem::Coq
        }
    }

    #[test]
    fn facade_capability_defaults() {
        let f = CoqFacade;
        assert!(f.has_checker());
        assert!(f.has_importer());
        assert!(f.has_proof_replay());
    }

    struct AgdaFacade;
    impl ForeignSystemFacade for AgdaFacade {
        fn system(&self) -> ForeignSystem {
            ForeignSystem::Agda
        }
    }

    #[test]
    fn agda_facade_has_replay_only() {
        let f = AgdaFacade;
        assert!(!f.has_checker(), "Agda has no hermetic checker");
        assert!(!f.has_importer(), "Agda is replay-only");
        assert!(f.has_proof_replay(), "Agda is a replay target");
    }
}
