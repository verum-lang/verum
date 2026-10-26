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
    structured `Level` (Concrete / Var / Succ / Max / Offset).  The Lean
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
  | offset   (base : Level) (by_ : Nat)
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

/-- Maximum concrete wire integer. Expressions can denote larger levels exactly. -/
def Level.wireMax : Nat := 4294967295

/-- Keep shifts compact; neither arithmetic overflow nor the offset magnitude
    may change the represented mathematical level. -/
def Level.offsetNode (base : Level) (by_ : Nat) : Level :=
  match by_ with
  | 0 => base
  | 1 => .succ base
  | n => .offset base n

partial def Level.addOffset (base : Level) (by_ : Nat) : Level :=
  if by_ = 0 then base else
  match base with
  | .concrete n =>
    if n + by_ ≤ Level.wireMax then .concrete (n + by_)
    else Level.offsetNode (.concrete Level.wireMax) (by_ - (Level.wireMax - n))
  | .succ inner =>
    if by_ + 1 ≤ Level.wireMax then Level.addOffset inner (by_ + 1)
    else Level.offsetNode base by_
  | .offset inner prior =>
    if prior + by_ ≤ Level.wireMax then Level.addOffset inner (prior + by_)
    else Level.offsetNode base by_
  | other => Level.offsetNode other by_

/-- Successor after the caller has checked the concrete wire ceiling. -/
def Level.succ' (l : Level) : Level :=
  match l with
  | .concrete n => .concrete (n + 1)
  | other => .succ other

/-- Lexicographic structural ordering on canonical Levels.  Used by
    [`Level.maxCanonical`] to produce a deterministic Max-summand
    order so commutative variants compare equal. -/
partial def Level.lexCompare (a b : Level) : Ordering :=
  let rank : Level → Nat
    | .concrete _ => 0
    | .var _      => 1
    | .succ _     => 2
    | .max _ _    => 3
    | .offset _ _ => 4
  match compare (rank a) (rank b) with
  | .eq =>
    match a, b with
    | .concrete x, .concrete y => compare x y
    | .var x,      .var y      => compare x y
    | .succ x,     .succ y     => Level.lexCompare x y
    | .offset x n, .offset y m =>
      match Level.lexCompare x y with
      | .eq => compare n m
      | order => order
    | .max xa xb,  .max ya yb  =>
      match Level.lexCompare xa ya with
      | .eq    => Level.lexCompare xb yb
      | other  => other
    | _, _ => .eq
  | other => other

/-- Common-prefix budget, capped at the wire integer width. This bound is
    only a factoring budget; it never saturates a universe's value. -/
partial def Level.succDepth : Level → Nat
  | .succ inner => min Level.wireMax (1 + Level.succDepth inner)
  | .offset inner by_ => min Level.wireMax (by_ + Level.succDepth inner)
  | _ => 0

/-- Strip an available prefix by visiting syntax, never by counting to its value. -/
partial def Level.stripSucc (l : Level) (count : Nat) : Level :=
  if count = 0 then l
  else match l with
    | .succ inner => Level.stripSucc inner (count - 1)
    | .offset inner by_ =>
      if by_ ≤ count then Level.stripSucc inner (count - by_)
      else Level.addOffset inner (by_ - count)
    | other => other

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
  match all.reverse with
  | [] => .concrete 0
  | last :: rest => rest.foldl (fun acc x => .max x acc) last

mutual

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
      let inner := Level.normalize (Level.assembleMax strippedConcrete strippedSymbolic)
      Level.addOffset inner common
    else
      Level.assembleMax concreteMax symbolicSorted
  else
    Level.assembleMax concreteMax symbolicSorted

/-- Normalise a level to canonical form.  Idempotent. -/
partial def Level.normalize (l : Level) : Level :=
  match l with
  | .concrete _  => l
  | .var _       => l
  | .succ inner => Level.addOffset (Level.normalize inner) 1
  | .offset inner by_ => Level.addOffset (Level.normalize inner) by_
  | .max a b =>
    let aN := Level.normalize a
    let bN := Level.normalize b
    Level.maxCanonical aN bN

end

/-- Definitional equality on levels — normalise both sides and
    compare structurally. -/
def Level.eq (a b : Level) : Bool :=
  Level.normalize a == Level.normalize b

/-- The successor with concrete-overflow tracking.  In Lean `Nat` is
    unbounded so this never reports overflow; the kernel-level
    decision is made on the Rust side via `Level::checked_succ`. -/
partial def Level.checkedSucc (l : Level) : Level := Level.succ' l

/-- Compact exact lifting, with work bounded by the expression size. -/
def Level.shiftedBy (l : Level) (by_ : Nat) : Level :=
  if by_ = 0 then l else Level.addOffset (Level.normalize l) by_

-- =============================================================================
-- Term-level operations
-- =============================================================================

/- Scope validation is total and independent of type inference/reduction.
A variable's index is checked at its actual binder depth. Constructor fields,
arguments and induction cases have no implicit term binders. -/
mutual

def scopeError (bound : Nat) (t : Term) : Option Nat :=
  match t with
  | .var i => if i < bound then none else some i
  | .universe _ => none
  | .letE value body => (scopeError bound value).or (scopeError (bound + 1) body)
  | .pi a b | .lam a b | .sigma a b => (scopeError bound a).or (scopeError (bound + 1) b)
  | .app a b | .pair a b => (scopeError bound a).or (scopeError bound b)
  | .fst value | .snd value | .refl value => scopeError bound value
  | .identity a b c | .transport a b c =>
    (scopeError bound a).or ((scopeError bound b).or (scopeError bound c))
  | .data _ _ constructors => scopeConstructors bound constructors
  | .construct data _ args => (scopeError bound data).or (scopeTerms bound args)
  | .induct value motive cases =>
    (scopeError bound value).or ((scopeError bound motive).or (scopeTerms bound cases))
termination_by sizeOf t

def scopeTerms (bound : Nat) (terms : List Term) : Option Nat :=
  match terms with
  | [] => none
  | term :: rest => (scopeError bound term).or (scopeTerms bound rest)
termination_by sizeOf terms

def scopeFields (bound : Nat) (fields : List (Option Term)) : Option Nat :=
  match fields with
  | [] => none
  | none :: rest => scopeFields bound rest
  | some term :: rest => (scopeError bound term).or (scopeFields bound rest)
termination_by sizeOf fields

def scopeConstructors (bound : Nat) (constructors : List (List (Option Term))) : Option Nat :=
  match constructors with
  | [] => none
  | fields :: rest => (scopeFields bound fields).or (scopeConstructors bound rest)
termination_by sizeOf constructors

end

def checkScope (bound : Nat) (t : Term) : Except CheckError Unit :=
  match scopeError bound t with
  | none => .ok ()
  | some index => .error (.unbound_variable index)

/- One total traversal owns the binding discipline for renaming and
    substitution. Constructor value fields, arguments and induction branches
    introduce no implicit binders; let bodies, Pi, Lam and Sigma introduce one.
    The explicit nested-list helpers expose decreasing structural recursion. -/
mutual

def mapVars (replace : Nat → Nat → Term) (cutoff : Nat) (t : Term) : Term :=
  match t with
  | .letE value body => .letE (mapVars replace cutoff value) (mapVars replace (cutoff+1) body)
  | .data name level constructors => .data name level (mapConstructors replace cutoff constructors)
  | .construct data ctor args => .construct (mapVars replace cutoff data) ctor (mapTerms replace cutoff args)
  | .induct value motive cases => .induct (mapVars replace cutoff value) (mapVars replace cutoff motive) (mapTerms replace cutoff cases)
  | .var i        => replace cutoff i
  | .universe l   => .universe l
  | .pi a b       => .pi (mapVars replace cutoff a) (mapVars replace (cutoff + 1) b)
  | .lam a body   => .lam (mapVars replace cutoff a) (mapVars replace (cutoff + 1) body)
  | .app f x      => .app (mapVars replace cutoff f) (mapVars replace cutoff x)
  | .sigma a b    => .sigma (mapVars replace cutoff a) (mapVars replace (cutoff + 1) b)
  | .pair a b     => .pair (mapVars replace cutoff a) (mapVars replace cutoff b)
  | .fst p        => .fst (mapVars replace cutoff p)
  | .snd p        => .snd (mapVars replace cutoff p)
  | .identity a x y => .identity (mapVars replace cutoff a) (mapVars replace cutoff x) (mapVars replace cutoff y)
  | .refl x => .refl (mapVars replace cutoff x)
  | .transport p h e => .transport (mapVars replace cutoff p) (mapVars replace cutoff h) (mapVars replace cutoff e)
termination_by sizeOf t

def mapTerms (replace : Nat → Nat → Term) (cutoff : Nat) (terms : List Term) : List Term :=
  match terms with
  | [] => []
  | term :: rest => mapVars replace cutoff term :: mapTerms replace cutoff rest
termination_by sizeOf terms

def mapFields (replace : Nat → Nat → Term) (cutoff : Nat)
    (fields : List (Option Term)) : List (Option Term) :=
  match fields with
  | [] => []
  | none :: rest => none :: mapFields replace cutoff rest
  | some term :: rest => some (mapVars replace cutoff term) :: mapFields replace cutoff rest
termination_by sizeOf fields

def mapConstructors (replace : Nat → Nat → Term) (cutoff : Nat)
    (constructors : List (List (Option Term))) : List (List (Option Term)) :=
  match constructors with
  | [] => []
  | fields :: rest => mapFields replace cutoff fields :: mapConstructors replace cutoff rest
termination_by sizeOf constructors

end

/-- Raise every variable at or above the cutoff; universe levels are unchanged. -/
def shiftUp (amount cutoff : Nat) (t : Term) : Term :=
  mapVars (fun cutoff i => if i ≥ cutoff then .var (i + amount) else .var i) cutoff t

/-- Lower variables above the removed interval. Variables inside it are left
    unchanged: this is a total operation, not a scoping validator. -/
def shiftDown (amount cutoff : Nat) (t : Term) : Term :=
  mapVars (fun cutoff i => if i ≥ cutoff + amount then .var (i - amount) else .var i) cutoff t

/-- Substitute at the target and discharge that binder. -/
def subst (target : Nat) (replacement : Term) (t : Term) : Term :=
  mapVars (fun target i =>
    if i = target then shiftUp target 0 replacement
    else if i > target then .var (i - 1)
    else .var i) target t

/-- Apply a constructor branch without allocating a zipped field/argument list.
    Recursive hypotheses follow their own field immediately. The reducer checks
    arity first; truncation on malformed input matches `zip` exactly. -/
def applyInductionCase (motive : Term) (cases : List Term)
    (fields : List (Option Term)) (args : List Term) (branch : Term) : Term :=
  match fields, args with
  | field :: rest, arg :: tail =>
    let next := Term.app branch arg
    let applied := match field with
      | none => Term.app next (.induct arg motive cases)
      | some _ => next
    applyInductionCase motive cases rest tail applied
  | _, _ => branch
termination_by structural fields

/-- Fuel ceiling for whnf. -/
def whnfFuelCeiling : Nat := 1 <<< 20

/-- Fuel-bounded whnf — handles β-reduction at App and Σ-projection
    at Fst/Snd. -/
def whnfFuel (fuel : Nat) (t : Term) : Term :=
  match fuel with
  | 0 => t
  | fuel + 1 =>
    match t with
    | .letE value body => whnfFuel fuel (subst 0 value body)
    | .induct value motive cases =>
      let head := whnfFuel fuel value
      match head with
      | .construct data ctor args =>
        match whnfFuel fuel data with
        | .data _ _ constructors =>
          match constructors[ctor]?, cases[ctor]? with
          | some fields, some branch =>
            if fields.length == args.length then
              whnfFuel fuel (applyInductionCase motive cases fields args branch)
            else .induct head motive cases
          | _, _ => .induct head motive cases
        | _ => .induct head motive cases
      | _ => .induct head motive cases
    | .app f x =>
      let f' := whnfFuel fuel f
      match f' with
      | .lam _ body => whnfFuel fuel (subst 0 x body)
      | other       => .app other x
    | .fst p =>
      let p' := whnfFuel fuel p
      match p' with
      | .pair a _ => whnfFuel fuel a
      | other     => .fst other
    | .snd p =>
      let p' := whnfFuel fuel p
      match p' with
      | .pair _ b => whnfFuel fuel b
      | other     => .snd other
    | .transport p h e =>
      match whnfFuel fuel e with
      | .refl _ => whnfFuel fuel h
      | other => .transport p h other
    | other => other
termination_by fuel

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

mutual

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
      checkAgainst Γ arg expected
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
      checkAgainst Γ branch expected
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
  | .app f x => do
    let fTy ← infer Γ f
    let .pi dom codom := whnf fTy | throw (.not_a_function (whnf fTy))
    checkDomain Γ x dom
    return subst 0 x codom
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
      checkDomain Γ endpoint a
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
    checkDomain Γ base (.app motive lhs)
    return .app motive rhs

/-- Check introductions using an already formed expected type. -/
partial def checkAgainst (Γ : Ctx) (t expected : Term) : Except CheckError Unit := do
  if let .letE value body := t then
    let _ ← infer Γ value
    return ← checkAgainst Γ (subst 0 value body) expected
  match t, whnf expected with
  | .lam domain body, .pi expectedDomain codomain =>
    let kind ← infer Γ domain
    let some _ := expectUniverse kind | throw (.not_a_type domain)
    unless defEq domain expectedDomain do throw (.type_mismatch expectedDomain domain)
    checkAgainst (domain :: Γ) body codomain
  | .pair first second, .sigma domain family =>
    checkAgainst Γ first domain
    checkAgainst Γ second (subst 0 first family)
  | .refl value, .identity carrier lhs rhs =>
    checkDomain Γ value carrier
    unless defEq value lhs && defEq value rhs do
      throw (.type_mismatch expected (.identity carrier value value))
  | _, _ =>
    let inferred ← infer Γ t
    unless defEq inferred expected do throw (.type_mismatch expected inferred)

partial def checkDomain (Γ : Ctx) (t expected : Term) : Except CheckError Unit :=
  match checkAgainst Γ t expected with
  | .error (.type_mismatch wanted actual) => .error (.domain_mismatch wanted actual)
  | result => result

end

/-- Check the expected type's formation before entering checking mode. -/
partial def check (Γ : Ctx) (t expected : Term) : Except CheckError Unit := do
  checkScope Γ.length expected
  checkScope Γ.length t
  match infer Γ expected with
  | .ok kind =>
    let some _ := expectUniverse kind | throw (.not_a_type expected)
  | .error (.universe_overflow _) =>
    -- A literal universe remains a type at the wire-level ceiling.
    -- Overflow inside a compound type cannot establish its formation.
    if let .universe _ := expected then pure ()
    else throw (.not_a_type expected)
  | .error error => throw error
  checkAgainst Γ t expected

/-- Verify a closed `(term, claimed_type)` certificate. -/
def verifyCertificate (term claimedType : Term) : Except CheckError Unit := do
  checkScope 0 claimedType
  checkScope 0 term
  match infer [] claimedType with
  | .ok kind =>
    let some _ := expectUniverse kind | throw (.claimed_type_not_a_type claimedType kind)
  | .error (.universe_overflow n) =>
    if let .universe _ := claimedType then pure ()
    else throw (.universe_overflow n)
  | .error error => throw error
  checkAgainst [] term claimedType

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
