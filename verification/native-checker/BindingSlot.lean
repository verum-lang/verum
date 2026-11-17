module
public import VerumProof
public section

open Aeneas Aeneas.Std

namespace verum_proof.binders

private theorem sub_exact (x y : Usize) (h : y.val ≤ x.val) :
    ∃ z : Usize, x - y = Result.ok z ∧ z.val = x.val - y.val := by
  change ∃ z : Usize, UScalar.sub x y = Result.ok z ∧ _
  unfold UScalar.sub
  rw [ite_eq_right (Nat.not_lt.mpr h)]
  refine ⟨_, rfl, ?_⟩
  exact Nat.mod_eq_of_lt (lt_of_le_of_lt (Nat.sub_le _ _) x.hBounds)

/-- Both machine subtractions return a value for every in-bounds index. -/
theorem binding_slot_in_bounds (depth index : Usize) (h : index.val < depth.val) :
    ∃ slot : Usize, binding_slot depth index = Result.ok (some slot) ∧
      slot.val + index.val + 1 = depth.val := by
  have hone : (1#usize).val = 1 := by simp
  obtain ⟨last, hlast, last_val⟩ :=
    sub_exact depth 1#usize (by omega)
  obtain ⟨slot, hslot, slot_val⟩ :=
    sub_exact last index (by omega)
  refine ⟨slot, ?_, by omega⟩
  simp [binding_slot, (UScalar.lt_equiv _ _).mpr h, hlast, hslot]

private theorem out_of_bounds (depth index : Usize) (h : depth.val ≤ index.val) :
    binding_slot depth index = Result.ok none := by
  have hcmp : ¬ index < depth := fun hc => Nat.not_lt.mpr h ((UScalar.lt_equiv _ _).mp hc)
  simp [binding_slot, hcmp]

theorem binding_slot_some_iff (depth index slot : Usize) :
    binding_slot depth index = Result.ok (some slot) ↔
      slot.val + index.val + 1 = depth.val := by
  by_cases h : index.val < depth.val
  · obtain ⟨actual, hactual, hsum⟩ := binding_slot_in_bounds depth index h
    constructor
    · intro heq
      have he := Option.some.inj (Result.ok_injective (hactual.symm.trans heq))
      simpa [← he] using hsum
    · intro hsum'
      have he := UScalar.val_eq_imp actual slot (by omega)
      simpa [he] using hactual
  · have hout := out_of_bounds depth index (Nat.le_of_not_gt h)
    constructor
    · intro heq
      cases Result.ok_injective (hout.symm.trans heq)
    · omega

theorem binding_slot_none_iff (depth index : Usize) :
    binding_slot depth index = Result.ok none ↔ depth.val ≤ index.val := by
  refine ⟨?_, out_of_bounds depth index⟩
  intro hnone
  by_contra! h
  obtain ⟨slot, hslot, _⟩ := binding_slot_in_bounds depth index h
  cases Result.ok_injective (hslot.symm.trans hnone)

theorem binding_slot_involutive (depth index slot : Usize) :
    binding_slot depth index = Result.ok (some slot) ↔
      binding_slot depth slot = Result.ok (some index) := by
  rw [binding_slot_some_iff, binding_slot_some_iff]
  omega

/-- A selected outermost-first element is the indexed innermost-first binding.
    The entries may have any type. -/
theorem binding_slot_selects_entry {α : Type _} (entries : List α) (depth index slot : Usize)
    (length_eq : entries.length = depth.val)
    (selected : binding_slot depth index = Result.ok (some slot)) :
    entries[slot.val]? = entries.reverse[index.val]? := by
  have h := (binding_slot_some_iff depth index slot).mp selected
  rw [List.getElem?_reverse (by omega)]
  have position : entries.length - 1 - index.val = slot.val := by omega
  rw [position]

/-- The shared Rust lookup returns the indexed binding for every slice and
    returns `none` precisely when the de Bruijn index is out of bounds. -/
theorem lookup_binding_exact {α : Type _} (entries : Slice α) (index : Usize) :
    lookup_binding entries index = Result.ok (entries.val.reverse[index.val]?) := by
  have len_eq := Slice.len_val entries
  by_cases h : index.val < entries.val.length
  · obtain ⟨slot, hslot, hsum⟩ := binding_slot_in_bounds entries.len index (by simpa using h)
    have bound : slot.val + index.val + 1 = entries.val.length := hsum.trans len_eq
    have hs : slot.val < entries.val.length := by omega
    have selected := binding_slot_selects_entry entries.val entries.len index slot len_eq.symm hslot
    have read : Slice.index_usize entries slot = Result.ok (entries.val[slot.val]'hs) := by
      simp [Slice.index_usize, hs]
    rw [← selected, List.getElem?_eq_getElem hs]
    simp [lookup_binding, hslot, read]
  · have hout := (binding_slot_none_iff entries.len index).mpr (by simpa using Nat.le_of_not_gt h)
    have missing : entries.val.reverse[index.val]? = none := by
      apply List.getElem?_eq_none
      simpa using Nat.le_of_not_gt h
    simp [lookup_binding, hout, missing]

open Lean in
run_cmd do
  let allowed := #[``propext, ``Classical.choice, ``Quot.sound]
  for decl in #[``binding_slot_in_bounds, ``binding_slot_some_iff, ``binding_slot_none_iff,
                ``binding_slot_involutive, ``binding_slot_selects_entry, ``lookup_binding_exact] do
    for ax in ← collectAxioms decl do
      unless allowed.contains ax do
        throwError "{decl} depends on unexpected axiom {ax}"

end verum_proof.binders
