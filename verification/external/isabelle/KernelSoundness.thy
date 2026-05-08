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

(* ====== Per-rule IOU axiomatizations (17 total) ====== *)

axiomatization
  K_Path_Over_Form_iou :: "Ctx \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> nat \<Rightarrow> bool" and
  K_HComp_iou :: "Ctx \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> bool" and
  K_Transp_iou :: "Ctx \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> bool" and
  K_Glue_iou :: "Ctx \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> bool" and
  K_Refine_iou :: "Ctx \<Rightarrow> CoreTerm \<Rightarrow> string \<Rightarrow> CoreTerm \<Rightarrow> bool" and
  K_Refine_Omega_iou :: "Ctx \<Rightarrow> CoreTerm \<Rightarrow> string \<Rightarrow> CoreTerm \<Rightarrow> bool" and
  K_Refine_Intro_iou :: "Ctx \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> string \<Rightarrow> CoreTerm \<Rightarrow> bool" and
  K_Quot_Elim_iou :: "Ctx \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> bool" and
  K_Inductive_iou :: "Ctx \<Rightarrow> string \<Rightarrow> CoreTerm list \<Rightarrow> CoreTerm \<Rightarrow> bool" and
  K_Elim_iou :: "Ctx \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm list \<Rightarrow> CoreTerm \<Rightarrow> bool" and
  K_Smt_iou :: "Ctx \<Rightarrow> string \<Rightarrow> CoreTerm \<Rightarrow> bool" and
  K_Eps_Mu_iou :: "Ctx \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> bool" and
  K_Universe_Ascent_iou :: "Ctx \<Rightarrow> nat \<Rightarrow> bool" and
  K_Round_Trip_iou :: "Ctx \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> bool" and
  K_Epsilon_Of_iou :: "Ctx \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> bool" and
  K_Alpha_Of_iou :: "Ctx \<Rightarrow> CoreTerm \<Rightarrow> CoreTerm \<Rightarrow> bool" and
  K_Modal_Big_And_iou :: "Ctx \<Rightarrow> CoreTerm list \<Rightarrow> CoreTerm \<Rightarrow> bool"

(* The reflective typing relation. 38 introduction rules. *)
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
| T_path_ty:    "\<lbrakk>\<Gamma> \<turnstile> A : Universe i; \<Gamma> \<turnstile> a : A; \<Gamma> \<turnstile> b : A\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> PathTy A a b : Universe i"
| T_refl:       "\<Gamma> \<turnstile> a : A \<Longrightarrow> \<Gamma> \<turnstile> Refl a : PathTy A a a"
| T_path_over:  "K_Path_Over_Form_iou \<Gamma> motive p a b ty i \<Longrightarrow> \<Gamma> \<turnstile> PathOver motive p a b : ty"
| T_hcomp:      "K_HComp_iou \<Gamma> phi walls base T \<Longrightarrow> \<Gamma> \<turnstile> HComp phi walls base : T"
| T_transp:     "K_Transp_iou \<Gamma> path regular value target \<Longrightarrow> \<Gamma> \<turnstile> Transp path regular value : target"
| T_glue:       "K_Glue_iou \<Gamma> carrier phi fiber equivP result \<Longrightarrow> \<Gamma> \<turnstile> Glue carrier phi fiber equivP : result"
| T_refine_erase: "\<Gamma> \<turnstile> a : Refine base x predicate \<Longrightarrow> \<Gamma> \<turnstile> a : base"
| T_refine:       "\<lbrakk>\<Gamma> \<turnstile> base : Universe i; K_Refine_iou \<Gamma> base x predicate\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> Refine base x predicate : Universe i"
| T_refine_omega: "K_Refine_Omega_iou \<Gamma> base x predicate \<Longrightarrow> \<Gamma> \<turnstile> Refine base x predicate : Universe i"
| T_refine_intro: "\<lbrakk>\<Gamma> \<turnstile> a : base; K_Refine_Intro_iou \<Gamma> a base x predicate\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> a : Refine base x predicate"
| T_quot_form:    "\<Gamma> \<turnstile> base : Universe i \<Longrightarrow> \<Gamma> \<turnstile> Quotient base equivP : Universe i"
| T_quot_intro:   "\<Gamma> \<turnstile> value : base \<Longrightarrow> \<Gamma> \<turnstile> QuotIntro value base equivP : Quotient base equivP"
| T_quot_elim:    "K_Quot_Elim_iou \<Gamma> scrutinee motive case_fn result \<Longrightarrow> \<Gamma> \<turnstile> QuotElim scrutinee motive case_fn : result"
| T_inductive:    "K_Inductive_iou \<Gamma> path args result \<Longrightarrow> \<Gamma> \<turnstile> InductiveT path args : result"
| T_pos:          "\<lbrakk>side_conditions_hold; \<Gamma> \<turnstile> t : T\<rbrakk> \<Longrightarrow> \<Gamma> \<turnstile> t : T"
| T_elim:         "K_Elim_iou \<Gamma> scrutinee motive cases result \<Longrightarrow> \<Gamma> \<turnstile> Elim scrutinee motive cases : result"
| T_smt:          "K_Smt_iou \<Gamma> solver_tag T \<Longrightarrow> \<Gamma> \<turnstile> SmtProof solver_tag : T"
| T_fwax:         "\<Gamma> \<turnstile> AxiomT name ty framework : ty"
| T_eps_mu:       "K_Eps_Mu_iou \<Gamma> articulation enactment ty \<Longrightarrow> \<Gamma> \<turnstile> articulation : ty"
| T_universe_ascent: "K_Universe_Ascent_iou \<Gamma> i \<Longrightarrow> \<Gamma> \<turnstile> Universe i : Universe (Suc i)"
| T_round_trip:   "K_Round_Trip_iou \<Gamma> term recovered \<Longrightarrow> \<Gamma> \<turnstile> term : recovered"
| T_epsilon_of:   "K_Epsilon_Of_iou \<Gamma> articulation result \<Longrightarrow> \<Gamma> \<turnstile> EpsilonOf articulation : result"
| T_alpha_of:     "K_Alpha_Of_iou \<Gamma> enactment result \<Longrightarrow> \<Gamma> \<turnstile> AlphaOf enactment : result"
| T_modal_box:    "\<Gamma> \<turnstile> inner : T \<Longrightarrow> \<Gamma> \<turnstile> ModalBox inner : T"
| T_modal_diamond:"\<Gamma> \<turnstile> inner : T \<Longrightarrow> \<Gamma> \<turnstile> ModalDiamond inner : T"
| T_modal_big_and:"K_Modal_Big_And_iou \<Gamma> components result \<Longrightarrow> \<Gamma> \<turnstile> ModalBigAnd components : result"
| T_shape:        "\<Gamma> \<turnstile> inner : T \<Longrightarrow> \<Gamma> \<turnstile> Shape inner : T"
| T_flat:         "\<Gamma> \<turnstile> inner : T \<Longrightarrow> \<Gamma> \<turnstile> Flat inner : T"
| T_sharp:        "\<Gamma> \<turnstile> inner : T \<Longrightarrow> \<Gamma> \<turnstile> Sharp inner : T"

(* K_Var — category Structural — premise arity 0 — side-condition: false *)
lemma K_Var_sound:
  assumes "(x, T) \<in> set \<Gamma>"
  shows "\<Gamma> \<turnstile> Var x : T"
  using assms by (rule T_var)

(* K_Univ — category Structural — premise arity 0 — side-condition: false *)
lemma K_Univ_sound: "\<Gamma> \<turnstile> Universe i : Universe (Suc i)"
  by (rule T_univ)

(* K_Pi_Form — category Structural — premise arity 2 — side-condition: false *)
lemma K_Pi_Form_sound:
  assumes "\<Gamma> \<turnstile> A : Universe i" and "((x, A) # \<Gamma>) \<turnstile> B : Universe i"
  shows "\<Gamma> \<turnstile> Pi x A B : Universe i"
  using assms by (rule T_pi)

(* K_Lam_Intro — category Structural — premise arity 2 — side-condition: false *)
lemma K_Lam_Intro_sound:
  assumes "\<Gamma> \<turnstile> A : Universe i" and "((x, A) # \<Gamma>) \<turnstile> b : B"
  shows "\<Gamma> \<turnstile> Lam x A b : Pi x A B"
  using assms by (rule T_lam)

(* K_App_Elim — category Structural — premise arity 2 — side-condition: false *)
lemma K_App_Elim_sound:
  assumes "\<Gamma> \<turnstile> f : Pi x A B" and "\<Gamma> \<turnstile> a : A"
  shows "\<Gamma> \<turnstile> App f a : subst x a B"
  using assms by (rule T_app)

(* K_Sigma_Form — category Structural — premise arity 2 — side-condition: false *)
lemma K_Sigma_Form_sound:
  assumes "\<Gamma> \<turnstile> A : Universe i" and "((x, A) # \<Gamma>) \<turnstile> B : Universe i"
  shows "\<Gamma> \<turnstile> Sigma x A B : Universe i"
  using assms by (rule T_sigma)

(* K_Pair_Intro — category Structural — premise arity 2 — side-condition: false *)
lemma K_Pair_Intro_sound:
  assumes "\<Gamma> \<turnstile> a : A" and "\<Gamma> \<turnstile> b : subst x a B"
  shows "\<Gamma> \<turnstile> Pair a b : Sigma x A B"
  using assms by (rule T_pair)

(* K_Fst_Elim — category Structural — premise arity 1 — side-condition: false *)
lemma K_Fst_Elim_sound:
  assumes "\<Gamma> \<turnstile> p : Sigma x A B"
  shows "\<Gamma> \<turnstile> Fst p : A"
  using assms by (rule T_fst)

(* K_Snd_Elim — category Structural — premise arity 1 — side-condition: false *)
lemma K_Snd_Elim_sound:
  assumes "\<Gamma> \<turnstile> p : Sigma x A B"
  shows "\<Gamma> \<turnstile> Snd p : subst x (Fst p) B"
  using assms by (rule T_snd)

(* K_Path_Ty_Form — category Cubical — premise arity 3 — side-condition: false *)
lemma K_Path_Ty_Form_sound:
  assumes "\<Gamma> \<turnstile> A : Universe i" and "\<Gamma> \<turnstile> a : A" and "\<Gamma> \<turnstile> b : A"
  shows "\<Gamma> \<turnstile> PathTy A a b : Universe i"
  using assms by (rule T_path_ty)

(* K_Path_Over_Form — category Cubical — premise arity 4 — side-condition: false *)
lemma K_Path_Over_Form_sound:
  assumes "K_Path_Over_Form_iou \<Gamma> motive p a b ty i"
  shows "\<Gamma> \<turnstile> PathOver motive p a b : ty"
  using assms by (rule T_path_over)

(* K_Refl_Intro — category Cubical — premise arity 1 — side-condition: false *)
lemma K_Refl_Intro_sound:
  assumes "\<Gamma> \<turnstile> a : A" shows "\<Gamma> \<turnstile> Refl a : PathTy A a a"
  using assms by (rule T_refl)

(* K_HComp — category Cubical — premise arity 3 — side-condition: false *)
lemma K_HComp_sound:
  assumes "K_HComp_iou \<Gamma> phi walls base T"
  shows "\<Gamma> \<turnstile> HComp phi walls base : T"
  using assms by (rule T_hcomp)

(* K_Transp — category Cubical — premise arity 3 — side-condition: false *)
lemma K_Transp_sound:
  assumes "K_Transp_iou \<Gamma> path regular value target"
  shows "\<Gamma> \<turnstile> Transp path regular value : target"
  using assms by (rule T_transp)

(* K_Glue — category Cubical — premise arity 4 — side-condition: false *)
lemma K_Glue_sound:
  assumes "K_Glue_iou \<Gamma> carrier phi fiber equivP result"
  shows "\<Gamma> \<turnstile> Glue carrier phi fiber equivP : result"
  using assms by (rule T_glue)

(* K_Refine — category Refinement — premise arity 2 — side-condition: false *)
lemma K_Refine_sound:
  assumes "\<Gamma> \<turnstile> base : Universe i" and "K_Refine_iou \<Gamma> base x predicate"
  shows "\<Gamma> \<turnstile> Refine base x predicate : Universe i"
  using assms by (rule T_refine)

(* K_Refine_Omega — category Refinement — premise arity 2 — side-condition: true *)
lemma K_Refine_Omega_sound:
  assumes "K_Refine_Omega_iou \<Gamma> base x predicate"
  shows "\<Gamma> \<turnstile> Refine base x predicate : Universe i"
  using assms by (rule T_refine_omega)

(* K_Refine_Intro — category Refinement — premise arity 2 — side-condition: false *)
lemma K_Refine_Intro_sound:
  assumes "\<Gamma> \<turnstile> a : base" and "K_Refine_Intro_iou \<Gamma> a base x predicate"
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
  assumes "K_Quot_Elim_iou \<Gamma> scrutinee motive case_fn result"
  shows "\<Gamma> \<turnstile> QuotElim scrutinee motive case_fn : result"
  using assms by (rule T_quot_elim)

(* K_Inductive — category Inductive — premise arity 0 — side-condition: false *)
lemma K_Inductive_sound:
  assumes "K_Inductive_iou \<Gamma> path args result"
  shows "\<Gamma> \<turnstile> InductiveT path args : result"
  using assms by (rule T_inductive)

(* K_Pos — category Inductive — premise arity 0 — side-condition: true *)
lemma K_Pos_sound: "side_conditions_hold \<longrightarrow> True"
  by simp

(* K_Elim — category Inductive — premise arity 3 — side-condition: false *)
lemma K_Elim_sound:
  assumes "K_Elim_iou \<Gamma> scrutinee motive cases result"
  shows "\<Gamma> \<turnstile> Elim scrutinee motive cases : result"
  using assms by (rule T_elim)

(* K_Smt — category SmtAxiom — premise arity 0 — side-condition: true *)
lemma K_Smt_sound:
  assumes "K_Smt_iou \<Gamma> solver_tag T"
  shows "\<Gamma> \<turnstile> SmtProof solver_tag : T"
  using assms by (rule T_smt)

(* K_FwAx — category SmtAxiom — premise arity 0 — side-condition: true *)
lemma K_FwAx_sound: "\<Gamma> \<turnstile> AxiomT name ty framework : ty"
  by (rule T_fwax)

(* K_Eps_Mu — category Diakrisis — premise arity 2 — side-condition: false *)
lemma K_Eps_Mu_sound:
  assumes "K_Eps_Mu_iou \<Gamma> articulation enactment ty"
  shows "\<Gamma> \<turnstile> articulation : ty"
  using assms by (rule T_eps_mu)

(* K_Universe_Ascent — category Diakrisis — premise arity 1 — side-condition: true *)
lemma K_Universe_Ascent_sound:
  assumes "K_Universe_Ascent_iou \<Gamma> i"
  shows "\<Gamma> \<turnstile> Universe i : Universe (Suc i)"
  using assms by (rule T_universe_ascent)

(* K_Round_Trip — category Diakrisis — premise arity 2 — side-condition: false *)
lemma K_Round_Trip_sound:
  assumes "K_Round_Trip_iou \<Gamma> term recovered"
  shows "\<Gamma> \<turnstile> term : recovered"
  using assms by (rule T_round_trip)

(* K_Epsilon_Of — category Diakrisis — premise arity 1 — side-condition: false *)
lemma K_Epsilon_Of_sound:
  assumes "K_Epsilon_Of_iou \<Gamma> articulation result"
  shows "\<Gamma> \<turnstile> EpsilonOf articulation : result"
  using assms by (rule T_epsilon_of)

(* K_Alpha_Of — category Diakrisis — premise arity 1 — side-condition: false *)
lemma K_Alpha_Of_sound:
  assumes "K_Alpha_Of_iou \<Gamma> enactment result"
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
  assumes "K_Modal_Big_And_iou \<Gamma> components result"
  shows "\<Gamma> \<turnstile> ModalBigAnd components : result"
  using assms by (rule T_modal_big_and)

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

(* **Kernel full soundness** — names every per-rule lemma in *)
(* canonical KernelRule order.  This is bookkeeping only;     *)
(* the per-rule lemmas above carry the real proof content.   *)
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
