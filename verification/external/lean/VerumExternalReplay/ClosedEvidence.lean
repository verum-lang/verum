import VerumExternalReplay.FixedKTNative
import VerumExternalReplay.PrefixSyntax

/-! Exact frozen-prefix diagnostics. Evaluating all prefixes of one captured
history does not prove the empirical ML bound for arbitrary histograms or the
physical permanence of a native ClosedSupport transition. -/
namespace ClosedEvidence
open FixedKTNative
set_option warningAsError true

structure Declaration where
  context : List Nat
  outcomes : List String
  deriving DecidableEq
structure Observation where
  context : List Nat
  outcome : String
  ordinal : Nat
  cut : Nat
  deriving DecidableEq

def valid (ds : List Declaration) : Bool :=
  (ds.map Declaration.context).eraseDups.length == ds.length &&
  ds.all (fun d => !d.outcomes.isEmpty && d.outcomes.eraseDups.length == d.outcomes.length)

def ingestOutcome (ds : List Declaration) (counts : List (List Nat))
    (key : List Nat) (label : String) :
    Option (List (List Nat)) := do
  let context ← ds.findIdx? (fun d => d.context == key)
  let declaration ← ds[context]?
  let outcome ← declaration.outcomes.findIdx? (· == label)
  modifyAt? (modifyAt? (fun n => some (n+1)) · outcome) counts context

def ingest (ds : List Declaration) (counts : List (List Nat)) (row : Observation) :=
  ingestOutcome ds counts row.context row.outcome

def histogram (ds : List Declaration) (rows : List Observation) : Option (List (List Nat)) :=
  if valid ds then rows.foldlM (ingest ds) (ds.map (fun d => List.replicate d.outcomes.length 0))
  else none

def likelihood (counts : List (List Nat)) : Fraction := product (counts.map parentML)

def prefixLowerBound (ds : List Declaration) (rows : List Observation) (K n : Nat) : Bool :=
  match histogram ds (rows.take n) with
  | none => false
  | some counts => decide ((Fraction.mk 1 (K^n)).le (likelihood counts))

/-- Catalog header floor; a missing catalog cannot supply a zero-cost header. -/
def headerBits (catalog : List PrefixSyntax.Grammar) (owners : List Nat) : Option Nat := do
  let widths ← owners.mapM fun owner => do
    let grammar ← PrefixSyntax.lookup catalog owner
    return 1 + PrefixSyntax.width (PrefixSyntax.alternatives grammar)
  return widths.sum

end ClosedEvidence
