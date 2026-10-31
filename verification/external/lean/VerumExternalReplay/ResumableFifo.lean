import VerumExternalReplay.FifoService

/-!
FIFO ownership across resumable finite service quanta. A held-head microstep
may append arrivals but cannot rotate or discard the selected owner. An empty
arrival list also describes a work/storage refusal in this queue projection.
Publication, completion or preparation of the next tuple ends one quantum;
the existing service operation then drops or requeues the head.

A Quantum is a finite completion witness, not a guarantee that native work
terminates. Producing such witnesses requires retained storage, eventually usable
work/storage grants, finite frozen operands/cases and terminating primitive
callbacks. If those premises fail, an infinite held-head interval is possible.
No fixed wall-time bound, successful candidate or Rust refinement is inferred.
-/
namespace ResumableFifo
set_option autoImplicit false
set_option warningAsError true

open AcquisitionScheduling (Turn service)
variable {A : Type}

inductive Step (A : Type) where
  | hold (arrivals : List A)
  | boundary (turn : Turn A)
  deriving DecidableEq

def advance (queue : List A) : Step A → List A
  | .hold arrivals => queue ++ arrivals
  | .boundary turn => service queue turn

def run (queue : List A) : List (Step A) → List A
  | [] => queue
  | step :: rest => run (advance queue step) rest

/-- Selection here records a completed service quantum, not a poll or a grant. -/
def serviced (queue : List A) : List (Step A) → List (Option A)
  | [] => []
  | .hold arrivals :: rest => serviced (queue ++ arrivals) rest
  | .boundary turn :: rest => queue.head? :: serviced (service queue turn) rest

def completed : List (Step A) → Nat
  | [] => 0
  | .hold _ :: rest => completed rest
  | .boundary _ :: rest => completed rest + 1

theorem held_owner_and_order (head : A) (tail arrivals : List A) :
    advance (head :: tail) (.hold arrivals) = head :: (tail ++ arrivals) := rfl

theorem resource_refusal_retains_queue (queue : List A) :
    advance queue (.hold []) = queue := by simp [advance]

theorem arrivals_precede_requeued_owner (head : A) (tail arrivals later : List A) :
    run (head :: tail) [.hold arrivals, .boundary ⟨true, later⟩] =
      tail ++ arrivals ++ [head] ++ later := by
  simp [run, advance, service, List.append_assoc]

theorem unfinished_owner_retained (head : A) (tail arrivals : List A) :
    head ∈ advance (head :: tail) (.boundary ⟨true, arrivals⟩) :=
  AcquisitionScheduling.unfinished_owner_retained head tail arrivals

theorem completed_append (first second : List (Step A)) :
    completed (first ++ second) = completed first + completed second := by
  induction first with
  | nil => simp [completed]
  | cons step rest ih => cases step <;> simp [completed, ih] <;> omega

/-- Tail arrivals cannot add predecessors. Each boundary removes exactly the
current predecessor, even if it is unfinished and requeued behind the target. -/
theorem finite_fifo_service (target : A) (ahead suffix : List A) (events : List (Step A))
    (enough : ahead.length + 1 ≤ completed events) :
    some target ∈ serviced (ahead ++ target :: suffix) events := by
  induction events generalizing ahead suffix with
  | nil => simp [completed] at enough
  | cons step rest ih =>
      cases step with
      | hold arrivals =>
          simpa [serviced, List.append_assoc] using ih ahead (suffix ++ arrivals) enough
      | boundary turn =>
          cases ahead with
          | nil => simp [serviced]
          | cons head ahead =>
              simp only [List.cons_append, serviced]
              apply List.mem_cons_of_mem
              rw [AcquisitionScheduling.tail_arrivals_keep_predecessors]
              apply ih
              simp only [List.length_cons, completed] at enough
              omega

/-- An arbitrary finite pause creates no completed service, even if callers
keep polling. This is a countermodel to unconditional/fixed-latency fairness. -/
theorem pauses_do_not_service (queue : List A) (calls : Nat) :
    serviced queue (List.replicate calls (.hold [])) = [] := by
  induction calls with
  | zero => rfl
  | succ calls ih => simpa [List.replicate_succ, serviced] using ih

structure Quantum (A : Type) where
  holds : List (List A)
  finish : Turn A

def Quantum.events (quantum : Quantum A) : List (Step A) :=
  quantum.holds.map Step.hold ++ [.boundary quantum.finish]

theorem holds_do_not_complete (holds : List (List A)) :
    completed (holds.map Step.hold) = 0 := by
  induction holds with
  | nil => rfl
  | cons _ _ ih => simpa [completed] using ih

theorem quantum_completes_once (quantum : Quantum A) :
    completed quantum.events = 1 := by
  simp [Quantum.events, completed_append, holds_do_not_complete, completed]

/-- The plan supplies finite completion witnesses under its environmental
premises. It may choose unrelated, arbitrarily large durations and arrivals. -/
def through (plan : Nat → Quantum A) : Nat → List (Step A)
  | 0 => []
  | count + 1 => through plan count ++ (plan count).events

theorem through_completed (plan : Nat → Quantum A) (count : Nat) :
    completed (through plan count) = count := by
  induction count with
  | zero => rfl
  | succ count ih => simp [through, completed_append, quantum_completes_once, ih]

/-- Every existing owner receives completed service after at most one quantum
per initial predecessor plus its own. Arbitrary later arrivals do not alter
this quantum-count bound; no bound on individual quantum duration is supplied. -/
theorem finite_quantum_fairness (target : A) (ahead suffix : List A)
    (plan : Nat → Quantum A) :
    some target ∈ serviced (ahead ++ target :: suffix) (through plan (ahead.length + 1)) := by
  apply finite_fifo_service
  simp [through_completed]

theorem eventual_service (target : A) (ahead suffix : List A) (plan : Nat → Quantum A) :
    ∃ count : Nat, some target ∈ serviced (ahead ++ target :: suffix) (through plan count) :=
  ⟨ahead.length + 1, finite_quantum_fairness target ahead suffix plan⟩

end ResumableFifo
