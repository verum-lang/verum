import VerumExternalReplay.ContextualStop
import VerumExternalReplay.AdaptiveResidual

/-! Finite export views for contextual prediction. Questions retain raw causal
prefixes; their canonical visits agree with the existing scoring/training model.
Sparse count comparison checks the union of both supports, so absent and extra
cells cannot escape validation. No native VM or allocation theorem is implied. -/
namespace SparseCounts
open FrozenTraining
set_option warningAsError true
variable {Key : Type} [DecidableEq Key]

structure Cell (Key : Type) where
  key : Key
  zeros : Nat
  ones : Nat
  deriving DecidableEq

def decode : List (Cell Key) → Counts Key
  | [], _, _ => 0
  | cell::rest, key, bit => if cell.key = key then
      (if bit then cell.ones else cell.zeros) else decode rest key bit

def agrees (events : List (Key × Bool)) (cells : List (Cell Key)) : Bool :=
  (events.map Prod.fst ++ cells.map Cell.key).all (fun key =>
    decide (decode cells key false = collect events (fun _ _ => 0) key false ∧
      decode cells key true = collect events (fun _ _ => 0) key true))

theorem decode_outside (cells : List (Cell Key)) (key : Key) (bit : Bool)
    (outside : key ∉ cells.map Cell.key) : decode cells key bit = 0 := by
  induction cells with
  | nil => rfl
  | cons cell rest ih =>
    simp only [List.map_cons,List.mem_cons,not_or] at outside
    simp [decode,Ne.symm outside.1,ih outside.2]

theorem collect_outside (events : List (Key × Bool)) (key : Key) (bit : Bool)
    (outside : key ∉ events.map Prod.fst) : collect events (fun _ _ => 0) key bit = 0 := by
  rw [collected_multiplicity]
  have absent : events.filter (fun e => decide (e = (key,bit))) = [] := by
    induction events with
    | nil => rfl
    | cons event rest ih =>
      simp only [List.map_cons,List.mem_cons,not_or] at outside
      have different : event ≠ (key,bit) := fun h => outside.1 (congrArg Prod.fst h).symm
      simp [different,ih outside.2]
  simp [absent]

/-- Finite support checking establishes equality at every possible key. -/
theorem agrees_exact (events : List (Key × Bool)) (cells : List (Cell Key))
    (checked : agrees events cells = true) (key : Key) (bit : Bool) :
    decode cells key bit = collect events (fun _ _ => 0) key bit := by
  by_cases present : key ∈ events.map Prod.fst ++ cells.map Cell.key
  · have both := List.all_eq_true.mp checked key present
    simp only [decide_eq_true_eq] at both
    cases bit with
    | false => exact both.1
    | true => exact both.2
  · simp only [List.mem_append,not_or] at present
    rw [decode_outside cells key bit present.2,collect_outside events key bit present.1]

end SparseCounts

namespace ContextualRows
open ContextualStop
set_option warningAsError true

structure Question where
  kind : Kind
  past : List Bool
  raw : Bool
  deriving DecidableEq

def Question.visit (select : Selector Key) (q : Question) : (Kind × Key) × Bool :=
  ContextualStop.visit select q.kind q.past q.raw

def bitQuestions (past : List Bool) : List Bool → List Question
  | [] => []
  | bit::rest => ⟨.bit,past,bit⟩ :: bitQuestions (bit::past) rest

def questions (past : List Bool) : List (List Bool) → List Question
  | [] => [⟨.stop,past,true⟩]
  | bits::rest => ⟨.stop,past,false⟩ ::
      (bitQuestions past bits ++ questions (bits.reverse ++ past) rest)

theorem bit_questions_visits (select : Selector Key) (past bits : List Bool) :
    (bitQuestions past bits).map (Question.visit select) = bitVisits select past bits := by
  induction bits generalizing past with
  | nil => rfl
  | cons bit rest ih => simp [bitQuestions,bitVisits,Question.visit,ih]

theorem questions_visits (select : Selector Key) (past : List Bool) (blocks : List (List Bool)) :
    (questions past blocks).map (Question.visit select) = rowVisits select past blocks := by
  induction blocks generalizing past with
  | nil => rfl
  | cons bits rest ih =>
    simp [questions,rowVisits,Question.visit,bit_questions_visits,ih]

end ContextualRows

namespace ContextualRows
open ExactMass ContextualStop ResidualPrediction
set_option warningAsError true

abbrev Key := Kind × List Nat

structure Bank where
  joint : List (SparseCounts.Cell Key)
  reference : List (SparseCounts.Cell Nat)
  lengths : AdaptiveLength.Counts

structure CursorView where
  kind : Kind
  past : List Bool
  done : Bool
  selecting : Bool
  selected : Option (List Nat × Bool)
  deriving DecidableEq

structure PendingView where
  cursor : CursorView
  target : List Nat
  position : Nat
  frozen : Bank

structure Frontier where
  samples : Nat
  bits : Nat
  ratio : Fraction
  staged : Bank
  pending : Option PendingView

def rowQuestions (target : List Nat) : List Question := questions [] (target.map byteBits)

def jointEvents (select : Selector (List Nat)) (rows : List (List Nat)) : List (Key × Bool) :=
  rows.flatMap (fun target => rowVisits select [] (target.map byteBits))

def rawEvents (qs : List Question) : List (Nat × Bool) :=
  qs.filterMap (fun q => if q.kind = .bit then some (q.past.length,q.raw) else none)

def referenceEvents (rows : List (List Nat)) : List (Nat × Bool) :=
  rows.flatMap (fun target => rawEvents (rowQuestions target))

def bankAgrees (select : Selector (List Nat)) (rows : List (List Nat))
    (prefixQuestions : List Question) (completed : List (List Nat)) (bank : Bank) : Bool :=
  SparseCounts.agrees (jointEvents select rows ++ prefixQuestions.map (Question.visit select)) bank.joint &&
  SparseCounts.agrees (referenceEvents rows ++ rawEvents prefixQuestions) bank.reference &&
  AdaptiveResidual.histogramAgrees (completed.map (fun target => ⟨[],[],target⟩)) bank.lengths

def candidate (select : Selector (List Nat)) (rows : List (List Nat)) (target : List Nat) : Fraction :=
  StoppedProcess.point (model select (FrozenTraining.collect (jointEvents select rows) (fun _ _ => 0))
    8 (by decide)) [] (target.map byteBits)

def reference (rows : List (List Nat)) (target : List Nat) : Fraction :=
  mul (AdaptiveLength.weight (rows.map (fun row => (row.length,1))) target.length)
    (PositionPrediction.frozen id ((referenceEvents rows).foldl
      (fun bank event => PositionPrediction.observe bank event.1 event.2) []) 0 (wireBits target))

/-- Projection for a declared fixture selector with a fixed number of paid
selection stages. Arbitrary acquired programs do not satisfy this interface by
assumption. The source adapter must bind its selector and stage schedule. -/
def cursorView (select : Selector (List Nat)) (selectionStages : Nat)
    (target : List Nat) (progress : Nat) : CursorView :=
  let qs := rowQuestions target
  let width := selectionStages+2
  match qs[progress / width]? with
  | none => ⟨.stop,(wireBits target).reverse,true,false,none⟩
  | some q =>
      let phase := progress % width
      let selected := select q.kind q.past
      ⟨q.kind,q.past,false,decide (0 < phase ∧ phase < selectionStages+1),
        if phase = selectionStages+1 then some (selected.key,selected.invert) else none⟩

/-- Counts are staged after each factor; lengths, score and sample counters are
published only by the separate completed-row step. -/
def frontierAgrees (select : Selector (List Nat)) (selectionStages : Nat)
    (rows : List (List Nat)) (target : List Nat) (previous : Fraction)
    (progress : Nat) (snapshot : Frontier) : Bool :=
  let qs := rowQuestions target
  let endFactors := qs.length*(selectionStages+2)
  let complete := progress == endFactors+1
  let completed := if complete then rows++[target] else rows
  let prefixQuestions := qs.take (progress / (selectionStages+2))
  decide (progress ≤ endFactors+1) &&
  snapshot.samples == completed.length &&
  snapshot.bits == 8*(completed.map List.length).sum &&
  bankAgrees select rows prefixQuestions completed snapshot.staged &&
  (if complete then decide (Equal (mul snapshot.ratio (reference rows target))
      (mul previous (candidate select rows target))) else decide (Equal snapshot.ratio previous)) &&
  (match snapshot.pending with
  | none => complete
  | some pending => !complete && pending.target == target &&
      pending.position == pending.cursor.past.length &&
      decide (pending.cursor = cursorView select selectionStages target progress) &&
      bankAgrees select rows [] rows pending.frozen)

/-- One attempted native advance, including zero-work preservation. These are
finite projection checks, not a source-level refinement theorem. -/
def attemptAgrees (select : Selector (List Nat)) (selectionStages : Nat)
    (rows : List (List Nat)) (target : List Nat) (previous : Fraction)
    (progress grant paid : Nat) (refused : Bool) (before after : Frontier) : Bool :=
  let cost := if grant = 0 then 0 else 1
  decide (progress < (rowQuestions target).length*(selectionStages+2)+1) &&
  paid == cost && refused == (grant == 0) &&
  frontierAgrees select selectionStages rows target previous progress before &&
  frontierAgrees select selectionStages rows target previous (progress+cost) after

end ContextualRows
