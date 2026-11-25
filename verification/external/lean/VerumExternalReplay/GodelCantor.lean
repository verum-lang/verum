import Init
import Init.Data.Nat.Sqrt.Lemmas

namespace Verum.GodelCantor
set_option warningAsError true

def triangle : Nat → Nat
  | 0 => 0
  | n + 1 => triangle n + n + 1

theorem twice_triangle (n : Nat) : 2 * triangle n = n * (n + 1) := by
  induction n with
  | zero => rfl
  | succ n ih =>
    calc
      2 * triangle (n+1) = 2 * triangle n + (2*n + 2) := by
        simp [triangle, Nat.mul_add, Nat.add_assoc]
      _ = (n+1)*(n+1+1) := by
        rw [ih, Nat.succ_mul n (n+1+1), Nat.mul_succ n (n+1)]
        omega

theorem triangle_formula (n : Nat) : triangle n = n * (n + 1) / 2 := by
  rw [← twice_triangle]; omega

theorem triangle_mono {i j : Nat} (h : i ≤ j) : triangle i ≤ triangle j := by
  induction h with
  | refl => exact Nat.le_refl _
  | step _ ih => simp only [triangle]; omega

theorem le_triangle (n : Nat) : n ≤ triangle n := by
  cases n with
  | zero => decide
  | succ n => simp only [triangle]; omega

def pair (a b : Nat) := triangle (a + b) + b

def diagonal (z : Nat) := ((8 * z + 1).sqrt - 1) / 2

private theorem square_boundary (n : Nat) :
    (2*n+1) * (2*n+1) = 8*triangle n+1 := by
  have h := twice_triangle n
  grind

theorem diagonal_interval (z : Nat) : triangle (diagonal z) ≤ z ∧ z < triangle (diagonal z + 1) := by
  let s := (8*z+1).sqrt
  have lower : s*s ≤ 8*z+1 := Nat.sqrt_le _
  have upper : 8*z+1 < (s+1)*(s+1) := Nat.lt_succ_sqrt _
  have positive : 0 < s := by
    by_cases hs : s = 0
    · rw [hs] at upper
      omega
    · omega
  have low : 2*diagonal z+1 ≤ s := by change 2*((s-1)/2)+1 ≤ s; omega
  have high : s+1 ≤ 2*(diagonal z+1)+1 := by change s+1 ≤ 2*((s-1)/2+1)+1; omega
  have left := Nat.le_trans (Nat.mul_le_mul low low) lower
  have right := Nat.lt_of_lt_of_le upper (Nat.mul_le_mul high high)
  rw [square_boundary] at left right
  omega

private theorem interval_unique {i j z : Nat}
    (hi : triangle i ≤ z ∧ z < triangle (i+1))
    (hj : triangle j ≤ z ∧ z < triangle (j+1)) : i = j := by
  rcases Nat.lt_trichotomy i j with h | h | h
  · have := triangle_mono (show i+1 ≤ j from h)
    omega
  · exact h
  · have := triangle_mono (show j+1 ≤ i from h)
    omega

def unpair (z : Nat) : Nat × Nat :=
  let d := diagonal z
  let b := z - triangle d
  (d - b, b)

theorem unpair_pair (a b : Nat) : unpair (pair a b) = (a,b) := by
  have interval : triangle (a+b) ≤ pair a b ∧ pair a b < triangle (a+b+1) := by
    simp only [pair, triangle]; omega
  have d := interval_unique (diagonal_interval (pair a b)) interval
  unfold unpair
  rw [d]
  unfold pair
  apply Prod.ext <;> dsimp <;> omega

theorem pair_unpair (z : Nat) : pair (unpair z).1 (unpair z).2 = z := by
  obtain ⟨lo, hi⟩ := diagonal_interval z
  simp only [triangle] at hi
  have sum : diagonal z - (z - triangle (diagonal z)) + (z - triangle (diagonal z)) = diagonal z := by omega
  simp only [unpair, pair, sum]
  omega

/-- Natural model of the two checked conversions in the native word encoder. -/
def wordMax := 2^64-1
def pairWord (a b : Nat) : Option Nat :=
  if a+b ≤ wordMax then
    let code := (a+b)*(a+b+1)/2+b
    if code ≤ wordMax then some code else none
  else none

theorem pairWord_exact (a b : Nat) :
    pairWord a b = if pair a b ≤ wordMax then some (pair a b) else none := by
  unfold pairWord
  rw [← triangle_formula]
  change (if a+b ≤ wordMax then (if pair a b ≤ wordMax then some (pair a b) else none) else none) = _
  by_cases hab : a+b ≤ wordMax
  · simp [hab]
  · have h := le_triangle (a+b)
    have bad : ¬pair a b ≤ wordMax := by unfold pair; omega
    simp [hab,bad]

theorem word_roundtrip {z : Nat} (hz : z ≤ wordMax) :
    pairWord (unpair z).1 (unpair z).2 = some z := by
  rw [pairWord_exact, pair_unpair]
  simp [hz]

theorem intermediate_bounds {z d b : Nat} (hz : z ≤ wordMax) (hd : d ≤ wordMax) (hb : b ≤ d) :
    8*z+1 < 2^128 ∧ d*(d+1)+b < 2^128 := by
  have h := Nat.mul_le_mul hd (Nat.succ_le_succ hd)
  dsimp [wordMax] at hz hd h
  omega

theorem decoded_components_fit {z : Nat} (hz : z ≤ wordMax) :
    (unpair z).1 ≤ wordMax ∧ (unpair z).2 ≤ wordMax := by
  obtain ⟨lo, hi⟩ := diagonal_interval z
  have d := le_triangle (diagonal z)
  simp only [triangle] at hi
  unfold unpair
  dsimp
  omega

theorem pair_injective {a b c d : Nat} (h : pair a b = pair c d) : (a,b) = (c,d) := by
  have eq := congrArg unpair h
  simpa only [unpair_pair] using eq

/-- The native list stores its length in the surrounding tag and requires
    zero residual payload at that exact length. -/
def encodeList : List Nat → Option Nat
  | [] => some 0
  | x :: xs => do return ← pairWord x (← encodeList xs)

def decodeList : Nat → Nat → Option (List Nat)
  | 0, z => if z = 0 then some [] else none
  | n+1, z => do return (unpair z).1 :: (← decodeList n (unpair z).2)

theorem list_roundtrip (xs : List Nat) {z : Nat} (encoded : encodeList xs = some z) :
    decodeList xs.length z = some xs := by
  induction xs generalizing z with
  | nil =>
    have h : z = 0 := by simpa [encodeList] using encoded.symm
    subst z; rfl
  | cons x xs ih =>
    cases ht : encodeList xs with
    | none => simp [encodeList, ht] at encoded
    | some tail =>
      have h : pairWord x tail = some z := by simpa [encodeList, ht] using encoded
      rw [pairWord_exact] at h
      split at h
      · have hz : pair x tail = z := Option.some.inj h
        subst z
        simp only [List.length_cons, decodeList, unpair_pair, ih ht]
        rfl
      · contradiction

theorem list_injective {xs ys : List Nat} {z : Nat}
    (sameLength : xs.length = ys.length)
    (hx : encodeList xs = some z) (hy : encodeList ys = some z) : xs = ys := by
  have h := list_roundtrip xs hx
  rw [sameLength, list_roundtrip ys hy] at h
  exact (Option.some.inj h).symm

inductive SyntaxTree where
  | node (symbol : Nat) (args : List SyntaxTree)

def encodeTree : Nat → SyntaxTree → Option Nat
  | 0, _ => none
  | depth+1, .node symbol args => do
    let tag ← pairWord symbol args.length
    let codes ← args.mapM (encodeTree depth)
    let payload ← encodeList codes
    pairWord tag payload

def decodeTree : Nat → Nat → Option SyntaxTree
  | 0, _ => none
  | depth+1, z => do
    let (tag, payload) := unpair z
    let (symbol, arity) := unpair tag
    let codes ← decodeList arity payload
    return .node symbol (← codes.mapM (decodeTree depth))

private theorem unpair_of_word {a b z : Nat} (h : pairWord a b = some z) : unpair z = (a,b) := by
  rw [pairWord_exact] at h
  split at h
  · cases h; exact unpair_pair a b
  · contradiction

private theorem map_roundtrip {X Y : Type} (enc : X → Option Y) (dec : Y → Option X)
    (xs : List X) (ys : List Y) (inverse : ∀ x ∈ xs, ∀ y, enc x = some y → dec y = some x)
    (h : xs.mapM enc = some ys) :
    ys.length = xs.length ∧ ys.mapM dec = some xs := by
  induction xs generalizing ys with
  | nil =>
    have eq : [] = ys := Option.some.inj h
    cases eq; exact ⟨rfl,rfl⟩
  | cons x xs ih =>
    rw [List.mapM_cons] at h
    obtain ⟨y, hy, rest⟩ := Option.bind_eq_some_iff.mp h
    obtain ⟨tail, ht, out⟩ := Option.bind_eq_some_iff.mp rest
    have eq : y :: tail = ys := Option.some.inj out
    cases eq
    obtain ⟨hlen, hd⟩ := ih tail (fun v hv => inverse v (List.mem_cons_of_mem _ hv)) ht
    exact ⟨congrArg Nat.succ hlen, by simp [List.mapM_cons, inverse x (by simp) y hy, hd]⟩

/-- The word codec returns the original symbol, arity and every ordered subtree
    for every successful encoding at every finite depth. -/
theorem tree_roundtrip (depth : Nat) (t : SyntaxTree) (z : Nat)
    (h : encodeTree depth t = some z) : decodeTree depth z = some t := by
  induction depth generalizing t z with
  | zero => simp [encodeTree] at h
  | succ depth ih =>
    cases t with
    | node symbol args =>
      obtain ⟨tag, htag, rest⟩ := Option.bind_eq_some_iff.mp h
      obtain ⟨codes, hc, rest⟩ := Option.bind_eq_some_iff.mp rest
      obtain ⟨payload, hp, hcode⟩ := Option.bind_eq_some_iff.mp rest
      obtain ⟨hlen, hchildren⟩ := map_roundtrip _ _ args codes (fun x _ => ih x) hc
      have hlist := list_roundtrip codes hp
      rw [hlen] at hlist
      simp [decodeTree, unpair_of_word hcode, unpair_of_word htag, hlist, hchildren]

private theorem decoded_list {n z : Nat} {xs : List Nat} (hz : z ≤ wordMax)
    (h : decodeList n z = some xs) :
    xs.length = n ∧ (∀ x ∈ xs, x ≤ wordMax) ∧ encodeList xs = some z := by
  induction n generalizing z xs with
  | zero =>
    simp only [decodeList] at h
    split at h
    · rename_i hzero
      subst z
      cases h
      simp [encodeList]
    · contradiction
  | succ n ih =>
    obtain ⟨ys, ht, hout⟩ := Option.bind_eq_some_iff.mp h
    have eq : (unpair z).1 :: ys = xs := Option.some.inj hout
    cases eq
    obtain ⟨ha,hb⟩ := decoded_components_fit hz
    obtain ⟨hlen, hsmall, henc⟩ := ih hb ht
    refine ⟨congrArg Nat.succ hlen, ?_, ?_⟩
    · intro x hx
      rcases List.mem_cons.mp hx with rfl | inTail
      · exact ha
      · exact hsmall x inTail
    · simp [encodeList, henc, word_roundtrip hz]

/-- Every accepted syntax code is canonical, including all nested arity tags
    and the residual payload after the last child. -/
theorem decoded_tree_reencodes (depth z : Nat) (t : SyntaxTree) (hz : z ≤ wordMax)
    (h : decodeTree depth z = some t) : encodeTree depth t = some z := by
  induction depth generalizing z t with
  | zero => simp [decodeTree] at h
  | succ depth ih =>
    obtain ⟨codes, hc, rest⟩ := Option.bind_eq_some_iff.mp h
    obtain ⟨args, ha, hout⟩ := Option.bind_eq_some_iff.mp rest
    have eq : .node (unpair (unpair z).1).1 args = t := Option.some.inj hout
    cases eq
    obtain ⟨hTagSmall, hPayloadSmall⟩ := decoded_components_fit hz
    obtain ⟨hcodeLen, hsmall, hp⟩ := decoded_list hPayloadSmall hc
    change codes.length = (unpair (unpair z).1).2 at hcodeLen
    obtain ⟨hargLen, hchildren⟩ := map_roundtrip _ _ codes args (fun x hx t ht => ih x t (hsmall x hx) ht) ha
    simp [encodeTree, hargLen, hcodeLen, word_roundtrip hTagSmall, hchildren, hp, word_roundtrip hz]

private theorem map_preserves_results {X Y : Type} (f g : X → Option Y)
    (preserves : ∀ x y, f x = some y → g x = some y) (xs : List X) (ys : List Y)
    (h : xs.mapM f = some ys) : xs.mapM g = some ys := by
  induction xs generalizing ys with
  | nil => exact h
  | cons x xs ih =>
    rw [List.mapM_cons] at h
    obtain ⟨y, hy, rest⟩ := Option.bind_eq_some_iff.mp h
    obtain ⟨tail, ht, out⟩ := Option.bind_eq_some_iff.mp rest
    have eq : y :: tail = ys := Option.some.inj out
    cases eq
    simp [List.mapM_cons, preserves x y hy, ih tail ht]

theorem encodeTree_extends {small large : Nat} (hle : small ≤ large) (t : SyntaxTree) (z : Nat)
    (h : encodeTree small t = some z) : encodeTree large t = some z := by
  induction small generalizing large t z with
  | zero => simp [encodeTree] at h
  | succ small ih =>
    cases large with
    | zero => cases hle
    | succ large =>
      cases t with
      | node s args =>
        obtain ⟨tag, htag, rest⟩ := Option.bind_eq_some_iff.mp h
        obtain ⟨codes, hc, rest⟩ := Option.bind_eq_some_iff.mp rest
        have lifted := map_preserves_results _ _ (ih (Nat.le_of_succ_le_succ hle)) args codes hc
        simpa [encodeTree, htag, lifted] using rest

theorem tree_injective {d e : Nat} (x y : SyntaxTree) (z : Nat)
    (hx : encodeTree d x = some z) (hy : encodeTree e y = some z) : x = y := by
  have common := tree_roundtrip (max d e) x z (encodeTree_extends (Nat.le_max_left d e) x z hx)
  have other := tree_roundtrip (max d e) y z (encodeTree_extends (Nat.le_max_right d e) y z hy)
  exact Option.some.inj (common.symm.trans other)

#print axioms tree_roundtrip
#print axioms decoded_tree_reencodes
#print axioms tree_injective
#print axioms unpair_pair
#print axioms pair_unpair
#print axioms pairWord_exact
#print axioms word_roundtrip
#print axioms intermediate_bounds
#print axioms decoded_components_fit
#print axioms list_roundtrip
#print axioms list_injective
end Verum.GodelCantor
