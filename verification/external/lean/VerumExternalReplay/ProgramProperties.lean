import VerumExternalReplay.TypedPrograms

/-! Reusable invariants and exact primitive embeddings for the existing typed
program semantics. These laws preserve partial failure as well as successful
values. An invariant premise must be established for each primitive; it is
never inferred from a test corpus. -/
namespace ProgramProperties
open TypedPrograms
set_option warningAsError true
universe u v w a
variable {Ty : Type u} {Prim Target : List Ty → Ty → Type v}
variable {Lit : Ty → Type v} {V : Ty → Type w} {Atom : Type a}

def all (P : ∀ {t}, V t → Prop) : HList V Γ → Prop
  | .nil => True
  | .cons x xs => P x ∧ all P xs

def collect {F : Ty → Type v} (atoms : ∀ {t}, F t → List Atom) : HList F Γ → List Atom
  | .nil => []
  | .cons x xs => atoms x ++ collect atoms xs

def argAtoms (S : Semantics Prim Lit V) (atoms : ∀ {t}, V t → List Atom) :
    Arg Lit Γ t → List Atom
  | .reg _ => []
  | .literal l => atoms (S.literal l)

def literalAtoms (S : Semantics Prim Lit V) (atoms : ∀ {t}, V t → List Atom) :
    Program Prim Lit Γ t → List Atom
  | .ret arg => argAtoms S @atoms arg
  | .call _ args rest => collect (argAtoms S @atoms) args ++ literalAtoms S @atoms rest

def supported {A : Type w} (atoms : A → List Atom) (allowed : Atom → Prop) (x : A) : Prop :=
  ∀ atom ∈ atoms x, allowed atom

theorem arg_supported (S : Semantics Prim Lit V) (atoms : ∀ {t}, V t → List Atom)
    (allowed : Atom → Prop) (arg : Arg Lit Γ t) (env : Env V Γ)
    (he : ∀ {t} (r : Ref Γ t), supported (@atoms t) allowed (env r))
    (hl : ∀ atom ∈ argAtoms S @atoms arg, allowed atom) :
    supported (@atoms t) allowed (arg.eval S env) := by
  cases arg with
  | reg r => exact he r
  | literal l => exact hl

theorem args_supported (S : Semantics Prim Lit V) (atoms : ∀ {t}, V t → List Atom)
    (allowed : Atom → Prop) (args : HList (Arg Lit Γ) params) (env : Env V Γ)
    (he : ∀ {t} (r : Ref Γ t), supported (@atoms t) allowed (env r))
    (hl : ∀ atom ∈ collect (argAtoms S @atoms) args, allowed atom) :
    all (fun {t} => supported (@atoms t) allowed) (args.map (Arg.eval S env)) := by
  induction args with
  | nil => trivial
  | cons arg args ih =>
      refine ⟨arg_supported S @atoms allowed arg env he ?_, ih ?_⟩
      · intro atom h; exact hl atom (List.mem_append_left _ h)
      · intro atom h; exact hl atom (List.mem_append_right _ h)

theorem run_supported (S : Semantics Prim Lit V) (atoms : ∀ {t}, V t → List Atom)
    (allowed : Atom → Prop)
    (primitive : ∀ {params t} (op : Prim params t) (xs : HList V params) (x : V t),
      all (fun {t} => supported (@atoms t) allowed) xs → S.primitive op xs = some x → supported (@atoms t) allowed x)
    (p : Program Prim Lit Γ t) (env : Env V Γ) (x : V t)
    (he : ∀ {t} (r : Ref Γ t), supported (@atoms t) allowed (env r))
    (hl : ∀ atom ∈ literalAtoms S @atoms p, allowed atom)
    (success : p.run S env = some x) : supported (@atoms t) allowed x := by
  induction p with
  | ret arg =>
      cases Option.some.inj success
      exact arg_supported S @atoms allowed arg env he hl
  | call op args rest ih =>
      simp only [Program.run] at success
      cases hr : S.primitive op (args.map (Arg.eval S env)) with
      | none => simp [hr] at success
      | some y =>
          simp only [hr, Option.bind_some] at success
          have hy := primitive op _ y
            (args_supported S @atoms allowed args env he
              (fun atom h => hl atom (List.mem_append_left _ h))) hr
          apply ih (env := env.cons y) (x := x) ?_ ?_ success
          · intro t r; cases r with
            | here => exact hy
            | there r => exact he r
          · intro atom h; exact hl atom (List.mem_append_right _ h)

/-- A primitive-only embedding preserves registers and sharing without macro
expansion or substitution. More general lowering uses Program.translate. -/
def embed (primitive : ∀ {params t}, Prim params t → Target params t) :
    Program Prim Lit Γ t → Program Target Lit Γ t
  | .ret arg => .ret arg
  | .call op args rest => .call (primitive op) args (embed primitive rest)

theorem run_embed (S : Semantics Prim Lit V) (T : Semantics Target Lit V)
    (primitive : ∀ {params t}, Prim params t → Target params t)
    (literal_exact : ∀ {t} (l : Lit t), T.literal l = S.literal l)
    (primitive_exact : ∀ {params t} (op : Prim params t) (xs : HList V params),
      T.primitive (primitive op) xs = S.primitive op xs)
    (p : Program Prim Lit Γ t) (env : Env V Γ) :
    (embed primitive p).run T env = p.run S env := by
  have arg_exact : ∀ {Γ t} (arg : Arg Lit Γ t) (env : Env V Γ),
      arg.eval T env = arg.eval S env := by
    intro Γ t arg env; cases arg with
    | reg r => rfl
    | literal l => exact literal_exact l
  have args_exact : ∀ {Γ params} (args : HList (Arg Lit Γ) params) (env : Env V Γ),
      args.map (Arg.eval T env) = args.map (Arg.eval S env) := by
    intro Γ params args env
    induction args with
    | nil => rfl
    | cons arg args ih => simp only [HList.map, arg_exact, ih]
  induction p with
  | ret arg => simp only [embed, Program.run, arg_exact]
  | call op args rest ih => simp only [embed, Program.run, args_exact, primitive_exact, ih]

/-- Strictness is a semantic witness, not the existence of a new opcode. -/
theorem strict_extension {Old New Input Output : Type}
    (oldRun : Old → Input → Output) (newRun : New → Input → Output)
    (inclusion : Old → New)
    (preserved : ∀ p input, newRun (inclusion p) input = oldRun p input)
    (candidate : New) (outside : ∀ p, ∃ input, newRun candidate input ≠ oldRun p input) :
    (∀ p, ∃ q, ∀ input, newRun q input = oldRun p input) ∧
    ¬ ∃ p, ∀ input, newRun candidate input = oldRun p input := by
  constructor
  · intro p; exact ⟨inclusion p, preserved p⟩
  · rintro ⟨p, hp⟩
    obtain ⟨input, h⟩ := outside p
    exact h (hp input)

end ProgramProperties
