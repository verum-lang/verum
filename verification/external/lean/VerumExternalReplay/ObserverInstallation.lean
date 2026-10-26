import VerumExternalReplay.PartialObservers

/- Semantic installation projection: check the receiver prefix against retained
   histories, refuse gaps, then replay from the definition's initial state.
   Resource refusal, admission acquisition, native origin fidelity and ownership
   of the resulting instance require separate runtime evidence. In particular,
   `none` here is not a classification of an observed resource refusal. -/

namespace ObserverInstallation

set_option warningAsError true

structure History (Origin Event : Type) where
  origin : Origin
  events : List (Option Event)

variable {H E S : Type}

def entries : List (Option E) → Option (List E)
  | [] => some []
  | none :: _ => none
  | some event :: tail => (entries tail).map (event :: ·)

theorem entries_exact (rows : List (Option E)) (events : List E) :
    entries rows = some events ↔ events.map some = rows := by
  induction rows generalizing events with
  | nil => cases events <;> simp [entries]
  | cons row tail ih =>
    cases row with
    | none => cases events <;> simp [entries]
    | some event =>
      cases events with
      | nil => simp [entries]
      | cons head rest =>
        simp [entries, Option.map_eq_some_iff, ih, eq_comm]
        exact and_comm

def compatible [DecidableEq H] [DecidableEq E]
    (known receiver : History H E) (cut : Nat) : Bool :=
  decide (known.origin = receiver.origin →
    known.events.take cut = (receiver.events.take cut).take known.events.length)

def admitted [DecidableEq H] [DecidableEq E]
    (known : List (History H E)) (receiver : History H E) (cut : Nat) : Bool :=
  decide (cut ≤ receiver.events.length) && known.all (fun old => compatible old receiver cut)

def install [DecidableEq H] [DecidableEq E]
    (step : S → E → Option S) (initial : S) (known : List (History H E))
    (receiver : History H E) (cut : Nat) : Option S :=
  if admitted known receiver cut then
    (entries (receiver.events.take cut)).bind (fun events => PartialObservers.fold step events initial)
  else none

theorem success_exact [DecidableEq H] [DecidableEq E]
    (step : S → E → Option S) (initial result : S) (known : List (History H E))
    (receiver : History H E) (cut : Nat) :
    install step initial known receiver cut = some result ↔
      admitted known receiver cut = true ∧ ∃ events,
        events.map some = receiver.events.take cut ∧
        PartialObservers.fold step events initial = some result := by
  simp only [install]
  split
  · simp_all [Option.bind_eq_some_iff, entries_exact]
  · simp_all

theorem conflicting_origin_refused [DecidableEq H] [DecidableEq E]
    (step : S → E → Option S) (initial : S) (known receiver : History H E) (cut : Nat)
    (origin : known.origin = receiver.origin)
    (changed : known.events.take cut ≠ (receiver.events.take cut).take known.events.length) :
    install step initial [known] receiver cut = none := by
  simp [install, admitted, compatible, origin, changed]

theorem beyond_prefix_refused [DecidableEq H] [DecidableEq E]
    (step : S → E → Option S) (initial : S) (known : List (History H E))
    (receiver : History H E) (cut : Nat) (beyond : receiver.events.length < cut) :
    install step initial known receiver cut = none := by
  simp [install, admitted, Nat.not_le_of_gt beyond]

-- Native layer birth is an offset into the global journal. The receiver uses
-- the suffix length; a global ordinal must not be used as its relative cut.
theorem relative_prefix_length (journal : List E) (birth : Nat) :
    (journal.drop birth).length = journal.length - birth := List.length_drop

theorem relative_prefix_replay (step : S → E → Option S) (initial : S)
    (journal : List E) (birth : Nat) :
    PartialObservers.fold step ((journal.drop birth).take (journal.length - birth)) initial =
      PartialObservers.fold step (journal.drop birth) initial := by
  rw [← List.length_drop (l := journal) (i := birth), List.take_length]

end ObserverInstallation
