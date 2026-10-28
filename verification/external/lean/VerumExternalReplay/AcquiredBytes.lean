import VerumExternalReplay.FiniteCode
import VerumExternalReplay.ProgramProperties

/-! Byte provenance of the acquired observer primitive language.

This is a success overapproximation of observer_acquired::Meaning: machine
integer, trace capacity and allocation refusals may remove successes. Metadata
and opaque actions are represented by natural identifiers; only sensory ports
contain byte leaves. Immutable pairs unfold as finite trees (sharing does not
change leaf membership). No theorem here covers arbitrary Program semantics,
external decoders, effects, costs, or an unbounded native heap.
-/
namespace AcquiredBytes
open TypedPrograms ProgramProperties
set_option warningAsError true

abbrev Bytes := List (Fin 256)

structure Sensory where
  schema : Bytes
  body : Bytes
  deriving DecidableEq

structure Record where
  sensory : Option Sensory
  /-- Equality-preserving code for the remaining source/lineage/episode fields. -/
  metadata : Nat := 0
  input : Nat := 0
  raw : Nat := 0
  age : Nat := 0
  pose : Option Nat := none
  action : Option Nat := none
  deriving DecidableEq

inductive Value where
  | unit
  | nat : Nat → Value
  | bool : Bool → Value
  | action : Nat → Value
  | event : Record → Value
  | query : Record → Value
  | pair : Value → Value → Value
  | trace : List Record → Value
  | outcome : Bool → Nat → Value
  | bytes : Bytes → Value
  deriving DecidableEq

def recordLeaves (r : Record) : List Bytes :=
  match r.sensory with | none => [] | some s => [s.schema,s.body]

def leaves : Value → List Bytes
  | .bytes b => [b]
  | .pair x y => leaves x ++ leaves y
  | .event r | .query r => recordLeaves r
  | .trace rs => rs.flatMap recordLeaves
  | _ => []

inductive Op where
  | unit | zero | emptyTrace | identity | append | at | last | length
  | eventInput | eventRaw | eventAction | eventAge | eventPose
  | queryInput | queryRaw | queryAge | queryAction | queryPose
  | pair | first | second | equal | select | add | rem | next | refused
  | eventSensory | querySensory
  deriving DecidableEq

def sensoryValue (r : Record) : Option Value :=
  r.sensory.map (fun s => .pair (.bytes s.schema) (.bytes s.body))

def apply : Op → List Value → Option Value
  | .unit, [] => some .unit
  | .zero, [] => some (.nat 0)
  | .emptyTrace, [] => some (.trace [])
  | .identity, [x] => some x
  | .append, [.trace rs,.event r] => some (.trace (rs ++ [r]))
  | .at, [.trace rs,.nat n] => rs[n]?.map .event
  | .at, [.bytes bs,.nat n] => bs[n]?.map (fun b => .nat b.val)
  | .last, [.trace rs] => rs.getLast?.map .event
  | .length, [.trace rs] => some (.nat rs.length)
  | .length, [.bytes bs] => some (.nat bs.length)
  | .eventSensory, [.event r] => sensoryValue r
  | .querySensory, [.query r] => sensoryValue r
  | .eventInput, [.event r] | .queryInput, [.query r] => some (.nat r.input)
  | .eventRaw, [.event r] | .queryRaw, [.query r] => some (.nat r.raw)
  | .eventAge, [.event r] | .queryAge, [.query r] => some (.nat r.age)
  | .eventPose, [.event r] | .queryPose, [.query r] => r.pose.map .nat
  | .eventAction, [.event r] | .queryAction, [.query r] => r.action.map .action
  | .pair, [x,y] => some (.pair x y)
  | .first, [.pair x _] => some x
  | .second, [.pair _ y] => some y
  | .equal, [x,y] => some (.bool (decide (x = y)))
  | .select, [.bool c,x,y] => some (if c then x else y)
  | .add, [.nat x,.nat y] => some (.nat (x+y))
  | .rem, [.nat x,.nat y] => if y = 0 then none else some (.nat (x%y))
  | .next, [.nat x] => some (.outcome true x)
  | .refused, [.nat x] => some (.outcome false x)
  | _, _ => none

def Safe (allowed : Bytes → Prop) (x : Value) : Prop := ∀ b ∈ leaves x, allowed b

theorem sensory_safe (allowed : Bytes → Prop) (r : Record) (x : Value)
    (h : ∀ b ∈ recordLeaves r, allowed b) (success : sensoryValue r = some x) : Safe allowed x := by
  cases hs : r.sensory with
  | none => simp [sensoryValue, hs] at success
  | some s =>
      simp only [sensoryValue, hs, Option.map_some, Option.some.injEq] at success
      subst x
      simpa [Safe, leaves, recordLeaves, hs] using h

theorem primitive_safe (allowed : Bytes → Prop) (op : Op) (args : List Value) (x : Value)
    (inputs : ∀ y ∈ args, Safe allowed y) (success : apply op args = some x) : Safe allowed x := by
  unfold apply at success
  split at success
  all_goals try (cases Option.some.inj success; simp [Safe,leaves]; done)
  all_goals try (cases Option.some.inj success; simpa [Safe,leaves] using inputs)
  all_goals try exact sensory_safe allowed _ x (by simpa [Safe, leaves] using inputs) success
  all_goals try simp only [Option.map_eq_some_iff] at success
  all_goals try (obtain ⟨a, ha, rfl⟩ := success; simp [Safe,leaves]; done)
  case h_5 =>
    rename_i rs r
    cases Option.some.inj success
    intro b hb
    obtain ⟨a, ha, hb⟩ := List.mem_flatMap.mp hb
    rcases List.mem_append.mp ha with ha | ha
    · exact inputs (.trace rs) (by simp) b (List.mem_flatMap.mpr ⟨a,ha,hb⟩)
    · have eq : a = r := by simpa using ha
      subst a
      exact inputs (.event r) (by simp) b hb
  case h_6 =>
    rename_i rs n
    obtain ⟨a, ha, rfl⟩ := success
    intro b hb
    exact inputs (.trace rs) (by simp) b
      (List.mem_flatMap.mpr ⟨a,List.mem_of_getElem? ha,hb⟩)
  case h_8 =>
    rename_i rs
    obtain ⟨a, ha, rfl⟩ := success
    intro b hb
    exact inputs (.trace rs) (by simp) b
      (List.mem_flatMap.mpr ⟨a,List.mem_of_getLast? ha,hb⟩)
  case h_23 =>
    rename_i left right
    cases Option.some.inj success
    intro b hb
    rcases List.mem_append.mp hb with hb | hb
    · exact inputs left (by simp) b hb
    · exact inputs right (by simp) b hb
  case h_24 =>
    rename_i left right
    cases Option.some.inj success
    intro b hb
    exact inputs (.pair x right) (by simp) b (List.mem_append_left _ hb)
  case h_25 =>
    rename_i left right
    cases Option.some.inj success
    intro b hb
    exact inputs (.pair left x) (by simp) b (List.mem_append_right _ hb)
  case h_27 =>
    rename_i c left right
    split at success <;> cases Option.some.inj success
    · exact inputs x (by simp)
    · exact inputs x (by simp)
  case h_29 =>
    split at success
    · cases success
    · cases Option.some.inj success; simp [Safe,leaves]
  case h_32 => cases success

abbrev Program := FiniteCode.Target Op Value [()]
abbrev semantics := FiniteCode.semantics apply

def literalLeaves (p : FiniteCode.Target Op Value Γ) : List Bytes :=
  literalAtoms semantics (fun {_} v => leaves v) p

theorem values_safe (allowed : Bytes → Prop) (xs : HList (FiniteCode.Carrier Value) Γ)
    (h : all (fun {_} v => Safe allowed v) xs) :
    ∀ v ∈ FiniteCode.values xs, Safe allowed v := by
  induction xs with
  | nil => simp [FiniteCode.values]
  | cons x xs ih =>
      simp only [all] at h
      simp only [FiniteCode.values, List.mem_cons, forall_eq_or_imp]
      exact ⟨h.1, ih h.2⟩

theorem program_safe (allowed : Bytes → Prop) (p : FiniteCode.Target Op Value Γ)
    (env : Env (FiniteCode.Carrier Value) Γ) (x : Value)
    (inputs : ∀ {t} (r : Ref Γ t), Safe allowed (env r))
    (literals : ∀ b ∈ literalLeaves p, allowed b)
    (success : p.run semantics env = some x) : Safe allowed x := by
  apply run_supported semantics (fun {_} v => leaves v) allowed ?_ p env x inputs literals success
  intro params t op xs y h hs
  cases op with
  | none => simp [semantics, FiniteCode.semantics] at hs
  | some op => exact primitive_safe allowed op _ y (values_safe allowed xs h) hs

theorem output_leaf (p : FiniteCode.Target Op Value Γ)
    (env : Env (FiniteCode.Carrier Value) Γ) (x : Value) (b : Bytes)
    (success : p.run semantics env = some x) (hb : b ∈ leaves x) :
    b ∈ literalLeaves p ∨ ∃ t, ∃ r : Ref Γ t, b ∈ leaves (env r) := by
  have h := program_safe
    (fun b => b ∈ literalLeaves p ∨ ∃ t, ∃ r : Ref Γ t, b ∈ leaves (env r)) p env x
    (fun {t} r b hb => Or.inr ⟨t,r,hb⟩) (fun b hb => Or.inl hb) success
  exact h b hb

theorem row_member (xs : List Value) (r : Ref (FiniteCode.Row xs.length) t) :
    (FiniteCode.row xs).get r ∈ xs := by
  induction xs with
  | nil => nomatch r
  | cons x xs ih =>
      cases r with
      | here => simp [FiniteCode.row,HList.get]
      | there r => exact List.mem_cons_of_mem x (ih r)

/-- Existing finite-code lowering includes nested lexical library calls. Its
literal support, rather than a new execution engine, is used here. -/
theorem finite_code_leaf (code : FiniteCode.Code Op Value) (xs : List Value)
    (x : Value) (b : Bytes) (success : FiniteCode.run apply code xs = some x)
    (hb : b ∈ leaves x) :
    b ∈ literalLeaves (FiniteCode.lower code xs.length) ∨ ∃ v ∈ xs, b ∈ leaves v := by
  rcases output_leaf (FiniteCode.lower code xs.length) (FiniteCode.row xs).get x b success hb with h | h
  · exact Or.inl h
  · obtain ⟨t,r,h⟩ := h
    exact Or.inr ⟨(FiniteCode.row xs).get r,row_member xs r,h⟩

/-- This exact decoder exposes only the two Bytes leaves. An arbitrary host
callback that constructs bytes is a different semantic language. -/
def decode : Value → Option (Bytes × Bytes)
  | .pair (.bytes context) (.bytes mask) => some (context,mask)
  | _ => none

theorem decoded_safe (allowed : Bytes → Prop) (x : Value) (context mask : Bytes)
    (h : Safe allowed x) (hd : decode x = some (context,mask)) :
    allowed context ∧ allowed mask := by
  unfold decode at hd
  split at hd <;> simp_all [Safe, leaves]

/-- Applies to arbitrary input bindings, including retained history plus a
query. Both decoded outputs have the same provenance obligation. -/
theorem decoded_program_safe (allowed : Bytes → Prop) (p : FiniteCode.Target Op Value Γ)
    (env : Env (FiniteCode.Carrier Value) Γ) (context mask : Bytes)
    (inputs : ∀ {t} (r : Ref Γ t), Safe allowed (env r))
    (literals : ∀ b ∈ literalLeaves p, allowed b)
    (success : (p.run semantics env).bind decode = some (context,mask)) :
    allowed context ∧ allowed mask := by
  obtain ⟨x,hx,hd⟩ := Option.bind_eq_some_iff.mp success
  exact decoded_safe allowed x context mask (program_safe allowed p env x inputs literals hx) hd

def runOn (p : Program) (input : Bytes) : Option (Bytes × Bytes) :=
  (p.run semantics (Env.cons (.bytes input) (fun r => nomatch r))).bind decode

def maxLength : List Bytes → Nat
  | [] => 0
  | b :: bs => max b.length (maxLength bs)

theorem length_le_max (bs : List Bytes) (b : Bytes) (h : b ∈ bs) : b.length ≤ maxLength bs := by
  induction bs with
  | nil => simp at h
  | cons c cs ih =>
      simp only [List.mem_cons] at h
      rcases h with rfl | h
      · exact Nat.le_max_left _ _
      · exact Nat.le_trans (ih h) (Nat.le_max_right _ _)

theorem output_length_bound (p : Program) (input context mask : Bytes)
    (hi : maxLength (literalLeaves p) ≤ input.length)
    (success : runOn p input = some (context,mask)) : mask.length ≤ input.length := by
  simp only [runOn, Option.bind_eq_some_iff] at success
  obtain ⟨value, hv, hd⟩ := success
  apply (decoded_safe (fun b => b.length ≤ input.length) value context mask ?_ hd).2
  refine program_safe (fun b => b.length ≤ input.length) p _ value ?_ ?_ hv
  · intro t r
    cases r with
    | here => simp [Safe, Env.cons, leaves]
    | there r => nomatch r
  · intro b hb; exact Nat.le_trans (length_le_max _ b hb) hi

/-- A different input is chosen for each finite literal pool. The theorem is
over mathematical finite strings, not a claim of unbounded native resources. -/
theorem append_zero_not_expressible (p : Program) :
    ∃ input, runOn p input ≠ some (input,input ++ [0]) := by
  let input : Bytes := List.replicate (maxLength (literalLeaves p) + 1) 0
  refine ⟨input, ?_⟩
  intro h
  have hi : maxLength (literalLeaves p) ≤ input.length := by simp [input]
  have impossible := output_length_bound p input input (input ++ [0]) hi h
  simp only [List.length_append, List.length_singleton] at impossible
  omega

/-- In a literal-free language the witness already fits in one byte. -/
theorem literal_free_witness (p : Program) (hl : literalLeaves p = []) :
    runOn p [] ≠ some ([],[0]) := by
  intro h
  have impossible := output_length_bound p [] [] [0] (by simp [hl,maxLength]) h
  simp at impossible

variable {New : Type}

def extendedApply (added : New → List Value → Option Value) : Sum Op New → List Value → Option Value
  | .inl op => apply op
  | .inr op => added op

def embedOld (p : Program) : FiniteCode.Target (Sum Op New) Value [()] :=
  embed (fun op => op.map Sum.inl) p

def extendedRun (added : New → List Value → Option Value)
    (p : FiniteCode.Target (Sum Op New) Value [()]) (input : Bytes) : Option (Bytes × Bytes) :=
  (p.run (FiniteCode.semantics (extendedApply added))
    (Env.cons (.bytes input) (fun r => nomatch r))).bind decode

theorem embedding_exact (added : New → List Value → Option Value) (p : Program) (input : Bytes) :
    extendedRun added (embedOld p) input = runOn p input := by
  unfold extendedRun runOn embedOld
  rw [run_embed semantics (FiniteCode.semantics (extendedApply added))
    (fun op => op.map Sum.inl) (fun _ => rfl)]
  intro params t op xs
  cases op <;> rfl

/-- Any extension implementing the witness is strictly more expressive under
the declared decoder. This alone proves neither universality nor learnability. -/
theorem extension_strict (added : New → List Value → Option Value)
    (candidate : FiniteCode.Target (Sum Op New) Value [()])
    (realizes : ∀ input, extendedRun added candidate input = some (input,input ++ [0])) :
    (∀ p : Program, ∃ q, ∀ input, extendedRun added q input = runOn p input) ∧
    ¬ ∃ p : Program, ∀ input, extendedRun added candidate input = runOn p input := by
  apply strict_extension runOn (extendedRun added) embedOld (embedding_exact added) candidate
  intro p
  obtain ⟨input, h⟩ := append_zero_not_expressible p
  exact ⟨input, by rw [realizes]; exact Ne.symm h⟩

/-! An illustrative instance of the contract using general sequence
construction. It is not an Omega patch or a complete language design.
Structural iteration, retained VM continuations, acquisition and cost require
their own semantics and correspondence proofs. -/
inductive SequenceOp where
  | singleton | concat

def sequenceApply : SequenceOp → List Value → Option Value
  | .singleton, [.nat n] => if h : n < 256 then some (.bytes [⟨n,h⟩]) else none
  | .concat, [.bytes x,.bytes y] => some (.bytes (x ++ y))
  | _, _ => none

def sequenceWitness : FiniteCode.Target (Sum Op SequenceOp) Value [()] :=
  .call (params := [()]) (s := ()) (some (.inr .singleton)) (.cons (.literal (.nat 0)) .nil)
    (.call (params := [(),()]) (s := ()) (some (.inr .concat))
      (.cons (.reg (.there .here)) (.cons (.reg .here) .nil))
      (.call (params := [(),()]) (s := ()) (some (.inl .pair))
        (.cons (.reg (.there (.there .here))) (.cons (.reg .here) .nil))
        (.ret (.reg .here))))

theorem sequence_realizes (input : Bytes) :
    extendedRun sequenceApply sequenceWitness input = some (input,input ++ [0]) := by
  rfl

theorem sequence_strict :
    (∀ p : Program, ∃ q, ∀ input, extendedRun sequenceApply q input = runOn p input) ∧
    ¬ ∃ p : Program, ∀ input, extendedRun sequenceApply sequenceWitness input = runOn p input :=
  extension_strict sequenceApply sequenceWitness sequence_realizes

end AcquiredBytes
