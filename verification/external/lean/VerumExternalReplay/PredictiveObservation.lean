import VerumExternalReplay.ResidualContinuation
import VerumExternalReplay.RetainedNative
import VerumExternalReplay.ObserverAdmission

/-! Representation checks for observed predictive frontiers. Sparse map order
and rational reduction are not semantic identity. Allocation identities remain
separate in AtomicPublication. Initial native scores are antecedents here;
these checks do not reconstruct their acquisition histories. -/
namespace PredictiveObservation
open PredictiveReplay
set_option warningAsError true

local instance : Ord (List Nat × Nat) := lexOrd

def entriesAgree {K V : Type} [Ord K] [DecidableEq K] [DecidableEq V]
    (a b : List (K × V)) : Bool :=
  decide (a.mergeSort (fun x y => compare x.1 y.1 != .gt) =
          b.mergeSort (fun x y => compare x.1 y.1 != .gt))

/-- Every allocated entry survives comparison. Native decoding rejects duplicate
keys; no ordering or digest equality can erase a differing key/count pair. -/
theorem entries_preserved {K V : Type} [Ord K] [DecidableEq K] [DecidableEq V]
    (a b : List (K × V)) (equal : entriesAgree a b = true) : a.Perm b := by
  have same := of_decide_eq_true equal
  exact (List.mergeSort_perm a _).symm.trans (same ▸ List.mergeSort_perm b _)

def lengthsAgree (a b : AdaptiveLength.Counts) : Bool :=
  AdaptiveLength.total a == AdaptiveLength.total b &&
  a.all (fun (n, _) => AdaptiveLength.countAt n a == AdaptiveLength.countAt n b) &&
  b.all (fun (n, _) => AdaptiveLength.countAt n a == AdaptiveLength.countAt n b)

def banksAgree (a b : Bank) : Bool :=
  a.body.samples == b.body.samples && a.body.bits == b.body.bits &&
  decide (ExactMass.Equal a.body.ratio b.body.ratio) &&
  entriesAgree a.body.residual b.body.residual &&
  entriesAgree a.body.reference b.body.reference && lengthsAgree a.lengths b.lengths

def privateBank : ResidualContinuation.Cursor → Bank
  | .pending frame _ => frame.bank
  | .ready bank => bank

/-- Fixture codecs are declared independently of case names. The width and
padding rule must be bound to the native callbacks by source evidence. -/
def wire (width modulus value : Nat) : List Nat :=
  (List.range width).map (fun i => (value / 256^i) % 256) ++
    List.replicate (if modulus = 0 then 0 else value % modulus) 0

/-- Reuse the native arithmetic adapter; decoded programs admit only add and
identity. The panic interpretation is unreachable in this finite vocabulary. -/
def scalarApply (op : RetainedNative.Primitive) (args : List Nat) : Option Nat :=
  (RetainedNative.interpret (fun _ => .error .arity) op args).toOption

def evaluate (code : FiniteCode.Code RetainedNative.Primitive Nat)
    (binding : ObserverAdmission.Binding Nat) (state interface : Nat)
    (inputs : List Nat) : Option Nat :=
  if interface = binding.interface then
    (FiniteCode.bind binding.width binding.slots state inputs).bind
      (FiniteCode.run scalarApply code)
  else none

end PredictiveObservation
