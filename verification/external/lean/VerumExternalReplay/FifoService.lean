import Std.Tactic

/- Canonical FIFO service boundary, independent of declaration/mass arithmetic.
   Keep these public names stable for existing acquisition proofs. -/
namespace AcquisitionScheduling
set_option warningAsError true

-- A is a stable owned-attempt identity, not merely a search address. This is
-- the queue's identity projection: continuation payloads and progress require
-- separate runtime evidence. An unfinished attempt retains its identity.
structure Turn (A : Type) where
  unfinished : Bool
  arrivals : List A
  deriving DecidableEq

def service (queue : List A) (turn : Turn A) : List A :=
  match queue with
  | [] => turn.arrivals
  | head :: tail => tail ++ (if turn.unfinished then [head] else []) ++ turn.arrivals

def selected (queue : List A) (plan : Nat → Turn A) : Nat → List (Option A)
  | 0 => []
  | n + 1 => queue.head? :: selected (service queue (plan 0)) (fun i => plan (i + 1)) n

theorem tail_arrivals_keep_predecessors (head target : A) (ahead suffix : List A) (turn : Turn A) :
    service (head :: (ahead ++ target :: suffix)) turn =
      ahead ++ target :: (suffix ++ (if turn.unfinished then [head] else []) ++ turn.arrivals) := by
  simp [service, List.append_assoc]

theorem fifo_service_bound (target : A) (ahead suffix : List A) (plan : Nat → Turn A) :
    some target ∈ selected (ahead ++ target :: suffix) plan (ahead.length + 1) := by
  induction ahead generalizing suffix plan with
  | nil => simp [selected]
  | cons head ahead ih =>
    simp only [List.cons_append, List.length_cons]
    change some target ∈ some head :: selected
      (service (head :: (ahead ++ target :: suffix)) (plan 0))
      (fun i => plan (i + 1)) (ahead.length + 1)
    apply List.mem_cons_of_mem
    rw [tail_arrivals_keep_predecessors]
    exact ih _ _

theorem unfinished_owner_retained (head : A) (tail : List A) (arrivals : List A) :
    head ∈ service (head :: tail) ⟨true, arrivals⟩ := by simp [service]

theorem arrivals_cannot_overtake (head : A) (tail arrivals : List A) :
    service (head :: tail) ⟨false, arrivals⟩ = tail ++ arrivals := by simp [service]

end AcquisitionScheduling
