/-! ============================================================== -/
/-! # Verum reference kernel checker                                 -/
/-! ============================================================== -/

/-! Independent re-implementation of the structural fragment of
    `crates/verum_kernel/src/proof_checker.rs` (Var / Universe /
    Pi / Lam / App, plus β-reduction, α-equivalence, and
    universe-overflow handling).

    This is the **Lean side of the differential checker**: a
    decidable `Kernel.check : Ctx → Term → Term → Except Error Unit`
    that the audit harness runs on the same `Certificate` battery
    the Rust kernel verifies, so any disagreement is a real bug
    in *one* of the two implementations.

    The Verum-side equivalent is `verum_kernel::proof_checker`;
    see `docs/architecture/verum-kernel-audit-2026.md` for the
    audit ledger this file is the load-bearing artefact for. -/

namespace VerumKernel

/-- Minimal CoC `Term` — mirrors `proof_checker::Term`. -/
inductive Term : Type where
  | var      (i : Nat)
  | universe (lvl : Nat)
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

/-- Shift up: every `var i` with `i ≥ cutoff` becomes `var (i + amount)`.
    Mirrors `proof_checker::shift_up`. -/
partial def shiftUp (amount : Nat) (cutoff : Nat) (t : Term) : Term :=
  match t with
  | .var i        => if i ≥ cutoff then .var (i + amount) else .var i
  | .universe n   => .universe n
  | .pi a b       => .pi (shiftUp amount cutoff a) (shiftUp amount (cutoff + 1) b)
  | .lam a body   => .lam (shiftUp amount cutoff a) (shiftUp amount (cutoff + 1) body)
  | .app f x      => .app (shiftUp amount cutoff f) (shiftUp amount cutoff x)

/-- Shift down — inverse of `shiftUp`. Used by the η-rule to reach
    under a removed binder.  Conservative: if a variable would
    underflow we leave it unchanged so the caller gets a structural
    `def_eq` failure rather than a wrong index. -/
partial def shiftDown (amount : Nat) (cutoff : Nat) (t : Term) : Term :=
  match t with
  | .var i =>
    if i ≥ cutoff + amount then .var (i - amount)
    else if i < cutoff then .var i
    else .var i
  | .universe n   => .universe n
  | .pi a b       => .pi (shiftDown amount cutoff a) (shiftDown amount (cutoff + 1) b)
  | .lam a body   => .lam (shiftDown amount cutoff a) (shiftDown amount (cutoff + 1) body)
  | .app f x      => .app (shiftDown amount cutoff f) (shiftDown amount cutoff x)

/-- Substitute `replacement` for `var target` in `t`.  Mirrors
    `proof_checker::subst`.  De-Bruijn-correct: indices > target
    decrement; the replacement shifts up by `target` to compensate
    for binders we descend into. -/
partial def subst (target : Nat) (replacement : Term) (t : Term) : Term :=
  match t with
  | .var i =>
    if i = target then shiftUp target 0 replacement
    else if i > target then .var (i - 1)
    else .var i
  | .universe n => .universe n
  | .pi a b     => .pi (subst target replacement a) (subst (target + 1) replacement b)
  | .lam a body => .lam (subst target replacement a) (subst (target + 1) replacement body)
  | .app f x    => .app (subst target replacement f) (subst target replacement x)

/-- Fuel ceiling for whnf — mirrors `proof_checker::WHNF_FUEL_CEILING`. -/
def whnfFuelCeiling : Nat := 1 <<< 20

/-- Inner whnf with explicit fuel.  Returns the partially-reduced
    term unchanged once fuel hits zero — sound, since downstream
    `defEq` rejects structurally if the pair isn't actually equal. -/
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

/-- Public whnf — fuel-bounded β-reduction at the head. -/
def whnf (t : Term) : Term := whnfFuel whnfFuelCeiling t

/-- Is `var target` free in `t`? Used by η to ensure the bound
    variable does not escape into the function part. -/
partial def isFreeIn (target : Nat) (t : Term) : Bool :=
  match t with
  | .var i        => i = target
  | .universe _   => false
  | .pi a b       => isFreeIn target a || isFreeIn (target + 1) b
  | .lam a body   => isFreeIn target a || isFreeIn (target + 1) body
  | .app f x      => isFreeIn target f || isFreeIn target x

-- Definitional equality — α + β + η, decided after whnf.  Mirrors
-- `proof_checker::def_eq`.  The mutual recursion (defEq ↔
-- defEqWhnf ↔ etaMatch) is wrapped in a `mutual` block so Lean
-- handles the cross-references uniformly.

mutual

/-- α + β + η-equality after WHNF reduction. -/
partial def defEq (a b : Term) : Bool :=
  let aw := whnf a
  let bw := whnf b
  defEqWhnf aw bw

partial def defEqWhnf : Term → Term → Bool
  | .var i,        .var j        => i = j
  | .universe n,   .universe m   => n = m
  | .pi a1 b1,     .pi a2 b2     => defEq a1 a2 && defEq b1 b2
  | .lam a1 b1,    .lam a2 b2    => defEq a1 a2 && defEq b1 b2
  | .app f1 x1,    .app f2 x2    => defEq f1 f2 && defEq x1 x2
  | .lam _ body,   other         => etaMatch body other
  | other,         .lam _ body   => etaMatch body other
  | _,             _             => false

/-- η-equivalence helper — mirrors `proof_checker::eta_match`.
    DEFECT-1 fix: whnf the argument before the `Var 0` test. -/
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

/-- Look up the type of `var i` in `Γ`, shifted to the use-site
    frame.  Mirrors `proof_checker::Context::lookup`. -/
def lookupCtx (Γ : Ctx) (i : Nat) : Option Term :=
  let len := Γ.length
  if h : i < len then
    -- Stack is innermost-first stored as head-first List, but the
    -- Rust impl stores innermost-last (Vec push).  We mirror the
    -- semantic: index 0 = innermost.  Lean's List has head = front,
    -- so we treat the front as innermost.
    let raw := Γ[i]
    some (shiftUp (i + 1) 0 raw)
  else
    none

/-- If `t` whnf-reduces to `Universe n`, return `n`. -/
def expectUniverse (t : Term) : Option Nat :=
  match whnf t with
  | .universe n => some n
  | _           => none

/-- Bidirectional type inference for the structural fragment.
    Mirrors `proof_checker::infer`.  Returns the inferred type or a
    `CheckError`. -/
partial def infer (Γ : Ctx) (t : Term) : Except CheckError Term :=
  match t with
  | .var i =>
    match lookupCtx Γ i with
    | some T => Except.ok T
    | none   => Except.error (.unbound_variable i)
  | .universe n =>
    -- DEFECT-2: explicit overflow — at the Lean side, Nat is
    -- unbounded so this branch is information-only; the Rust kernel
    -- emits UniverseOverflow when the level would exceed u32::MAX.
    Except.ok (.universe (n + 1))
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
          | some m => .ok (.universe (Nat.max n m))
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

/-- Check that `t` has type `expected` in `Γ`.  Mirrors
    `proof_checker::check`. -/
partial def check (Γ : Ctx) (t expected : Term) : Except CheckError Unit :=
  match infer Γ t with
  | .error e => .error e
  | .ok inferred =>
    if defEq inferred expected then .ok ()
    else .error (.type_mismatch expected inferred)

/-- Verify a closed `(term, claimed_type)` certificate.  Includes
    the DEFECT-4 fix (claimed_type must itself be a type). -/
def verifyCertificate (term claimedType : Term) : Except CheckError Unit :=
  match infer [] claimedType with
  | .error e => .error e
  | .ok kind =>
    match expectUniverse kind with
    | none => .error (.claimed_type_not_a_type claimedType kind)
    | some _ => check [] term claimedType

end VerumKernel

/-! Sanity tests — mirror the Rust-side `proof_checker::tests` for
    the structural fragment.  These are `#eval` statements rather
    than `example`/`theorem` because the kernel functions are
    `partial def` (Lean's termination checker would otherwise need
    to verify CoC strong-normalisation, which is a separate effort
    tracked under FV-2).  Each `#eval` runs at elaboration time;
    a failed match prints `false` to the build log.

    The full differential battery lives Rust-side: see FV-3 in
    `docs/architecture/verum-kernel-audit-2026.md` §5 for the
    JSON-roundtrip protocol that compares verdicts against
    `crates/verum_kernel::proof_checker`. -/

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

#eval! expectInfer [] (.universe 0) (.universe 1)
  -- T-Univ — true

#eval! expectInferError [] (.var 0) (fun e => match e with | .unbound_variable 0 => true | _ => false)
  -- T-Var unbound — true

#eval! expectInfer [.universe 0] (.var 0) (.universe 0)
  -- T-Var with hypothesis — true

#eval! expectInfer [] (.pi (.universe 2) (.universe 5)) (.universe 6)
  -- T-Pi-Form max(3, 6) = 6 — true

#eval! expectInfer []
        (.lam (.universe 0) (.var 0))
        (.pi (.universe 0) (.universe 0))
  -- T-Lam-Intro identity at Universe(0) — true

#eval! expectInferError []
        (.app (.universe 0) (.universe 0))
        (fun e => match e with | .not_a_function _ => true | _ => false)
  -- T-App-Elim rejects non-function — true

#eval! expectInfer [] (.universe 1000) (.universe 1001)
  -- universe successor (Nat-unbounded on Lean side) — true

#eval! (
  let id := Term.lam (.universe 0) (.var 0)
  match verifyCertificate id id with
  | .error (.claimed_type_not_a_type _ _) => true
  | _                                      => false)
  -- DEFECT-4 mirror: claimed-type-not-a-type rejection — true

end VerumKernel.Tests
