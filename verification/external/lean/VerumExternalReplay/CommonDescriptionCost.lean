import Lean

/- Exact algebra for one fixed corpus, alphabet and coding interpretation.
   `baseline` is the complete parent code, including unselected contexts.
   The selected parent sum, child sum and both fees must describe the same
   candidate. These are correspondence premises, not consequences of algebra.
   No floating-point, logarithm, admission, provenance or search-completeness
   claim follows from these theorems. -/
namespace CommonDescriptionCost
open Lean.Grind AddCommMonoid AddCommGroup
set_option warningAsError true

structure Candidate (A : Type) where
  selectedParent : A
  children : A
  domainFee : A
  nameFee : A

variable {A : Type} [AddCommGroup A]

def gain (candidate : Candidate A) : A :=
  candidate.selectedParent - candidate.children - candidate.domainFee

def net (candidate : Candidate A) : A := gain candidate - candidate.nameFee

def full (baseline : A) (candidate : Candidate A) : A :=
  baseline - candidate.selectedParent + candidate.children + candidate.domainFee + candidate.nameFee

theorem common_summand_cancels (a b c : A) : (a + c) - (b + c) = a - b := by
  apply sub_eq_iff.mpr
  rw [← add_assoc, sub_add_cancel]

theorem common_minuend_cancels (a b c : A) : (a - b) - (a - c) = c - b := by
  calc
    (a - b) - (a - c) = (-b + a) - (-c + a) := by
      simp only [sub_eq_add_neg, add_comm]
    _ = -b - -c := common_summand_cancels _ _ _
    _ = c - b := by simp only [sub_eq_add_neg, neg_neg, add_comm]

theorem full_eq_baseline_sub_net (baseline : A) (candidate : Candidate A) :
    full baseline candidate = baseline - net candidate := by
  simp [full, net, gain, sub_eq_add_neg, neg_add, neg_neg, add_assoc,
    add_comm, add_left_comm]

theorem selected_replacement (outside : A) (candidate : Candidate A) :
    full (outside + candidate.selectedParent) candidate =
      outside + candidate.children + candidate.domainFee + candidate.nameFee := by
  simp only [full, add_sub_cancel]

theorem outside_cost_preserved (baseline outside : A) (candidate : Candidate A) :
    full (baseline + outside) candidate = full baseline candidate + outside := by
  simp only [full, sub_eq_add_neg, add_assoc, add_comm, add_left_comm]

theorem unchanged_outside_cancels (baseline outside : A) (left right : Candidate A) :
    full (baseline + outside) left - full (baseline + outside) right =
      full baseline left - full baseline right := by
  rw [outside_cost_preserved, outside_cost_preserved]
  exact common_summand_cancels _ _ _

theorem same_domain_baseline_cancels (left right : Candidate A)
    (parent : left.selectedParent = right.selectedParent)
    (domain : left.domainFee = right.domainFee) :
    net left - net right = right.children - left.children + (right.nameFee - left.nameFee) := by
  have l : net left = (right.selectedParent - right.domainFee) - (left.children + left.nameFee) := by
    simp only [net, gain, parent, domain, sub_eq_add_neg, neg_add,
      add_assoc, add_left_comm]
  have r : net right = (right.selectedParent - right.domainFee) - (right.children + right.nameFee) := by
    simp only [net, gain, sub_eq_add_neg, neg_add, add_assoc, add_left_comm]
  rw [l, r, common_minuend_cancels]
  simp only [sub_eq_add_neg, neg_add, add_assoc, add_left_comm]

-- Enlarging a naming family may change fees. Only unchanged outside code
-- cancels; no invariance under changed candidate fees is asserted.
theorem fee_change_retained (baseline extraDomain extraName : A) (candidate : Candidate A) :
    full baseline { candidate with
      domainFee := candidate.domainFee + extraDomain
      nameFee := candidate.nameFee + extraName } =
      full baseline candidate + extraDomain + extraName := by
  simp only [full, sub_eq_add_neg, add_assoc, add_comm, add_left_comm]

theorem ranking_tie (baseline : A) (left right : Candidate A) :
    full baseline left = full baseline right ↔ net left = net right := by
  rw [full_eq_baseline_sub_net, full_eq_baseline_sub_net]
  simp only [sub_eq_add_neg, add_right_inj, neg_eq_iff, neg_neg]

variable [LE A] [Std.IsPreorder A] [OrderedAdd A]

theorem ranking_le (baseline : A) (left right : Candidate A) :
    full baseline left ≤ full baseline right ↔ net right ≤ net left := by
  rw [full_eq_baseline_sub_net, full_eq_baseline_sub_net]
  simp only [sub_eq_add_neg, ← OrderedAdd.add_le_right_iff]
  rw [OrderedAdd.neg_le_iff, neg_neg]

theorem ranking_lt [LT A] [Std.LawfulOrderLT A] (baseline : A) (left right : Candidate A) :
    full baseline left < full baseline right ↔ net right < net left := by
  simp only [Std.lt_iff_le_and_not_ge, ranking_le]

end CommonDescriptionCost
