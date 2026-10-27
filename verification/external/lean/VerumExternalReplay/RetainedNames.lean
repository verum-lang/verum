import VerumExternalReplay.ExactNames

/-! One paid comparison or insertion in a retained namespace scan. The caller
pins the full key and exact shared namespace owner. A mutex serializes each
step with append-only fork inserts. Successful allocation and exact comparison
are adapter premises. This model does not authenticate native owners or prove
callback purity, allocation success or arbitrary meter behavior. -/
namespace RetainedNames
set_option warningAsError true
variable {K : Type} [DecidableEq K]

inductive Progress where
  | pending : Nat → Progress
  | named : Nat → Progress
  | capacity
  deriving DecidableEq

/-- Every skipped entry was compared with this same full key. -/
def Checked (keys : List K) (key : K) (scanned : Nat) : Prop :=
  scanned ≤ keys.length ∧ ∀ i, i < scanned → keys[i]? ≠ some key

/-- `paid` means this one structural unit was admitted. Failed work admission
leaves both the table and cursor unchanged. Capacity is checked after scanning,
so an existing name remains readable in a full table. -/
def step (capacity : Nat) (keys : List K) (key : K) (scanned : Nat)
    (paid : Bool) : List K × Progress :=
  if !paid then (keys, .pending scanned) else
  match keys[scanned]? with
  | some value =>
      if value = key then (keys, .named scanned) else (keys, .pending (scanned + 1))
  | none =>
      if keys.length < capacity then (keys ++ [key], .named keys.length)
      else (keys, .capacity)

omit [DecidableEq K] in
theorem initial (keys : List K) (key : K) : Checked keys key 0 := by
  simp [Checked]

omit [DecidableEq K] in
theorem checked_append (keys suffix : List K) (key : K) (scanned : Nat)
    (checked : Checked keys key scanned) : Checked (keys ++ suffix) key scanned := by
  have bound := checked.1
  constructor
  · simp only [List.length_append]; omega
  · intro i hi
    rw [List.getElem?_append_left (by omega)]
    exact checked.2 i hi

omit [DecidableEq K] in
theorem advance_checked (keys : List K) (key value : K) (scanned : Nat)
    (checked : Checked keys key scanned) (lookup : keys[scanned]? = some value)
    (different : value ≠ key) : Checked keys key (scanned + 1) := by
  have bound := (List.getElem?_eq_some_iff.mp lookup).1
  constructor
  · omega
  · intro i hi
    by_cases old : i < scanned
    · exact checked.2 i old
    · have same : i = scanned := by omega
      subst i
      simpa [lookup] using different

omit [DecidableEq K] in
theorem end_absent (keys : List K) (key : K) (scanned : Nat)
    (checked : Checked keys key scanned) (lookup : keys[scanned]? = none) : key ∉ keys := by
  have last : scanned = keys.length := by
    have := List.getElem?_eq_none_iff.mp lookup
    have := checked.1
    omega
  intro member
  obtain ⟨i, hi⟩ := List.mem_iff_getElem?.mp member
  exact checked.2 i (by have := (List.getElem?_eq_some_iff.mp hi).1; omega) hi

theorem first_index (keys : List K) (key : K) (scanned : Nat)
    (checked : Checked keys key scanned) (lookup : keys[scanned]? = some key) :
    keys.idxOf key = scanned := by
  induction keys generalizing scanned with
  | nil => simp at lookup
  | cons head tail ih =>
    cases scanned with
    | zero =>
      simp only [List.getElem?_cons_zero, Option.some.injEq] at lookup
      subst head
      exact List.idxOf_cons_self
    | succ n =>
      have different : head ≠ key := by
        simpa using checked.2 0 (Nat.zero_lt_succ n)
      have tailChecked : Checked tail key n := by
        constructor
        · have := checked.1; simp only [List.length_cons] at this; omega
        · intro i hi
          simpa using checked.2 (i + 1) (by omega)
      have rest := ih n tailChecked (by simpa using lookup)
      simp [List.idxOf_cons, different, rest]

theorem unpaid_preserves (capacity : Nat) (keys : List K) (key : K) (scanned : Nat) :
    step capacity keys key scanned false = (keys, .pending scanned) := rfl

theorem pending_preserves (capacity : Nat) (keys : List K) (key : K) (scanned next : Nat)
    (paid : Bool) (checked : Checked keys key scanned)
    (pending : (step capacity keys key scanned paid).2 = .pending next) :
    (step capacity keys key scanned paid).1 = keys ∧ Checked keys key next := by
  cases paid with
  | false =>
    have same : scanned = next := by simpa [step] using pending
    subst next
    exact ⟨rfl, checked⟩
  | true =>
    cases lookup : keys[scanned]? with
    | none =>
      by_cases room : keys.length < capacity <;> simp [step, lookup, room] at pending
    | some value =>
      by_cases same : value = key
      · simp [step, lookup, same] at pending
      · have nextEq : scanned + 1 = next := by simpa [step, lookup, same] using pending
        subst next
        exact ⟨by simp [step, lookup, same],
          advance_checked keys key value scanned checked lookup same⟩

theorem named_projects (capacity : Nat) (keys : List K) (key : K) (scanned name : Nat)
    (paid : Bool) (checked : Checked keys key scanned)
    (named : (step capacity keys key scanned paid).2 = .named name) :
    ExactNames.intern capacity keys key = ((step capacity keys key scanned paid).1, some name) := by
  cases paid with
  | false => simp [step] at named
  | true =>
    cases lookup : keys[scanned]? with
    | none =>
      have absent := end_absent keys key scanned checked lookup
      by_cases room : keys.length < capacity
      · have same : keys.length = name := by simpa [step, lookup, room] using named
        subst name
        simp [step, lookup, room, ExactNames.intern, absent]
      · simp [step, lookup, room] at named
    | some value =>
      by_cases same : value = key
      · subst value
        have nameEq : scanned = name := by simpa [step, lookup] using named
        subst name
        rw [ExactNames.existing capacity keys key (List.mem_iff_getElem?.mpr ⟨scanned, lookup⟩),
            first_index keys key scanned checked lookup]
        simp [step, lookup]
      · simp [step, lookup, same] at named

theorem capacity_preserves (capacity : Nat) (keys : List K) (key : K) (scanned : Nat)
    (paid : Bool) (refused : (step capacity keys key scanned paid).2 = .capacity) :
    (step capacity keys key scanned paid).1 = keys := by
  cases paid with
  | false => simp [step] at refused
  | true =>
    cases lookup : keys[scanned]? with
    | none =>
      by_cases room : keys.length < capacity
      · simp [step, lookup, room] at refused
      · simp [step, lookup, room]
    | some value =>
      by_cases same : value = key <;> simp [step, lookup, same] at refused

theorem named_sound (capacity : Nat) (keys : List K) (key : K) (scanned name : Nat)
    (paid : Bool) (checked : Checked keys key scanned)
    (named : (step capacity keys key scanned paid).2 = .named name) :
    (step capacity keys key scanned paid).1[name]? = some key := by
  have projection := named_projects capacity keys key scanned name paid checked named
  have accepted := congrArg Prod.snd projection
  have sound := ExactNames.sound capacity keys key name accepted
  simpa only [projection] using sound

theorem named_preserves_unique (capacity : Nat) (keys : List K) (key : K)
    (scanned name : Nat) (paid : Bool) (checked : Checked keys key scanned)
    (unique : keys.Nodup) (named : (step capacity keys key scanned paid).2 = .named name) :
    (step capacity keys key scanned paid).1.Nodup := by
  have projection := named_projects capacity keys key scanned name paid checked named
  simpa only [projection] using ExactNames.preserves_unique capacity keys key unique

theorem sequential_injective (capacity : Nat) (keys : List K) (left right : K)
    (leftScan rightScan name : Nat) (leftPaid rightPaid : Bool)
    (leftChecked : Checked keys left leftScan)
    (rightChecked : Checked (step capacity keys left leftScan leftPaid).1 right rightScan)
    (a : (step capacity keys left leftScan leftPaid).2 = .named name)
    (b : (step capacity (step capacity keys left leftScan leftPaid).1 right rightScan rightPaid).2 = .named name) :
    left = right := by
  have first := named_projects capacity keys left leftScan name leftPaid leftChecked a
  have second := named_projects capacity _ right rightScan name rightPaid rightChecked b
  apply ExactNames.sequential_injective capacity keys left right name (congrArg Prod.snd first)
  rw [congrArg Prod.fst first]
  exact congrArg Prod.snd second

end RetainedNames
