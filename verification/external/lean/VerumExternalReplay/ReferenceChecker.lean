/-! ============================================================== -/
/-! # Verum reference kernel checker                                 -/
/-! ============================================================== -/

/-! Independent re-implementation of the structural fragment of
    `crates/verum_kernel/src/proof_checker.rs` (Var / Universe /
    Pi / Lam / App, plus β-reduction, α-equivalence, η-equivalence,
    universe-overflow handling, and post-FV-19 universe
    polymorphism).

    This is the **Lean side of the differential checker**: a
    decidable `Kernel.check : Ctx → Term → Term → Except Error Unit`
    that the audit harness runs on the same `Certificate` battery
    the Rust kernel verifies, so any disagreement is a real bug
    in *one* of the two implementations.

    The Verum-side equivalent is `verum_kernel::proof_checker`;
    see `docs/architecture/verum-kernel-audit-2026.md` for the
    audit ledger this file is the load-bearing artefact for.

    **Universe polymorphism (FV-19)**: `Universe` carries a
    structured `Level` (Concrete / Var / Succ / Max).  The Lean
    side uses `Nat` for the concrete carrier (unbounded — overflow
    is reported information-only); the Rust side uses `u32` and
    rejects `u32::MAX + 1`.  Equality is decided up to algebraic
    normalisation (idempotency, identity at zero, common-succ
    factoring, lexicographic flattening of `Max`). -/

namespace VerumKernel

/-- A universe level — concrete number, level variable, or
    expression (`succ`, `max`) over them.  Mirrors the Rust
    `Level` enum at `proof_checker::Level`. -/
inductive Level : Type where
  | concrete (n : Nat)
  | var      (name : String)
  | succ     (l : Level)
  | max      (a b : Level)
  deriving DecidableEq, Inhabited, Repr

/-- Minimal CoC `Term` — mirrors `proof_checker::Term`. -/
inductive Term : Type where
  | var      (i : Nat)
  | universe (lvl : Level)
  | pi       (dom : Term) (body : Term)
  | lam      (dom : Term) (body : Term)
  | app      (f : Term) (x : Term)
  deriving DecidableEq, Inhabited, Repr

/-- Type-checking error — mirrors `proof_checker::CheckError` for
    the structural fragment. -/
inductive CheckError : Type where
  | unbound_variable     (idx : Nat)
  | not_a_type           (t : Term)
  | not_a_function       (t : Term)
  | domain_mismatch      (expected actual : Term)
  | type_mismatch        (expected actual : Term)
  | universe_overflow    (level : Nat)
  | claimed_type_not_a_type (claimed actual : Term)
  | fuel_exhausted
  deriving Repr

/-- Context = inner-first stack of binder types, identical to
    `proof_checker::Context`. -/
abbrev Ctx := List Term

-- =============================================================================
-- Level normalisation — mirrors `proof_checker::Level::normalize`
-- =============================================================================

/-- Convenience: build the `Concrete(0)` level. -/
def Level.zero : Level := Level.concrete 0

/-- Build `succ(l)`, applying the algebraic rule `succ(C(n)) = C(n+1)`
    when the inner level is concrete. -/
partial def Level.succ' (l : Level) : Level :=
  match l with
  | .concrete n => .concrete (n + 1)
  | other       => .succ other

/-- Lexicographic structural ordering on canonical Levels.  Used by
    [`Level.maxCanonical`] to produce a deterministic Max-summand
    order so commutative variants compare equal. -/
partial def Level.lexCompare (a b : Level) : Ordering :=
  let rank : Level → Nat
    | .concrete _ => 0
    | .var _      => 1
    | .succ _     => 2
    | .max _ _    => 3
  match Nat.compare (rank a) (rank b) with
  | .eq =>
    match a, b with
    | .concrete x, .concrete y => Nat.compare x y
    | .var x,      .var y      => compare x y
    | .succ x,     .succ y     => Level.lexCompare x y
    | .max xa xb,  .max ya yb  =>
      match Level.lexCompare xa ya with
      | .eq    => Level.lexCompare xb yb
      | other  => other
    | _, _ => .eq
  | other => other

/-- Number of `succ` constructors at the head of a level. -/
partial def Level.succDepth : Level → Nat
  | .succ inner => 1 + Level.succDepth inner
  | _           => 0

/-- Strip up to `count` leading `succ` constructors. -/
partial def Level.stripSucc (l : Level) (count : Nat) : Level :=
  if count = 0 then l
  else match l with
    | .succ inner => Level.stripSucc inner (count - 1)
    | other       => other

/-- Flatten a (possibly-nested) `max` into a flat list of summands. -/
partial def Level.flattenMax (l : Level) : List Level :=
  match l with
  | .max a b => Level.flattenMax a ++ Level.flattenMax b
  | other    => [other]

/-- Sort + dedupe summands using structural ordering.  Implements the
    canonical-form invariant for `max`. -/
partial def Level.sortDedupe (xs : List Level) : List Level :=
  let sorted := xs.toArray.qsort (fun a b => Level.lexCompare a b == .ord.lt) |>.toList
  -- Dedupe consecutive equals (we just sorted, so equals cluster).
  sorted.foldr (fun x acc =>
    match acc with
    | []      => [x]
    | h :: _  => if x == h then acc else x :: acc) []

/-- Normalise a level to canonical form.  Idempotent. -/
partial def Level.normalize (l : Level) : Level :=
  match l with
  | .concrete _  => l
  | .var _       => l
  | .succ inner =>
    let n := Level.normalize inner
    match n with
    | .concrete k => .concrete (k + 1)
    | other       => .succ other
  | .max a b =>
    let aN := Level.normalize a
    let bN := Level.normalize b
    Level.maxCanonical aN bN

/-- Build the canonical form of `max(a, b)` assuming both are
    already individually normalised. -/
partial def Level.maxCanonical (a b : Level) : Level :=
  let summands := Level.flattenMax a ++ Level.flattenMax b
  -- Split into concrete / symbolic.
  let (concretes, symbolics) := summands.partition (fun s =>
    match s with | .concrete _ => true | _ => false)
  let concreteMax : Option Nat :=
    concretes.foldl (fun acc s =>
      match s, acc with
      | .concrete n, none      => some n
      | .concrete n, some prev => some (Nat.max prev n)
      | _,           acc       => acc) none
  let symbolicSorted := Level.sortDedupe symbolics
  let totalCount := symbolicSorted.length + (if concreteMax.isSome then 1 else 0)
  if totalCount ≥ 2 then
    -- Try to factor a common `succ` prefix.
    let depthList : List Nat :=
      symbolicSorted.map Level.succDepth
        ++ (match concreteMax with | some n => [n] | none => [])
    let common := depthList.foldl Nat.min (depthList.headD 0)
    if common > 0 then
      let strippedConcrete := concreteMax.map (fun n => n - common)
      let strippedSymbolic := symbolicSorted.map (fun s => Level.stripSucc s common)
      let inner := Level.assembleMax strippedConcrete strippedSymbolic
      let mut acc := inner
      for _ in [0:common] do
        acc := .succ acc
      acc
    else
      Level.assembleMax concreteMax symbolicSorted
  else
    Level.assembleMax concreteMax symbolicSorted

/-- Reassemble a `max` from a concrete summand (if any) and a list
    of symbolic summands. -/
partial def Level.assembleMax (concrete : Option Nat) (symbolic : List Level) : Level :=
  -- Drop a concrete-zero summand when at least one symbolic summand exists
  -- (`max(0, x) = x`).
  let concreteToKeep :=
    match concrete with
    | some 0 => if symbolic.isEmpty then some 0 else none
    | other  => other
  let all : List Level :=
    (match concreteToKeep with
     | some n => [.concrete n]
     | none   => []) ++ symbolic
  match all with
  | []      => .concrete 0
  | [x]     => x
  | x :: rest => rest.foldr (fun y acc => .max y acc) x

/-- Definitional equality on levels — normalise both sides and
    compare structurally. -/
def Level.eq (a b : Level) : Bool :=
  Level.normalize a == Level.normalize b

/-- The successor with concrete-overflow tracking.  In Lean `Nat` is
    unbounded so this never reports overflow; the kernel-level
    decision is made on the Rust side via `Level::checked_succ`. -/
partial def Level.checkedSucc (l : Level) : Level := Level.succ' l

/-- Lift a level by `by` levels — `concrete` adds, symbolic wraps in
    `by` `succ`s.  Mirrors `Level::shifted_by` on the Rust side. -/
partial def Level.shiftedBy (l : Level) (by_ : Nat) : Level :=
  if by_ = 0 then l
  else match l with
    | .concrete n => .concrete (n + by_)
    | other =>
      let mut acc := other
      for _ in [0:by_] do
        acc := .succ acc
      Level.normalize acc

-- =============================================================================
-- Term-level operations
-- =============================================================================

/-- Shift up: every `var i` with `i ≥ cutoff` becomes `var (i + amount)`.
    Mirrors `proof_checker::shift_up`.  Universe levels pass through
    unchanged — they're not term binders. -/
partial def shiftUp (amount : Nat) (cutoff : Nat) (t : Term) : Term :=
  match t with
  | .var i        => if i ≥ cutoff then .var (i + amount) else .var i
  | .universe l   => .universe l
  | .pi a b       => .pi (shiftUp amount cutoff a) (shiftUp amount (cutoff + 1) b)
  | .lam a body   => .lam (shiftUp amount cutoff a) (shiftUp amount (cutoff + 1) body)
  | .app f x      => .app (shiftUp amount cutoff f) (shiftUp amount cutoff x)

/-- Shift down — inverse of `shiftUp`. -/
partial def shiftDown (amount : Nat) (cutoff : Nat) (t : Term) : Term :=
  match t with
  | .var i =>
    if i ≥ cutoff + amount then .var (i - amount)
    else if i < cutoff then .var i
    else .var i
  | .universe l   => .universe l
  | .pi a b       => .pi (shiftDown amount cutoff a) (shiftDown amount (cutoff + 1) b)
  | .lam a body   => .lam (shiftDown amount cutoff a) (shiftDown amount (cutoff + 1) body)
  | .app f x      => .app (shiftDown amount cutoff f) (shiftDown amount cutoff x)

/-- Substitute `replacement` for `var target` in `t`. -/
partial def subst (target : Nat) (replacement : Term) (t : Term) : Term :=
  match t with
  | .var i =>
    if i = target then shiftUp target 0 replacement
    else if i > target then .var (i - 1)
    else .var i
  | .universe l => .universe l
  | .pi a b     => .pi (subst target replacement a) (subst (target + 1) replacement b)
  | .lam a body => .lam (subst target replacement a) (subst (target + 1) replacement body)
  | .app f x    => .app (subst target replacement f) (subst target replacement x)

/-- Fuel ceiling for whnf. -/
def whnfFuelCeiling : Nat := 1 <<< 20

/-- Fuel-bounded whnf. -/
partial def whnfFuel (fuel : Nat) (t : Term) : Term :=
  if fuel = 0 then t
  else
    match t with
    | .app f x =>
      let f' := whnfFuel (fuel - 1) f
      match f' with
      | .lam _ body => whnfFuel (fuel - 1) (subst 0 x body)
      | other       => .app other x
    | other => other

/-- Public whnf. -/
def whnf (t : Term) : Term := whnfFuel whnfFuelCeiling t

/-- Is `var target` free in `t`? -/
partial def isFreeIn (target : Nat) (t : Term) : Bool :=
  match t with
  | .var i        => i = target
  | .universe _   => false
  | .pi a b       => isFreeIn target a || isFreeIn (target + 1) b
  | .lam a body   => isFreeIn target a || isFreeIn (target + 1) body
  | .app f x      => isFreeIn target f || isFreeIn target x

mutual

/-- α + β + η-equality after WHNF reduction. -/
partial def defEq (a b : Term) : Bool :=
  let aw := whnf a
  let bw := whnf b
  defEqWhnf aw bw

partial def defEqWhnf : Term → Term → Bool
  | .var i,        .var j        => i = j
  -- Universe equality is decided structurally on canonical levels —
  -- mirrors `proof_checker::level_eq`.
  | .universe l1,  .universe l2  => Level.eq l1 l2
  | .pi a1 b1,     .pi a2 b2     => defEq a1 a2 && defEq b1 b2
  | .lam a1 b1,    .lam a2 b2    => defEq a1 a2 && defEq b1 b2
  | .app f1 x1,    .app f2 x2    => defEq f1 f2 && defEq x1 x2
  | .lam _ body,   other         => etaMatch body other
  | other,         .lam _ body   => etaMatch body other
  | _,             _             => false

/-- η-equivalence helper — mirrors `proof_checker::eta_match`. -/
partial def etaMatch (lamBody other : Term) : Bool :=
  match whnf lamBody with
  | .app f x =>
    match whnf x with
    | .var 0 =>
      if isFreeIn 0 f then false
      else defEq (shiftDown 1 0 f) other
    | _ => false
  | _ => false

end

/-- Look up the type of `var i` in `Γ`, shifted to the use-site frame. -/
def lookupCtx (Γ : Ctx) (i : Nat) : Option Term :=
  let len := Γ.length
  if h : i < len then
    let raw := Γ[i]
    some (shiftUp (i + 1) 0 raw)
  else
    none

/-- If `t` whnf-reduces to `Universe l`, return the normalised level. -/
def expectUniverse (t : Term) : Option Level :=
  match whnf t with
  | .universe l => some (Level.normalize l)
  | _           => none

/-- Bidirectional type inference for the structural fragment. -/
partial def infer (Γ : Ctx) (t : Term) : Except CheckError Term :=
  match t with
  | .var i =>
    match lookupCtx Γ i with
    | some T => Except.ok T
    | none   => Except.error (.unbound_variable i)
  | .universe l =>
    -- T-Univ: Universe(l) : Universe(succ(l)).  Lean's Nat is
    -- unbounded so concrete overflow is information-only — the
    -- Rust side's `Level::checked_succ` is the load-bearing
    -- gate that pins parity at the u32::MAX boundary.
    Except.ok (.universe (Level.checkedSucc (Level.normalize l)))
  | .pi a b =>
    match infer Γ a with
    | .error e => .error e
    | .ok aTy =>
      match expectUniverse aTy with
      | none => .error (.not_a_type a)
      | some n =>
        match infer (a :: Γ) b with
        | .error e => .error e
        | .ok bTy =>
          match expectUniverse bTy with
          | none => .error (.not_a_type b)
          | some m => .ok (.universe (Level.maxCanonical n m))
  | .lam a body =>
    match infer Γ a with
    | .error e => .error e
    | .ok aTy =>
      match expectUniverse aTy with
      | none => .error (.not_a_type a)
      | some _ =>
        match infer (a :: Γ) body with
        | .error e => .error e
        | .ok bodyTy => .ok (.pi a bodyTy)
  | .app f x =>
    match infer Γ f with
    | .error e => .error e
    | .ok fTy =>
      match whnf fTy with
      | .pi dom codom =>
        match infer Γ x with
        | .error e => .error e
        | .ok xTy =>
          if defEq dom xTy then .ok (subst 0 x codom)
          else .error (.domain_mismatch dom xTy)
      | other => .error (.not_a_function other)

/-- Check that `t` has type `expected` in `Γ`. -/
partial def check (Γ : Ctx) (t expected : Term) : Except CheckError Unit :=
  match infer Γ t with
  | .error e => .error e
  | .ok inferred =>
    if defEq inferred expected then .ok ()
    else .error (.type_mismatch expected inferred)

/-- Verify a closed `(term, claimed_type)` certificate. -/
def verifyCertificate (term claimedType : Term) : Except CheckError Unit :=
  match infer [] claimedType with
  | .error e => .error e
  | .ok kind =>
    match expectUniverse kind with
    | none => .error (.claimed_type_not_a_type claimedType kind)
    | some _ => check [] term claimedType

end VerumKernel

/-! Sanity tests — mirror the Rust-side `proof_checker::tests`,
    including the FV-19 universe-polymorphism battery. -/

namespace VerumKernel.Tests

open VerumKernel

/-- Wrap an inference result + expected outcome into a Bool so
    `#eval` reports pass/fail uniformly. -/
def expectInfer (Γ : Ctx) (t expected : Term) : Bool :=
  match infer Γ t with
  | .ok inferred => defEq inferred expected
  | _            => false

def expectInferError (Γ : Ctx) (t : Term) (matchErr : CheckError → Bool) : Bool :=
  match infer Γ t with
  | .error e => matchErr e
  | _        => false

-- Helper: build a concrete-level Universe term.
def universe (n : Nat) : Term := .universe (.concrete n)

-- Helper: build a level-variable Universe term.
def universeVar (name : String) : Term := .universe (.var name)

#eval! expectInfer [] (universe 0) (universe 1)
  -- T-Univ — true

#eval! expectInferError [] (.var 0) (fun e => match e with | .unbound_variable 0 => true | _ => false)
  -- T-Var unbound — true

#eval! expectInfer [universe 0] (.var 0) (universe 0)
  -- T-Var with hypothesis — true

#eval! expectInfer [] (.pi (universe 2) (universe 5)) (universe 6)
  -- T-Pi-Form max(3, 6) = 6 — true

#eval! expectInfer []
        (.lam (universe 0) (.var 0))
        (.pi (universe 0) (universe 0))
  -- T-Lam-Intro identity at Universe(0) — true

#eval! expectInferError []
        (.app (universe 0) (universe 0))
        (fun e => match e with | .not_a_function _ => true | _ => false)
  -- T-App-Elim rejects non-function — true

#eval! expectInfer [] (universe 1000) (universe 1001)
  -- universe successor (Nat-unbounded on Lean side) — true

#eval! (
  let id := Term.lam (universe 0) (.var 0)
  match verifyCertificate id id with
  | .error (.claimed_type_not_a_type _ _) => true
  | _                                      => false)
  -- DEFECT-4 mirror: claimed-type-not-a-type rejection — true

-- =============================================================================
-- FV-19 universe-polymorphism battery (mirrors proof_checker::tests)
-- =============================================================================

#eval! (Level.eq
        (Level.max (Level.var "u") (Level.var "v"))
        (Level.max (Level.var "v") (Level.var "u")))
  -- max commutativity via canonical form — true

#eval! (Level.eq
        (Level.max (Level.var "u") (Level.var "u"))
        (Level.var "u"))
  -- max idempotency — true

#eval! (Level.eq
        (Level.max Level.zero (Level.var "u"))
        (Level.var "u"))
  -- zero-identity — true

#eval! (Level.eq
        (Level.max (Level.succ (Level.var "u")) (Level.succ (Level.var "v")))
        (Level.succ (Level.max (Level.var "u") (Level.var "v"))))
  -- common-succ factoring — true

#eval! expectInfer []
        (universeVar "u")
        (.universe (.succ (.var "u")))
  -- T-Univ on a level variable: Universe(u) : Universe(succ(u)) — true

#eval! expectInfer []
        (.pi (universeVar "u") (universeVar "v"))
        (.universe (.succ (Level.maxCanonical (.var "u") (.var "v"))))
  -- T-Pi-Form on polymorphic levels: Π(_:Type@u). Type@v : Type@succ(max(u,v)) — true

#eval! expectInfer []
        (.lam (universeVar "u") (.lam (.var 0) (.var 0)))
        (.pi (universeVar "u") (.pi (.var 0) (.var 1)))
  -- Polymorphic identity at Type@u — true

#eval! (
  let term := Term.lam (universeVar "u") (.lam (.var 0) (.var 0))
  let claim := Term.pi (universeVar "u") (.pi (.var 0) (.var 1))
  match verifyCertificate term claim with
  | .ok _ => true
  | _     => false)
  -- Certificate at Type@u verifies — true

#eval! (
  -- Distinct universe variables in term vs claim → TypeMismatch.
  let term := Term.lam (universeVar "u") (.lam (.var 0) (.var 0))
  let claim := Term.pi (universeVar "v") (.pi (.var 0) (.var 1))
  match verifyCertificate term claim with
  | .error (.type_mismatch _ _) => true
  | _                            => false)
  -- Distinct level variables reject — true

end VerumKernel.Tests
