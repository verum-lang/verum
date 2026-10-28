import Lean

/-! Executable exact arithmetic for finite categorical-score replay.
Labels index a declared fixed alphabet; child identifiers denote full reading
values within one context. No support is inferred from observed labels.
This computes ratios, not a statistical guarantee or an allocation/runtime bound. -/
namespace CategoricalScore
set_option warningAsError true

structure Ratio where
  numerator : Nat
  denominator : Nat
  deriving DecidableEq

def one : Ratio := ⟨1, 1⟩
def mul (a b : Ratio) : Ratio := ⟨a.numerator*b.numerator, a.denominator*b.denominator⟩
def div (a b : Ratio) : Ratio := ⟨a.numerator*b.denominator, a.denominator*b.numerator⟩
def equivalent (a b : Ratio) : Bool := a.denominator != 0 && b.denominator != 0 &&
  a.numerator*b.denominator == b.numerator*a.denominator

def rising (base : Nat) : Nat → Nat
  | 0 => 1
  | n+1 => rising base n * (base+2*n)

def product (ns : List Nat) : Nat := ns.foldr (· * ·) 1

def kt (alphabet : Nat) (counts : List Nat) : Ratio :=
  ⟨product (counts.map (rising 1)), rising alphabet counts.sum⟩

def parentML (counts : List Nat) : Ratio :=
  ⟨product (counts.map (fun n => n^n)), counts.sum^counts.sum⟩

structure Row where
  reading : Nat
  label : Nat
  deriving DecidableEq

structure Context where
  alphabet : Nat
  rows : List Row
  deriving DecidableEq

def counts (K : Nat) (rows : List Row) : List Nat :=
  (List.range K).map (fun label => (rows.filter (fun r => r.label == label)).length)

def children (c : Context) : List Ratio :=
  (c.rows.map Row.reading).eraseDups.map (fun key =>
    kt c.alphabet (counts c.alphabet (c.rows.filter (fun r => r.reading == key))))

structure Score where
  compression : Ratio
  predictive : Ratio
  samples : Nat
  deriving DecidableEq

def contextScore (c : Context) : Score :=
  let child := (children c).foldr mul one
  let parent := counts c.alphabet c.rows
  ⟨div child (kt c.alphabet parent), div child (parentML parent), c.rows.length⟩

def score (contexts : List Context) : Option Score :=
  if contexts.all (fun c => c.alphabet > 0 && c.rows.all (fun r => r.label < c.alphabet)) then
    some (contexts.foldr (fun c acc =>
      let s := contextScore c
      ⟨mul s.compression acc.compression, mul s.predictive acc.predictive, s.samples+acc.samples⟩)
      ⟨one, one, 0⟩)
  else none

def agrees (contexts : List Context) (expected : Score) : Bool :=
  match score contexts with
  | none => false
  | some actual => equivalent actual.compression expected.compression &&
    equivalent actual.predictive expected.predictive && actual.samples == expected.samples

/-- The strict native family thresholds; denominator-zero evidence is refused. -/
def purchased (s : Score) (selection risk : Nat) : Bool :=
  selection > 0 && risk > 0 && s.compression.denominator > 0 && s.predictive.denominator > 0 &&
    s.compression.numerator > selection*s.compression.denominator &&
    s.predictive.numerator > selection*risk*s.predictive.denominator

/-- Same-family/fresh-history premises are checked separately by the adapter.
The fresh-only score is not the endpoint of this conditional comparison. -/
def confirmed (training joint : Score) (selection risk : Nat) : Bool :=
  training.compression.denominator > 0 && joint.compression.denominator > 0 &&
    joint.compression.numerator*training.compression.denominator >
      training.compression.numerator*joint.compression.denominator && purchased joint selection risk

end CategoricalScore
