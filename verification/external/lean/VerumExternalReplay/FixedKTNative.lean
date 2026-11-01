import Std.Tactic

/-! Exact integer-product interpretation for frozen scorer observations.
The portable Verum certificates prove the arbitrary-partition maximum.
This module also checks the zero-origin denominator traversal, and evaluates
closed native exports. It does not give Rust, Clone, Ord or BigUint semantics.
-/
namespace FixedKTNative
set_option warningAsError true

def rise2 (a : Nat) : Nat → Nat
  | 0 => 1
  | n + 1 => rise2 a n * (a + 2*n)

def shifted (a start : Nat) : Nat → Nat
  | 0 => 1
  | n + 1 => shifted a start n * (a + 2*(start+n))

theorem factor_range (a start n : Nat) :
    rise2 a start * shifted a start n = rise2 a (start+n) := by
  induction n with
  | zero => simp [shifted]
  | succ n ih =>
    simp only [shifted, ← Nat.mul_assoc, ih, rise2]
    rfl

def counterDen (K : Nat) : List Nat → Nat → Nat
  | [], _ => 1
  | n :: ns, processed => shifted K processed n * counterDen K ns (processed+n)

theorem counter_range (K : Nat) (counts : List Nat) (processed : Nat) :
    rise2 K processed * counterDen K counts processed = rise2 K (processed+counts.sum) := by
  induction counts generalizing processed with
  | nil => simp [counterDen]
  | cons n ns ih =>
    simp only [counterDen, ← Nat.mul_assoc, factor_range, ih, List.sum_cons, Nat.add_assoc]

theorem counter_zero_origin (K : Nat) (counts : List Nat) :
    counterDen K counts 0 = rise2 K counts.sum := by
  simpa only [rise2, Nat.one_mul, Nat.zero_add] using counter_range K counts 0

structure Fraction where
  numerator : Nat
  denominator : Nat
  deriving DecidableEq

def Fraction.positive (a : Fraction) : Prop := 0 < a.numerator ∧ 0 < a.denominator
instance (a : Fraction) : Decidable a.positive := inferInstanceAs (Decidable (_ ∧ _))
def Fraction.same (a b : Fraction) : Prop :=
  a.positive ∧ b.positive ∧ a.numerator * b.denominator = b.numerator * a.denominator
instance (a b : Fraction) : Decidable (a.same b) := inferInstanceAs (Decidable (_ ∧ _))
def Fraction.le (a b : Fraction) : Prop :=
  a.positive ∧ b.positive ∧ a.numerator * b.denominator ≤ b.numerator * a.denominator
instance (a b : Fraction) : Decidable (a.le b) := inferInstanceAs (Decidable (_ ∧ _))
def Fraction.mul (a b : Fraction) : Fraction :=
  ⟨a.numerator*b.numerator, a.denominator*b.denominator⟩
def Fraction.div (a b : Fraction) : Fraction :=
  ⟨a.numerator*b.denominator, a.denominator*b.numerator⟩
def one : Fraction := ⟨1, 1⟩
def product : List Fraction → Fraction
  | [] => one
  | x :: xs => x.mul (product xs)

def cell (K : Nat) (counts : List Nat) : Fraction :=
  ⟨(counts.map (rise2 1)).prod, rise2 K counts.sum⟩
def pure (K n : Nat) : Fraction := ⟨rise2 1 n, rise2 K n⟩
def envelope (K : Nat) (counts : List Nat) : Fraction := product (counts.map (pure K))
def parentML (counts : List Nat) : Fraction :=
  ⟨(counts.map (fun n => n^n)).prod, counts.sum^counts.sum⟩

structure Context where
  width : Nat
  parent : List Nat
  children : List (List Nat)
  deriving DecidableEq

def childScore (contexts : List Context) : Fraction :=
  product (contexts.map (fun c => product (c.children.map (cell c.width))))
def parentCode (contexts : List Context) : Fraction :=
  product (contexts.map (fun c => cell c.width c.parent))
def parentLikelihood (contexts : List Context) : Fraction :=
  product (contexts.map (fun c => parentML c.parent))
def ceiling (contexts : List Context) : Fraction :=
  product (contexts.map (fun c => envelope c.width c.parent))

structure Row where
  ordinal : Nat
  context : Nat
  cell : Nat
  outcome : Nat
  cut : Nat
  deriving DecidableEq

def modifyAt? (f : A → Option A) : List A → Nat → Option (List A)
  | [], _ => none
  | a :: as, 0 => (f a).map (· :: as)
  | a :: as, i+1 => (modifyAt? f as i).map (a :: ·)

def ingest (contexts : List Context) (row : Row) : Option (List Context) :=
  modifyAt? (fun c => do
    let parent ← modifyAt? (fun n => some (n+1)) c.parent row.outcome
    let children ← modifyAt? (fun counts =>
      modifyAt? (fun n => some (n+1)) counts row.outcome) c.children row.cell
    some ⟨c.width, parent, children⟩) contexts row.context

def intakeAgrees (contexts : List Context) (rows : List Row) : Bool :=
  let empty := contexts.map fun c =>
    Context.mk c.width (List.replicate c.width 0)
      (List.replicate c.children.length (List.replicate c.width 0))
  contexts.all (fun c => c.width > 0) && rows.foldlM ingest empty == some contexts

def rowIdentity (rows : List Row) : List (Nat × Nat × Nat × Nat) :=
  rows.map fun r => (r.ordinal, r.context, r.outcome, r.cut)

end FixedKTNative
