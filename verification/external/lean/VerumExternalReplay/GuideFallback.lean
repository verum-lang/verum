import VerumExternalReplay.ExactMass

/-! Guide relaxation over one fixed finite family of exact names. A completed
`Drain` is a finite interleaving prefix that has exhausted the fallback; it may
leave a guide suffix unread. Obtaining such a prefix is an explicit scheduler
premise, not a theorem about native work, learning, or elapsed time. Names and
mass assignments share one frozen namespace. There is no hash/lexical quotient,
renormalization, favorable-order, Rust-refinement or world-equivalence claim. -/
namespace GuideFallback
open ExactMass
set_option warningAsError true

variable {K : Type}

def oldGuide (family : List K) (C R S : K → Bool) : List K :=
  family.filter (fun key => C key && R key && S key)

def newGuide (family : List K) (C R : K → Bool) : List K :=
  family.filter (fun key => C key && R key)

theorem guide_enlargement (family : List K) (C R S : K → Bool) (key : K)
    (old : key ∈ oldGuide family C R S) : key ∈ newGuide family C R := by
  simp only [oldGuide, newGuide, List.mem_filter, Bool.and_eq_true_iff] at *
  exact ⟨old.1, old.2.1⟩

theorem old_guide_subset (family : List K) (C R S : K → Bool) :
    ∀ key ∈ oldGuide family C R S, key ∈ family := by
  intro key h
  exact (List.mem_filter.mp h).1

theorem new_guide_subset (family : List K) (C R : K → Bool) :
    ∀ key ∈ newGuide family C R, key ∈ family := by
  intro key h
  exact (List.mem_filter.mp h).1

/-- Each step visits exactly the next guide or fallback name. Only exhaustion
of fallback licenses `done`; the remaining guide need not be exhausted. -/
inductive Drain : List K → List K → List K → Prop where
  | done (guide) : Drain guide [] []
  | guide {guide fallback trace} (key) (rest : Drain guide fallback trace) :
      Drain (key :: guide) fallback (key :: trace)
  | fallback {guide fallback trace} (key) (rest : Drain guide fallback trace) :
      Drain guide (key :: fallback) (key :: trace)

/-- A fallback-only schedule witnesses that the finite drain premise is
inhabited. It does not assert that a native scheduler chooses this schedule. -/
theorem fallback_only (guide family : List K) : Drain guide family family := by
  induction family with
  | nil => exact Drain.done guide
  | cons head tail ih => exact Drain.fallback head ih

theorem fallback_covered {guide fallback trace : List K}
    (drained : Drain guide fallback trace) : ∀ key ∈ fallback, key ∈ trace := by
  induction drained with
  | done guide => simp
  | guide head rest ih =>
    intro key h
    exact List.mem_cons_of_mem head (ih key h)
  | fallback head rest ih =>
    intro key h
    cases List.mem_cons.mp h with
    | inl same => exact same ▸ List.Mem.head _
    | inr mem => exact List.mem_cons_of_mem head (ih key mem)

theorem trace_sound {guide fallback trace : List K}
    (drained : Drain guide fallback trace) :
    ∀ key ∈ trace, key ∈ guide ∨ key ∈ fallback := by
  induction drained with
  | done guide => simp
  | guide head rest ih =>
    intro key h
    cases List.mem_cons.mp h with
    | inl same => exact Or.inl (same ▸ List.Mem.head _)
    | inr mem =>
      cases ih key mem with
      | inl hg => exact Or.inl (List.mem_cons_of_mem head hg)
      | inr hf => exact Or.inr hf
  | fallback head rest ih =>
    intro key h
    cases List.mem_cons.mp h with
    | inl same => exact Or.inr (same ▸ List.Mem.head _)
    | inr mem =>
      cases ih key mem with
      | inl hg => exact Or.inl hg
      | inr hf => exact Or.inr (List.mem_cons_of_mem head hf)

variable [DecidableEq K]

/-- Executable first-occurrence emission. `seen` contains complete exact names;
comparison never substitutes a dependency predicate or a hash for equality. -/
def emit (seen : List K) : List K → List K
  | [] => []
  | key :: rest =>
      if key ∈ seen then emit seen rest else key :: emit (key :: seen) rest

theorem emit_mem (key : K) (seen trace : List K) :
    key ∈ emit seen trace ↔ key ∈ trace ∧ key ∉ seen := by
  induction trace generalizing seen with
  | nil => simp [emit]
  | cons head tail ih =>
    by_cases old : head ∈ seen
    · by_cases same : key = head
      · subst key; simp [emit, old, ih]
      · simp [emit, old, ih, same]
    · by_cases same : key = head
      · subst key; simp [emit, old]
      · simp [emit, old, ih, same]

theorem emit_unique (seen trace : List K) : (emit seen trace).Nodup := by
  induction trace generalizing seen with
  | nil => simp [emit]
  | cons head tail ih =>
    by_cases old : head ∈ seen
    · simpa [emit, old] using ih seen
    · simp only [emit, ite_eq_right old, List.nodup_cons]
      exact ⟨fun h => (emit_mem head (head :: seen) tail).mp h |>.2 (by simp), ih _⟩

theorem complete_set {guide family trace : List K}
    (inside : ∀ key ∈ guide, key ∈ family) (drained : Drain guide family trace)
    (key : K) : key ∈ emit [] trace ↔ key ∈ family := by
  simp only [emit_mem, List.not_mem_nil, not_false_eq_true, and_true]
  constructor
  · intro h
    exact (trace_sound drained key h).elim (inside key) id
  · exact fallback_covered drained key

/-- Finite set equality plus exact-name uniqueness gives a permutation, not
an equality of order. The original family itself must have unique names. -/
theorem complete_permutation {guide family trace : List K}
    (unique : family.Nodup) (inside : ∀ key ∈ guide, key ∈ family)
    (drained : Drain guide family trace) : (emit [] trace).Perm family := by
  apply (List.perm_ext_iff_of_nodup (emit_unique [] trace) unique).mpr
  exact complete_set inside drained

theorem relaxed_permutation (family : List K) (C R S : K → Bool)
    (unique : family.Nodup) (oldTrace newTrace : List K)
    (old : Drain (oldGuide family C R S) family oldTrace)
    (new : Drain (newGuide family C R) family newTrace) :
    (emit [] oldTrace).Perm (emit [] newTrace) :=
  (complete_permutation unique (old_guide_subset family C R S) old).trans
    (complete_permutation unique (new_guide_subset family C R) new).symm

/-- Fractions need only have the existing exact positive-denominator contract.
Mass may be zero; preservation does not require or infer positivity/fairness. -/
theorem sum_permutation {left right : List Fraction} (same : left.Perm right) :
    Equal (sum left) (sum right) := by
  induction same with
  | nil => exact eq_refl _
  | cons value rest ih => exact add_congr_left value ih
  | swap a b rest =>
    exact eq_trans (eq_symm (add_assoc _ _ _))
      (eq_trans (add_congr_right _ (add_comm _ _)) (add_assoc _ _ _))
  | trans first second ih₁ ih₂ => exact eq_trans ih₁ ih₂

/-- The output carries the original frozen mass per name, never an ordinal or
rank-dependent replacement. Exact pair membership also preserves this identity. -/
def weighted (mass : K → Fraction) (trace : List K) : List (K × Fraction) :=
  (emit [] trace).map (fun key => (key, mass key))

theorem weighted_member {guide family trace : List K} (mass : K → Fraction)
    (inside : ∀ key ∈ guide, key ∈ family) (drained : Drain guide family trace)
    (key : K) (value : Fraction) :
    (key, value) ∈ weighted mass trace ↔ key ∈ family ∧ value = mass key := by
  simp only [weighted, List.mem_map]
  constructor
  · rintro ⟨candidate, present, same⟩
    obtain ⟨hk, hm⟩ := Prod.mk.inj same
    subst candidate
    exact ⟨(complete_set inside drained key).mp present, hm.symm⟩
  · rintro ⟨present, rfl⟩
    exact ⟨key, (complete_set inside drained key).mpr present, rfl⟩

theorem total_mass_preserved {guide family trace : List K} (mass : K → Fraction)
    (unique : family.Nodup) (inside : ∀ key ∈ guide, key ∈ family)
    (drained : Drain guide family trace) :
    Equal (sum ((emit [] trace).map mass)) (sum (family.map mass)) :=
  sum_permutation ((complete_permutation unique inside drained).map mass)

theorem relaxed_mass (family : List K) (C R S : K → Bool)
    (unique : family.Nodup) (oldTrace newTrace : List K) (mass : K → Fraction)
    (old : Drain (oldGuide family C R S) family oldTrace)
    (new : Drain (newGuide family C R) family newTrace) :
    Equal (sum ((emit [] oldTrace).map mass)) (sum ((emit [] newTrace).map mass)) :=
  sum_permutation ((relaxed_permutation family C R S unique oldTrace newTrace old new).map mass)

end GuideFallback
