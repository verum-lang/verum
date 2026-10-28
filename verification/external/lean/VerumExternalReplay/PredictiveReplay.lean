import VerumExternalReplay.AdaptiveResidual
import VerumExternalReplay.PartialObservers

/-! Causal replay transports the VM and learned bank together. A prediction
reads only the pre-event VM and query; the actual target is a separate field.
`none` denotes semantic unavailability, not a budget or implementation failure.
Native continuations and their metering need separate correspondence evidence. -/
namespace PredictiveReplay
set_option warningAsError true

structure Prediction where
  context : List Nat
  mask : List Nat

structure Record (Q E : Type) where
  query : Q
  event : E
  target : List Nat

structure Bank where
  body : ResidualPrediction.State
  lengths : AdaptiveLength.Counts
  deriving DecidableEq

def initial : Bank := ⟨ResidualPrediction.initial, []⟩

def learn (bank : Bank) (prediction : Prediction) (target : List Nat) : Bank :=
  ⟨ResidualPrediction.row bank.body ⟨prediction.context, prediction.mask, target⟩,
   AdaptiveLength.commit bank.lengths target.length⟩

def Coherent (bank : Bank) : Prop := AdaptiveLength.total bank.lengths = bank.body.samples

instance (bank : Bank) : Decidable (Coherent bank) := inferInstanceAs (Decidable (_ = _))

theorem initial_coherent : Coherent initial := rfl

theorem learn_samples (bank : Bank) (prediction : Prediction) (target : List Nat) :
    (learn bank prediction target).body.samples = bank.body.samples + 1 := rfl

theorem learn_bits (bank : Bank) (prediction : Prediction) (target : List Nat) :
    (learn bank prediction target).body.bits = bank.body.bits + 8 * target.length := rfl

theorem learn_lengths (bank : Bank) (prediction : Prediction) (target : List Nat) :
    AdaptiveLength.total (learn bank prediction target).lengths =
      AdaptiveLength.total bank.lengths + 1 := AdaptiveLength.commit_total _ _

theorem learn_coherent (bank : Bank) (prediction : Prediction) (target : List Nat)
    (before : Coherent bank) : Coherent (learn bank prediction target) := by
  unfold Coherent
  rw [learn_lengths, learn_samples, before]

structure State (V : Type) where
  vm : V
  bank : Bank

variable {V Q E : Type}

def step (read : V → Q → Option Prediction) (update : V → E → Option V)
    (s : State V) (r : Record Q E) : Option (State V) := do
  let prediction ← read s.vm r.query
  let next ← update s.vm r.event
  pure ⟨next, learn s.bank prediction r.target⟩

theorem step_exact (read : V → Q → Option Prediction) (update : V → E → Option V)
    (s next : State V) (r : Record Q E) :
    step read update s r = some next ↔ ∃ prediction vm,
      read s.vm r.query = some prediction ∧ update s.vm r.event = some vm ∧
      next = ⟨vm, learn s.bank prediction r.target⟩ := by
  cases hr : read s.vm r.query with
  | none => simp [step, hr]
  | some prediction =>
    cases hu : update s.vm r.event with
    | none => simp [step, hr, hu]
    | some vm => simp [step, hr, hu, eq_comm]

theorem missing_prediction_refused (read : V → Q → Option Prediction)
    (update : V → E → Option V) (s : State V) (r : Record Q E)
    (missing : read s.vm r.query = none) : step read update s r = none := by
  simp [step, missing]

theorem missing_update_refused (read : V → Q → Option Prediction)
    (update : V → E → Option V) (s : State V) (r : Record Q E)
    (missing : update s.vm r.event = none) : step read update s r = none := by
  simp [step, missing]

def replay (read : V → Q → Option Prediction) (update : V → E → Option V)
    (rows : List (Record Q E)) (s : State V) : Option (State V) :=
  PartialObservers.fold (step read update) rows s

theorem replay_append (read : V → Q → Option Prediction) (update : V → E → Option V)
    (before after : List (Record Q E)) (s : State V) :
    replay read update (before ++ after) s =
      (replay read update before s).bind (replay read update after) :=
  PartialObservers.fold_append _ _ _ _

theorem replay_samples (read : V → Q → Option Prediction) (update : V → E → Option V)
    (rows : List (Record Q E)) (s next : State V)
    (success : replay read update rows s = some next) :
    next.bank.body.samples = s.bank.body.samples + rows.length := by
  induction rows generalizing s with
  | nil => simp only [replay, PartialObservers.fold, Option.some.injEq] at success
           subst next; simp
  | cons r rows ih =>
    obtain ⟨middle, first, rest⟩ := Option.bind_eq_some_iff.mp success
    obtain ⟨prediction, vm, _, _, rfl⟩ := (step_exact read update s middle r).mp first
    rw [ih _ rest, learn_samples]
    simp [Nat.add_assoc, Nat.add_comm]

theorem replay_coherent (read : V → Q → Option Prediction) (update : V → E → Option V)
    (rows : List (Record Q E)) (s next : State V) (before : Coherent s.bank)
    (success : replay read update rows s = some next) : Coherent next.bank := by
  induction rows generalizing s with
  | nil => simpa only [replay, PartialObservers.fold, Option.some.injEq] using
      (show s = next from Option.some.inj success) ▸ before
  | cons r rows ih =>
    obtain ⟨middle, first, rest⟩ := Option.bind_eq_some_iff.mp success
    obtain ⟨prediction, vm, _, _, rfl⟩ := (step_exact read update s middle r).mp first
    exact ih _ (learn_coherent _ _ _ before) rest

/-- Even empty target bodies add a length observation. Advancing only the VM
cannot equal an accepted replay of any nonempty tail. -/
theorem vm_only_catchup_insufficient (read : V → Q → Option Prediction)
    (update : V → E → Option V) (rows : List (Record Q E)) (s next : State V)
    (nonempty : rows ≠ []) (success : replay read update rows s = some next) :
    next.bank ≠ s.bank := by
  intro equal
  have counts := replay_samples read update rows s next success
  rw [equal] at counts
  have positive : 0 < rows.length := List.length_pos_iff.mpr nonempty
  omega

end PredictiveReplay
