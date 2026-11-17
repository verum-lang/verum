module
public import VerumProof
public section

open Aeneas Aeneas.Std Result ControlFlow

namespace verum_proof.proof_checker

/-- The mathematical value of the Rust level extracted from the production crate. -/
def Level.denote (ρ : String → Nat) : Level → Nat
  | .Concrete n => n.val
  | .Var x => ρ x
  | .Succ base => base.denote ρ + 1
  | .Max left right => max (left.denote ρ) (right.denote ρ)
  | .Offset base n => base.denote ρ + n.val

namespace universe_offset

private abbrev wireMax := core.num.U32.MAX
private abbrev chunk := UScalar.cast .U128 wireMax

private theorem max_val : wireMax.val = 4294967295 := by decide
private theorem chunk_val : chunk.val = 4294967295 := by decide

private theorem sub_exact {t : UScalarTy} (x y : UScalar t) (h : y.val ≤ x.val) :
    ∃ z : UScalar t, x - y = Result.ok z ∧ z.val = x.val - y.val := by
  change ∃ z : UScalar t, UScalar.sub x y = Result.ok z ∧ _
  unfold UScalar.sub
  rw [ite_eq_right (Nat.not_lt.mpr h)]
  refine ⟨_, rfl, ?_⟩
  exact Nat.mod_eq_of_lt (lt_of_le_of_lt (Nat.sub_le _ _) x.hBounds)

private theorem narrow_exact (x : U128) (h : x.val ≤ 4294967295) :
    (UScalar.cast .U32 x).val = x.val := by
  rw [UScalar.cast_val_eq]
  apply Nat.mod_eq_of_lt
  change x.val < 4294967296
  omega

private theorem add_exact (x y : U32) (h : x.val + y.val ≤ 4294967295) :
    ∃ z : U32, x + y = Result.ok z ∧ z.val = x.val + y.val := by
  have spec := @U32.add_spec x y (by simpa only [U32.max_eq] using h)
  generalize run : (x + y : Result U32) = r at spec ⊢
  cases r with
  | ret z => exact ⟨z, rfl, by simpa only [WP.spec_ok] using spec⟩
  | vis e k => exact False.elim ((WP.spec_vis _ _).mp spec)
  | div => exact False.elim (WP.spec_div.mp spec)

private theorem loop_exact (base : Level) (amount : U128) :
    ∃ out rem, append_wire_chunks_loop base amount = Result.ok (out, rem) ∧
      rem.val ≤ 4294967295 ∧ ∀ ρ, out.denote ρ + rem.val = base.denote ρ + amount.val := by
  by_cases more : chunk < amount
  · have greater := (UScalar.lt_equiv _ _).mp more
    obtain ⟨next, sub, value⟩ := sub_exact amount chunk (Nat.le_of_lt greater)
    have smaller : next.val < amount.val := by rw [chunk_val] at value greater; omega
    obtain ⟨out, rem, run, bound, invariant⟩ := loop_exact (.Offset base wireMax) next
    refine ⟨out, rem, ?_, bound, ?_⟩
    · unfold append_wire_chunks_loop at run ⊢
      rw [loop.eq_def]
      simpa only [append_wire_chunks_loop.body, append_wire_chunks.CHUNK,
        bind_ok, ite_eq_left more, sub] using run
    · intro ρ
      rw [invariant]
      simp only [Level.denote, max_val]
      rw [chunk_val] at value greater
      omega
  · refine ⟨base, amount, ?_, ?_, fun _ => rfl⟩
    · unfold append_wire_chunks_loop
      rw [loop.eq_def]
      simp only [append_wire_chunks_loop.body, append_wire_chunks.CHUNK,
        bind_ok, ite_eq_right more]
    · have bound := Nat.le_of_not_gt (fun h => more ((UScalar.lt_equiv _ _).mpr h))
      simpa only [chunk_val] using bound
termination_by amount.val

private theorem append_exact (base : Level) (amount : U128) :
    ∃ out, append_wire_chunks base amount = Result.ok out ∧
      ∀ ρ, out.denote ρ = base.denote ρ + amount.val := by
  obtain ⟨mid, rem, run, bound, invariant⟩ := loop_exact base amount
  have exactCast := narrow_exact rem bound
  unfold append_wire_chunks
  simp only [run, lift, bind_ok]
  change ∃ out, (match UScalar.cast .U32 rem with
    | 0#uscalar => Result.ok mid
    | 1#uscalar => Result.ok (.Succ mid)
    | _ => Result.ok (.Offset mid (UScalar.cast .U32 rem))) = Result.ok out ∧ _
  by_cases zero : UScalar.cast .U32 rem = 0#u32
  · refine ⟨mid, by rw [zero]; rfl, ?_⟩
    have val := congrArg UScalar.val zero
    simp only [exactCast] at val
    intro ρ
    specialize invariant ρ
    change rem.val = 0 at val
    omega
  by_cases one : UScalar.cast .U32 rem = 1#u32
  · refine ⟨.Succ mid, by rw [one]; rfl, ?_⟩
    have val := congrArg UScalar.val one
    simp only [exactCast] at val
    change rem.val = 1 at val
    intro ρ
    specialize invariant ρ
    simp only [Level.denote]
    omega
  · refine ⟨.Offset mid (UScalar.cast .U32 rem), ?_, fun ρ => ?_⟩
    · split <;> first | rfl | contradiction
    · simp only [Level.denote, exactCast, invariant]

/-- The actual native offset function returns a level for every machine input.
    Its denotation is exact for all parameter valuations, including values
    above the wire integer bound. -/
theorem level_with_offset_exact (base : Level) (amount : U128) :
    ∃ out, level_with_offset base amount = Result.ok out ∧
      ∀ ρ, out.denote ρ = base.denote ρ + amount.val := by
  cases base with
  | Concrete n =>
    have bounded := n.hBounds
    change n.val < 4294967296 at bounded
    obtain ⟨room, roomRun, roomVal⟩ := sub_exact wireMax n (by rw [max_val]; omega)
    rw [max_val] at roomVal
    unfold level_with_offset
    simp only [roomRun, lift, bind_ok]
    have wide := U32.cast_U128_val_eq room
    by_cases fits : amount ≤ UScalar.cast .U128 room
    · rw [ite_eq_left fits]
      have small := (UScalar.le_equiv _ _).mp fits
      have narrow := narrow_exact amount (by omega)
      obtain ⟨sum, sumRun, sumVal⟩ := add_exact n (UScalar.cast .U32 amount) (by
        rw [narrow]; omega)
      refine ⟨.Concrete sum, ?_, ?_⟩
      · simp only [sumRun, bind_ok]
      · intro ρ
        simp only [Level.denote, sumVal, narrow]
    · rw [ite_eq_right fits]
      have large : room.val < amount.val := by
        apply Nat.lt_of_not_ge; intro h
        apply fits
        exact (UScalar.le_equiv _ _).mpr (by omega)
      obtain ⟨rest, restRun, restVal⟩ := sub_exact amount (UScalar.cast .U128 room) (by omega)
      obtain ⟨out, run, invariant⟩ := append_exact (.Concrete wireMax) rest
      refine ⟨out, ?_, ?_⟩
      · simpa only [restRun, bind_ok] using run
      · intro ρ
        rw [invariant]
        simp only [Level.denote, max_val]
        omega
  | Var x => exact append_exact (.Var x) amount
  | Succ base => exact append_exact (.Succ base) amount
  | Max l r => exact append_exact (.Max l r) amount
  | Offset base k => exact append_exact (.Offset base k) amount

/-- At shift one, the operation used by T-Universe strictly raises every level. -/
theorem universe_successor_exact (base : Level) :
    ∃ out, universe_successor base = Result.ok out ∧
      ∀ ρ, out.denote ρ = base.denote ρ + 1 :=
  level_with_offset_exact base 1#u128

/-- The resulting universe can never classify itself at the original level. -/
theorem universe_successor_ne_input (base out : Level) (run : universe_successor base = Result.ok out)
    (ρ : String → Nat) : out.denote ρ ≠ base.denote ρ := by
  obtain ⟨actual, checked, value⟩ := universe_successor_exact base
  have same := Result.ok_injective (checked.symm.trans run)
  rw [← same, value]
  omega

open Lean in
run_cmd do
  let allowed := #[``propext, ``Classical.choice, ``Quot.sound]
  for decl in #[``level_with_offset_exact, ``universe_successor_exact, ``universe_successor_ne_input] do
    for ax in ← collectAxioms decl do
      unless allowed.contains ax do
        throwError "{decl} depends on unexpected axiom {ax}"

end universe_offset
end verum_proof.proof_checker
