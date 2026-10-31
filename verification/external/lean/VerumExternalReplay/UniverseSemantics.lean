import VerumExternalReplay.ReferenceChecker

/-! Mathematical meaning of the reference checker's exact successor operation.
This does not assert correctness of the entire level normalizer or Rust checker. -/
namespace VerumKernel
set_option warningAsError true

def Level.denote (environment : String → Nat) : Level → Nat
  | .concrete n => n
  | .var name => environment name
  | .succ base => base.denote environment + 1
  | .max a b => Nat.max (a.denote environment) (b.denote environment)
  | .offset base amount => base.denote environment + amount

theorem Level.successor_exact (environment : String → Nat) (level : Level) :
    level.successor.denote environment = level.denote environment + 1 := by
  cases level with
  | concrete n =>
    simp only [Level.successor]
    split <;> rfl
  | var name | succ base | max a b | offset base amount => rfl

theorem Level.successor_strict (environment : String → Nat) (level : Level) :
    level.denote environment < level.successor.denote environment := by
  rw [Level.successor_exact]
  exact Nat.lt_succ_self _

/-- info: 'VerumKernel.Level.successor_exact' depends on axioms: [propext] -/
#guard_msgs in
#print axioms Level.successor_exact
/-- info: 'VerumKernel.Level.successor_strict' depends on axioms: [propext] -/
#guard_msgs in
#print axioms Level.successor_strict
end VerumKernel
