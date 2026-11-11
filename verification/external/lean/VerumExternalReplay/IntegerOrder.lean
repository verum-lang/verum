import Std

/- Mathematical model of ConstEvaluator::integer_order's mixed-signedness
   branch. The native checked conversion succeeds exactly for nonnegative
   signed integers. No floating-point or narrowing conversion is used. -/
namespace Verum.IntegerOrder

def signedUnsigned (signed : Int) (unsigned : Nat) : Ordering :=
  if signed < 0 then .lt else compare signed.toNat unsigned

theorem sound (signed : Int) (unsigned : Nat) :
    signedUnsigned signed unsigned = compare signed (Int.ofNat unsigned) := by
  cases signed with
  | ofNat n =>
    have h : ¬ ((n : Int) < 0) := by omega
    simp [signedUnsigned, compare, compareOfLessAndEq, h, Int.ofNat_inj]
  | negSucc n =>
    simp [signedUnsigned, compare, compareOfLessAndEq]
    omega

#print axioms sound
end Verum.IntegerOrder
