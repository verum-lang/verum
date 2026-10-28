import VerumExternalReplay.PositionPrediction

/-! Exact completed-row replay for paired residual and raw KT frontiers.
The mask and complete context bytes are fixed before a body. The two banks
are retained across rows. The common positive geometric length factor cancels
by ExactMass.divide_common_scale; it is not omitted from only one law.
Native VM execution, physical causality and private partial states are separate. -/
namespace ResidualPrediction
open ExactMass PositionPrediction
set_option warningAsError true

structure Row where
  context : List Nat
  mask : List Nat
  target : List Nat
  deriving DecidableEq

structure State where
  samples : Nat
  bits : Nat
  ratio : Fraction
  residual : Bank (List Nat × Nat)
  reference : Bank Nat
  deriving DecidableEq

def initial : State := ⟨0, 0, one, [], []⟩

def byteBits (byte : Nat) : List Bool := (List.range 8).map byte.testBit
def wireBits (bytes : List Nat) : List Bool := bytes.flatMap byteBits

def step (s : State) (context : List Nat) (position : Nat) (predicted target : Bool) : State :=
  let child := lookup s.residual (context, position)
  let reference := lookup s.reference position
  let residual := Bool.xor predicted target
  let q := BinaryPrediction.kernel.probability child residual
  let p := BinaryPrediction.kernel.probability reference target
  { s with
    ratio := mul s.ratio (divide q p (BinaryPrediction.probability_positive reference target))
    residual := observe s.residual (context, position) residual
    reference := observe s.reference position target }

def body (s : State) (context : List Nat) (position : Nat) (mask : List Bool) : List Bool → State
  | [] => s
  | bit :: rest => body (step s context position (mask.headD false) bit) context
      (position+1) mask.tail rest

def row (s : State) (r : Row) : State :=
  let result := body s r.context 0 (wireBits r.mask) (wireBits r.target)
  { result with samples := s.samples+1, bits := s.bits+8*r.target.length }

def fold (rows : List Row) (s : State) : State := rows.foldl row s

/-- Snapshots enumerate every allocated cell in first-use order. Native storage
may use a different order; the adapter must retain the exact key/count mapping. -/
structure Snapshot where
  samples : Nat
  bits : Nat
  ratio : Fraction
  residual : List ((List Nat × Nat) × List Nat)
  reference : List (Nat × List Nat)

def agrees (s : State) (snapshot : Snapshot) : Bool :=
  s.samples == snapshot.samples && s.bits == snapshot.bits && decide (Equal s.ratio snapshot.ratio) &&
  s.residual.map (fun (key, count) => (key, count.val.counts)) == snapshot.residual &&
  s.reference.map (fun (key, count) => (key, count.val.counts)) == snapshot.reference

/-- Consume the complete stream exactly once. Repeated frontiers are allowed
for adoption; neither a fresh boundary nor a live commit resets either bank. -/
def replay (rows : List Row) (s : State) : List Snapshot → Bool
  | [] => rows.isEmpty
  | snapshot :: rest =>
    if snapshot.samples < s.samples then false else
    let count := snapshot.samples-s.samples
    let next := fold (rows.take count) s
    agrees next snapshot && replay (rows.drop count) next rest

/-- The residual cell schedule is injective while the current context is fixed. -/
theorem context_positions_injective (context : List Nat) :
    Function.Injective (fun position : Nat => (context, position)) := by
  intro a b same
  exact (Prod.mk.inj same).2

/-- Read-only forecasts and the incremental scorer use identical factors for
one complete body's residual bank, because positions do not reuse cells. -/
theorem residual_frozen (bank : Bank (List Nat × Nat)) (context : List Nat)
    (position : Nat) (bits : List Bool) :
    CausalCoding.likelihood (kernel (fun p => (context, p))) ⟨position, bank⟩ bits =
      frozen (fun p => (context, p)) bank position bits :=
  incremental_eq_frozen _ (context_positions_injective context) _ _ _

theorem reference_frozen (bank : Bank Nat) (position : Nat) (bits : List Bool) :
    CausalCoding.likelihood (kernel id) ⟨position, bank⟩ bits = frozen id bank position bits :=
  incremental_eq_frozen _ (fun _ _ h => h) _ _ _

end ResidualPrediction
