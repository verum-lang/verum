import VerumExternalReplay.FiniteCode

/-! Exact compositional success conditions for the existing intrinsically typed
IR. Primitive domain facts remain proof obligations. The condition is derived
from syntax and checked against Program.run; it is not another evaluator.
This covers finite straight-line IR and the current finite-code lowering, not
native allocation, resource completion, arbitrary recursion or Branch/Fold VM
correspondence. -/
namespace ProgramTotality
open TypedPrograms
set_option warningAsError true
universe u v w
variable {Ty : Type u} {Prim : List Ty → Ty → Type v}
variable {Lit : Ty → Type v} {V : Ty → Type w}

/-- Every eager call must be defined, including calls whose result is unused. -/
def wp (semantics : Semantics Prim Lit V) : (p : Program Prim Lit Γ t) →
    (V t → Prop) → Env V Γ → Prop
  | .ret arg, post, env => post (arg.eval semantics env)
  | .call op args rest, post, env => ∃ value,
      semantics.primitive op (args.map (Arg.eval semantics env)) = some value ∧
      wp semantics rest post (env.cons value)

theorem wp_iff_success (semantics : Semantics Prim Lit V) (p : Program Prim Lit Γ t)
    (post : V t → Prop) (env : Env V Γ) :
    wp semantics p post env ↔ ∃ value, p.run semantics env = some value ∧ post value := by
  induction p with
  | ret arg => simp [wp,Program.run]
  | call op args rest ih =>
    simp only [wp,Program.run]
    cases h : semantics.primitive op (args.map (Arg.eval semantics env)) with
    | none => simp
    | some value =>
      simp only [Option.some.injEq,Option.bind_some]
      constructor
      · rintro ⟨_,rfl,certificate⟩
        exact (ih post (env.cons value)).mp certificate
      · intro result
        exact ⟨value,rfl,(ih post (env.cons value)).mpr result⟩

/-- This executable form also allows kernel-reduced checks of closed contracts. -/
theorem wp_iff_result (semantics : Semantics Prim Lit V) (p : Program Prim Lit Γ t)
    (post : V t → Prop) (env : Env V Γ) :
    wp semantics p post env ↔ match p.run semantics env with
      | none => False | some value => post value := by
  rw [wp_iff_success]
  cases h : p.run semantics env <;> simp

instance decidableWP (semantics : Semantics Prim Lit V) (p : Program Prim Lit Γ t)
    (post : V t → Prop) [DecidablePred post] (env : Env V Γ) : Decidable (wp semantics p post env) := by
  haveI : Decidable (match p.run semantics env with | none => False | some value => post value) := by
    cases p.run semantics env <;> dsimp only <;> infer_instance
  exact decidable_of_iff (match p.run semantics env with | none => False | some value => post value)
    (wp_iff_result semantics p post env).symm

/-- A certificate may restrict inputs and relate outputs to the original input.
It makes no blanket assertion about all values admitted by the host language. -/
def Certificate (semantics : Semantics Prim Lit V) (p : Program Prim Lit Γ t)
    (pre : Env V Γ → Prop) (post : Env V Γ → V t → Prop) : Prop :=
  ∀ env, pre env → wp semantics p (post env) env

theorem certificate_iff (semantics : Semantics Prim Lit V) (p : Program Prim Lit Γ t)
    (pre : Env V Γ → Prop) (post : Env V Γ → V t → Prop) :
    Certificate semantics p pre post ↔
      ∀ env, pre env → ∃ value, p.run semantics env = some value ∧ post env value := by
  constructor
  · intro certificate env valid
    exact (wp_iff_success semantics p (post env) env).mp (certificate env valid)
  · intro success env valid
    exact (wp_iff_success semantics p (post env) env).mpr (success env valid)

/-- Primitive contracts establish definedness and an intermediate refinement;
the continuation must accept every result satisfying that refinement. -/
theorem call_rule (semantics : Semantics Prim Lit V) (op : Prim params s)
    (args : HList (Arg Lit Γ) params) (rest : Program Prim Lit (s::Γ) t)
    (env : Env V Γ) (middle : V s → Prop) (post : V t → Prop)
    (primitive : ∃ value, semantics.primitive op (args.map (Arg.eval semantics env)) = some value ∧ middle value)
    (continuation : ∀ value, middle value → wp semantics rest post (env.cons value)) :
    wp semantics (.call op args rest) post env := by
  obtain ⟨value,defined,refined⟩ := primitive
  exact ⟨value,defined,continuation value refined⟩

theorem post_weaken (semantics : Semantics Prim Lit V) (p : Program Prim Lit Γ t)
    (post weaker : V t → Prop) (env : Env V Γ) (implies : ∀ value, post value → weaker value)
    (certificate : wp semantics p post env) : wp semantics p weaker env := by
  obtain ⟨value,defined,valid⟩ := (wp_iff_success semantics p post env).mp certificate
  exact (wp_iff_success semantics p weaker env).mpr ⟨value,defined,implies value valid⟩

/-- Existing proved splicing preserves the totality obligation of both pieces. -/
theorem splice_iff (semantics : Semantics Prim Lit V) (first : Program Prim Lit Γ s)
    (rest : Program Prim Lit (s::Γ) t) (post : V t → Prop) (env : Env V Γ) :
    wp semantics (first.splice rest) post env ↔
      wp semantics first (fun value => wp semantics rest post (env.cons value)) env := by
  rw [wp_iff_result,Program.run_splice,wp_iff_result]
  cases h : first.run semantics env with
  | none => rfl
  | some value => exact (wp_iff_result semantics rest post (env.cons value)).symm

end ProgramTotality

namespace ProgramTotality
open TypedPrograms FiniteCode
set_option warningAsError true

/-- The same condition covers lexical calls, operand resolution and output
selection through the existing lowering. It cannot erase an invalid eager call. -/
theorem finite_code_iff (apply : O → List V → Option V) (code : Code O V)
    (inputs : List V) (post : V → Prop) :
    wp (semantics apply) (lower code inputs.length) post (row inputs).get ↔
      ∃ value, FiniteCode.run apply code inputs = some value ∧ post value :=
  wp_iff_success _ _ _ _

end ProgramTotality
