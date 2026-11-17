import VerumExternalReplay.FunctionExtensionality

/-! ============================================================== -/
/-! # Verum reference kernel checker                                 -/
/-! ============================================================== -/

/-! Independent re-implementation of the structural fragment of
    `crates/verum_kernel/src/proof_checker.rs` (Var / Universe /
    Pi / Lam / App / Sigma / Id / Refl / J, plus β-reduction, α-equivalence, η-equivalence,
    exact universe arithmetic, and post-FV-19 universe
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
    side uses `Nat` for storage and enforces Rust `u32` bounds on wire
    literals; structured levels can denote larger values exactly. Equality compares
    the maximum of the constant and shifted parameter terms under every natural
    valuation. `UniverseLevelEquality` proves this criterion for the function below. -/

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

/-- Value fields use the surrounding context; dependent fields also see
    every earlier constructor field. Recursive occurrences remain explicit. -/
inductive DataField (T : Type) where
  | value : T → DataField T
  | dependent : T → DataField T
  | recursive : DataField T
  | recursiveAt : List T → DataField T
  deriving Inhabited, Repr

structure DataConstructor (T : Type) where
  fields : List (DataField T)
  indices : List T
  deriving Inhabited, Repr

/-- Native certificate terms: dependent functions and pairs, with
    intensional equality and transport. -/
inductive Term : Type where
  | data (name : String) (level : Level) (indices : List Term) (constructors : List (DataConstructor Term))
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
  | idElim (motive base path : Term)
  | funExt (functionType lhs rhs pointwise : Term)
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
  | foundation_required
  | fuel_exhausted
  deriving Repr

inductive Foundation where
  | intensional
  | functionExtensionality
  deriving BEq, Inhabited, Repr

/-- A local definition retains its checked source in the frame where it was introduced. -/
structure Binding where
  ty : Term
  value : Option Term := none

structure Ctx where
  bindings : List Binding := []
  foundation : Foundation := .intensional

def Ctx.length (ctx : Ctx) : Nat := ctx.bindings.length
def Ctx.extend (ctx : Ctx) (ty : Term) : Ctx :=
  { ctx with bindings := ⟨ty, none⟩ :: ctx.bindings }
def Ctx.define (ctx : Ctx) (ty value : Term) : Ctx :=
  { ctx with bindings := ⟨ty, some value⟩ :: ctx.bindings }

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

def Level.addOffset (base : Level) (by_ : Nat) : Level :=
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
termination_by structural base

/-- Exact successor: bounded wire literals do not cap the universe hierarchy. -/
def Level.successor (l : Level) : Level :=
  match l with
  | .concrete n => if n < Level.wireMax then .concrete (n + 1) else .succ l
  | other => .succ other

/-- Common-prefix budget, capped at the wire integer width. This bound is
    only a factoring budget; it never saturates a universe's value. -/
def Level.succDepth : Level → Nat
  | .succ inner => min Level.wireMax (1 + Level.succDepth inner)
  | .offset inner by_ => min Level.wireMax (by_ + Level.succDepth inner)
  | _ => 0

/-- Strip an available prefix by visiting syntax, never by counting to its value. -/
def Level.stripSucc (l : Level) (count : Nat) : Level :=
  if count = 0 then l
  else match l with
    | .succ inner => Level.stripSucc inner (count - 1)
    | .offset inner by_ =>
      if by_ ≤ count then Level.stripSucc inner (count - by_)
      else Level.addOffset inner (by_ - count)
    | other => other
termination_by structural l

/-- Flatten a (possibly-nested) `max` into a flat list of summands. -/
def Level.flattenMax (l : Level) : List Level :=
  match l with
  | .max a b => Level.flattenMax a ++ Level.flattenMax b
  | other    => [other]

/-- Reassemble a `max` from a concrete summand (if any) and a list
    of symbolic summands. -/
def Level.assembleMax (concrete : Option Nat) (symbolic : List Level) : Level :=
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

/-- A constant or a parameter plus a natural offset. -/
structure Level.Atom where
  parameter : Option String
  offset : Nat
  deriving DecidableEq, Repr

/-- Distribute each shift only along its source paths. A large wire offset
    contributes one addition regardless of its numerical value. -/
def Level.atomsFrom (shift : Nat) : Level → List Level.Atom
  | .concrete n => [⟨none, shift + n⟩]
  | .var x => [⟨some x, shift⟩]
  | .succ base => base.atomsFrom (shift + 1)
  | .offset base amount => base.atomsFrom (shift + amount)
  | .max left right => left.atomsFrom shift ++ right.atomsFrom shift

/-- Coverage for every nonnegative valuation. A constant needs only a large
    enough offset, while a parameter needs an offset on the same name. -/
def Level.Atom.coveredBy (a b : Level.Atom) : Bool :=
  decide (a.offset ≤ b.offset ∧ (a.parameter = none ∨ a.parameter = b.parameter))

def Level.leEverywhere (a b : Level) : Bool :=
  let right := b.atomsFrom 0
  (a.atomsFrom 0).all (fun x => right.any (fun y => x.coveredBy y))

/-- Exact universal equality, independent of the chosen wire representation. -/
def Level.eq (a b : Level) : Bool := a.leEverywhere b && b.leEverywhere a

/-- Drop a dominated maximum branch. For incomparable branches retain both.
    The reference checker may choose a different expression from native replay. -/
def Level.maxCanonical (a b : Level) : Level :=
  if a.leEverywhere b then b else if b.leEverywhere a then a else .max a b

/-- Executable simplification. Equality is decided by the complete valuation
    criterion above and does not require a particular ordering of branches. -/
def Level.normalize : Level → Level
  | .concrete n => .concrete n
  | .var name => .var name
  | .succ base => Level.addOffset base.normalize 1
  | .offset base by_ => Level.addOffset base.normalize by_
  | .max a b => Level.maxCanonical a.normalize b.normalize

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
  | .funExt a b c d =>
    (scopeError bound a).or ((scopeError bound b).or ((scopeError bound c).or (scopeError bound d)))
  | .identity a b c | .transport a b c | .idElim a b c =>
    (scopeError bound a).or ((scopeError bound b).or (scopeError bound c))
  | .data _ _ indices constructors => (scopeTelescope bound indices).or (scopeConstructors bound constructors)
  | .construct data _ args => (scopeError bound data).or (scopeTerms bound args)
  | .induct value motive cases =>
    (scopeError bound value).or ((scopeError bound motive).or (scopeTerms bound cases))
termination_by sizeOf t

def scopeTerms (bound : Nat) (terms : List Term) : Option Nat :=
  match terms with
  | [] => none
  | term :: rest => (scopeError bound term).or (scopeTerms bound rest)
termination_by sizeOf terms

def scopeTelescope (bound : Nat) (types : List Term) : Option Nat :=
  match types with
  | [] => none
  | ty :: rest => (scopeError bound ty).or (scopeTelescope (bound + 1) rest)
termination_by sizeOf types

def scopeFields (bound : Nat) (fields : List (DataField Term)) (preceding : Nat := 0) : Option Nat :=
  match fields with
  | [] => none
  | .recursive :: rest => scopeFields bound rest (preceding + 1)
  | .value term :: rest => (scopeError bound term).or (scopeFields bound rest (preceding + 1))
  | .dependent term :: rest =>
    (scopeError (bound + preceding) term).or (scopeFields bound rest (preceding + 1))
  | .recursiveAt indices :: rest =>
    (scopeTerms (bound + preceding) indices).or (scopeFields bound rest (preceding + 1))
termination_by sizeOf fields

def scopeConstructors (bound : Nat) (constructors : List (DataConstructor Term)) : Option Nat :=
  match constructors with
  | [] => none
  | ⟨fields, indices⟩ :: rest => (scopeFields bound fields).or
    ((scopeTerms (bound + fields.length) indices).or (scopeConstructors bound rest))
termination_by sizeOf constructors

end

def checkScope (bound : Nat) (t : Term) : Except CheckError Unit :=
  match scopeError bound t with
  | none => .ok ()
  | some index => .error (.unbound_variable index)

/- One total traversal owns the binding discipline for renaming and
    substitution. Dependent fields bind each earlier field; value fields,
    arguments and induction branches introduce no implicit binders; let bodies, Pi, Lam and Sigma introduce one.
    The explicit nested-list helpers expose decreasing structural recursion. -/
mutual

def mapVars (replace : Nat → Nat → Term) (cutoff : Nat) (t : Term) : Term :=
  match t with
  | .letE value body => .letE (mapVars replace cutoff value) (mapVars replace (cutoff+1) body)
  | .data name level indices constructors => .data name level (mapTelescope replace cutoff indices) (mapConstructors replace cutoff constructors)
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
  | .idElim p h e => .idElim (mapVars replace cutoff p) (mapVars replace cutoff h) (mapVars replace cutoff e)
  | .funExt a b c d => .funExt (mapVars replace cutoff a) (mapVars replace cutoff b)
      (mapVars replace cutoff c) (mapVars replace cutoff d)
termination_by sizeOf t

def mapTerms (replace : Nat → Nat → Term) (cutoff : Nat) (terms : List Term) : List Term :=
  match terms with
  | [] => []
  | term :: rest => mapVars replace cutoff term :: mapTerms replace cutoff rest
termination_by sizeOf terms

def mapTelescope (replace : Nat → Nat → Term) (cutoff : Nat) (types : List Term) : List Term :=
  match types with
  | [] => []
  | ty :: rest => mapVars replace cutoff ty :: mapTelescope replace (cutoff + 1) rest
termination_by sizeOf types

def mapFields (replace : Nat → Nat → Term) (cutoff : Nat)
    (fields : List (DataField Term)) (preceding : Nat := 0) : List (DataField Term) :=
  match fields with
  | [] => []
  | .recursive :: rest => .recursive :: mapFields replace cutoff rest (preceding + 1)
  | .value term :: rest => .value (mapVars replace cutoff term) :: mapFields replace cutoff rest (preceding + 1)
  | .dependent term :: rest =>
    .dependent (mapVars replace (cutoff + preceding) term) :: mapFields replace cutoff rest (preceding + 1)
  | .recursiveAt indices :: rest =>
    .recursiveAt (mapTerms replace (cutoff + preceding) indices) :: mapFields replace cutoff rest (preceding + 1)
termination_by sizeOf fields

def mapConstructors (replace : Nat → Nat → Term) (cutoff : Nat)
    (constructors : List (DataConstructor Term)) : List (DataConstructor Term) :=
  match constructors with
  | [] => []
  | ⟨fields, indices⟩ :: rest =>
    ⟨mapFields replace cutoff fields, mapTerms replace (cutoff + fields.length) indices⟩ ::
      mapConstructors replace cutoff rest
termination_by sizeOf constructors

end

theorem mapFields_length (replace : Nat → Nat → Term) (cutoff : Nat)
    (fields : List (DataField Term)) (preceding : Nat := 0) :
    (mapFields replace cutoff fields preceding).length = fields.length := by
  induction fields generalizing preceding with
  | nil => simp only [mapFields, List.length_nil]
  | cons field rest ih => cases field <;> simp only [mapFields, List.length_cons, ih]

/-- Raise every variable at or above the cutoff; universe levels are unchanged. -/
def shiftUp (amount cutoff : Nat) (t : Term) : Term :=
  mapVars (fun cutoff i => if i ≥ cutoff then .var (i + amount) else .var i) cutoff t

/-- Apply an outer function to the fresh variable of an extended context. -/
def etaBody (f : Term) : Term := .app (shiftUp 1 0 f) (.var 0)

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

/-- Conversion can descend under fresh binders before their types enter the checking context. -/
structure Conversion where
  bindings : List Binding := []
  depth : Nat := 0

def Ctx.conversion (ctx : Ctx) : Conversion := { bindings := ctx.bindings }

def Conversion.under (scope : Conversion) (count : Nat) : Conversion :=
  { scope with depth := scope.depth + count }

/-- Protect the fresh binders and lift a referenced source into the use-site frame. -/
def Conversion.definition (scope : Conversion) (i : Nat) : Option Term :=
  if scope.depth ≤ i then
    scope.bindings[i - scope.depth]?.bind fun binding => binding.value.map (shiftUp (i + 1) 0)
  else none

/-- Apply a constructor branch without allocating a zipped field/argument list.
    Recursive hypotheses follow their own field immediately. The reducer checks
    arity first; truncation on malformed input matches `zip` exactly. -/
def applyInductionCase (motive : Term) (cases : List Term)
    (fields : List (DataField Term)) (args : List Term) (branch : Term) : Term :=
  match fields, args with
  | field :: rest, arg :: tail =>
    let next := Term.app branch arg
    let applied := match field with
      | .recursive | .recursiveAt _ => Term.app next (.induct arg motive cases)
      | .value _ | .dependent _ => next
    applyInductionCase motive cases rest tail applied
  | _, _ => branch
termination_by structural fields

/-- Fuel ceiling for whnf. -/
def whnfFuelCeiling : Nat := 1 <<< 20

/-- Structurally fuel-bounded independent reduction. This bounds recursive
reduction depth, not Rust's cumulative head-step counter. Exhaustion yields no
term and therefore cannot become a semantic mismatch in either checker. -/
def whnfFuel (fuel : Nat) (t : Term) (definition : Nat → Option Term := fun _ => none) :
    Except CheckError Term :=
  match fuel with
  | 0 => .error .fuel_exhausted
  | fuel + 1 => do
    let reduce := fun term => whnfFuel fuel term definition
    match t with
    | .var i => match definition i with
      | some value => reduce value
      | none => pure (.var i)
    | .letE value body => reduce (subst 0 value body)
    | .induct value motive cases =>
      let head ← reduce value
      match head with
      | .construct data ctor args =>
        match ← reduce data with
        | .data _ _ _ constructors =>
          match constructors[ctor]?, cases[ctor]? with
          | some fields, some branch =>
            if fields.fields.length == args.length then
              reduce (applyInductionCase motive cases fields.fields args branch)
            else pure (.induct head motive cases)
          | _, _ => pure (.induct head motive cases)
        | _ => pure (.induct head motive cases)
      | _ => pure (.induct head motive cases)
    | .app f x =>
      match ← reduce f with
      | .lam _ body => reduce (subst 0 x body)
      | other => pure (.app other x)
    | .fst p =>
      match ← reduce p with
      | .pair a _ => reduce a
      | other => pure (.fst other)
    | .snd p =>
      match ← reduce p with
      | .pair _ b => reduce b
      | other => pure (.snd other)
    | .transport p h e =>
      match ← reduce e with
      | .refl _ => reduce h
      | other => pure (.transport p h other)
    | .idElim p h e =>
      match ← reduce e with
      | .refl _ => reduce h
      | other => pure (.idElim p h other)
    | other => pure other
termination_by fuel

/-- Public reduction never exposes an exhausted partial normal form. -/
def whnf (t : Term) (scope : Conversion := {}) : Except CheckError Term :=
  whnfFuel whnfFuelCeiling t scope.definition

/-- Is `var target` free in `t`? -/
def fieldValueType (field : DataField Term) (preceding : Nat) : Option Term :=
  match field with
  | .value ty => some (shiftUp preceding 0 ty)
  | .dependent ty => some ty
  | .recursive | .recursiveAt _ => none

/-- Eliminate a telescope using values in the common ambient context. -/
def instantiateFields (term : Term) (arguments : List Term) : Term :=
  arguments.zipIdx.foldr (fun (arg, remaining) result => subst 0 (shiftUp remaining 0 arg) result) term

def recursiveIndices : DataField Term → Option (List Term)
  | .recursive => some []
  | .recursiveAt indices => some indices
  | _ => none

def applyTerms (head : Term) (arguments : List Term) : Term := arguments.foldl Term.app head

def fieldDomain (data : Term) (field : DataField Term) (preceding : Nat) : Term :=
  (fieldValueType field preceding).getD (applyTerms (shiftUp preceding 0 data) ((recursiveIndices field).getD []))

partial def splitFamily (scope : Conversion) (term : Term) (values : List Term := []) : Except CheckError (Option (Term × List Term)) := do
  match term with
  | .app head arg => splitFamily scope (← whnf head scope) (arg :: values)
  | .data _ _ indices _ => return if indices.length == values.length then some (term, values) else none
  | _ => return none

/-- Short-circuit on a completed mismatch; never erase an operational error. -/
def allChecked (items : List α) (check : α → Except CheckError Bool) : Except CheckError Bool := do
  match items with
  | [] => pure true
  | item :: rest => if ← check item then allChecked rest check else pure false

mutual

/-- α + β + η-equality after completed WHNF reduction. -/
partial def defEq (a b : Term) (scope : Conversion := {}) : Except CheckError Bool := do
  let aw ← whnf a scope
  let bw ← whnf b scope
  defEqWhnf scope aw bw

partial def defEqWhnf (scope : Conversion) : Term → Term → Except CheckError Bool
  | .data an al ai ac, .data bn bl bi bc => do
    unless an == bn && Level.eq al bl do return false
    unless ai.length == bi.length do return false
    unless ← allChecked (ai.zip bi).zipIdx (fun ((x,y), i) => defEq x y (scope.under i)) do return false
    unless ac.length == bc.length do return false
    allChecked (ac.zip bc) (fun (a,b) => do
      unless a.fields.length == b.fields.length do return false
      unless ← termsDefEq a.indices b.indices (scope.under a.fields.length) do return false
      allChecked (a.fields.zip b.fields).zipIdx (fun ((x,y), index) =>
        match recursiveIndices x, recursiveIndices y with
        | some x, some y => termsDefEq x y (scope.under index)
        | none, none => match fieldValueType x index, fieldValueType y index with
          | some x, some y => defEq x y (scope.under index)
          | _, _ => pure false
        | _, _ => pure false))
  | .construct a ai aa, .construct b bi ba => do
    unless ai == bi do return false
    unless ← defEq a b scope do return false
    termsDefEq aa ba scope
  | .induct a am ac, .induct b bm bc => do
    unless ← defEq a b scope do return false
    unless ← defEq am bm scope do return false
    termsDefEq ac bc scope
  | .var i, .var j => pure (i == j)
  | .universe l1, .universe l2 => pure (Level.eq l1 l2)
  | .pi a b, .pi c d | .lam a b, .lam c d | .sigma a b, .sigma c d => do
    unless ← defEq a c scope do return false
    defEq b d (scope.under 1)
  | .app a b, .app c d | .pair a b, .pair c d => termsDefEq [a,b] [c,d] scope
  | .pair a b, other | other, .pair a b => termsDefEq [a,b] [.fst other,.snd other] scope
  | .fst a, .fst b | .snd a, .snd b | .refl a, .refl b => defEq a b scope
  | .identity a b c, .identity d e f | .transport a b c, .transport d e f
    | .idElim a b c, .idElim d e f => termsDefEq [a,b,c] [d,e,f] scope
  | .funExt a b c d, .funExt e f g h => termsDefEq [a,b,c,d] [e,f,g,h] scope
  | .lam _ body, other | other, .lam _ body => etaMatch scope body other
  | _, _ => pure false

partial def termsDefEq (a b : List Term) (scope : Conversion) : Except CheckError Bool := do
  unless a.length == b.length do return false
  allChecked (a.zip b) (fun (a,b) => defEq a b scope)

/-- Compare in the body context. The outer function is weakened before application. -/
partial def etaMatch (scope : Conversion) (body other : Term) : Except CheckError Bool :=
  defEq body (etaBody other) (scope.under 1)

end

/-- Look up the type of `var i` in `Γ`, shifted to the use-site frame. -/
def lookupCtx (Γ : Ctx) (i : Nat) : Option Term :=
  Γ.bindings[i]?.map (fun binding => shiftUp (i + 1) 0 binding.ty)

/-- If `t` whnf-reduces to `Universe l`, return the normalised level. -/
def expectUniverse (t : Term) (scope : Conversion) : Except CheckError (Option Level) := do
  match ← whnf t scope with
  | .universe l => pure (some (Level.normalize l))
  | _ => pure none

/-- Independently construct each case telescope from explicit recursive slots.
    Captured parameters remain in the surrounding de Bruijn context. -/
def rebindFields (term : Term) (positions : List Nat) (depth : Nat) : Term :=
  instantiateFields (shiftUp depth positions.length term) (positions.map (fun p => .var (depth - 1 - p)))

def inductionCaseType (data motive : Term) (ctor : Nat) (constructor : DataConstructor Term) : Term := Id.run do
  let mut domains : List Term := []
  let mut positions : List Nat := []
  for field in constructor.fields do
    let depth := domains.length
    let domain := rebindFields (fieldDomain data field positions.length) positions depth
    domains := domains ++ [domain]
    if let some indices := recursiveIndices field then
      let arguments := indices.map (fun t => rebindFields t positions (depth + 1))
      domains := domains ++ [.app (applyTerms (shiftUp (depth + 1) 0 motive) arguments) (.var 0)]
    positions := positions ++ [depth]
  let depth := domains.length
  let indices := constructor.indices.map (fun t => rebindFields t positions depth)
  let value := Term.construct (shiftUp depth 0 data) ctor (positions.map (fun i => .var (depth - 1 - i)))
  return domains.foldr Term.pi (.app (applyTerms (shiftUp depth 0 motive) indices) value)

mutual

/-- Bidirectional type inference for the structural fragment. -/
partial def infer (Γ : Ctx) (t : Term) : Except CheckError Term :=
  match t with
  | .funExt functionType lhs rhs pointwise => do
    unless Γ.foundation == .functionExtensionality do throw .foundation_required
    let some _ ← expectUniverse (← infer Γ functionType) Γ.conversion | throw (.not_a_type functionType)
    let .pi domain codomain ← whnf functionType Γ.conversion | throw (.not_a_function functionType)
    checkAgainst Γ lhs functionType
    checkAgainst Γ rhs functionType
    let premise := .pi domain (.identity codomain
      (.app (shiftUp 1 0 lhs) (.var 0)) (.app (shiftUp 1 0 rhs) (.var 0)))
    checkAgainst Γ pointwise premise
    return .identity functionType lhs rhs
  | .letE value body => do
    let ty ← infer Γ value
    -- The body shares its checked definition. Only its resulting type leaves this frame.
    let bodyType ← infer (Γ.define ty value) body
    return subst 0 value bodyType
  | .data _ level indices constructors => do
    let mut indexContext := Γ
    for ty in indices do
      let kind ← infer indexContext ty
      let some _ ← expectUniverse kind indexContext.conversion | throw (.not_a_type ty)
      indexContext := indexContext.extend ty
    for constructor in constructors do
      let mut fieldContext := Γ
      for (field, index) in constructor.fields.zipIdx do
        let domain := fieldDomain t field index
        if let some values := recursiveIndices field then
          checkIndices fieldContext indices values index
        else
          let kind ← infer fieldContext domain
          let some fieldLevel ← expectUniverse kind fieldContext.conversion | throw (.not_a_type domain)
          unless Level.eq (Level.maxCanonical fieldLevel level) level do
            throw (.invalid_inductive "field exceeds declaration universe")
        fieldContext := fieldContext.extend domain
      checkIndices fieldContext indices constructor.indices constructor.fields.length
    return indices.foldr Term.pi (.universe (Level.normalize level))
  | .construct data ctor args => do
    let _ ← infer Γ data
    let declaration ← whnf data Γ.conversion
    let .data _ _ _ constructors := declaration | throw (.invalid_inductive "constructor owner is not a data declaration")
    let some constructor := constructors[ctor]? | throw (.invalid_inductive "unknown constructor")
    unless constructor.fields.length == args.length do throw (.invalid_inductive "constructor field arity mismatch")
    for ((field, arg), index) in (constructor.fields.zip args).zipIdx do
      checkAgainst Γ arg (instantiateFields (fieldDomain declaration field index) (args.take index))
    return applyTerms declaration (constructor.indices.map (fun t => instantiateFields t args))
  | .induct value motive cases => do
    let actual ← whnf (← infer Γ value) Γ.conversion
    let some (declaration, values) ← splitFamily Γ.conversion actual | throw (.invalid_inductive "induction requires a fully applied data family")
    let .data _ _ indices constructors := declaration | throw (.invalid_inductive "invalid family head")
    let _ ← infer Γ declaration
    let mut motiveKind ← infer Γ motive
    for (ty, i) in indices.zipIdx do
      let scope := Γ.conversion.under i
      let .pi domain result ← whnf motiveKind scope | throw (.invalid_inductive "induction motive omits a family index")
      unless ← defEq domain ty scope do throw (.invalid_inductive "induction motive index domain mismatch")
      motiveKind := result
    let scope := Γ.conversion.under indices.length
    let .pi domain result ← whnf motiveKind scope | throw (.invalid_inductive "induction motive is not a family")
    let references := (List.range indices.length).reverse.map Term.var
    let expectedDomain := applyTerms (shiftUp indices.length 0 declaration) references
    unless ← defEq domain expectedDomain scope do
      throw (.invalid_inductive "induction motive domain mismatch")
    unless (← expectUniverse result (scope.under 1)).isSome do
      throw (.invalid_inductive "induction motive domain or universe mismatch")
    unless cases.length == constructors.length do throw (.invalid_inductive "induction case count mismatch")
    for ((constructor, branch), index) in (constructors.zip cases).zipIdx do
      checkAgainst Γ branch (inductionCaseType declaration motive index constructor)
    return .app (applyTerms motive values) value
  | .var i =>
    match lookupCtx Γ i with
    | some T => Except.ok T
    | none   => Except.error (.unbound_variable i)
  | .universe l => .ok (.universe (Level.successor (Level.normalize l)))
  | .pi a b => do
    let some n ← expectUniverse (← infer Γ a) Γ.conversion | throw (.not_a_type a)
    let some m ← expectUniverse (← infer (Γ.extend a) b) (Γ.conversion.under 1) | throw (.not_a_type b)
    return .universe (Level.maxCanonical n m)
  | .lam a body => do
    let some _ ← expectUniverse (← infer Γ a) Γ.conversion | throw (.not_a_type a)
    return .pi a (← infer (Γ.extend a) body)
  | .app f x => do
    let fTy ← infer Γ f
    let head ← whnf fTy Γ.conversion
    let .pi dom codom := head | throw (.not_a_function head)
    checkDomain Γ x dom
    return subst 0 x codom
  -- T-Sigma-Form: Σ(x : A). B : Universe(max(level(A), level(B))).
  | .sigma a b => do
    let some n ← expectUniverse (← infer Γ a) Γ.conversion | throw (.not_a_type a)
    let some m ← expectUniverse (← infer (Γ.extend a) b) (Γ.conversion.under 1) | throw (.not_a_type b)
    return .universe (Level.maxCanonical n m)
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
  | .fst p => do
    match ← whnf (← infer Γ p) Γ.conversion with
    | .sigma a _ => pure a
    | other => throw (.not_a_sigma other)
  | .snd p => do
    match ← whnf (← infer Γ p) Γ.conversion with
    | .sigma _ b => pure (subst 0 (.fst p) b)
    | other => throw (.not_a_sigma other)
  | .identity a x y => do
    let kind ← infer Γ a
    let some level ← expectUniverse kind Γ.conversion | .error (.not_a_type a)
    for endpoint in [x, y] do
      checkDomain Γ endpoint a
    return .universe level
  | .refl x => do
    let ty ← infer Γ x
    return .identity ty x x
  | .transport motive base path => do
    let pathTy ← infer Γ path
    let head ← whnf pathTy Γ.conversion
    let .identity carrier lhs rhs := head | .error (.not_an_identity head)
    let motiveTy ← infer Γ motive
    let .pi domain codomain ← whnf motiveTy Γ.conversion | .error (.not_a_valid_j_motive motive)
    unless ← defEq domain carrier Γ.conversion do throw (.domain_mismatch carrier domain)
    let some _ ← expectUniverse codomain (Γ.conversion.under 1) | .error (.not_a_valid_j_motive motive)
    checkDomain Γ base (.app motive lhs)
    return .app motive rhs
  | .idElim motive base path => do
    let .identity carrier lhs rhs ← whnf (← infer Γ path) Γ.conversion | .error (.not_an_identity path)
    let .pi domain codomain ← whnf (← infer Γ motive) Γ.conversion | .error (.not_a_valid_j_motive motive)
    unless ← defEq domain carrier Γ.conversion do throw (.domain_mismatch carrier domain)
    let .pi proofDomain resultKind ← whnf codomain (Γ.conversion.under 1) | .error (.not_a_valid_j_motive motive)
    let expectedPath := .identity (shiftUp 1 0 carrier) (shiftUp 1 0 lhs) (.var 0)
    unless ← defEq proofDomain expectedPath (Γ.conversion.under 1) do throw (.domain_mismatch expectedPath proofDomain)
    let some _ ← expectUniverse resultKind (Γ.conversion.under 2) | .error (.not_a_valid_j_motive motive)
    checkDomain Γ base (.app (.app motive lhs) (.refl lhs))
    return .app (.app motive rhs) path


/-- Check introductions using an already formed expected type. -/
partial def checkAgainst (Γ : Ctx) (t expected : Term) : Except CheckError Unit := do
  if let .letE value body := t then
    let ty ← infer Γ value
    return ← checkAgainst (Γ.define ty value) body (shiftUp 1 0 expected)
  if let .var i := t then
    let some ty := lookupCtx Γ i | throw (.unbound_variable i)
    if ← defEq ty expected Γ.conversion then return ()
    -- A concrete introduction can check at a more dependent type than it synthesizes.
    if let some source := Γ.conversion.definition i then
      return ← checkAgainst Γ source expected
    throw (.type_mismatch expected ty)
  match t, (← whnf expected Γ.conversion) with
  | .lam domain body, .pi expectedDomain codomain =>
    let kind ← infer Γ domain
    let some _ ← expectUniverse kind Γ.conversion | throw (.not_a_type domain)
    unless ← defEq domain expectedDomain Γ.conversion do throw (.type_mismatch expectedDomain domain)
    checkAgainst (Γ.extend domain) body codomain
  | .pair first second, .sigma domain family =>
    checkAgainst Γ first domain
    checkAgainst Γ second (subst 0 first family)
  | .refl value, .identity carrier lhs rhs =>
    checkDomain Γ value carrier
    unless ← termsDefEq [value,value] [lhs,rhs] Γ.conversion do
      throw (.type_mismatch expected (.identity carrier value value))
  | _, _ =>
    let inferred ← infer Γ t
    unless ← defEq inferred expected Γ.conversion do throw (.type_mismatch expected inferred)

partial def checkIndices (Γ : Ctx) (telescope arguments : List Term) (ambientShift : Nat) : Except CheckError Unit := do
  unless telescope.length == arguments.length do throw (.invalid_inductive "family index arity mismatch")
  for ((ty,value), index) in (telescope.zip arguments).zipIdx do
    checkAgainst Γ value (instantiateFields (shiftUp ambientShift index ty) (arguments.take index))

partial def checkDomain (Γ : Ctx) (t expected : Term) : Except CheckError Unit :=
  match checkAgainst Γ t expected with
  | .error (.type_mismatch wanted actual) => .error (.domain_mismatch wanted actual)
  | result => result

end

/-- Check the expected type's formation before entering checking mode. -/
partial def check (Γ : Ctx) (t expected : Term) : Except CheckError Unit := do
  checkScope Γ.length expected
  checkScope Γ.length t
  let kind ← infer Γ expected
  let some _ ← expectUniverse kind Γ.conversion | throw (.not_a_type expected)
  checkAgainst Γ t expected

/-- Verify a closed `(term, claimed_type)` certificate. -/
def verifyCertificate (term claimedType : Term)
    (foundation : Foundation := .intensional) : Except CheckError Unit := do
  let ctx : Ctx := { foundation }
  checkScope 0 claimedType
  checkScope 0 term
  let kind ← infer ctx claimedType
  let some _ ← expectUniverse kind ctx.conversion | throw (.claimed_type_not_a_type claimedType kind)
  checkAgainst ctx term claimedType

end VerumKernel

/-! Sanity tests — mirror the Rust-side `proof_checker::tests`,
    including the FV-19 universe-polymorphism battery. -/

namespace VerumKernel.Tests

open VerumKernel

/-- Positive smoke assertion: every error, including exhaustion, fails. -/
def expectInfer (Γ : Ctx) (t expected : Term) : Bool :=
  match infer Γ t with
  | .ok inferred => match defEq inferred expected Γ.conversion with
    | .ok true => true
    | _ => false
  | _            => false

def expectInferError (Γ : Ctx) (t : Term) (matchErr : CheckError → Bool) : Bool :=
  match infer Γ t with
  | .error e => matchErr e
  | _        => false

-- Helper: build a concrete-level Universe term.
def univ (n : Nat) : Term := .universe (.concrete n)

-- Helper: build a level-variable Universe term.
def univVar (name : String) : Term := .universe (.var name)

/-- info: true -/
#guard_msgs in
#eval! expectInfer {} (univ 0) (univ 1)
  -- T-Univ — true

/-- info: true -/
#guard_msgs in
#eval! expectInferError {} (.var 0) (fun e => match e with | .unbound_variable 0 => true | _ => false)
  -- T-Var unbound — true

/-- info: true -/
#guard_msgs in
#eval! expectInfer (({} : Ctx).extend (univ 0)) (.var 0) (univ 0)
  -- T-Var with hypothesis — true

/-- info: true -/
#guard_msgs in
#eval! expectInfer {} (.pi (univ 2) (univ 5)) (univ 6)
  -- T-Pi-Form max(3, 6) = 6 — true

/-- info: true -/
#guard_msgs in
#eval! expectInfer {}
        (.lam (univ 0) (.var 0))
        (.pi (univ 0) (univ 0))
  -- T-Lam-Intro identity at Universe(0) — true

/-- info: true -/
#guard_msgs in
#eval! expectInferError {}
        (.app (univ 0) (univ 0))
        (fun e => match e with | .not_a_function _ => true | _ => false)
  -- T-App-Elim rejects non-function — true

/-- info: true -/
#guard_msgs in
#eval! expectInfer {} (univ 1000) (univ 1001)
  -- universe successor (Nat-unbounded on Lean side) — true

/-- info: true -/
#guard_msgs in
#eval! (
  let id := Term.lam (univ 0) (.var 0)
  match verifyCertificate id id with
  | .error (.claimed_type_not_a_type _ _) => true
  | _                                      => false)
  -- DEFECT-4 mirror: claimed-type-not-a-type rejection — true

-- =============================================================================
-- FV-19 universe-polymorphism battery (mirrors proof_checker::tests)
-- =============================================================================

/-- info: true -/
#guard_msgs in
#eval! (Level.eq
        (Level.max (Level.var "u") (Level.var "v"))
        (Level.max (Level.var "v") (Level.var "u")))
  -- max commutativity via canonical form — true

/-- info: true -/
#guard_msgs in
#eval! (Level.eq
        (Level.max (Level.var "u") (Level.var "u"))
        (Level.var "u"))
  -- max idempotency — true

/-- info: true -/
#guard_msgs in
#eval! (Level.eq
        (Level.max Level.zero (Level.var "u"))
        (Level.var "u"))
  -- zero-identity — true

/-- info: true -/
#guard_msgs in
#eval! (Level.eq
        (Level.max (Level.succ (Level.var "u")) (Level.succ (Level.var "v")))
        (Level.succ (Level.max (Level.var "u") (Level.var "v"))))
  -- common-succ factoring — true

/-- info: true -/
#guard_msgs in
#eval! expectInfer {}
        (univVar "u")
        (.universe (.succ (.var "u")))
  -- T-Univ on a level variable: Universe(u) : Universe(succ(u)) — true

/-- info: true -/
#guard_msgs in
#eval! expectInfer {}
        (.pi (univVar "u") (univVar "v"))
        (.universe (.succ (Level.maxCanonical (.var "u") (.var "v"))))
  -- T-Pi-Form on polymorphic levels: Π(_:Type@u). Type@v : Type@succ(max(u,v)) — true

/-- info: true -/
#guard_msgs in
#eval! expectInfer {}
        (.lam (univVar "u") (.lam (.var 0) (.var 0)))
        (.pi (univVar "u") (.pi (.var 0) (.var 1)))
  -- Polymorphic identity at Type@u — true

/-- info: true -/
#guard_msgs in
#eval! (
  let term := Term.lam (univVar "u") (.lam (.var 0) (.var 0))
  let claim := Term.pi (univVar "u") (.pi (.var 0) (.var 1))
  match verifyCertificate term claim with
  | .ok _ => true
  | _     => false)
  -- Certificate at Type@u verifies — true

/-- info: true -/
#guard_msgs in
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

/-- info: true -/
#guard_msgs in
#eval! expectInfer {}
        (.sigma (univ 0) (univ 0))
        (univ 1)
  -- T-Sigma-Form at concrete universe — true

/-- info: true -/
#guard_msgs in
#eval! expectInfer {}
        (.sigma (univ 2) (univ 5))
        (univ 6)
  -- T-Sigma-Form takes max universe — true

/-- info: true -/
#guard_msgs in
#eval! expectInfer {}
        (.sigma (univVar "u") (univVar "v"))
        (.universe (.succ (Level.maxCanonical (.var "u") (.var "v"))))
  -- T-Sigma-Form polymorphic — true

/-- info: true -/
#guard_msgs in
#eval! expectInfer {}
        (.pair (univ 0) (univ 0))
        (.sigma (univ 1) (univ 1))
  -- T-Pair-Intro non-dependent — true

/-- info: true -/
#guard_msgs in
#eval! match defEq (.fst (.pair (univ 0) (univ 1))) (univ 0) with
  | .ok true => true
  | _ => false
  -- β-projection: fst(pair(a, b)) ≡ a — true

/-- info: true -/
#guard_msgs in
#eval! match defEq (.snd (.pair (univ 0) (univ 1))) (univ 1) with
  | .ok true => true
  | _ => false
  -- β-projection: snd(pair(a, b)) ≡ b — true

/-- info: true -/
#guard_msgs in
#eval! expectInferError {}
        (.fst (univ 0))
        (fun e => match e with | .not_a_sigma _ => true | _ => false)
  -- T-Fst-Elim rejects non-Σ — true

/-- info: true -/
#guard_msgs in
#eval! expectInferError {}
        (.snd (univ 0))
        (fun e => match e with | .not_a_sigma _ => true | _ => false)
  -- T-Snd-Elim rejects non-Σ — true

/-- info: true -/
#guard_msgs in
#eval! (
  let term := Term.pair (univ 0) (univ 0)
  let claim := Term.sigma (univ 1) (univ 1)
  match verifyCertificate term claim with
  | .ok _ => true
  | _     => false)
  -- Pair certificate at non-dependent Σ verifies — true

/-- info: true -/
#guard_msgs in
#eval! expectInfer {}
        (.sigma (univ 0) (.sigma (univ 0) (univ 0)))
        (univ 1)
  -- Nested Σ — true

end VerumKernel.Tests
