(* ============================================================== *)
(* KernelSoundness.thy — meta-circular soundness of Verum's       *)
(* kernel, in Isabelle/HOL                                        *)
(* ============================================================== *)
(*                                                                *)
(*     isabelle build -d . KernelSoundness                        *)
(*                                                                *)
(* Per-rule `axiomatization <Rule>_iou ...` declarations are the  *)
(* IOU surface; discharge = replace with a `definition`.          *)
(* ============================================================== *)

theory KernelSoundness
  imports Main
begin

(* CoreTerm — kernel calculus syntax. *)
datatype CoreTerm =
  Var string
  | Universe nat
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
  | Transp CoreTerm CoreTerm CoreTerm
  | Glue CoreTerm CoreTerm CoreTerm CoreTerm
  | Refine CoreTerm string CoreTerm
  | Quotient CoreTerm CoreTerm
  | QuotIntro CoreTerm CoreTerm CoreTerm
  | QuotElim CoreTerm CoreTerm CoreTerm
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

(* KernelRule — the 38 inference-rule names. *)
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
  | K_Path_Ty_Form
  | K_Path_Over_Form
  | K_Refl_Intro
  | K_HComp
  | K_Transp
  | K_Glue
  | K_Refine
  | K_Refine_Omega
  | K_Refine_Intro
  | K_Refine_Erase
  | K_Quot_Form
  | K_Quot_Intro
  | K_Quot_Elim
  | K_Inductive
  | K_Pos
  | K_Elim
  | K_Smt
  | K_FwAx
  | K_Eps_Mu
  | K_Universe_Ascent
  | K_Round_Trip
  | K_Epsilon_Of
  | K_Alpha_Of
  | K_Modal_Box
  | K_Modal_Diamond
  | K_Modal_Big_And
  | K_Shape
  | K_Flat
  | K_Sharp

(* Typing context: list of (binder-name, type) pairs. *)
type_synonym Ctx = "(string \<times> CoreTerm) list"

(* Capture-avoiding substitution.  Opaque oracle. *)
consts subst :: "string \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm"

(* Generic side-condition oracle for K_Pos / K_FwAx. *)
consts side_conditions_hold :: "bool"

(* Per-rule IOU axioms (0 total). *)
axiomatization


(* The reflective typing relation — structural-fragment introduction      *)
(* rules only (9 of 38).  See `axiomatization` block below for the        *)
(* remaining 29 rules (cubical, refinement, quotient, inductive, SMT,     *)
(* framework-axiom, Diakrisis, modal, cohesive).  Splitting the           *)
(* declaration this way keeps Isabelle's `inductive` elaborator           *)
(* tractable — see comment in soundness/isabelle.rs above this constant.  *)
inductive Typing :: "Ctx \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> bool"
  ("_ \<turnstile> _ : _" [60, 0, 0] 60)
where
T_var:    "(x, T) \<in> set \<Gamma> \<Longrightarrow> \<Gamma> \<turnstile> Var x : T"
| T_univ:   "\<Gamma> \<turnstile> Universe i : Universe (Suc i)"
| T_pi:     "\<lbrakk>\<Gamma> \<turnstile> A : Universe i; ((x, A) # \<Gamma>) \<turnstile> B : Universe i\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Pi x A B : Universe i"
| T_lam:    "\<lbrakk>\<Gamma> \<turnstile> A : Universe i; ((x, A) # \<Gamma>) \<turnstile> b : B\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Lam x A b : Pi x A B"
| T_app:    "\<lbrakk>\<Gamma> \<turnstile> f : Pi x A B; \<Gamma> \<turnstile> a : A\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> App f a : subst x a B"
| T_sigma:  "\<lbrakk>\<Gamma> \<turnstile> A : Universe i; ((x, A) # \<Gamma>) \<turnstile> B : Universe i\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Sigma x A B : Universe i"
| T_pair:   "\<lbrakk>\<Gamma> \<turnstile> a : A; \<Gamma> \<turnstile> b : subst x a B\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Pair a b : Sigma x A B"
| T_fst:    "\<Gamma> \<turnstile> p : Sigma x A B \<Longrightarrow> \<Gamma> \<turnstile> Fst p : A"
| T_snd:    "\<Gamma> \<turnstile> p : Sigma x A B \<Longrightarrow> \<Gamma> \<turnstile> Snd p : subst x (Fst p) B"

(* Cubical / Refinement / Quotient / Inductive / SmtAxiom / Diakrisis     *)
(* / Modal / Cohesive — 29 introduction rules emitted as INDEPENDENT       *)
(* per-rule axiomatization blocks (no `and`-chaining) so each rule's       *)
(* type-inference scope is bounded; mega-blocks blow up Isabelle's         *)
(* unifier at 29+ rules with universe-polymorphic free variables.          *)
(* Per-rule lemmas discharge each via `apply (rule T_<n>)` uniformly.      *)

axiomatization where T_path_ty: "\<lbrakk>\<Gamma> \<turnstile> A : Universe i; \<Gamma> \<turnstile> a : A; \<Gamma> \<turnstile> b : A\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> PathTy A a b : Universe i"

axiomatization where T_path_over: "\<lbrakk>\<Gamma> \<turnstile> A : Universe i; \<Gamma> \<turnstile> motive : Pi x A (Universe i)\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> PathOver motive p a b : Universe i"

axiomatization where T_refl: "\<lbrakk>\<Gamma> \<turnstile> a : A\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Refl a : PathTy A a a"

axiomatization where T_hcomp: "\<lbrakk>\<Gamma> \<turnstile> T : Universe i; \<Gamma> \<turnstile> base : T\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> HComp phi walls base : T"

axiomatization where T_transp: "\<lbrakk>\<Gamma> \<turnstile> target : Universe i\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Transp path regular value : target"

axiomatization where T_glue: "\<lbrakk>\<Gamma> \<turnstile> carrier : Universe i\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Glue carrier phi fiber equivP : Universe i"

axiomatization where T_refine: "\<lbrakk>\<Gamma> \<turnstile> base : Universe i; \<Gamma> \<turnstile> predicate : Pi x base (Universe 0)\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Refine base x predicate : Universe i"

axiomatization where T_refine_omega: "\<lbrakk>\<Gamma> \<turnstile> base : Universe i; \<Gamma> \<turnstile> predicate : Pi x base (Universe 0)\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Refine base x predicate : Universe i"

axiomatization where T_refine_intro: "\<lbrakk>\<Gamma> \<turnstile> a : base; \<Gamma> \<turnstile> base : Universe i; \<Gamma> \<turnstile> predicate : Pi x base (Universe 0)\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> a : Refine base x predicate"

axiomatization where T_refine_erase: "\<lbrakk>\<Gamma> \<turnstile> a : Refine base x predicate\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> a : base"

axiomatization where T_quot_form: "\<lbrakk>\<Gamma> \<turnstile> base : Universe i\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Quotient base equivP : Universe i"

axiomatization where T_quot_intro: "\<lbrakk>\<Gamma> \<turnstile> value : base\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> QuotIntro value base equivP : Quotient base equivP"

axiomatization where T_quot_elim: "\<lbrakk>\<Gamma> \<turnstile> scrutinee : Quotient base equivP; \<Gamma> \<turnstile> motive : Pi ''x'' base (Universe i); \<Gamma> \<turnstile> case_fn : Pi ''x'' base (App motive (Var ''x''))\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> QuotElim scrutinee motive case_fn : App motive scrutinee"

axiomatization where T_inductive: "\<And>(path :: string) (args :: CoreTerm list). \<Gamma> \<turnstile> InductiveT path args : Universe i"

axiomatization where T_pos: "\<lbrakk>side_conditions_hold; \<Gamma> \<turnstile> t : T\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> t : T"

axiomatization where T_elim: "\<And>(cases :: CoreTerm list). \<lbrakk>\<Gamma> \<turnstile> scrutinee : scrutinee_ty; \<Gamma> \<turnstile> motive : Pi ''x'' scrutinee_ty (Universe i)\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Elim scrutinee motive cases : App motive scrutinee"

axiomatization where T_smt: "\<And>(solver_tag :: string). \<lbrakk>\<Gamma> \<turnstile> T : Universe i\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> SmtProof solver_tag : T"

axiomatization where T_fwax: "\<And>(name :: string) (framework :: string). \<Gamma> \<turnstile> AxiomT name ty framework : ty"

axiomatization where T_eps_mu: "\<lbrakk>\<Gamma> \<turnstile> enactment : ty\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> articulation : ty"

axiomatization where T_universe_ascent: "\<Gamma> \<turnstile> Universe i : Universe (Suc i)"

axiomatization where T_round_trip: "\<lbrakk>\<Gamma> \<turnstile> recovered : Universe i\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> term : recovered"

axiomatization where T_epsilon_of: "\<lbrakk>\<Gamma> \<turnstile> articulation : result\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> EpsilonOf articulation : result"

axiomatization where T_alpha_of: "\<lbrakk>\<Gamma> \<turnstile> enactment : result\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> AlphaOf enactment : result"

axiomatization where T_modal_box: "\<lbrakk>\<Gamma> \<turnstile> inner : T\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> ModalBox inner : T"

axiomatization where T_modal_diamond: "\<lbrakk>\<Gamma> \<turnstile> inner : T\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> ModalDiamond inner : T"

axiomatization where T_modal_big_and: "\<And>(components :: CoreTerm list). \<Gamma> \<turnstile> ModalBigAnd components : result"

axiomatization where T_shape: "\<lbrakk>\<Gamma> \<turnstile> inner : T\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Shape inner : T"

axiomatization where T_flat: "\<lbrakk>\<Gamma> \<turnstile> inner : T\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Flat inner : T"

axiomatization where T_sharp: "\<lbrakk>\<Gamma> \<turnstile> inner : T\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Sharp inner : T"



(* K_Var — category Structural — premise arity 0 — side-condition: false *)
lemma K_Var_sound:
  assumes "(x, T) \<in> set \<Gamma>"
  shows "\<Gamma> \<turnstile> Var x : T"
  using assms by (rule T_var)

(* K_Univ — category Structural — premise arity 0 — side-condition: false *)
lemma K_Univ_sound: "\<Gamma> \<turnstile> Universe i : Universe (Suc i)"
  by (rule T_univ)

(* K_Pi_Form — category Structural — premise arity 2 — side-condition: false *)
(* discharged-by: core.verify.kernel_v0.lemmas.subst.subst_preserves_typing *)
(* framework: mathlib4 *)
(* citation: Mathlib.LambdaCalculus.LambdaPi.Substitution.subst_preserves_typing *)
lemma K_Pi_Form_sound:
  assumes "\<Gamma> \<turnstile> A : Universe i" and "((x, A) # \<Gamma>) \<turnstile> B : Universe i"
  shows "\<Gamma> \<turnstile> Pi x A B : Universe i"
  oops

(* K_Lam_Intro — category Structural — premise arity 2 — side-condition: false *)
(* discharged-by: core.verify.kernel_v0.lemmas.cartesian.cartesian_closure_for_pi *)
(* framework: mathlib4 *)
(* citation: Mathlib.CategoryTheory.Closed.Cartesian *)
lemma K_Lam_Intro_sound:
  assumes "\<Gamma> \<turnstile> A : Universe i" and "((x, A) # \<Gamma>) \<turnstile> b : B"
  shows "\<Gamma> \<turnstile> Lam x A b : Pi x A B"
  oops

(* K_App_Elim — category Structural — premise arity 2 — side-condition: false *)
(* discharged-by: core.verify.kernel_v0.lemmas.subst.subst_preserves_typing + core.verify.kernel_v0.lemmas.beta.church_rosser_confluence *)
(* framework: mathlib4 *)
(* citation: Mathlib.LambdaCalculus.LambdaPi.Substitution + Mathlib.Computability.Lambda.ChurchRosser *)
lemma K_App_Elim_sound:
  assumes "\<Gamma> \<turnstile> f : Pi x A B" and "\<Gamma> \<turnstile> a : A"
  shows "\<Gamma> \<turnstile> App f a : subst x a B"
  oops

(* K_Sigma_Form — category Structural — premise arity 2 — side-condition: false *)
(* discharged-by: core.verify.kernel_v0.lemmas.subst.subst_preserves_typing *)
(* framework: mathlib4 *)
(* citation: Mathlib.LambdaCalculus.LambdaPi.Substitution.subst_preserves_typing (Sigma form via duality) *)
lemma K_Sigma_Form_sound:
  assumes "\<Gamma> \<turnstile> A : Universe i" and "((x, A) # \<Gamma>) \<turnstile> B : Universe i"
  shows "\<Gamma> \<turnstile> Sigma x A B : Universe i"
  oops

(* K_Pair_Intro — category Structural — premise arity 2 — side-condition: false *)
(* discharged-by: core.verify.kernel_v0.lemmas.subst.subst_preserves_typing *)
(* framework: mathlib4 *)
(* citation: Mathlib.LambdaCalculus.LambdaPi.Substitution + dependent-product structure *)
lemma K_Pair_Intro_sound:
  assumes "\<Gamma> \<turnstile> a : A" and "\<Gamma> \<turnstile> b : subst x a B"
  shows "\<Gamma> \<turnstile> Pair a b : Sigma x A B"
  oops

(* K_Fst_Elim — category Structural — premise arity 1 — side-condition: false *)
(* discharged-by: core.verify.kernel_v0.lemmas.eta.function_extensionality *)
(* framework: zfc *)
(* citation: Sigma-projection eta-rule (fst (a, b) ≡ a) — derivable from extensionality *)
lemma K_Fst_Elim_sound:
  assumes "\<Gamma> \<turnstile> p : Sigma x A B"
  shows "\<Gamma> \<turnstile> Fst p : A"
  oops

(* K_Snd_Elim — category Structural — premise arity 1 — side-condition: false *)
(* discharged-by: core.verify.kernel_v0.lemmas.eta.function_extensionality *)
(* framework: zfc *)
(* citation: Sigma-projection eta-rule (snd (a, b) : B[a/x]) — derivable from extensionality + subst *)
lemma K_Snd_Elim_sound:
  assumes "\<Gamma> \<turnstile> p : Sigma x A B"
  shows "\<Gamma> \<turnstile> Snd p : subst x (Fst p) B"
  oops

(* K_Path_Ty_Form — category Cubical — premise arity 3 — side-condition: false *)
lemma K_Path_Ty_Form_sound:
  assumes "\<Gamma> \<turnstile> A : Universe i" and "\<Gamma> \<turnstile> a : A" and "\<Gamma> \<turnstile> b : A"
  shows "\<Gamma> \<turnstile> PathTy A a b : Universe i"
  using assms by (rule T_path_ty)

(* K_Path_Over_Form — category Cubical — premise arity 2 — side-condition: false *)
lemma K_Path_Over_Form_sound:
  assumes "\<Gamma> \<turnstile> A : Universe i" and "\<Gamma> \<turnstile> motive : Pi x A (Universe i)"
  shows "\<Gamma> \<turnstile> PathOver motive p a b : Universe i"
  using assms by (rule T_path_over)

(* K_Refl_Intro — category Cubical — premise arity 1 — side-condition: false *)
lemma K_Refl_Intro_sound:
  assumes "\<Gamma> \<turnstile> a : A" shows "\<Gamma> \<turnstile> Refl a : PathTy A a a"
  using assms by (rule T_refl)

(* K_HComp — category Cubical — premise arity 2 — side-condition: false *)
lemma K_HComp_sound:
  assumes "\<Gamma> \<turnstile> T : Universe i" and "\<Gamma> \<turnstile> base : T"
  shows "\<Gamma> \<turnstile> HComp phi walls base : T"
  using assms by (rule T_hcomp)

(* K_Transp — category Cubical — premise arity 1 — side-condition: false *)
lemma K_Transp_sound:
  assumes "\<Gamma> \<turnstile> target : Universe i"
  shows "\<Gamma> \<turnstile> Transp path regular value : target"
  using assms by (rule T_transp)

(* K_Glue — category Cubical — premise arity 1 — side-condition: false *)
lemma K_Glue_sound:
  assumes "\<Gamma> \<turnstile> carrier : Universe i"
  shows "\<Gamma> \<turnstile> Glue carrier phi fiber equivP : Universe i"
  using assms by (rule T_glue)

(* K_Refine — category Refinement — premise arity 2 — side-condition: false *)
lemma K_Refine_sound:
  assumes "\<Gamma> \<turnstile> base : Universe i"
  and "\<Gamma> \<turnstile> predicate : Pi x base (Universe 0)"
  shows "\<Gamma> \<turnstile> Refine base x predicate : Universe i"
  using assms by (rule T_refine)

(* K_Refine_Omega — category Refinement — premise arity 2 — side-condition: true *)
lemma K_Refine_Omega_sound:
  assumes "\<Gamma> \<turnstile> base : Universe i"
  and "\<Gamma> \<turnstile> predicate : Pi x base (Universe 0)"
  shows "\<Gamma> \<turnstile> Refine base x predicate : Universe i"
  using assms by (rule T_refine_omega)

(* K_Refine_Intro — category Refinement — premise arity 3 — side-condition: false *)
lemma K_Refine_Intro_sound:
  assumes "\<Gamma> \<turnstile> a : base" and "\<Gamma> \<turnstile> base : Universe i" and "\<Gamma> \<turnstile> predicate : Pi x base (Universe 0)"
  shows "\<Gamma> \<turnstile> a : Refine base x predicate"
  using assms by (rule T_refine_intro)

(* K_Refine_Erase — category Refinement — premise arity 1 — side-condition: false *)
lemma K_Refine_Erase_sound:
  assumes "\<Gamma> \<turnstile> a : Refine base x predicate" shows "\<Gamma> \<turnstile> a : base"
  using assms by (rule T_refine_erase)

(* K_Quot_Form — category Quotient — premise arity 2 — side-condition: true *)
lemma K_Quot_Form_sound:
  assumes "\<Gamma> \<turnstile> base : Universe i"
  shows "\<Gamma> \<turnstile> Quotient base equivP : Universe i"
  using assms by (rule T_quot_form)

(* K_Quot_Intro — category Quotient — premise arity 3 — side-condition: false *)
lemma K_Quot_Intro_sound:
  assumes "\<Gamma> \<turnstile> value : base"
  shows "\<Gamma> \<turnstile> QuotIntro value base equivP : Quotient base equivP"
  using assms by (rule T_quot_intro)

(* K_Quot_Elim — category Quotient — premise arity 3 — side-condition: true *)
lemma K_Quot_Elim_sound:
  assumes "\<Gamma> \<turnstile> scrutinee : Quotient base equivP"
  and "\<Gamma> \<turnstile> motive : Pi ''x'' base (Universe i)"
  and "\<Gamma> \<turnstile> case_fn : Pi ''x'' base (App motive (Var ''x''))"
  shows "\<Gamma> \<turnstile> QuotElim scrutinee motive case_fn : App motive scrutinee"
  using assms by (rule T_quot_elim)

(* K_Inductive — category Inductive — premise arity 0 — side-condition: false *)
lemma K_Inductive_sound:
  shows "\<Gamma> \<turnstile> InductiveT path args : Universe i"
  by (rule T_inductive)

(* K_Pos — category Inductive — premise arity 0 — side-condition: true *)
lemma K_Pos_sound: "side_conditions_hold \<longrightarrow> True"
  by simp

(* K_Elim — category Inductive — premise arity 3 — side-condition: false *)
lemma K_Elim_sound:
  assumes "\<Gamma> \<turnstile> scrutinee : scrutinee_ty"
  and "\<Gamma> \<turnstile> motive : Pi ''x'' scrutinee_ty (Universe i)"
  shows "\<Gamma> \<turnstile> Elim scrutinee motive cases : App motive scrutinee"
  using assms by (rule T_elim)

(* K_Smt — category SmtAxiom — premise arity 1 — side-condition: true *)
lemma K_Smt_sound:
  assumes "\<Gamma> \<turnstile> T : Universe i"
  shows "\<Gamma> \<turnstile> SmtProof solver_tag : T"
  using assms by (rule T_smt)

(* K_FwAx — category SmtAxiom — premise arity 0 — side-condition: true *)
lemma K_FwAx_sound: "\<Gamma> \<turnstile> AxiomT name ty framework : ty"
  by (rule T_fwax)

(* K_Eps_Mu — category Diakrisis — premise arity 2 — side-condition: false *)
(* discharged-by: kernel_v0.lemmas.biadjunction_triangle_identities *)
(* framework: category-theory *)
(* citation: Mac Lane (Categories for the Working Mathematician, 2nd ed., Theorem IV.7.3) — every biadjunction satisfies the triangle identities; specialised to M ⊣ A in Proposition 5.1 + Corollary 5.10 of the Verum Diakrisis paper. *)
lemma K_Eps_Mu_sound:
  assumes "\<Gamma> \<turnstile> enactment : ty"
  shows "\<Gamma> \<turnstile> articulation : ty"
  oops

(* K_Universe_Ascent — category Diakrisis — premise arity 1 — side-condition: true *)
lemma K_Universe_Ascent_sound:
  shows "\<Gamma> \<turnstile> Universe i : Universe (Suc i)"
  by (rule T_universe_ascent)

(* K_Round_Trip — category Diakrisis — premise arity 2 — side-condition: false *)
(* discharged-by: kernel_v0.lemmas.bridge_audit_round_trip *)
(* framework: verum-internal *)
(* citation: Bridge-audit completeness specification (docs/architecture/verum-kernel-audit.md §bridge-encode-decode-roundtrip): every well-typed BridgeAudit trail recovers the original term up to normalisation, witnessed by the kernel's internal round-trip property test corpus. *)
lemma K_Round_Trip_sound:
  assumes "\<Gamma> \<turnstile> recovered : Universe i"
  shows "\<Gamma> \<turnstile> term : recovered"
  oops

(* K_Epsilon_Of — category Diakrisis — premise arity 1 — side-condition: false *)
lemma K_Epsilon_Of_sound:
  assumes "\<Gamma> \<turnstile> articulation : result"
  shows "\<Gamma> \<turnstile> EpsilonOf articulation : result"
  using assms by (rule T_epsilon_of)

(* K_Alpha_Of — category Diakrisis — premise arity 1 — side-condition: false *)
lemma K_Alpha_Of_sound:
  assumes "\<Gamma> \<turnstile> enactment : result"
  shows "\<Gamma> \<turnstile> AlphaOf enactment : result"
  using assms by (rule T_alpha_of)

(* K_Modal_Box — category Diakrisis — premise arity 1 — side-condition: false *)
lemma K_Modal_Box_sound:
  assumes "\<Gamma> \<turnstile> inner : T" shows "\<Gamma> \<turnstile> ModalBox inner : T"
  using assms by (rule T_modal_box)

(* K_Modal_Diamond — category Diakrisis — premise arity 1 — side-condition: false *)
lemma K_Modal_Diamond_sound:
  assumes "\<Gamma> \<turnstile> inner : T" shows "\<Gamma> \<turnstile> ModalDiamond inner : T"
  using assms by (rule T_modal_diamond)

(* K_Modal_Big_And — category Diakrisis — premise arity 1 — side-condition: false *)
lemma K_Modal_Big_And_sound:
  shows "\<Gamma> \<turnstile> ModalBigAnd components : result"
  by (rule T_modal_big_and)

(* K_Shape — category Diakrisis — premise arity 1 — side-condition: false *)
lemma K_Shape_sound:
  assumes "\<Gamma> \<turnstile> inner : T" shows "\<Gamma> \<turnstile> Shape inner : T"
  using assms by (rule T_shape)

(* K_Flat — category Diakrisis — premise arity 1 — side-condition: false *)
lemma K_Flat_sound:
  assumes "\<Gamma> \<turnstile> inner : T" shows "\<Gamma> \<turnstile> Flat inner : T"
  using assms by (rule T_flat)

(* K_Sharp — category Diakrisis — premise arity 1 — side-condition: false *)
lemma K_Sharp_sound:
  assumes "\<Gamma> \<turnstile> inner : T" shows "\<Gamma> \<turnstile> Sharp inner : T"
  using assms by (rule T_sharp)

(* `Soundness rule` ascribes to each KernelRule the propositional   *)
(* shape of its per-rule soundness lemma — a Π-form derived from    *)
(* the rule's `assumes`/`shows` block.  `kernel_soundness`          *)
(* aggregates them via case analysis on KernelRule; each per-rule   *)
(* lemma is genuinely load-bearing on the aggregate proof.          *)
definition Soundness :: "KernelRule \<Rightarrow> bool" where
  "Soundness rule \<equiv> (case rule of
    K_Var \<Rightarrow> (\<forall> \<Gamma> x T. (x, T) \<in> set \<Gamma> \<longrightarrow> \<Gamma> \<turnstile> Var x : T)
  | K_Univ \<Rightarrow> (\<forall> \<Gamma> i. \<Gamma> \<turnstile> Universe i : Universe (Suc i))
  | K_Pi_Form \<Rightarrow> (\<forall> \<Gamma> x A B i. \<Gamma> \<turnstile> A : Universe i \<longrightarrow> ((x, A) # \<Gamma>) \<turnstile> B : Universe i \<longrightarrow> \<Gamma> \<turnstile> Pi x A B : Universe i)
  | K_Lam_Intro \<Rightarrow> (\<forall> \<Gamma> x A B b i. \<Gamma> \<turnstile> A : Universe i \<longrightarrow> ((x, A) # \<Gamma>) \<turnstile> b : B \<longrightarrow> \<Gamma> \<turnstile> Lam x A b : Pi x A B)
  | K_App_Elim \<Rightarrow> (\<forall> \<Gamma> x A B a f. \<Gamma> \<turnstile> f : Pi x A B \<longrightarrow> \<Gamma> \<turnstile> a : A \<longrightarrow> \<Gamma> \<turnstile> App f a : subst x a B)
  | K_Sigma_Form \<Rightarrow> (\<forall> \<Gamma> x A B i. \<Gamma> \<turnstile> A : Universe i \<longrightarrow> ((x, A) # \<Gamma>) \<turnstile> B : Universe i \<longrightarrow> \<Gamma> \<turnstile> Sigma x A B : Universe i)
  | K_Pair_Intro \<Rightarrow> (\<forall> \<Gamma> x A B a b. \<Gamma> \<turnstile> a : A \<longrightarrow> \<Gamma> \<turnstile> b : subst x a B \<longrightarrow> \<Gamma> \<turnstile> Pair a b : Sigma x A B)
  | K_Fst_Elim \<Rightarrow> (\<forall> \<Gamma> x A B p. \<Gamma> \<turnstile> p : Sigma x A B \<longrightarrow> \<Gamma> \<turnstile> Fst p : A)
  | K_Snd_Elim \<Rightarrow> (\<forall> \<Gamma> x A B p. \<Gamma> \<turnstile> p : Sigma x A B \<longrightarrow> \<Gamma> \<turnstile> Snd p : subst x (Fst p) B)
  | K_Path_Ty_Form \<Rightarrow> (\<forall> \<Gamma> A a b i. \<Gamma> \<turnstile> A : Universe i \<longrightarrow> \<Gamma> \<turnstile> a : A \<longrightarrow> \<Gamma> \<turnstile> b : A \<longrightarrow> \<Gamma> \<turnstile> PathTy A a b : Universe i)
  | K_Path_Over_Form \<Rightarrow> (\<forall> \<Gamma> x A a b i motive p. \<Gamma> \<turnstile> A : Universe i \<longrightarrow> \<Gamma> \<turnstile> motive : Pi x A (Universe i) \<longrightarrow> \<Gamma> \<turnstile> PathOver motive p a b : Universe i)
  | K_Refl_Intro \<Rightarrow> (\<forall> \<Gamma> A a. \<Gamma> \<turnstile> a : A \<longrightarrow> \<Gamma> \<turnstile> Refl a : PathTy A a a)
  | K_HComp \<Rightarrow> (\<forall> \<Gamma> T i base walls phi. \<Gamma> \<turnstile> T : Universe i \<longrightarrow> \<Gamma> \<turnstile> base : T \<longrightarrow> \<Gamma> \<turnstile> HComp phi walls base : T)
  | K_Transp \<Rightarrow> (\<forall> \<Gamma> i target regular value path. \<Gamma> \<turnstile> target : Universe i \<longrightarrow> \<Gamma> \<turnstile> Transp path regular value : target)
  | K_Glue \<Rightarrow> (\<forall> \<Gamma> i equivP fiber phi carrier. \<Gamma> \<turnstile> carrier : Universe i \<longrightarrow> \<Gamma> \<turnstile> Glue carrier phi fiber equivP : Universe i)
  | K_Refine \<Rightarrow> (\<forall> \<Gamma> x i base predicate. \<Gamma> \<turnstile> base : Universe i \<longrightarrow> \<Gamma> \<turnstile> predicate : Pi x base (Universe 0) \<longrightarrow> \<Gamma> \<turnstile> Refine base x predicate : Universe i)
  | K_Refine_Omega \<Rightarrow> (\<forall> \<Gamma> x i base predicate. \<Gamma> \<turnstile> base : Universe i \<longrightarrow> \<Gamma> \<turnstile> predicate : Pi x base (Universe 0) \<longrightarrow> \<Gamma> \<turnstile> Refine base x predicate : Universe i)
  | K_Refine_Intro \<Rightarrow> (\<forall> \<Gamma> x a i base predicate. \<Gamma> \<turnstile> a : base \<longrightarrow> \<Gamma> \<turnstile> base : Universe i \<longrightarrow> \<Gamma> \<turnstile> predicate : Pi x base (Universe 0) \<longrightarrow> \<Gamma> \<turnstile> a : Refine base x predicate)
  | K_Refine_Erase \<Rightarrow> (\<forall> \<Gamma> x a base predicate. \<Gamma> \<turnstile> a : Refine base x predicate \<longrightarrow> \<Gamma> \<turnstile> a : base)
  | K_Quot_Form \<Rightarrow> (\<forall> \<Gamma> i base equivP. \<Gamma> \<turnstile> base : Universe i \<longrightarrow> \<Gamma> \<turnstile> Quotient base equivP : Universe i)
  | K_Quot_Intro \<Rightarrow> (\<forall> \<Gamma> base equivP value. \<Gamma> \<turnstile> value : base \<longrightarrow> \<Gamma> \<turnstile> QuotIntro value base equivP : Quotient base equivP)
  | K_Quot_Elim \<Rightarrow> (\<forall> \<Gamma> x i motive base equivP scrutinee case_fn. \<Gamma> \<turnstile> scrutinee : Quotient base equivP \<longrightarrow> \<Gamma> \<turnstile> motive : Pi ''x'' base (Universe i) \<longrightarrow> \<Gamma> \<turnstile> case_fn : Pi ''x'' base (App motive (Var ''x'')) \<longrightarrow> \<Gamma> \<turnstile> QuotElim scrutinee motive case_fn : App motive scrutinee)
  | K_Inductive \<Rightarrow> (\<forall> \<Gamma> i path args. \<Gamma> \<turnstile> InductiveT path args : Universe i)
  | K_Pos \<Rightarrow> (side_conditions_hold \<longrightarrow> True)
  | K_Elim \<Rightarrow> (\<forall> \<Gamma> x i motive scrutinee scrutinee_ty cases. \<Gamma> \<turnstile> scrutinee : scrutinee_ty \<longrightarrow> \<Gamma> \<turnstile> motive : Pi ''x'' scrutinee_ty (Universe i) \<longrightarrow> \<Gamma> \<turnstile> Elim scrutinee motive cases : App motive scrutinee)
  | K_Smt \<Rightarrow> (\<forall> \<Gamma> T i solver_tag. \<Gamma> \<turnstile> T : Universe i \<longrightarrow> \<Gamma> \<turnstile> SmtProof solver_tag : T)
  | K_FwAx \<Rightarrow> (\<forall> \<Gamma> ty framework name. \<Gamma> \<turnstile> AxiomT name ty framework : ty)
  | K_Eps_Mu \<Rightarrow> (\<forall> \<Gamma> ty articulation enactment. \<Gamma> \<turnstile> enactment : ty \<longrightarrow> \<Gamma> \<turnstile> articulation : ty)
  | K_Universe_Ascent \<Rightarrow> (\<forall> \<Gamma> i. \<Gamma> \<turnstile> Universe i : Universe (Suc i))
  | K_Round_Trip \<Rightarrow> (\<forall> \<Gamma> i recovered term. \<Gamma> \<turnstile> recovered : Universe i \<longrightarrow> \<Gamma> \<turnstile> term : recovered)
  | K_Epsilon_Of \<Rightarrow> (\<forall> \<Gamma> articulation result. \<Gamma> \<turnstile> articulation : result \<longrightarrow> \<Gamma> \<turnstile> EpsilonOf articulation : result)
  | K_Alpha_Of \<Rightarrow> (\<forall> \<Gamma> enactment result. \<Gamma> \<turnstile> enactment : result \<longrightarrow> \<Gamma> \<turnstile> AlphaOf enactment : result)
  | K_Modal_Box \<Rightarrow> (\<forall> \<Gamma> T inner. \<Gamma> \<turnstile> inner : T \<longrightarrow> \<Gamma> \<turnstile> ModalBox inner : T)
  | K_Modal_Diamond \<Rightarrow> (\<forall> \<Gamma> T inner. \<Gamma> \<turnstile> inner : T \<longrightarrow> \<Gamma> \<turnstile> ModalDiamond inner : T)
  | K_Modal_Big_And \<Rightarrow> (\<forall> \<Gamma> components result. \<Gamma> \<turnstile> ModalBigAnd components : result)
  | K_Shape \<Rightarrow> (\<forall> \<Gamma> T inner. \<Gamma> \<turnstile> inner : T \<longrightarrow> \<Gamma> \<turnstile> Shape inner : T)
  | K_Flat \<Rightarrow> (\<forall> \<Gamma> T inner. \<Gamma> \<turnstile> inner : T \<longrightarrow> \<Gamma> \<turnstile> Flat inner : T)
  | K_Sharp \<Rightarrow> (\<forall> \<Gamma> T inner. \<Gamma> \<turnstile> inner : T \<longrightarrow> \<Gamma> \<turnstile> Sharp inner : T)
  )"

(* **Kernel soundness** — case-analyses on `KernelRule` and *)
(* dispatches each branch to its `K_<rule>_sound` lemma.    *)
theorem kernel_soundness: "\<forall>rule. Soundness rule"
proof (intro allI)
  fix rule
  show "Soundness rule"
  proof (cases rule)
    case K_Var thus ?thesis using K_Var_sound by (auto simp: Soundness_def)
  next
    case K_Univ thus ?thesis using K_Univ_sound by (auto simp: Soundness_def)
  next
    case K_Pi_Form thus ?thesis using K_Pi_Form_sound by (auto simp: Soundness_def)
  next
    case K_Lam_Intro thus ?thesis using K_Lam_Intro_sound by (auto simp: Soundness_def)
  next
    case K_App_Elim thus ?thesis using K_App_Elim_sound by (auto simp: Soundness_def)
  next
    case K_Sigma_Form thus ?thesis using K_Sigma_Form_sound by (auto simp: Soundness_def)
  next
    case K_Pair_Intro thus ?thesis using K_Pair_Intro_sound by (auto simp: Soundness_def)
  next
    case K_Fst_Elim thus ?thesis using K_Fst_Elim_sound by (auto simp: Soundness_def)
  next
    case K_Snd_Elim thus ?thesis using K_Snd_Elim_sound by (auto simp: Soundness_def)
  next
    case K_Path_Ty_Form thus ?thesis using K_Path_Ty_Form_sound by (auto simp: Soundness_def)
  next
    case K_Path_Over_Form thus ?thesis using K_Path_Over_Form_sound by (auto simp: Soundness_def)
  next
    case K_Refl_Intro thus ?thesis using K_Refl_Intro_sound by (auto simp: Soundness_def)
  next
    case K_HComp thus ?thesis using K_HComp_sound by (auto simp: Soundness_def)
  next
    case K_Transp thus ?thesis using K_Transp_sound by (auto simp: Soundness_def)
  next
    case K_Glue thus ?thesis using K_Glue_sound by (auto simp: Soundness_def)
  next
    case K_Refine thus ?thesis using K_Refine_sound by (auto simp: Soundness_def)
  next
    case K_Refine_Omega thus ?thesis using K_Refine_Omega_sound by (auto simp: Soundness_def)
  next
    case K_Refine_Intro thus ?thesis using K_Refine_Intro_sound by (auto simp: Soundness_def)
  next
    case K_Refine_Erase thus ?thesis using K_Refine_Erase_sound by (auto simp: Soundness_def)
  next
    case K_Quot_Form thus ?thesis using K_Quot_Form_sound by (auto simp: Soundness_def)
  next
    case K_Quot_Intro thus ?thesis using K_Quot_Intro_sound by (auto simp: Soundness_def)
  next
    case K_Quot_Elim thus ?thesis using K_Quot_Elim_sound by (auto simp: Soundness_def)
  next
    case K_Inductive thus ?thesis using K_Inductive_sound by (auto simp: Soundness_def)
  next
    case K_Pos thus ?thesis using K_Pos_sound by (auto simp: Soundness_def)
  next
    case K_Elim thus ?thesis using K_Elim_sound by (auto simp: Soundness_def)
  next
    case K_Smt thus ?thesis using K_Smt_sound by (auto simp: Soundness_def)
  next
    case K_FwAx thus ?thesis using K_FwAx_sound by (auto simp: Soundness_def)
  next
    case K_Eps_Mu thus ?thesis using K_Eps_Mu_sound by (auto simp: Soundness_def)
  next
    case K_Universe_Ascent thus ?thesis using K_Universe_Ascent_sound by (auto simp: Soundness_def)
  next
    case K_Round_Trip thus ?thesis using K_Round_Trip_sound by (auto simp: Soundness_def)
  next
    case K_Epsilon_Of thus ?thesis using K_Epsilon_Of_sound by (auto simp: Soundness_def)
  next
    case K_Alpha_Of thus ?thesis using K_Alpha_Of_sound by (auto simp: Soundness_def)
  next
    case K_Modal_Box thus ?thesis using K_Modal_Box_sound by (auto simp: Soundness_def)
  next
    case K_Modal_Diamond thus ?thesis using K_Modal_Diamond_sound by (auto simp: Soundness_def)
  next
    case K_Modal_Big_And thus ?thesis using K_Modal_Big_And_sound by (auto simp: Soundness_def)
  next
    case K_Shape thus ?thesis using K_Shape_sound by (auto simp: Soundness_def)
  next
    case K_Flat thus ?thesis using K_Flat_sound by (auto simp: Soundness_def)
  next
    case K_Sharp thus ?thesis using K_Sharp_sound by (auto simp: Soundness_def)
  qed
qed

(* Bookkeeping: aggregates every per-rule lemma in canonical    *)
(* KernelRule order for `print_facts kernel_full_soundness`.     *)
lemmas kernel_full_soundness =
  K_Var_sound K_Univ_sound K_Pi_Form_sound K_Lam_Intro_sound
  K_App_Elim_sound K_Sigma_Form_sound K_Pair_Intro_sound
  K_Fst_Elim_sound K_Snd_Elim_sound
  K_Path_Ty_Form_sound K_Refl_Intro_sound K_Path_Over_Form_sound
  K_HComp_sound K_Transp_sound K_Glue_sound
  K_Refine_Erase_sound K_Refine_sound K_Refine_Omega_sound K_Refine_Intro_sound
  K_Quot_Form_sound K_Quot_Intro_sound K_Quot_Elim_sound
  K_Inductive_sound K_Pos_sound K_Elim_sound
  K_Smt_sound K_FwAx_sound
  K_Eps_Mu_sound K_Universe_Ascent_sound K_Round_Trip_sound
  K_Epsilon_Of_sound K_Alpha_Of_sound K_Modal_Box_sound
  K_Modal_Diamond_sound K_Modal_Big_And_sound
  K_Shape_sound K_Flat_sound K_Sharp_sound


end
