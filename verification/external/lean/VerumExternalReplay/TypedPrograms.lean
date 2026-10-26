import Std.Tactic

/-! Finite, intrinsically typed straight-line programs with shared registers.
Primitive implementations are arbitrary pure partial functions. Translation
expands each primitive into a finite target program. The local correspondence
premise includes exact partial definedness; it is not inferred from examples.

Source and target share an arbitrary universe of sort labels, with separate
value carriers. There are no effects, loops, resource bounds or implicit
assumptions of injectivity. Result reflection below explicitly requires an
injective value translation; the exact execution equation does not.

This is an object-language theorem checked by Lean, not a native Verum
certificate or a correspondence proof for the Verum compiler/runtime.
-/
namespace TypedPrograms
set_option warningAsError true
universe u v w
variable {Ty : Type u}

inductive Ref : List Ty → Ty → Type u where
  | here : Ref (t :: Γ) t
  | there : Ref Γ t → Ref (s :: Γ) t

inductive HList (F : Ty → Type v) : List Ty → Type (max u v) where
  | nil : HList F []
  | cons : F t → HList F Γ → HList F (t :: Γ)

def HList.get {F : Ty → Type v} : HList F Γ → Ref Γ t → F t
  | .cons x _, .here => x
  | .cons _ xs, .there r => xs.get r

def HList.map {F : Ty → Type v} {G : Ty → Type w}
    (f : ∀ {t}, F t → G t) : HList F Γ → HList G Γ
  | .nil => .nil
  | .cons x xs => .cons (f x) (xs.map f)

@[simp] theorem HList.get_map {F : Ty → Type v} {G : Ty → Type w}
    (f : ∀ {t}, F t → G t) (xs : HList F Γ) (r : Ref Γ t) :
    (xs.map f).get r = f (xs.get r) := by
  induction r with
  | here => cases xs; rfl
  | there r ih => cases xs with | cons _ xs => exact ih xs

abbrev Env (V : Ty → Type v) (Γ : List Ty) := ∀ {t}, Ref Γ t → V t

def Env.cons {V : Ty → Type v} (x : V t) (env : Env V Γ) : Env V (t :: Γ)
  | _, .here => x
  | _, .there r => env r

def Env.map {V : Ty → Type v} {W : Ty → Type w}
    (j : ∀ {t}, V t → W t) (env : Env V Γ) : Env W Γ := fun r => j (env r)

theorem Env.map_cons {V : Ty → Type v} {W : Ty → Type w}
    (j : ∀ {t}, V t → W t) (env : Env V Γ) (x : V s) :
    (fun {t} (r : Ref (s :: Γ) t) => Env.map j (Env.cons x env) r) =
      (fun {t} (r : Ref (s :: Γ) t) => Env.cons (j x) (Env.map j env) r) := by
  funext t r
  cases r <;> rfl

inductive Arg (Lit : Ty → Type v) (Γ : List Ty) (t : Ty) : Type (max u v) where
  | reg : Ref Γ t → Arg Lit Γ t
  | literal : Lit t → Arg Lit Γ t

abbrev Subst (Lit : Ty → Type v) (Γ Δ : List Ty) := ∀ {t}, Ref Γ t → Arg Lit Δ t

def Arg.subst {Lit : Ty → Type v} (σ : Subst Lit Γ Δ) : Arg Lit Γ t → Arg Lit Δ t
  | .reg r => σ r
  | .literal l => .literal l

def Arg.weaken {Lit : Ty → Type v} : Arg Lit Γ t → Arg Lit (s :: Γ) t :=
  Arg.subst (fun r => .reg (.there r))

def Subst.lift {Lit : Ty → Type v} (σ : Subst Lit Γ Δ) : Subst Lit (s :: Γ) (s :: Δ)
  | _, .here => .reg .here
  | _, .there r => (σ r).weaken

def Subst.extend {Lit : Ty → Type v} (σ : Subst Lit Γ Δ) (a : Arg Lit Δ s) :
    Subst Lit (s :: Γ) Δ
  | _, .here => a
  | _, .there r => σ r

inductive Program (Prim : List Ty → Ty → Type v) (Lit : Ty → Type v) :
    List Ty → Ty → Type (max u v) where
  | ret : Arg Lit Γ t → Program Prim Lit Γ t
  | call : Prim params s → HList (Arg Lit Γ) params →
      Program Prim Lit (s :: Γ) t → Program Prim Lit Γ t

def Program.subst {Prim : List Ty → Ty → Type v} {Lit : Ty → Type v}
    (p : Program Prim Lit Γ t) (σ : Subst Lit Γ Δ) : Program Prim Lit Δ t :=
  match p with
  | .ret a => .ret (a.subst σ)
  | .call op args rest => .call op (args.map (Arg.subst σ)) (rest.subst σ.lift)

/-- Traverse the prefix once, accumulating the embedding of the continuation's
inputs. Substitute the continuation only at the prefix's return; sharing is
preserved and intermediate registers cannot capture continuation references. -/
def Program.spliceInto {Prim : List Ty → Ty → Type v} {Lit : Ty → Type v}
    (p : Program Prim Lit Γ s) (σ : Subst Lit Δ Γ) (k : Program Prim Lit (s :: Δ) t) :
    Program Prim Lit Γ t :=
  match p with
  | .ret a => k.subst (σ.extend a)
  | .call op args rest =>
      .call op args (rest.spliceInto (fun r => (σ r).weaken) k)

def Program.splice {Prim : List Ty → Ty → Type v} {Lit : Ty → Type v}
    (p : Program Prim Lit Γ s) (k : Program Prim Lit (s :: Γ) t) :
    Program Prim Lit Γ t := p.spliceInto (fun r => .reg r) k

structure Semantics (Prim : List Ty → Ty → Type v) (Lit : Ty → Type v)
    (V : Ty → Type w) where
  literal : ∀ {t}, Lit t → V t
  primitive : ∀ {params t}, Prim params t → HList V params → Option (V t)

def Arg.eval {Prim : List Ty → Ty → Type v} {Lit : Ty → Type v} {V : Ty → Type w}
    (S : Semantics Prim Lit V) (env : Env V Γ) : Arg Lit Γ t → V t
  | .reg r => env r
  | .literal l => S.literal l

def Program.run {Prim : List Ty → Ty → Type v} {Lit : Ty → Type v} {V : Ty → Type w}
    (S : Semantics Prim Lit V) (p : Program Prim Lit Γ t) (env : Env V Γ) : Option (V t) :=
  match p with
  | .ret a => some (a.eval S env)
  | .call op args rest =>
      (S.primitive op (args.map (Arg.eval S env))).bind (fun x => rest.run S (env.cons x))

section Substitution
variable {Prim : List Ty → Ty → Type v} {Lit : Ty → Type v} {V : Ty → Type w}
variable (S : Semantics Prim Lit V)

@[simp] theorem Arg.eval_subst (a : Arg Lit Γ t) (σ : Subst Lit Γ Δ) (env : Env V Δ) :
    (a.subst σ).eval S env = a.eval S (fun r => (σ r).eval S env) := by
  cases a <;> rfl

@[simp] theorem Arg.eval_weaken (a : Arg Lit Γ t) (env : Env V Γ) (x : V s) :
    a.weaken.eval S (env.cons x) = a.eval S env := by
  cases a <;> rfl

theorem eval_lift (σ : Subst Lit Γ Δ) (env : Env V Δ) (x : V s) :
    (fun {t} (r : Ref (s :: Γ) t) => (σ.lift r).eval S (env.cons x)) =
      (fun {_} r => Env.cons x (fun r => (σ r).eval S env) r) := by
  funext t r
  cases r with
  | here => rfl
  | there r => exact Arg.eval_weaken S (σ r) env x

theorem eval_args_subst (args : HList (Arg Lit Γ) params) (σ : Subst Lit Γ Δ)
    (env : Env V Δ) :
    (args.map (Arg.subst σ)).map (Arg.eval S env) =
      args.map (Arg.eval S (fun r => (σ r).eval S env)) := by
  induction args with
  | nil => rfl
  | cons a args ih => simp [HList.map, ih]

theorem Program.run_subst (p : Program Prim Lit Γ t) (σ : Subst Lit Γ Δ) (env : Env V Δ) :
    (p.subst σ).run S env = p.run S (fun r => (σ r).eval S env) := by
  induction p generalizing Δ with
  | ret a => simp [Program.subst, Program.run]
  | call op args rest ih =>
      simp only [Program.subst, Program.run, eval_args_subst]
      congr 1
      funext x
      rw [ih, eval_lift]

theorem Program.run_spliceInto (p : Program Prim Lit Γ s) (σ : Subst Lit Δ Γ)
    (k : Program Prim Lit (s :: Δ) t) (env : Env V Γ) :
    (p.spliceInto σ k).run S env =
      (p.run S env).bind (fun x => k.run S (Env.cons x (fun r => (σ r).eval S env))) := by
  induction p with
  | ret a =>
      simp only [Program.spliceInto, Program.run_subst, Program.run, Option.bind_some]
      congr 1
      funext t r
      cases r <;> rfl
  | call op args rest ih =>
      simp only [Program.spliceInto, Program.run, ih, Option.bind_assoc, Arg.eval_weaken]

theorem Program.run_splice (p : Program Prim Lit Γ s) (k : Program Prim Lit (s :: Γ) t)
    (env : Env V Γ) :
    (p.splice k).run S env = (p.run S env).bind (fun x => k.run S (env.cons x)) := by
  exact p.run_spliceInto S (fun r => .reg r) k env
end Substitution

section Translation
variable {Source Target : List Ty → Ty → Type v} {SLit TLit : Ty → Type v}
variable (literal : ∀ {t}, SLit t → TLit t)
variable (expand : ∀ {params t}, Source params t → Program Target TLit params t)

def Arg.translate : Arg SLit Γ t → Arg TLit Γ t
  | .reg r => .reg r
  | .literal l => .literal (literal l)

def Program.translate (p : Program Source SLit Γ t) : Program Target TLit Γ t :=
  match p with
  | .ret a => .ret (a.translate literal)
  | .call op args rest =>
      ((expand op).subst (args.map (Arg.translate literal)).get).splice
        (rest.translate)

variable {V W : Ty → Type w} (S : Semantics Source SLit V) (T : Semantics Target TLit W)
variable (j : ∀ {t}, V t → W t)
variable (literal_exact : ∀ {t} (l : SLit t), T.literal (literal l) = j (S.literal l))
variable (primitive_exact : ∀ {params t} (op : Source params t) (xs : HList V params),
  (expand op).run T (xs.map j).get = (S.primitive op xs).map j)

include literal_exact

theorem Arg.eval_translate (a : Arg SLit Γ t) (env : Env V Γ) :
    (a.translate literal).eval T (env.map j) = j (a.eval S env) := by
  cases a with
  | reg r => rfl
  | literal l => exact literal_exact l

theorem translated_args (args : HList (Arg SLit Γ) params) (env : Env V Γ) :
    (fun {t} (r : Ref params t) =>
      ((args.map (Arg.translate literal)).get r).eval T (env.map j)) =
      (fun {_} r => ((args.map (Arg.eval S env)).map j).get r) := by
  funext t r
  simp only [HList.get_map, Arg.eval_translate literal S T j literal_exact]

include primitive_exact

theorem Program.run_translate (p : Program Source SLit Γ t) (env : Env V Γ) :
    (p.translate literal expand).run T (env.map j) = (p.run S env).map j := by
  induction p with
  | ret a =>
      simp only [Program.translate, Program.run, Arg.eval_translate literal S T j literal_exact,
        Option.map_some]
  | call op args rest ih =>
      rw [Program.translate, Program.run_splice, Program.run_subst,
        translated_args literal S T j literal_exact, primitive_exact]
      simp only [Program.run]
      cases h : S.primitive op (args.map (Arg.eval S env)) with
      | none => rfl
      | some x =>
          simp only [Option.map_some, Option.bind_some]
          rw [← Env.map_cons, ih]

theorem Program.undefinedness_reflected (p : Program Source SLit Γ t) (env : Env V Γ) :
    (p.translate literal expand).run T (env.map j) = none ↔ p.run S env = none := by
  rw [p.run_translate literal expand S T j literal_exact primitive_exact]
  simp

theorem Program.definedness_reflected (p : Program Source SLit Γ t) (env : Env V Γ) :
    ((p.translate literal expand).run T (env.map j)).isSome = (p.run S env).isSome := by
  rw [p.run_translate literal expand S T j literal_exact primitive_exact]
  simp

theorem Program.result_reflected (p : Program Source SLit Γ t) (env : Env V Γ)
    (injective : Function.Injective (@j t)) (value : V t) :
    (p.translate literal expand).run T (env.map j) = some (j value) ↔
      p.run S env = some value := by
  rw [p.run_translate literal expand S T j literal_exact primitive_exact]
  cases p.run S env with
  | none => simp
  | some x => simp only [Option.map_some, Option.some.injEq]; exact injective.eq_iff
end Translation

/-- info: 'TypedPrograms.Program.run_subst' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Program.run_subst
/-- info: 'TypedPrograms.Program.run_spliceInto' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Program.run_spliceInto
/-- info: 'TypedPrograms.Program.run_translate' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Program.run_translate
/-- info: 'TypedPrograms.Program.undefinedness_reflected' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Program.undefinedness_reflected
/-- info: 'TypedPrograms.Program.definedness_reflected' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Program.definedness_reflected
/-- info: 'TypedPrograms.Program.result_reflected' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Program.result_reflected
end TypedPrograms
