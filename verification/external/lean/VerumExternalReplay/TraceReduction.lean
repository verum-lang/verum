import Lean

/- Build ordinary kernel proofs one constructor at a time. The supplied length
   controls tactic iteration only: the final equality must still close exactly.
   No native evaluator, additional axiom or raised reduction limit is used. -/
open Lean Elab Tactic

elab "reduce_trace_length " count:num : tactic => do
  for _ in [:count.getNat] do
    evalTactic (← `(tactic| apply congrArg Nat.succ))
  evalTactic (← `(tactic| rfl))

elab "reduce_trace_append " count:num : tactic => do
  for _ in [:count.getNat] do
    evalTactic (← `(tactic| apply congrArg (List.cons _)))
  evalTactic (← `(tactic| rfl))
