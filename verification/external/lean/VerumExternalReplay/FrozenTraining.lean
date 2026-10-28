import VerumExternalReplay.PositionPrediction

/-! Frozen-parameter scoring with a causal path and private training deltas.
The delta carrier is generic: a native implementation can use a sparse summary
instead of retaining a per-bit log. These pure laws establish neither resource
admission nor native allocation/atomicity. Publication uses AtomicPublication
only after whole-row completion and a separately checked state transition. -/
namespace FrozenTraining
open ExactMass CausalCoding
set_option warningAsError true

structure Rule (Bank Path Delta : Type) where
  forecast : Bank → Kernel Path
  empty : Delta
  record : Bank → Path → Bool → Delta → Delta
  publish : Bank → Delta → Bank

structure Frame (Bank Path Delta : Type) where
  frozen : Bank
  path : Path
  delta : Delta
  mass : Fraction

structure Result (Bank Path : Type) where
  bank : Bank
  path : Path
  mass : Fraction

variable {B P D : Type}

def begin (rule : Rule B P D) (bank : B) (path : P) : Frame B P D :=
  ⟨bank,path,rule.empty,one⟩

def step (rule : Rule B P D) (frame : Frame B P D) (bit : Bool) : Frame B P D :=
  ⟨frame.frozen, (rule.forecast frame.frozen).next frame.path bit,
   rule.record frame.frozen frame.path bit frame.delta,
   mul frame.mass ((rule.forecast frame.frozen).probability frame.path bit)⟩

def feed (rule : Rule B P D) (frame : Frame B P D) (body : List Bool) : Frame B P D :=
  body.foldl (step rule) frame

theorem feed_frozen (rule : Rule B P D) (frame : Frame B P D) (body : List Bool) :
    (feed rule frame body).frozen = frame.frozen := by
  induction body generalizing frame with
  | nil => rfl
  | cons bit rest ih => exact ih (step rule frame bit)

theorem feed_path (rule : Rule B P D) (frame : Frame B P D) (body : List Bool) :
    (feed rule frame body).path = advance (rule.forecast frame.frozen) frame.path body := by
  induction body generalizing frame with
  | nil => rfl
  | cons bit rest ih => exact ih (step rule frame bit)

theorem feed_mass (rule : Rule B P D) (frame : Frame B P D) (body : List Bool) :
    Equal (feed rule frame body).mass
      (mul frame.mass (likelihood (rule.forecast frame.frozen) frame.path body)) := by
  induction body generalizing frame with
  | nil => exact eq_symm (mul_one _)
  | cons bit rest ih =>
      have h := ih (step rule frame bit)
      simpa only [feed, List.foldl_cons, step, likelihood, mul_assoc_exact] using h

theorem score_matches_forecast (rule : Rule B P D) (bank : B) (path : P) (body : List Bool) :
    Equal (feed rule (begin rule bank path) body).mass (likelihood (rule.forecast bank) path body) :=
  eq_trans (feed_mass rule (begin rule bank path) body)
    (eq_trans (mul_comm _ _) (mul_one _))

theorem scored_normalized (rule : Rule B P D) (bank : B) (path : P) (n : Nat) :
    Equal (sum ((words n).map (fun body => (feed rule (begin rule bank path) body).mass))) one :=
  eq_trans (sum_congr (words n) _ _ (fun body _ => score_matches_forecast rule bank path body))
    (body_normalized (rule.forecast bank) path n)

theorem feed_append (rule : Rule B P D) (frame : Frame B P D) (before after : List Bool) :
    feed rule frame (before ++ after) = feed rule (feed rule frame before) after :=
  List.foldl_append

/-- A denied admission has no effect on the scoring or delta state. -/
def resume (rule : Rule B P D) (frame : Frame B P D) (steps : List (Option Bool)) : Frame B P D :=
  steps.foldl (fun frame bit => match bit with | none => frame | some b => step rule frame b) frame

theorem pauses_preserve_result (rule : Rule B P D) (frame : Frame B P D) (steps : List (Option Bool)) :
    resume rule frame steps = feed rule frame (steps.filterMap id) := by
  induction steps generalizing frame with
  | nil => rfl
  | cons bit rest ih =>
      cases bit with
      | none => exact ih frame
      | some bit => exact ih (step rule frame bit)

structure Pending (Bank Path Delta : Type) where
  frame : Frame Bank Path Delta
  remaining : List Bool

def advance (rule : Rule B P D) (pending : Pending B P D) : Pending B P D :=
  match pending.remaining with
  | [] => pending
  | bit :: rest => ⟨step rule pending.frame bit,rest⟩

def result (rule : Rule B P D) (frame : Frame B P D) : Result B P :=
  ⟨rule.publish frame.frozen frame.delta,frame.path,frame.mass⟩

/-- Completion is a separate operation. A partial row has no publishable result. -/
def finish (rule : Rule B P D) (pending : Pending B P D) : Option (Result B P) :=
  match pending.remaining with
  | [] => some (result rule pending.frame)
  | _ :: _ => none

def completion (rule : Rule B P D) (pending : Pending B P D) : Result B P :=
  result rule (feed rule pending.frame pending.remaining)

theorem advance_retains_bank (rule : Rule B P D) (pending : Pending B P D) :
    (advance rule pending).frame.frozen = pending.frame.frozen := by
  cases pending with
  | mk frame remaining => cases remaining <;> rfl

theorem advance_preserves_completion (rule : Rule B P D) (pending : Pending B P D) :
    completion rule (advance rule pending) = completion rule pending := by
  cases pending with
  | mk frame remaining => cases remaining <;> rfl

theorem partial_row_refused (rule : Rule B P D) (frame : Frame B P D) (bit : Bool) (rest : List Bool) :
    finish rule ⟨frame,bit::rest⟩ = none := rfl

theorem finished_exact (rule : Rule B P D) (pending : Pending B P D) (r : Result B P)
    (finished : finish rule pending = some r) : r = completion rule pending := by
  cases pending with
  | mk frame remaining => cases remaining <;> simp_all [finish,completion,feed]

/-- Every occurrence is retained; neither uniqueness nor set deduplication is
required. This reference delta is a sparse count function, not a native layout. -/
abbrev Counts (Key : Type) := Key → Bool → Nat

def increment [DecidableEq Key] (counts : Counts Key) (key : Key) (bit : Bool) : Counts Key :=
  fun k b => if (key,bit) = (k,b) then counts k b + 1 else counts k b

def collect [DecidableEq Key] (events : List (Key × Bool)) (counts : Counts Key) : Counts Key :=
  events.foldl (fun counts event => increment counts event.1 event.2) counts

theorem collected_multiplicity [DecidableEq Key] (events : List (Key × Bool))
    (counts : Counts Key) (key : Key) (bit : Bool) :
    collect events counts key bit = counts key bit + (events.filter (fun e => decide (e = (key,bit)))).length := by
  induction events generalizing counts with
  | nil => simp [collect]
  | cons event rest ih =>
      rcases event with ⟨k,b⟩
      simp only [collect,List.foldl_cons] at *
      rw [ih]
      by_cases same : (k,b) = (key,bit) <;>
        simp [increment,same,Nat.add_assoc,Nat.add_comm]

theorem merged_multiplicity [DecidableEq Key] (events : List (Key × Bool))
    (bank : Counts Key) (key : Key) (bit : Bool) :
    bank key bit + collect events (fun _ _ => 0) key bit =
      bank key bit + (events.filter (fun e => decide (e = (key,bit)))).length := by
  simp only [collected_multiplicity,Nat.zero_add]

def ktKernel (key : P → Key) (next : P → Bool → P) (bank : Counts Key) : Kernel P where
  probability path bit := BinaryPrediction.kernel.probability
    (BinaryPrediction.fromCounts (bank (key path) false) (bank (key path) true)) bit
  next := next
  normalized _ := BinaryPrediction.probabilities_normalized _

def ktRule [DecidableEq Key] (key : P → Key) (next : P → Bool → P) :
    Rule (Counts Key) P (Counts Key) where
  forecast := ktKernel key next
  empty := fun _ _ => 0
  record _ path bit delta := increment delta (key path) bit
  publish bank delta := fun k b => bank k b + delta k b

def visits (key : P → Key) (next : P → Bool → P) (path : P) : List Bool → List (Key × Bool)
  | [] => []
  | bit :: rest => (key path,bit) :: visits key next (next path bit) rest

theorem kt_delta_exact [DecidableEq Key] (key : P → Key) (next : P → Bool → P)
    (frame : Frame (Counts Key) P (Counts Key)) (body : List Bool) :
    (feed (ktRule key next) frame body).delta = collect (visits key next frame.path body) frame.delta := by
  induction body generalizing frame with
  | nil => rfl
  | cons bit rest ih => exact ih (step (ktRule key next) frame bit)

theorem kt_published_multiplicity [DecidableEq Key] (key : P → Key) (next : P → Bool → P)
    (bank : Counts Key) (path : P) (body : List Bool) (k : Key) (b : Bool) :
    (result (ktRule key next) (feed (ktRule key next) (begin (ktRule key next) bank path) body)).bank k b =
      bank k b + ((visits key next path body).filter (fun e => decide (e = (k,b)))).length := by
  change (feed (ktRule key next) (begin (ktRule key next) bank path) body).frozen k b +
    (feed (ktRule key next) (begin (ktRule key next) bank path) body).delta k b = _
  rw [feed_frozen,kt_delta_exact]
  exact merged_multiplicity _ bank k b

end FrozenTraining
