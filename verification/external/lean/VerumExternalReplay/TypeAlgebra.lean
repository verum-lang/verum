import Std

/- Independent semantics for the finite distributive type algebra. Atoms denote
   membership propositions for one fixed value, store and ownership world. No
   atomic subtyping, representation conversion or callable law is assumed. -/
namespace Verum.TypeAlgebra

inductive Formula where
  | bottom
  | top
  | atom (id : Nat)
  | union (left right : Formula)
  | intersection (left right : Formula)
  deriving DecidableEq

def denote (valuation : Nat → Prop) : Formula → Prop
  | .bottom => False
  | .top => True
  | .atom id => valuation id
  | .union a b => denote valuation a ∨ denote valuation b
  | .intersection a b => denote valuation a ∧ denote valuation b

def All (v : Nat → Prop) (context : List Formula) : Prop :=
  ∀ a ∈ context, denote v a

def Any (v : Nat → Prop) (context : List Formula) : Prop :=
  ∃ a ∈ context, denote v a

def Valid (left right : List Formula) : Prop :=
  ∀ v, All v left → Any v right

def Same (a b : List Formula) : Bool :=
  a.all (fun x => decide (x ∈ b)) && b.all (fun x => decide (x ∈ a))

theorem same_forward {a b : List Formula} (h : Same a b = true) :
    ∀ x ∈ a, x ∈ b := by
  simp only [Same, Bool.and_eq_true, List.all_eq_true, decide_eq_true_eq] at h
  exact h.1

theorem reframe {l r l' r' : List Formula}
    (hl : Same l l' = true) (hr : Same r r' = true) (h : Valid l r) : Valid l' r' := by
  intro v hypotheses
  obtain ⟨a, ha, value⟩ := h v (fun x hx => hypotheses x (same_forward hl x hx))
  exact ⟨a, same_forward hr a ha, value⟩

theorem identity {l r : List Formula} (a : Formula)
    (hl : a ∈ l) (hr : a ∈ r) : Valid l r := by
  intro v hypotheses
  exact ⟨a, hr, hypotheses a hl⟩

theorem bottomLeft {l r : List Formula} (h : Formula.bottom ∈ l) : Valid l r := by
  intro v hypotheses
  exact (hypotheses .bottom h).elim

theorem topRight {l r : List Formula} (h : Formula.top ∈ r) : Valid l r := by
  intro v _
  exact ⟨.top, h, True.intro⟩

theorem intersectionLeft {a b : Formula} {l r : List Formula}
    (h : Valid (a :: b :: l) r) : Valid (.intersection a b :: l) r := by
  intro v hypotheses
  have both : denote v a ∧ denote v b := hypotheses (.intersection a b) (by simp)
  apply h v
  intro x hx
  simp only [List.mem_cons] at hx
  rcases hx with hx | hx | hx
  · simpa only [hx] using both.1
  · simpa only [hx] using both.2
  · exact hypotheses x (by simp [hx])

theorem unionRight {a b : Formula} {l r : List Formula}
    (h : Valid l (a :: b :: r)) : Valid l (.union a b :: r) := by
  intro v hypotheses
  obtain ⟨x, hx, value⟩ := h v hypotheses
  simp only [List.mem_cons] at hx
  rcases hx with hx | hx | hx
  · exact ⟨.union a b, by simp, Or.inl (hx ▸ value)⟩
  · exact ⟨.union a b, by simp, Or.inr (hx ▸ value)⟩
  · exact ⟨x, by simp [hx], value⟩

theorem topLeft {l r : List Formula}
    (h : Valid l r) : Valid (.top :: l) r := by
  intro v hypotheses
  exact h v (fun x hx => hypotheses x (by simp [hx]))

theorem bottomRight {l r : List Formula}
    (h : Valid l r) : Valid l (.bottom :: r) := by
  intro v hypotheses
  obtain ⟨x, hx, value⟩ := h v hypotheses
  exact ⟨x, by simp [hx], value⟩

theorem unionLeft {a b : Formula} {l r : List Formula}
    (ha : Valid (a :: l) r) (hb : Valid (b :: l) r) : Valid (.union a b :: l) r := by
  intro v hypotheses
  have rest : All v l := fun x hx => hypotheses x (by simp [hx])
  rcases hypotheses (.union a b) (by simp) with aValue | bValue
  · apply ha v
    intro x hx
    rcases List.mem_cons.mp hx with rfl | hx
    · exact aValue
    · exact rest x hx
  · apply hb v
    intro x hx
    rcases List.mem_cons.mp hx with rfl | hx
    · exact bValue
    · exact rest x hx

theorem intersectionRight {a b : Formula} {l r : List Formula}
    (ha : Valid l (a :: r)) (hb : Valid l (b :: r)) : Valid l (.intersection a b :: r) := by
  classical
  intro v hypotheses
  by_cases already : Any v r
  · obtain ⟨x, hx, value⟩ := already
    exact ⟨x, by simp [hx], value⟩
  · have aValue : denote v a := by
      obtain ⟨x, hx, value⟩ := ha v hypotheses
      rcases List.mem_cons.mp hx with rfl | hx
      · exact value
      · exact (already ⟨x, hx, value⟩).elim
    have bValue : denote v b := by
      obtain ⟨x, hx, value⟩ := hb v hypotheses
      rcases List.mem_cons.mp hx with rfl | hx
      · exact value
      · exact (already ⟨x, hx, value⟩).elim
    exact ⟨.intersection a b, by simp, aValue, bValue⟩

theorem entails {source target : Formula} (h : Valid [source] [target]) :
    ∀ v, denote v source → denote v target := by
  intro v sourceValue
  have allSource : All v [source] := by
    intro x hx
    simpa only [List.mem_singleton.mp hx] using sourceValue
  obtain ⟨x, hx, targetValue⟩ := h v allSource
  simpa only [List.mem_singleton.mp hx] using targetValue

end Verum.TypeAlgebra
