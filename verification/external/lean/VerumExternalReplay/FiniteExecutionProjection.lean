import VerumExternalReplay.FiniteExecution

/-! Non-budget finite execution agrees with the intrinsically typed lowering.
This connects the two mathematical definitions; Rust correspondence is separate.
-/
namespace FiniteExecutionProjection
open TypedPrograms FiniteCode FiniteExecution
set_option warningAsError true

variable {O V E : Type}

def partialApply (apply : O → List V → Except E V) (op : O) (xs : List V) : Option V :=
  (apply op xs).toOption

def result (selected : Option Nat) (report : Report V E) : Option V :=
  if report.stop = .complete then output selected report.registers else none

def envValues : (n : Nat) → Env (Carrier V) (Row n) → List V
  | 0, _ => []
  | n + 1, env => env .here :: envValues n (fun r => env (.there r))

@[simp] theorem envValues_row (xs : List V) : envValues xs.length (row xs).get = xs := by
  induction xs with
  | nil => rfl
  | cons x xs ih => simpa [row, envValues, HList.get] using congrArg (x :: ·) ih

@[simp] theorem envValues_args (S : Semantics (Prim O) (Carrier V) (Carrier V))
    (env : Env (Carrier V) Γ) (xs : List (Arg (Carrier V) Γ ())) :
    envValues xs.length (fun r => ((row xs).get r).eval S env) =
      xs.map (fun a => a.eval S env) := by
  induction xs with
  | nil => rfl
  | cons x xs ih => simpa [row, envValues, HList.get] using congrArg (x.eval S env :: ·) ih

@[simp] theorem eval_inputs (S : Semantics (Prim O) (Carrier V) (Carrier V))
    (n : Nat) (env : Env (Carrier V) (Row n)) :
    (inputs n).map (fun a => a.eval S env) = envValues n env := by
  induction n with
  | zero => rfl
  | succ n ih =>
      simp only [inputs, List.map_cons, List.map_map, envValues]
      congr 1
      have h : ∀ (a : Arg (Carrier V) (Row n) ()),
          (a.weaken (s := ())).eval S env = a.eval S (fun r => env (.there r)) := by
        intro a; cases a <;> rfl
      simpa only [Function.comp_def, h] using ih (fun r => env (.there r))

@[simp] theorem operand_projection (apply : O → List V → Except E V)
    (env : Env (Carrier V) Γ) (ins regs : List (Arg (Carrier V) Γ ()))
    (a : Operand V) (position : Nat) :
    (operand (E := E) (ins.map (fun x => x.eval (semantics (partialApply apply)) env))
      (regs.map (fun x => x.eval (semantics (partialApply apply)) env)) position (some a)).toOption =
      (resolve ins regs a).map (fun x => x.eval (semantics (partialApply apply)) env) := by
  cases a with
  | input n => simp only [operand, resolve, List.getElem?_map]; cases ins[n]? <;> rfl
  | register n => simp only [operand, resolve, List.getElem?_map]; cases regs[n]? <;> rfl
  | literal v => rfl


@[simp] theorem except_bind_option (x : Except A B) (f : B → Except A C) :
    (x.bind f).toOption = x.toOption.bind (fun y => (f y).toOption) := by
  cases x <;> rfl

theorem operand_position (ins regs : List V) (a : Option (Operand V)) (i j : Nat) :
    (operand (E := E) ins regs i a).toOption = (operand (E := E) ins regs j a).toOption := by
  cases a with
  | none => rfl
  | some a =>
      cases a with
      | input n => simp only [operand]; cases ins[n]? <;> rfl
      | register n => simp only [operand]; cases regs[n]? <;> rfl
      | literal v => rfl

theorem arguments_tail (ins regs : List V) (a : Operand V) (args : List (Operand V))
    (start count : Nat) :
    (FiniteExecution.arguments (E := E) ins regs (a :: args) (start + 1) count).toOption =
      (FiniteExecution.arguments (E := E) ins regs args start count).toOption := by
  induction count generalizing start with
  | zero => rfl
  | succ n ih =>
      simp only [FiniteExecution.arguments, Bind.bind, except_bind_option, List.getElem?_cons_succ, ih]
      rw [operand_position ins regs args[start]? (start + 1) start]


theorem arguments_projection (apply : O → List V → Except E V)
    (env : Env (Carrier V) Γ) (ins regs : List (Arg (Carrier V) Γ ()))
    (args : List (Operand V)) (count : Nat) :
    (FiniteExecution.arguments (E := E)
      (ins.map (fun x => x.eval (semantics (partialApply apply)) env))
      (regs.map (fun x => x.eval (semantics (partialApply apply)) env)) args 0 count).toOption =
      ((if count ≤ args.length then some (args.take count) else none).bind
        (FiniteCode.arguments ins regs)).map
          (List.map (fun x => x.eval (semantics (partialApply apply)) env)) := by
  induction count generalizing args with
  | zero => simp [FiniteExecution.arguments, FiniteCode.arguments, Except.toOption]
  | succ n ih =>
      cases args with
      | nil => simp [FiniteExecution.arguments, operand, Except.toOption, Bind.bind, Except.bind]
      | cons a args =>
          simp only [FiniteExecution.arguments, Bind.bind, except_bind_option,
            List.getElem?_cons_zero, arguments_tail, ih, operand_projection,
            List.length_cons, Nat.add_le_add_iff_right, List.take_succ_cons]
          split <;> simp_all only [Option.bind_some, Option.bind_none, Option.map_none]
          · simp only [FiniteCode.arguments, Bind.bind]
            cases resolve ins regs a <;> cases FiniteCode.arguments ins regs (args.take n) <;> rfl
          · cases resolve ins regs a <;> rfl


theorem step_arguments_projection (apply : O → List V → Except E V)
    (env : Env (Carrier V) Γ) (ins regs : List (Arg (Carrier V) Γ ())) (step : Step O V) :
    (FiniteExecution.arguments (E := E)
      (ins.map (fun x => x.eval (semantics (partialApply apply)) env))
      (regs.map (fun x => x.eval (semantics (partialApply apply)) env))
      step.arguments 0 step.arity).toOption =
      (step.operands.bind (FiniteCode.arguments ins regs)).map
        (List.map (fun x => x.eval (semantics (partialApply apply)) env)) := by
  exact arguments_projection apply env ins regs step.arguments step.arity

theorem output_projection (f : A → V) (xs : List A) (selected : Option Nat) :
    output selected (xs.map f) = (output selected xs).map f := by
  cases selected <;> simp [output, List.getLast?_map]

theorem lower_end (apply : O → List V → Except E V)
    (library : List ((n : Nat) → Target O V (Row n))) (selected : Option Nat)
    (env : Env (Carrier V) Γ) (ins regs : List (Arg (Carrier V) Γ ())) :
    (FiniteCode.steps library selected [] ins regs).run (semantics (partialApply apply)) env =
      output selected (regs.map (fun x => x.eval (semantics (partialApply apply)) env)) := by
  rw [output_projection]
  cases selected with
  | none =>
      simp only [FiniteCode.steps, Option.bind_none, Option.isNone_none, ite_true, output]
      cases regs.getLast? <;> rfl
  | some n =>
      simp only [FiniteCode.steps, Option.bind_some, Option.isNone_some, output]
      cases regs[n]? <;> rfl

/-- Callee induction hypothesis, relating corresponding lexical library entries. -/
def Libraries (apply : O → List V → Except E V)
    (typed : List ((n : Nat) → Target O V (Row n))) (machine : List (Callee V E)) : Prop :=
  ∀ (index : Nat), match typed[index]?, machine[index]? with
  | none, none => True
  | some t, some m => ∀ n (env : Env (Carrier V) (Row n)) limits usage,
      (m limits (envValues n env) usage).1.stop ≠ .budget →
      (t n).run (semantics (partialApply apply)) env =
        result (m limits (envValues n env) usage).2 (m limits (envValues n env) usage).1
  | _, _ => False


theorem values_args (S : Semantics (Prim O) (Carrier V) (Carrier V))
    (env : Env (Carrier V) Γ) (args : List (Arg (Carrier V) Γ ())) :
    values ((row args).map (fun {t} a => Arg.eval S env (t := t) a)) =
      args.map (fun a => a.eval S env) := by
  induction args with
  | nil => rfl
  | cons a args ih => simpa [row, HList.map, values] using congrArg (a.eval S env :: ·) ih

theorem run_primitive (apply : O → List V → Except E V)
    (env : Env (Carrier V) Γ) (op : O) (args : List (Arg (Carrier V) Γ ()))
    (rest : Target O V (() :: Γ)) :
    (Program.call (some op) (row args) rest).run (semantics (partialApply apply)) env =
      (apply op (args.map (fun a => a.eval (semantics (partialApply apply)) env))).toOption.bind
        (fun x => rest.run (semantics (partialApply apply)) (env.cons x)) := by
  change (partialApply apply op (values ((row args).map _))).bind _ = _
  rw [values_args]
  rfl

theorem steps_projection (apply : O → List V → Except E V)
    (typed : List ((n : Nat) → Target O V (Row n))) (machine : List (Callee V E))
    (related : Libraries apply typed machine) (selected : Option Nat) (code : List (Step O V))
    (limits : Limits) (env : Env (Carrier V) Γ)
    (ins regs : List (Arg (Carrier V) Γ ())) (usage : Usage)
    (finished : (FiniteExecution.steps apply machine limits
      (ins.map (fun x => x.eval (semantics (partialApply apply)) env)) code
      (regs.map (fun x => x.eval (semantics (partialApply apply)) env)) usage).stop ≠ .budget) :
    (FiniteCode.steps typed selected code ins regs).run (semantics (partialApply apply)) env =
      result selected (FiniteExecution.steps apply machine limits
        (ins.map (fun x => x.eval (semantics (partialApply apply)) env)) code
        (regs.map (fun x => x.eval (semantics (partialApply apply)) env)) usage) := by
  induction code generalizing Γ env ins regs usage with
  | nil => simpa only [FiniteExecution.steps, result, ite_true] using lower_end apply typed selected env ins regs
  | cons step rest ih =>
      cases hg : gate (E := E) limits step usage with
      | error refusal => simp [FiniteExecution.steps, hg, stopped] at finished
      | ok paid =>
          have argsExact := step_arguments_projection apply env ins regs step
          cases ha : FiniteExecution.arguments (E := E)
            (ins.map (fun x => x.eval (semantics (partialApply apply)) env))
            (regs.map (fun x => x.eval (semantics (partialApply apply)) env))
            step.arguments 0 step.arity with
          | error error =>
              simp only [ha, Except.toOption] at argsExact
              have empty : step.operands.bind (FiniteCode.arguments ins regs) = none := by
                cases h : step.operands.bind (FiniteCode.arguments ins regs) <;> simp_all
              simp [FiniteCode.steps, empty, FiniteExecution.steps, hg, ha, result, stopped]
          | ok vals =>
              cases hp : step.operands.bind (FiniteCode.arguments ins regs) with
              | none => simp [ha, hp, Except.toOption] at argsExact
              | some args =>
                  have valuesExact : vals = args.map (fun x => x.eval (semantics (partialApply apply)) env) := by
                    simpa [ha, hp, Except.toOption] using argsExact
                  subst vals
                  cases hop : step.operation with
                  | primitive op =>
                      simp only [FiniteCode.steps, hp, hop, run_primitive]
                      cases hex : apply op (args.map (fun a => a.eval (semantics (partialApply apply)) env)) with
                      | error error => simp [FiniteExecution.steps, hg, ha, hop, hex, result, stopped, Except.toOption]
                      | ok x =>
                          simp only [Except.toOption, Option.bind_some]
                          simp only [FiniteExecution.steps, hg, ha, hop, hex] at finished ⊢
                          have next := ih (env.cons (t := ()) x)
                            (ins.map (Arg.weaken (s := ())))
                            (regs.map (Arg.weaken (s := ())) ++ [.reg .here]) paid
                          simp only [List.map_append, List.map_map, Function.comp_def, Arg.eval_weaken, List.map_cons, List.map_nil] at next
                          simp only [Arg.eval, Env.cons] at next
                          exact next finished
                  | call index =>
                      have calleeRelated := related index
                      cases ht : typed[index]? with
                      | none =>
                          cases hm : machine[index]? with
                          | none => simp [FiniteCode.steps, hp, hop, ht, FiniteExecution.steps, hg, ha, hm, result, stopped]
                          | some m => simp [ht, hm] at calleeRelated
                      | some t =>
                          cases hm : machine[index]? with
                          | none => simp [ht, hm] at calleeRelated
                          | some m =>
                              simp only [ht, hm] at calleeRelated
                              simp only [FiniteCode.steps, hp, hop, ht, Program.run_splice, Program.run_subst]
                              have exactCallee := calleeRelated args.length
                                (fun r => ((row args).get r).eval (semantics (partialApply apply)) env) limits paid
                              simp only [envValues_args] at exactCallee
                              cases hc : m limits (args.map (fun a => a.eval (semantics (partialApply apply)) env)) paid with
                              | mk child returned =>
                                  simp only [hc] at exactCallee
                                  simp only [FiniteExecution.steps, hg, ha, hop, hm, hc] at finished ⊢
                                  by_cases complete : child.stop = .complete
                                  · rw [exactCallee (by simp [complete])]
                                    simp only [result, complete, ite_true]
                                    cases ho : output returned child.registers with
                                    | none => rfl
                                    | some x =>
                                        simp only [Option.bind_some]
                                        simp only [complete, ite_true, ho] at finished
                                        have next := ih (env.cons (t := ()) x)
                                          (ins.map (Arg.weaken (s := ())))
                                          (regs.map (Arg.weaken (s := ())) ++ [.reg .here]) child.usage
                                        simp only [List.map_append, List.map_map, Function.comp_def,
                                          Arg.eval_weaken, List.map_cons, List.map_nil] at next
                                        simp only [Arg.eval, Env.cons] at next
                                        exact next finished
                                  · simp only [complete, ite_false] at finished ⊢
                                    rw [exactCallee finished]
                                    simp [result, complete]


theorem frame_projection (apply : O → List V → Except E V) (code : Code O V)
    (n : Nat) (env : Env (Carrier V) (Row n)) (limits : Limits) (usage : Usage)
    (finished : (frame apply code limits (envValues n env) usage).1.stop ≠ .budget) :
    (lower code n).run (semantics (partialApply apply)) env =
      result (frame apply code limits (envValues n env) usage).2
        (frame apply code limits (envValues n env) usage).1 := by
  cases code with
  | mk instructions selected library =>
      simp only [FiniteCode.lower, frame] at finished ⊢
      have related : Libraries apply
          (library.attach.map (fun child => fun n => lower child.val n))
          (library.attach.map (fun child => frame apply child.val)) := by
        intro index
        simp only [List.getElem?_map]
        cases lookup : library.attach[index]? with
        | none => trivial
        | some child =>
            simp only [Option.map_some]
            intro n env limits usage finished
            exact frame_projection apply child.val n env limits usage finished
      have projected := steps_projection apply _ _ related selected instructions limits env (inputs n) [] usage
      simp only [eval_inputs, List.map_nil] at projected
      exact projected finished
termination_by sizeOf code
decreasing_by
  have h := List.sizeOf_lt_of_mem child.property
  subst code
  simp only [Code.mk.sizeOf_spec]
  omega

theorem selectRoot_projection (instructions : List (Step O V)) (selected : Option Nat)
    (library : List (Code O V)) (report : Report V E) :
    (match selectRoot (.mk instructions selected library) report with
      | .selected value => some value
      | .missing | .unattempted => none) = result selected report := by
  by_cases complete : report.stop = .complete
  · simp only [selectRoot, result, complete, ite_true]
    cases output selected report.registers <;> rfl
  · simp [selectRoot, result, complete]

/-- Every completed or semantically refused finite run has exactly the same
partial result as the typed lowering. A budget stop yields no such conclusion. -/
theorem execute_projection (apply : O → List V → Except E V) (code : Code O V)
    (limits : Limits) (ins : List V) (used : Nat)
    (finished : (execute apply code limits ins used).stop ≠ .budget) :
    FiniteCode.run (partialApply apply) code ins =
      match selectRoot code (execute apply code limits ins used) with
      | .selected value => some value
      | .missing | .unattempted => none := by
  cases hr : reserve limits.work 1 used with
  | error error => simp [execute, hr, stopped] at finished
  | ok work =>
      have projected := frame_projection apply code ins.length (row ins).get limits {work}
      simp only [envValues_row] at projected
      simp only [execute, hr] at finished ⊢
      have exact := projected finished
      cases code with
      | mk instructions selected library =>
          rw [selectRoot_projection]
          simpa only [FiniteCode.run, frame] using exact

end FiniteExecutionProjection

/-- info: 'FiniteExecutionProjection.execute_projection' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms FiniteExecutionProjection.execute_projection
