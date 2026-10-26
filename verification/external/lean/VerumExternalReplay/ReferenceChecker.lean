/-! ============================================================== -/
/-! # Verum reference kernel checker                                 -/
/-! ============================================================== -/

/-! Independent re-implementation of the structural fragment of
    `crates/verum_kernel/src/proof_checker.rs` (Var / Universe /
    Pi / Lam / App / Sigma / Id / Refl / J, plus β-reduction, α-equivalence, η-equivalence,
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
    side uses `Nat` for storage but enforces the Rust `u32`
    concrete-level ceiling during inference. Equality is decided up to algebraic
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

/-- Native certificate terms: dependent functions and pairs, with
    intensional equality and transport. -/
inductive Term : Type where
  | data (name : String) (level : Level) (constructors : List (List (Option Term)))
  | construct (data : Term) (constructor : Nat) (args : List Term)
  | induct (scrutinee motive : Term) (cases : List Term)
  | letE (value body : Term)
  | var      (i : Nat)
  | universe (lvl : Level)
  | pi       (dom : Term) (body : Term)
  | lam      (dom : Term) (body : Term)
  | app      (f : Term) (x : Term)
  -- FV-20: Σ-types (dependent pairs).
  | sigma    (dom : Term) (body : Term)
  | pair     (fst : Term) (snd : Term)
  | fst      (p : Term)
  | snd      (p : Term)
  | identity (ty lhs rhs : Term)
  | refl     (value : Term)
  | transport (motive base path : Term)
  deriving Inhabited, Repr

/-- Type-checking error — mirrors `proof_checker::CheckError` for
    the structural fragment. -/
inductive CheckError : Type where
  | invalid_inductive (message : String)
  | unbound_variable     (idx : Nat)
  | not_a_type           (t : Term)
  | not_a_function       (t : Term)
  | not_a_sigma          (t : Term)  -- FV-20: T-Fst/Snd-Elim
  | not_an_identity      (t : Term)
  | not_a_valid_j_motive (t : Term)
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
  match compare (rank a) (rank b) with
  | .eq =>
    match a, b with
    | .concrete x, .concrete y => compare x y
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
  let sorted := xs.toArray.qsort (fun a b => Level.lexCompare a b == .lt) |>.toList
  -- Dedupe consecutive equals (we just sorted, so equals cluster).
  sorted.foldr (fun x acc =>
    match acc with
    | []      => [x]
    | h :: _  => if x == h then acc else x :: acc) []

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
      Nat.repeat Level.succ common
        (Level.assembleMax strippedConcrete strippedSymbolic)
    else
      Level.assembleMax concreteMax symbolicSorted
  else
    Level.assembleMax concreteMax symbolicSorted

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
      Level.normalize (Nat.repeat Level.succ by_ other)

-- =============================================================================
-- Term-level operations
-- =============================================================================

/-- Shift up: every `var i` with `i ≥ cutoff` becomes `var (i + amount)`.
    Mirrors `proof_checker::shift_up`.  Universe levels pass through
    unchanged — they're not term binders.  Σ extends Π's binder
    discipline; Pair/Fst/Snd are non-binding. -/
partial def shiftUp (amount : Nat) (cutoff : Nat) (t : Term) : Term :=
  match t with
  | .letE value body => .letE (shiftUp amount cutoff value) (shiftUp amount (cutoff+1) body)
  | .data name level constructors => .data name level (constructors.map (fun fields => fields.map (Option.map (shiftUp amount cutoff))))
  | .construct data ctor args => .construct (shiftUp amount cutoff data) ctor (args.map (shiftUp amount cutoff))
  | .induct value motive cases => .induct (shiftUp amount cutoff value) (shiftUp amount cutoff motive) (cases.map (shiftUp amount cutoff))
  | .var i        => if i ≥ cutoff then .var (i + amount) else .var i
  | .universe l   => .universe l
  | .pi a b       => .pi (shiftUp amount cutoff a) (shiftUp amount (cutoff + 1) b)
  | .lam a body   => .lam (shiftUp amount cutoff a) (shiftUp amount (cutoff + 1) body)
  | .app f x      => .app (shiftUp amount cutoff f) (shiftUp amount cutoff x)
  | .sigma a b    => .sigma (shiftUp amount cutoff a) (shiftUp amount (cutoff + 1) b)
  | .pair a b     => .pair (shiftUp amount cutoff a) (shiftUp amount cutoff b)
  | .fst p        => .fst (shiftUp amount cutoff p)
  | .snd p        => .snd (shiftUp amount cutoff p)
  | .identity a x y => .identity (shiftUp amount cutoff a) (shiftUp amount cutoff x) (shiftUp amount cutoff y)
  | .refl x => .refl (shiftUp amount cutoff x)
  | .transport p h e => .transport (shiftUp amount cutoff p) (shiftUp amount cutoff h) (shiftUp amount cutoff e)

/-- Shift down — inverse of `shiftUp`. -/
partial def shiftDown (amount : Nat) (cutoff : Nat) (t : Term) : Term :=
  match t with
  | .letE value body => .letE (shiftDown amount cutoff value) (shiftDown amount (cutoff+1) body)
  | .data name level constructors => .data name level (constructors.map (fun fields => fields.map (Option.map (shiftDown amount cutoff))))
  | .construct data ctor args => .construct (shiftDown amount cutoff data) ctor (args.map (shiftDown amount cutoff))
  | .induct value motive cases => .induct (shiftDown amount cutoff value) (shiftDown amount cutoff motive) (cases.map (shiftDown amount cutoff))
  | .var i =>
    if i ≥ cutoff + amount then .var (i - amount)
    else if i < cutoff then .var i
    else .var i
  | .universe l   => .universe l
  | .pi a b       => .pi (shiftDown amount cutoff a) (shiftDown amount (cutoff + 1) b)
  | .lam a body   => .lam (shiftDown amount cutoff a) (shiftDown amount (cutoff + 1) body)
  | .app f x      => .app (shiftDown amount cutoff f) (shiftDown amount cutoff x)
  | .sigma a b    => .sigma (shiftDown amount cutoff a) (shiftDown amount (cutoff + 1) b)
  | .pair a b     => .pair (shiftDown amount cutoff a) (shiftDown amount cutoff b)
  | .fst p        => .fst (shiftDown amount cutoff p)
  | .snd p        => .snd (shiftDown amount cutoff p)
  | .identity a x y => .identity (shiftDown amount cutoff a) (shiftDown amount cutoff x) (shiftDown amount cutoff y)
  | .refl x => .refl (shiftDown amount cutoff x)
  | .transport p h e => .transport (shiftDown amount cutoff p) (shiftDown amount cutoff h) (shiftDown amount cutoff e)

/-- Substitute `replacement` for `var target` in `t`. -/
partial def subst (target : Nat) (replacement : Term) (t : Term) : Term :=
  match t with
  | .letE value body => .letE (subst target replacement value) (subst (target+1) replacement body)
  | .data name level constructors => .data name level (constructors.map (fun fields => fields.map (Option.map (subst target replacement))))
  | .construct data ctor args => .construct (subst target replacement data) ctor (args.map (subst target replacement))
  | .induct value motive cases => .induct (subst target replacement value) (subst target replacement motive) (cases.map (subst target replacement))
  | .var i =>
    if i = target then shiftUp target 0 replacement
    else if i > target then .var (i - 1)
    else .var i
  | .universe l => .universe l
  | .pi a b     => .pi (subst target replacement a) (subst (target + 1) replacement b)
  | .lam a body => .lam (subst target replacement a) (subst (target + 1) replacement body)
  | .app f x    => .app (subst target replacement f) (subst target replacement x)
  | .sigma a b  => .sigma (subst target replacement a) (subst (target + 1) replacement b)
  | .pair a b   => .pair (subst target replacement a) (subst target replacement b)
  | .fst p      => .fst (subst target replacement p)
  | .snd p      => .snd (subst target replacement p)
  | .identity a x y => .identity (subst target replacement a) (subst target replacement x) (subst target replacement y)
  | .refl x => .refl (subst target replacement x)
  | .transport p h e => .transport (subst target replacement p) (subst target replacement h) (subst target replacement e)

/-- Fuel ceiling for whnf. -/
def whnfFuelCeiling : Nat := 1 <<< 20

/-- Fuel-bounded whnf — handles β-reduction at App and Σ-projection
    at Fst/Snd. -/
partial def whnfFuel (fuel : Nat) (t : Term) : Term :=
  if fuel = 0 then t
  else
    match t with
    | .letE value body => whnfFuel (fuel-1) (subst 0 value body)
    | .induct value motive cases =>
      let head := whnfFuel (fuel - 1) value
      match head with
      | .construct data ctor args =>
        match whnfFuel (fuel - 1) data with
        | .data _ _ constructors =>
          match constructors[ctor]?, cases[ctor]? with
          | some fields, some branch =>
            if fields.length == args.length then
              let applied := (fields.zip args).foldl (fun body (field, arg) =>
                let next := Term.app body arg
                match field with
                | none => .app next (.induct arg motive cases)
                | some _ => next) branch
              whnfFuel (fuel - 1) applied
            else .induct head motive cases
          | _, _ => .induct head motive cases
        | _ => .induct head motive cases
      | _ => .induct head motive cases
    | .app f x =>
      let f' := whnfFuel (fuel - 1) f
      match f' with
      | .lam _ body => whnfFuel (fuel - 1) (subst 0 x body)
      | other       => .app other x
    | .fst p =>
      let p' := whnfFuel (fuel - 1) p
      match p' with
      | .pair a _ => whnfFuel (fuel - 1) a
      | other     => .fst other
    | .snd p =>
      let p' := whnfFuel (fuel - 1) p
      match p' with
      | .pair _ b => whnfFuel (fuel - 1) b
      | other     => .snd other
    | .transport p h e =>
      match whnfFuel (fuel - 1) e with
      | .refl _ => whnfFuel (fuel - 1) h
      | other => .transport p h other
    | other => other

/-- Public whnf. -/
def whnf (t : Term) : Term := whnfFuel whnfFuelCeiling t

/-- Is `var target` free in `t`? -/
partial def isFreeIn (target : Nat) (t : Term) : Bool :=
  match t with
  | .letE value body => isFreeIn target value || isFreeIn (target+1) body
  | .data _ _ constructors => constructors.any (fun fields => fields.any (fun field => field.any (isFreeIn target)))
  | .construct data _ args => isFreeIn target data || args.any (isFreeIn target)
  | .induct value motive cases => isFreeIn target value || isFreeIn target motive || cases.any (isFreeIn target)
  | .var i        => i = target
  | .universe _   => false
  | .pi a b       => isFreeIn target a || isFreeIn (target + 1) b
  | .lam a body   => isFreeIn target a || isFreeIn (target + 1) body
  | .app f x      => isFreeIn target f || isFreeIn target x
  | .sigma a b    => isFreeIn target a || isFreeIn (target + 1) b
  | .pair a b     => isFreeIn target a || isFreeIn target b
  | .fst p        => isFreeIn target p
  | .snd p        => isFreeIn target p
  | .identity a x y => isFreeIn target a || isFreeIn target x || isFreeIn target y
  | .refl x => isFreeIn target x
  | .transport p h e => isFreeIn target p || isFreeIn target h || isFreeIn target e

mutual

/-- α + β + η-equality after WHNF reduction. -/
partial def defEq (a b : Term) : Bool :=
  let aw := whnf a
  let bw := whnf b
  defEqWhnf aw bw

partial def defEqWhnf : Term → Term → Bool

  | .data an al ac, .data bn bl bc =>
    an == bn && Level.eq al bl && ac.length == bc.length && (ac.zip bc).all (fun (a,b) =>
      a.length == b.length && (a.zip b).all (fun (x,y) => match x,y with
        | none, none => true
        | some x, some y => defEq x y
        | _, _ => false))
  | .construct a ai aa, .construct b bi ba =>
    ai == bi && defEq a b && aa.length == ba.length && (aa.zip ba).all (fun (x,y) => defEq x y)
  | .induct a am ac, .induct b bm bc =>
    defEq a b && defEq am bm && ac.length == bc.length && (ac.zip bc).all (fun (x,y) => defEq x y)
  | .var i,        .var j        => i = j
  -- Universe equality is decided structurally on canonical levels —
  -- mirrors `proof_checker::level_eq`.
  | .universe l1,  .universe l2  => Level.eq l1 l2
  | .pi a1 b1,     .pi a2 b2     => defEq a1 a2 && defEq b1 b2
  | .lam a1 b1,    .lam a2 b2    => defEq a1 a2 && defEq b1 b2
  | .app f1 x1,    .app f2 x2    => defEq f1 f2 && defEq x1 x2
  -- Σ-types (FV-20): structural component-wise equality.
  -- β-projections (Fst(Pair) / Snd(Pair)) collapse in whnfFuel
  -- before reaching here.
  | .sigma a1 b1,  .sigma a2 b2  => defEq a1 a2 && defEq b1 b2
  | .pair a1 b1,   .pair a2 b2   => defEq a1 a2 && defEq b1 b2
  | .fst p1,       .fst p2       => defEq p1 p2
  | .snd p1,       .snd p2       => defEq p1 p2
  | .identity a x y, .identity b u v => defEq a b && defEq x u && defEq y v
  | .refl x, .refl y => defEq x y
  | .transport p h e, .transport q k f => defEq p q && defEq h k && defEq e f
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

/-- Independently construct each case telescope from explicit recursive slots.
    Captured parameters remain in the surrounding de Bruijn context. -/
def inductionCaseType (data motive : Term) (ctor : Nat) (fields : List (Option Term)) : Term := Id.run do
  let mut domains : List Term := []
  let mut positions : List Nat := []
  for field in fields do
    let depth := domains.length
    let domain := shiftUp depth 0 (field.getD data)
    positions := positions ++ [depth]
    domains := domains ++ [domain]
    if field.isNone then
      domains := domains ++ [.app (shiftUp (depth + 1) 0 motive) (.var 0)]
  let depth := domains.length
  let value := Term.construct (shiftUp depth 0 data) ctor (positions.map (fun i => .var (depth - 1 - i)))
  return domains.foldr Term.pi (.app (shiftUp depth 0 motive) value)

/-- Bidirectional type inference for the structural fragment. -/
partial def infer (Γ : Ctx) (t : Term) : Except CheckError Term :=
  match t with
  | .letE value body => do
    let _ ← infer Γ value
    infer Γ (subst 0 value body)
  | .data _ level constructors => do
    for fields in constructors do
      for field in fields do
        if let some ty := field then
          let kind ← infer Γ ty
          let some fieldLevel := expectUniverse kind | throw (.not_a_type ty)
          unless Level.eq (Level.maxCanonical fieldLevel level) level do
            throw (.invalid_inductive "field exceeds declaration universe")
    return .universe (Level.normalize level)
  | .construct data ctor args => do
    let _ ← infer Γ data
    let declaration := whnf data
    let .data _ _ constructors := declaration | throw (.invalid_inductive "constructor owner is not a data declaration")
    let some fields := constructors[ctor]? | throw (.invalid_inductive "unknown constructor")
    unless fields.length == args.length do throw (.invalid_inductive "constructor field arity mismatch")
    for (field, arg) in fields.zip args do
      let expected := field.getD declaration
      let actual ← infer Γ arg
      unless defEq actual expected do throw (.type_mismatch expected actual)
    return declaration
  | .induct value motive cases => do
    let declaration := whnf (← infer Γ value)
    let .data _ _ constructors := declaration | throw (.invalid_inductive "induction requires a data scrutinee")
    let _ ← infer Γ declaration
    let motiveKind := whnf (← infer Γ motive)
    let .pi domain result := motiveKind | throw (.invalid_inductive "induction motive is not a family")
    unless defEq domain declaration && (expectUniverse result).isSome do
      throw (.invalid_inductive "induction motive domain or universe mismatch")
    unless cases.length == constructors.length do throw (.invalid_inductive "induction case count mismatch")
    for (index, (fields, branch)) in (constructors.zip cases).zipIdx |>.map (fun (pair,i) => (i,pair)) do
      let expected := inductionCaseType declaration motive index fields
      let actual ← infer Γ branch
      unless defEq actual expected do throw (.type_mismatch expected actual)
    return .app motive value
  | .var i =>
    match lookupCtx Γ i with
    | some T => Except.ok T
    | none   => Except.error (.unbound_variable i)
  | .universe l =>
    -- The certificate format uses Rust's bounded concrete universe levels.
    -- The independent checker enforces that boundary, despite using Nat.
    let normal := Level.normalize l
    match normal with
    | .concrete n =>
      if n ≥ 4294967295 then .error (.universe_overflow n)
      else .ok (.universe (Level.checkedSucc normal))
    | _ => .ok (.universe (Level.checkedSucc normal))
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
  -- T-Sigma-Form: Σ(x : A). B : Universe(max(level(A), level(B))).
  | .sigma a b =>
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
  -- T-Pair-Intro: synthesis-mode mirror.  Synthesises the most-
  -- specific non-dependent Σ that fits.
  | .pair fst snd =>
    match infer Γ fst with
    | .error e => .error e
    | .ok fstTy =>
      match infer Γ snd with
      | .error e => .error e
      | .ok sndTy =>
        let body := shiftUp 1 0 sndTy
        .ok (.sigma fstTy body)
  -- T-Fst-Elim: p : Σ(A, B) ⇒ Fst(p) : A.
  | .fst p =>
    match infer Γ p with
    | .error e => .error e
    | .ok pTy =>
      match whnf pTy with
      | .sigma a _ => .ok a
      | other      => .error (.not_a_sigma other)
  -- T-Snd-Elim: p : Σ(A, B) ⇒ Snd(p) : B[Fst(p)/0].
  | .snd p =>
    match infer Γ p with
    | .error e => .error e
    | .ok pTy =>
      match whnf pTy with
      | .sigma _ b => .ok (subst 0 (.fst p) b)
      | other      => .error (.not_a_sigma other)
  | .identity a x y => do
    let kind ← infer Γ a
    let some level := expectUniverse kind | .error (.not_a_type a)
    for endpoint in [x, y] do
      let actual ← infer Γ endpoint
      unless defEq a actual do throw (.domain_mismatch a actual)
    return .universe level
  | .refl x => do
    let ty ← infer Γ x
    return .identity ty x x
  | .transport motive base path => do
    let pathTy ← infer Γ path
    let .identity carrier lhs rhs := whnf pathTy | .error (.not_an_identity (whnf pathTy))
    let motiveTy ← infer Γ motive
    let .pi domain codomain := whnf motiveTy | .error (.not_a_valid_j_motive motive)
    unless defEq domain carrier do throw (.domain_mismatch carrier domain)
    let some _ := expectUniverse codomain | .error (.not_a_valid_j_motive motive)
    let baseTy ← infer Γ base
    let expected := whnf (.app motive lhs)
    unless defEq baseTy expected do throw (.domain_mismatch expected baseTy)
    return .app motive rhs

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
def univ (n : Nat) : Term := .universe (.concrete n)

-- Helper: build a level-variable Universe term.
def univVar (name : String) : Term := .universe (.var name)

#eval! expectInfer [] (univ 0) (univ 1)
  -- T-Univ — true

#eval! expectInferError [] (.var 0) (fun e => match e with | .unbound_variable 0 => true | _ => false)
  -- T-Var unbound — true

#eval! expectInfer [univ 0] (.var 0) (univ 0)
  -- T-Var with hypothesis — true

#eval! expectInfer [] (.pi (univ 2) (univ 5)) (univ 6)
  -- T-Pi-Form max(3, 6) = 6 — true

#eval! expectInfer []
        (.lam (univ 0) (.var 0))
        (.pi (univ 0) (univ 0))
  -- T-Lam-Intro identity at Universe(0) — true

#eval! expectInferError []
        (.app (univ 0) (univ 0))
        (fun e => match e with | .not_a_function _ => true | _ => false)
  -- T-App-Elim rejects non-function — true

#eval! expectInfer [] (univ 1000) (univ 1001)
  -- universe successor (Nat-unbounded on Lean side) — true

#eval! (
  let id := Term.lam (univ 0) (.var 0)
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
        (univVar "u")
        (.universe (.succ (.var "u")))
  -- T-Univ on a level variable: Universe(u) : Universe(succ(u)) — true

#eval! expectInfer []
        (.pi (univVar "u") (univVar "v"))
        (.universe (.succ (Level.maxCanonical (.var "u") (.var "v"))))
  -- T-Pi-Form on polymorphic levels: Π(_:Type@u). Type@v : Type@succ(max(u,v)) — true

#eval! expectInfer []
        (.lam (univVar "u") (.lam (.var 0) (.var 0)))
        (.pi (univVar "u") (.pi (.var 0) (.var 1)))
  -- Polymorphic identity at Type@u — true

#eval! (
  let term := Term.lam (univVar "u") (.lam (.var 0) (.var 0))
  let claim := Term.pi (univVar "u") (.pi (.var 0) (.var 1))
  match verifyCertificate term claim with
  | .ok _ => true
  | _     => false)
  -- Certificate at Type@u verifies — true

#eval! (
  -- Distinct universe variables in term vs claim → TypeMismatch.
  let term := Term.lam (univVar "u") (.lam (.var 0) (.var 0))
  let claim := Term.pi (univVar "v") (.pi (.var 0) (.var 1))
  match verifyCertificate term claim with
  | .error (.type_mismatch _ _) => true
  | _                            => false)
  -- Distinct level variables reject — true

-- =============================================================================
-- FV-20 Σ-types battery (mirrors proof_checker::tests Σ section)
-- =============================================================================

#eval! expectInfer []
        (.sigma (univ 0) (univ 0))
        (univ 1)
  -- T-Sigma-Form at concrete universe — true

#eval! expectInfer []
        (.sigma (univ 2) (univ 5))
        (univ 6)
  -- T-Sigma-Form takes max universe — true

#eval! expectInfer []
        (.sigma (univVar "u") (univVar "v"))
        (.universe (.succ (Level.maxCanonical (.var "u") (.var "v"))))
  -- T-Sigma-Form polymorphic — true

#eval! expectInfer []
        (.pair (univ 0) (univ 0))
        (.sigma (univ 1) (univ 1))
  -- T-Pair-Intro non-dependent — true

#eval! defEq (.fst (.pair (univ 0) (univ 1))) (univ 0)
  -- β-projection: fst(pair(a, b)) ≡ a — true

#eval! defEq (.snd (.pair (univ 0) (univ 1))) (univ 1)
  -- β-projection: snd(pair(a, b)) ≡ b — true

#eval! expectInferError []
        (.fst (univ 0))
        (fun e => match e with | .not_a_sigma _ => true | _ => false)
  -- T-Fst-Elim rejects non-Σ — true

#eval! expectInferError []
        (.snd (univ 0))
        (fun e => match e with | .not_a_sigma _ => true | _ => false)
  -- T-Snd-Elim rejects non-Σ — true

#eval! (
  let term := Term.pair (univ 0) (univ 0)
  let claim := Term.sigma (univ 1) (univ 1)
  match verifyCertificate term claim with
  | .ok _ => true
  | _     => false)
  -- Pair certificate at non-dependent Σ verifies — true

#eval! expectInfer []
        (.sigma (univ 0) (.sigma (univ 0) (univ 0)))
        (univ 1)
  -- Nested Σ — true

end VerumKernel.Tests
