{-# OPTIONS --cubical --safe #-}
-- KernelSoundness.agda. Syntax and rule inventory only.
-- Agda checks the data declarations. Executable typing and soundness are unverified.
module KernelSoundness where

open import Agda.Builtin.Nat using (Nat)
open import Agda.Builtin.String using (String)
open import Agda.Builtin.List using (List)
open import Agda.Builtin.Sigma using (Σ)

------------------------------------------------------------
-- `CoreTerm` — kernel calculus syntax (mirror of
-- `verum_kernel::CoreTerm` at term.rs).
------------------------------------------------------------
data CoreTerm : Set where
  Var          : String → CoreTerm
  Universe     : Nat → CoreTerm
  PropSort     : CoreTerm
  Pi           : String → CoreTerm → CoreTerm → CoreTerm
  Lam          : String → CoreTerm → CoreTerm → CoreTerm
  App          : CoreTerm → CoreTerm → CoreTerm
  Sigma        : String → CoreTerm → CoreTerm → CoreTerm
  Pair         : CoreTerm → CoreTerm → CoreTerm
  Fst          : CoreTerm → CoreTerm
  Snd          : CoreTerm → CoreTerm
  PathTy       : CoreTerm → CoreTerm → CoreTerm → CoreTerm
  PathOver     : CoreTerm → CoreTerm → CoreTerm → CoreTerm → CoreTerm
  Refl         : CoreTerm → CoreTerm
  HComp        : CoreTerm → CoreTerm → CoreTerm → CoreTerm
  Transport    : CoreTerm → CoreTerm → CoreTerm
  Glue         : CoreTerm → CoreTerm → CoreTerm → CoreTerm → CoreTerm
  Refine       : CoreTerm → String → CoreTerm → CoreTerm
  Quotient     : CoreTerm → CoreTerm → CoreTerm
  QuotIntro    : CoreTerm → CoreTerm → CoreTerm → CoreTerm
  QuotElim     : CoreTerm → CoreTerm → CoreTerm → CoreTerm → CoreTerm
  InductiveT   : String → List CoreTerm → CoreTerm
  Elim         : CoreTerm → CoreTerm → List CoreTerm → CoreTerm
  SmtProof     : String → CoreTerm
  AxiomT       : String → CoreTerm → String → CoreTerm
  EpsilonOf    : CoreTerm → CoreTerm
  AlphaOf      : CoreTerm → CoreTerm
  ModalBox     : CoreTerm → CoreTerm
  ModalDiamond : CoreTerm → CoreTerm
  ModalBigAnd  : List CoreTerm → CoreTerm
  Shape        : CoreTerm → CoreTerm
  Flat         : CoreTerm → CoreTerm
  Sharp        : CoreTerm → CoreTerm

Ctx : Set
Ctx = List (Σ String (λ _ → CoreTerm))

------------------------------------------------------------
-- `KernelRule` — the supported kernel rule taxonomy
-- (mirror of `verum_kernel::proof_tree::KernelRule`).
------------------------------------------------------------
data KernelRule : Set where
  K-Var : KernelRule
  K-Univ : KernelRule
  K-Pi-Form : KernelRule
  K-Lam-Intro : KernelRule
  K-App-Elim : KernelRule
  K-Sigma-Form : KernelRule
  K-Pair-Intro : KernelRule
  K-Fst-Elim : KernelRule
  K-Snd-Elim : KernelRule
  K-Path-Ty-Form : KernelRule
  K-Path-Over-Form : KernelRule
  K-Refl-Intro : KernelRule
  K-Transp : KernelRule
  K-Refine : KernelRule
  K-Refine-Intro : KernelRule
  K-Refine-Erase : KernelRule
  K-Quot-Form : KernelRule
  K-Quot-Intro : KernelRule
  K-Quot-Elim : KernelRule
  K-Universe-Ascent : KernelRule
  K-Epsilon-Of : KernelRule
  K-Alpha-Of : KernelRule
  K-Modal-Box : KernelRule
  K-Modal-Diamond : KernelRule
  K-Modal-Big-And : KernelRule
  K-Shape : KernelRule
  K-Flat : KernelRule
  K-Sharp : KernelRule

-- K_Var — Structural schema, 0 premises

-- K_Univ — Structural schema, 0 premises

-- K_Pi_Form — Unverified metatheory reference. Source core.verify.kernel_v0.lemmas.subst.subst_preserves_typing. Framework mathlib4. Citation Mathlib.LambdaCalculus.LambdaPi.Substitution.subst_preserves_typing.

-- K_Lam_Intro — Unverified metatheory reference. Source core.verify.kernel_v0.lemmas.cartesian.cartesian_closure_for_pi. Framework mathlib4. Citation Mathlib.CategoryTheory.Closed.Cartesian.

-- K_App_Elim — Unverified metatheory reference. Source core.verify.kernel_v0.lemmas.subst.subst_preserves_typing + core.verify.kernel_v0.lemmas.beta.church_rosser_confluence. Framework mathlib4. Citation Mathlib.LambdaCalculus.LambdaPi.Substitution + Mathlib.Computability.Lambda.ChurchRosser.

-- K_Sigma_Form — Unverified metatheory reference. Source core.verify.kernel_v0.lemmas.subst.subst_preserves_typing. Framework mathlib4. Citation Mathlib.LambdaCalculus.LambdaPi.Substitution.subst_preserves_typing (Sigma form via duality).

-- K_Pair_Intro — Unverified metatheory reference. Source core.verify.kernel_v0.lemmas.subst.subst_preserves_typing. Framework mathlib4. Citation Mathlib.LambdaCalculus.LambdaPi.Substitution + dependent-product structure.

-- K_Fst_Elim — Unverified metatheory reference. Source core.verify.kernel_v0.lemmas.eta.function_extensionality. Framework zfc. Citation Sigma-projection eta-rule (fst (a, b) ≡ a) — derivable from extensionality.

-- K_Snd_Elim — Unverified metatheory reference. Source core.verify.kernel_v0.lemmas.eta.function_extensionality. Framework zfc. Citation Sigma-projection eta-rule (snd (a, b) : B[a/x]) — derivable from extensionality + subst.

-- K_Path_Ty_Form — Cubical schema, 3 premises

-- K_Path_Over_Form — Cubical schema, 4 premises

-- K_Refl_Intro — Cubical schema, 1 premises

-- UNSUPPORTED K_HComp: The executable kernel refuses composition until partial-element support and boundary coherence are checked

-- K_Transp — Cubical schema, 4 premises

-- UNSUPPORTED K_Glue: The executable kernel refuses gluing until equivalence and boundary coherence are checked

-- K_Refine — Refinement schema, 3 premises

-- MODEL UNAVAILABLE K_Refine_Omega: The export lacks the ordinal modal-depth inequality checked by the executable omega formation rule

-- K_Refine_Intro — Refinement schema, 3 premises

-- K_Refine_Erase — Refinement schema, 1 premises

-- K_Quot_Form — Quotient schema, 3 premises

-- K_Quot_Intro — Quotient schema, 2 premises

-- K_Quot_Elim — Quotient schema, 10 premises

-- MODEL UNAVAILABLE K_Inductive: The export lacks the declaration registry, parameter checks and declared universe used for inductive formation

-- MODEL UNAVAILABLE K_Pos: The export lacks the checked strict-positivity judgment for constructor argument types

-- MODEL UNAVAILABLE K_Elim: The export lacks the registered declaration and the typing judgment for every constructor branch

-- UNSUPPORTED K_Smt: Opaque SMT envelopes have no checked reconstruction and are refused by the executable kernel

-- MODEL UNAVAILABLE K_FwAx: The export lacks the authoritative axiom registry lookup and coordinate policy used by the executable kernel

-- UNSUPPORTED K_Eps_Mu: No executable rule authorizes assigning an arbitrary articulation the type of an enactment; the biadjunction correspondence remains unverified

-- K_Universe_Ascent — Diakrisis schema, 1 premises

-- UNSUPPORTED K_Round_Trip: No executable rule authorizes assigning an arbitrary term a recovered type; an encoding round-trip requires checked source correspondence

-- K_Epsilon_Of — Diakrisis schema, 1 premises

-- K_Alpha_Of — Diakrisis schema, 1 premises

-- K_Modal_Box — Diakrisis schema, 1 premises

-- K_Modal_Diamond — Diakrisis schema, 1 premises

-- K_Modal_Big_And — Diakrisis schema, 1 premises

-- K_Shape — Diakrisis schema, 1 premises

-- K_Flat — Diakrisis schema, 1 premises

-- K_Sharp — Diakrisis schema, 1 premises



-- End of syntax and rule inventory.
