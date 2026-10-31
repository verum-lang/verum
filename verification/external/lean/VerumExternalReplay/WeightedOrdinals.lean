import Std.Tactic

/-! Ordinals for ordered sums of finite products. Exact cardinalities describe
the original catalogue, including alternatives not selected by a priority view.
An empty child product is one; a zero child cardinality removes its whole block.
Ranks use arithmetic, never enumeration of preceding addresses. Native factor
ownership, cardinality computation, lexical order and encoding correspondence
are separate obligations; capped lower bounds are not exact factors.
-/
namespace WeightedOrdinals
set_option warningAsError true

structure Alternative where
  arity : Nat
  children : List Nat
  deriving DecidableEq

def Alternative.width (pool : Nat) (choice : Alternative) : Nat :=
  pool ^ choice.arity * choice.children.prod

def radix (pool : Nat) (choices : List Alternative) : Nat :=
  (choices.map (Alternative.width pool)).sum

theorem flat_width (pool arity : Nat) : Alternative.width pool ⟨arity, []⟩ = pool ^ arity := by
  simp [Alternative.width]

/-- A position in an ordered sum. Zero-size blocks have no inhabitant. -/
inductive Choice : List Nat → Type where
  | here {size rest} : Fin size → Choice (size :: rest)
  | there {size rest} : Choice rest → Choice (size :: rest)

def Choice.rank : Choice sizes → Nat
  | .here offset => offset.val
  | .there (size := size) choice => size + choice.rank

def Choice.skip (before : List Nat) (choice : Choice sizes) : Choice (before ++ sizes) :=
  match before with
  | [] => choice
  | _ :: rest => .there (choice.skip rest)

theorem Choice.rank_skip (before : List Nat) (choice : Choice sizes) :
    (choice.skip before).rank = before.sum + choice.rank := by
  induction before with
  | nil => simp [skip]
  | cons size rest ih => simp [skip, rank, ih, Nat.add_assoc]

theorem Choice.rank_bound (choice : Choice sizes) : choice.rank < sizes.sum := by
  induction choice with
  | here offset => simpa only [rank, List.sum_cons] using
      Nat.lt_of_lt_of_le offset.isLt (Nat.le_add_right _ _)
  | there choice ih => simpa only [rank, List.sum_cons] using Nat.add_lt_add_left ih _

theorem Choice.rank_injective (left right : Choice sizes) (same : left.rank = right.rank) :
    left = right := by
  induction left with
  | here offset =>
      cases right with
      | here other => exact congrArg Choice.here (Fin.ext same)
      | there other => have bound := offset.isLt; simp only [rank] at same; exfalso; omega
  | there left ih =>
      cases right with
      | here offset => have bound := offset.isLt; simp only [rank] at same; exfalso; omega
      | there right => exact congrArg Choice.there (ih right (Nat.add_left_cancel same))

theorem choice_complete (sizes : List Nat) (index : Nat) (bound : index < sizes.sum) :
    ∃ choice : Choice sizes, choice.rank = index := by
  induction sizes generalizing index with
  | nil => simp at bound
  | cons size rest ih =>
      by_cases here : index < size
      · exact ⟨.here ⟨index, here⟩, rfl⟩
      · have tail : index - size < rest.sum := by simp only [List.sum_cons] at bound; omega
        obtain ⟨choice, exact⟩ := ih (index - size) tail
        exact ⟨.there choice, by simp only [Choice.rank, exact]; omega⟩

/-- Positions in a fixed Cartesian product; ill-shaped or out-of-range words
are unrepresentable. Radices may be zero, in which case there is no word. -/
inductive Digits : List Nat → Type where
  | nil : Digits []
  | cons {radix rest} : Fin radix → Digits rest → Digits (radix :: rest)

def Digits.rank : Digits radices → Nat
  | .nil => 0
  | .cons (rest := rest) digit tail => digit.val * rest.prod + tail.rank

def Digits.append (left : Digits first) (right : Digits second) : Digits (first ++ second) :=
  match left with
  | .nil => right
  | .cons digit tail => .cons digit (tail.append right)

theorem Digits.rank_append (left : Digits first) (right : Digits second) :
    (left.append right).rank = left.rank * second.prod + right.rank := by
  induction left with
  | nil => simp [append, rank]
  | cons digit tail ih => simp [append, rank, List.prod_append, ih, Nat.add_mul, Nat.mul_assoc, Nat.add_assoc]

/-- The native left-to-right recurrence follows from Cartesian rank, rather
than being justified by running the preceding enumeration. -/
theorem Digits.rank_step (word : Digits radices) {base : Nat} (digit : Fin base) :
    (word.append (.cons digit .nil)).rank = word.rank * base + digit.val := by
  simpa [rank] using word.rank_append (.cons digit .nil)

theorem Digits.rank_bound (word : Digits radices) : word.rank < radices.prod := by
  induction word with
  | nil => simp [rank]
  | @cons base rest digit tail ih =>
      have row := Nat.mul_le_mul_right rest.prod (Nat.succ_le_of_lt digit.isLt)
      simp only [Nat.succ_mul] at row
      simp only [rank, List.prod_cons]
      omega

private theorem quotient (digit suffix width : Nat) (bound : suffix < width) :
    (digit * width + suffix) / width = digit := by
  rw [Nat.add_comm, Nat.add_mul_div_right _ _ (by omega : 0 < width), Nat.div_eq_of_lt bound]
  omega

theorem Digits.rank_injective (left right : Digits radices) (same : left.rank = right.rank) :
    left = right := by
  induction left with
  | nil => cases right; rfl
  | @cons base rest digit tail ih =>
      cases right with
      | cons other restDigits =>
          have digits := congrArg (fun n => n / rest.prod) same
          simp only [rank, quotient _ _ _ tail.rank_bound,
            quotient _ _ _ restDigits.rank_bound] at digits
          have equal : digit = other := Fin.ext digits
          subst other
          have tails : tail.rank = restDigits.rank := by simp only [rank] at same; omega
          exact congrArg (Digits.cons digit) (ih restDigits tails)

theorem digits_complete (radices : List Nat) (index : Nat) (bound : index < radices.prod) :
    ∃ word : Digits radices, word.rank = index := by
  induction radices generalizing index with
  | nil => exact ⟨.nil, by simp only [List.prod_nil] at bound; simp [Digits.rank]; omega⟩
  | cons radix rest ih =>
      have positive : 0 < rest.prod := by
        by_cases zero : rest.prod = 0
        · simp [zero] at bound
        · omega
      have head : index / rest.prod < radix :=
        (Nat.div_lt_iff_lt_mul positive).mpr bound
      obtain ⟨tail, exact⟩ := ih (index % rest.prod) (Nat.mod_lt _ positive)
      exact ⟨.cons ⟨index / rest.prod, head⟩ tail, by
        simp only [Digits.rank, exact]
        exact Nat.div_add_mod' _ _⟩

/-- Prefix sizes of disjoint length blocks. For structural bodies, supply
count n = n * (the first n instruction radices).prod; n counts output choices. -/
def prefixSum (count : Nat → Nat) : Nat → Nat
  | 0 => 0
  | n + 1 => prefixSum count n + count n

theorem prefix_monotone (count : Nat → Nat) (a b : Nat) (order : a ≤ b) :
    prefixSum count a ≤ prefixSum count b := by
  induction b with
  | zero =>
      have : a = 0 := by omega
      subst a
      exact Nat.le_refl _
  | succ b ih =>
      by_cases same : a = b + 1
      · subst a; exact Nat.le_refl _
      · have := ih (by omega)
        simp only [prefixSum]
        omega

theorem length_blocks_disjoint (count : Nat → Nat) (a b x y : Nat)
    (order : a < b) (inside : x < count a) :
    prefixSum count a + x < prefixSum count b + y := by
  have := prefix_monotone count (a + 1) b (by omega)
  simp only [prefixSum] at this
  omega

def addressRank (shorter : Nat) (word : Digits radices) (output : Fin radices.length) : Nat :=
  shorter + word.rank * radices.length + output.val

theorem address_interval (shorter : Nat) (word : Digits radices) (output : Fin radices.length) :
    shorter ≤ addressRank shorter word output ∧
      addressRank shorter word output < shorter + radices.prod * radices.length := by
  have row := Nat.mul_le_mul_right radices.length (Nat.succ_le_of_lt word.rank_bound)
  have out := output.isLt
  simp only [Nat.succ_mul] at row
  unfold addressRank
  constructor <;> omega

theorem address_injective (shorter : Nat) (left right : Digits radices)
    (first second : Fin radices.length)
    (same : addressRank shorter left first = addressRank shorter right second) :
    left = right ∧ first = second := by
  have bodies := congrArg (fun n => (n - shorter) / radices.length) same
  simp only [addressRank, Nat.add_assoc, Nat.add_sub_cancel_left,
    quotient _ _ _ first.isLt, quotient _ _ _ second.isLt] at bodies
  have words := left.rank_injective right bodies
  subst right
  have digits : first.val = second.val := by unfold addressRank at same; omega
  exact ⟨rfl, Fin.ext digits⟩

end WeightedOrdinals
