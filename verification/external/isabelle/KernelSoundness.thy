(* KernelSoundness.thy. Structural typing schemas. Substitution is an abstract constant. *)
theory KernelSoundness
  imports Main
begin

(* CoreTerm — kernel calculus syntax. *)
datatype CoreTerm =
  Var string
  | Universe nat
  | PropSort
  | Pi string CoreTerm CoreTerm
  | Lam string CoreTerm CoreTerm
  | App CoreTerm CoreTerm
  | Sigma string CoreTerm CoreTerm
  | Pair CoreTerm CoreTerm
  | Fst CoreTerm
  | Snd CoreTerm
  | PathTy CoreTerm CoreTerm CoreTerm
  | PathOver CoreTerm CoreTerm CoreTerm CoreTerm
  | Refl CoreTerm
  | HComp CoreTerm CoreTerm CoreTerm
  | Transport CoreTerm CoreTerm
  | Glue CoreTerm CoreTerm CoreTerm CoreTerm
  | Refine CoreTerm string CoreTerm
  | Quotient CoreTerm CoreTerm
  | QuotIntro CoreTerm CoreTerm CoreTerm
  | QuotElim CoreTerm CoreTerm CoreTerm CoreTerm
  | InductiveT string "CoreTerm list"
  | Elim CoreTerm CoreTerm "CoreTerm list"
  | SmtProof string
  | AxiomT string CoreTerm string
  | EpsilonOf CoreTerm
  | AlphaOf CoreTerm
  | ModalBox CoreTerm
  | ModalDiamond CoreTerm
  | ModalBigAnd "CoreTerm list"
  | Shape CoreTerm
  | Flat CoreTerm
  | Sharp CoreTerm

(* CoreType — structural type-head view. *)
datatype CoreType =
  UniverseTy nat
  | PiTy
  | SigmaTy
  | PathTyHead
  | RefineTy
  | GlueTy
  | InductiveTy string
  | OtherTy

(* KernelRule — the supported inference-rule names. *)
datatype KernelRule =
    K_Var
  | K_Univ
  | K_Pi_Form
  | K_Lam_Intro
  | K_App_Elim
  | K_Sigma_Form
  | K_Pair_Intro
  | K_Fst_Elim
  | K_Snd_Elim

type_synonym Ctx = "(string \<times> CoreTerm) list"

consts subst :: "string \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm"

inductive Typing :: "Ctx \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> bool"
  ("_ \<turnstile> _ : _" [60, 0, 0] 60)
where
  T_prop: "\<Gamma> \<turnstile> PropSort : Universe 0"
| T_var: "map_of \<Gamma> x = Some T \<Longrightarrow> \<Gamma> \<turnstile> Var x : T"
| T_univ: "i < 2^32 - 1 \<Longrightarrow> \<Gamma> \<turnstile> Universe i : Universe (Suc i)"
| T_pi: "\<lbrakk>\<Gamma> \<turnstile> A : Universe i; ((x, A) # \<Gamma>) \<turnstile> B : Universe i\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Pi x A B : Universe i"
| T_lam: "\<lbrakk>\<Gamma> \<turnstile> A : Universe i; ((x, A) # \<Gamma>) \<turnstile> b : B\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Lam x A b : Pi x A B"
| T_app: "\<lbrakk>\<Gamma> \<turnstile> f : Pi x A B; \<Gamma> \<turnstile> a : A\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> App f a : subst x a B"
| T_sigma: "\<lbrakk>\<Gamma> \<turnstile> A : Universe i; ((x, A) # \<Gamma>) \<turnstile> B : Universe i\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Sigma x A B : Universe i"
| T_pair: "\<lbrakk>\<Gamma> \<turnstile> Sigma x A B : Universe i; \<Gamma> \<turnstile> a : A; \<Gamma> \<turnstile> b : subst x a B\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Pair a b : Sigma x A B"
| T_fst: "\<Gamma> \<turnstile> p : Sigma x A B \<Longrightarrow> \<Gamma> \<turnstile> Fst p : A"
| T_snd: "\<Gamma> \<turnstile> p : Sigma x A B \<Longrightarrow> \<Gamma> \<turnstile> Snd p : subst x (Fst p) B"

(* K_Var — Structural schema, 0 premises *)
lemma K_Var_sound: "map_of \<Gamma> x = Some T \<Longrightarrow> \<Gamma> \<turnstile> Var x : T"
  by (fact T_var)

(* K_Univ — Structural schema, 0 premises *)
lemma K_Univ_sound: "i < 2^32 - 1 \<Longrightarrow> \<Gamma> \<turnstile> Universe i : Universe (Suc i)"
  by (fact T_univ)

(* K_Pi_Form — Unverified metatheory reference. Source core.verify.kernel_v0.lemmas.subst.subst_preserves_typing. Framework mathlib4. Citation Mathlib.LambdaCalculus.LambdaPi.Substitution.subst_preserves_typing. *)
lemma K_Pi_Form_sound: "\<lbrakk>\<Gamma> \<turnstile> A : Universe i; ((x, A) # \<Gamma>) \<turnstile> B : Universe i\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Pi x A B : Universe i"
  by (fact T_pi)

(* K_Lam_Intro — Unverified metatheory reference. Source core.verify.kernel_v0.lemmas.cartesian.cartesian_closure_for_pi. Framework mathlib4. Citation Mathlib.CategoryTheory.Closed.Cartesian. *)
lemma K_Lam_Intro_sound: "\<lbrakk>\<Gamma> \<turnstile> A : Universe i; ((x, A) # \<Gamma>) \<turnstile> b : B\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Lam x A b : Pi x A B"
  by (fact T_lam)

(* K_App_Elim — Unverified metatheory reference. Source core.verify.kernel_v0.lemmas.subst.subst_preserves_typing + core.verify.kernel_v0.lemmas.beta.church_rosser_confluence. Framework mathlib4. Citation Mathlib.LambdaCalculus.LambdaPi.Substitution + Mathlib.Computability.Lambda.ChurchRosser. *)
lemma K_App_Elim_sound: "\<lbrakk>\<Gamma> \<turnstile> f : Pi x A B; \<Gamma> \<turnstile> a : A\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> App f a : subst x a B"
  by (fact T_app)

(* K_Sigma_Form — Unverified metatheory reference. Source core.verify.kernel_v0.lemmas.subst.subst_preserves_typing. Framework mathlib4. Citation Mathlib.LambdaCalculus.LambdaPi.Substitution.subst_preserves_typing (Sigma form via duality). *)
lemma K_Sigma_Form_sound: "\<lbrakk>\<Gamma> \<turnstile> A : Universe i; ((x, A) # \<Gamma>) \<turnstile> B : Universe i\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Sigma x A B : Universe i"
  by (fact T_sigma)

(* K_Pair_Intro — Unverified metatheory reference. Source core.verify.kernel_v0.lemmas.subst.subst_preserves_typing. Framework mathlib4. Citation Mathlib.LambdaCalculus.LambdaPi.Substitution + dependent-product structure. *)
lemma K_Pair_Intro_sound: "\<lbrakk>\<Gamma> \<turnstile> Sigma x A B : Universe i; \<Gamma> \<turnstile> a : A; \<Gamma> \<turnstile> b : subst x a B\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Pair a b : Sigma x A B"
  by (fact T_pair)

(* K_Fst_Elim — Unverified metatheory reference. Source core.verify.kernel_v0.lemmas.eta.function_extensionality. Framework zfc. Citation Sigma-projection eta-rule (fst (a, b) ≡ a) — derivable from extensionality. *)
lemma K_Fst_Elim_sound: "\<Gamma> \<turnstile> p : Sigma x A B \<Longrightarrow> \<Gamma> \<turnstile> Fst p : A"
  by (fact T_fst)

(* K_Snd_Elim — Unverified metatheory reference. Source core.verify.kernel_v0.lemmas.eta.function_extensionality. Framework zfc. Citation Sigma-projection eta-rule (snd (a, b) : B[a/x]) — derivable from extensionality + subst. *)
lemma K_Snd_Elim_sound: "\<Gamma> \<turnstile> p : Sigma x A B \<Longrightarrow> \<Gamma> \<turnstile> Snd p : subst x (Fst p) B"
  by (fact T_snd)

(* K_Path_Ty_Form — Cubical schema, 3 premises *)
(* Outside this structural typing relation. *)

(* K_Path_Over_Form — Cubical schema, 4 premises *)
(* Outside this structural typing relation. *)

(* K_Refl_Intro — Cubical schema, 1 premises *)
(* Outside this structural typing relation. *)

(* UNSUPPORTED K_HComp: The executable kernel refuses composition until partial-element support and boundary coherence are checked *)

(* K_Transp — Cubical schema, 4 premises *)
(* Outside this structural typing relation. *)

(* UNSUPPORTED K_Glue: The executable kernel refuses gluing until equivalence and boundary coherence are checked *)

(* K_Refine — Refinement schema, 3 premises *)
(* Outside this structural typing relation. *)

(* MODEL UNAVAILABLE K_Refine_Omega: The export lacks the ordinal modal-depth inequality checked by the executable omega formation rule *)

(* K_Refine_Intro — Refinement schema, 3 premises *)
(* Outside this structural typing relation. *)

(* K_Refine_Erase — Refinement schema, 1 premises *)
(* Outside this structural typing relation. *)

(* K_Quot_Form — Quotient schema, 3 premises *)
(* Outside this structural typing relation. *)

(* K_Quot_Intro — Quotient schema, 2 premises *)
(* Outside this structural typing relation. *)

(* K_Quot_Elim — Quotient schema, 10 premises *)
(* Outside this structural typing relation. *)

(* MODEL UNAVAILABLE K_Inductive: The export lacks the declaration registry, parameter checks and declared universe used for inductive formation *)

(* MODEL UNAVAILABLE K_Pos: The export lacks the checked strict-positivity judgment for constructor argument types *)

(* MODEL UNAVAILABLE K_Elim: The export lacks the registered declaration and the typing judgment for every constructor branch *)

(* UNSUPPORTED K_Smt: Opaque SMT envelopes have no checked reconstruction and are refused by the executable kernel *)

(* MODEL UNAVAILABLE K_FwAx: The export lacks the authoritative axiom registry lookup and coordinate policy used by the executable kernel *)

(* UNSUPPORTED K_Eps_Mu: No executable rule authorizes assigning an arbitrary articulation the type of an enactment; the biadjunction correspondence remains unverified *)

(* K_Universe_Ascent — Diakrisis schema, 1 premises *)
(* Outside this structural typing relation. *)

(* UNSUPPORTED K_Round_Trip: No executable rule authorizes assigning an arbitrary term a recovered type; an encoding round-trip requires checked source correspondence *)

(* K_Epsilon_Of — Diakrisis schema, 1 premises *)
(* Outside this structural typing relation. *)

(* K_Alpha_Of — Diakrisis schema, 1 premises *)
(* Outside this structural typing relation. *)

(* K_Modal_Box — Diakrisis schema, 1 premises *)
(* Outside this structural typing relation. *)

(* K_Modal_Diamond — Diakrisis schema, 1 premises *)
(* Outside this structural typing relation. *)

(* K_Modal_Big_And — Diakrisis schema, 1 premises *)
(* Outside this structural typing relation. *)

(* K_Shape — Diakrisis schema, 1 premises *)
(* Outside this structural typing relation. *)

(* K_Flat — Diakrisis schema, 1 premises *)
(* Outside this structural typing relation. *)

(* K_Sharp — Diakrisis schema, 1 premises *)
(* Outside this structural typing relation. *)

lemmas structural_schema_lemmas =
  K_Var_sound
  K_Univ_sound
  K_Pi_Form_sound
  K_Lam_Intro_sound
  K_App_Elim_sound
  K_Sigma_Form_sound
  K_Pair_Intro_sound
  K_Fst_Elim_sound
  K_Snd_Elim_sound

end
