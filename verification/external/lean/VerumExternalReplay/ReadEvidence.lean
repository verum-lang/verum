import VerumExternalReplay.FiniteExecution

/-! Full-report noninterference for the existing finite execution semantics.
The footprint includes every root instruction's input operands, including
unused results and eager call arguments. It is deliberately conservative about
declared arity and unreachable instructions. Callees receive equal complete
argument lists; their semantics and lexical libraries are fixed by `code` and
`apply`. This is a model theorem, not a native ownership or cache certificate.
It covers primitive/call code with the stated bounded meter, not native
Branch/Fold or arbitrary WorkMeter behavior. Native observation traces retain
whole input environments: footprint projection requires traces absent/erased,
or a separate proof that all their captured observations also agree.
-/
namespace ReadEvidence
open FiniteCode FiniteExecution
set_option warningAsError true

variable {O V E : Type}

def inputs (args : List (Operand V)) : List Nat :=
  args.filterMap (fun arg => match arg with | .input n => some n | _ => none)

def footprint (code : Code O V) : List Nat :=
  match code with
  | .mk instructions _ _ => instructions.flatMap (fun step => inputs step.arguments)

/-- Missing inputs remain visible; an absent slot is not a default value. -/
def key (code : Code O V) (ins : List V) : List (Option V) :=
  (footprint code).map (fun n => ins[n]?)

def Agrees (args : List (Operand V)) (left right : List V) : Prop :=
  ∀ n, Operand.input n ∈ args → left[n]? = right[n]?

theorem operand_agreement (args : List (Operand V)) (left right regs : List V)
    (h : Agrees args left right) (position : Nat) :
    operand (E := E) left regs position args[position]? =
      operand right regs position args[position]? := by
  cases entry : args[position]? with
  | none => rfl
  | some arg =>
      cases arg with
      | input n => simp only [operand, h n (List.mem_of_getElem? entry)]
      | register n => rfl
      | literal x => rfl

theorem arguments_agreement (args : List (Operand V)) (left right regs : List V)
    (h : Agrees args left right) (start count : Nat) :
    FiniteExecution.arguments (E := E) left regs args start count =
      FiniteExecution.arguments right regs args start count := by
  induction count generalizing start with
  | zero => rfl
  | succ count ih =>
      simp only [FiniteExecution.arguments, operand_agreement args left right regs h, ih]

theorem steps_agreement (apply : O → List V → Except E V)
    (library : List (Callee V E)) (limits : Limits) (code : List (Step O V))
    (left right regs : List V) (usage : Usage)
    (h : ∀ step ∈ code, Agrees step.arguments left right) :
    FiniteExecution.steps apply library limits left code regs usage =
      FiniteExecution.steps apply library limits right code regs usage := by
  induction code generalizing regs usage with
  | nil => rfl
  | cons step rest ih =>
      have head := arguments_agreement (E := E) step.arguments left right regs
        (h step (by simp)) 0 step.arity
      have tail : ∀ step ∈ rest, Agrees step.arguments left right := by
        intro step member
        exact h step (by simp [member])
      simp only [FiniteExecution.steps, head]
      split
      · rfl
      · split
        · rfl
        · split
          · split
            · rfl
            · exact ih _ _ tail
          · split
            · rfl
            · split
              · split
                · exact ih _ _ tail
                · rfl
              · rfl

theorem key_agreement (code : Code O V) (left right : List V)
    (h : key code left = key code right) :
    ∀ n ∈ footprint code, left[n]? = right[n]? := by
  exact List.map_inj_left.mp h

/-- Equality includes stop, every retained register, usage, halt and meter error.
Limits and initial usage must be the same; this does not charge a cache hit or
authorize a recipient. The native adapter must establish immutable semantics,
input correspondence and its own interface/ownership/lifecycle checks. -/
theorem frame_agreement (apply : O → List V → Except E V) (code : Code O V)
    (limits : Limits) (left right : List V) (usage : Usage)
    (h : key code left = key code right) :
    frame apply code limits left usage = frame apply code limits right usage := by
  have agree := key_agreement code left right h
  cases code with
  | mk instructions selected library =>
      simp only [frame]
      congr 1
      apply steps_agreement
      intro step member n used
      apply agree n
      simp only [footprint, List.mem_flatMap]
      refine ⟨step, member, ?_⟩
      simp only [inputs, List.mem_filterMap]
      exact ⟨.input n, used, rfl⟩

theorem execute_agreement (apply : O → List V → Except E V) (code : Code O V)
    (limits : Limits) (left right : List V) (used : Nat)
    (h : key code left = key code right) :
    execute apply code limits left used = execute apply code limits right used := by
  simp only [execute]
  split
  · rfl
  · rw [frame_agreement apply code limits left right _ h]

theorem selected_agreement (apply : O → List V → Except E V) (code : Code O V)
    (limits : Limits) (left right : List V) (used : Nat)
    (h : key code left = key code right) :
    selectRoot code (execute apply code limits left used) =
      selectRoot code (execute apply code limits right used) := by
  rw [execute_agreement apply code limits left right used h]

end ReadEvidence
