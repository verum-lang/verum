import VerumExternalReplay.CheckedPairs

namespace FiniteCodeControls
open CheckedPairs
set_option warningAsError true

def constant (n : UInt64) : Code :=
  .mk [⟨.primitive .first, [.literal (.pair (.nat n) (.bool false))]⟩] none []

def lexical : Code := .mk [⟨.call 0, []⟩] none
  [.mk [⟨.call 0, []⟩] none [constant 42], constant 99]

def registers : Code := .mk
  [⟨.primitive .pair, [.input 1, .input 0]⟩,
   ⟨.call 0, [.register 0, .input 0]⟩,
   ⟨.primitive .pair, [.register 0, .register 1]⟩] none
  [.mk [⟨.primitive .second, [.input 0]⟩] none []]

def unusedInput : Code := .mk [⟨.call 0, [.input 0]⟩] none [constant 42]
def eager : Code := .mk
  [⟨.call 0, []⟩, ⟨.primitive .rem, [.literal (.nat 1), .literal (.nat 0)]⟩]
  (some 0) [constant 42]
def explicitOutput : Code := .mk
  [⟨.call 0, []⟩, ⟨.call 1, []⟩] (some 0) [constant 42, constant 99]
def lastOutput : Code := .mk
  [⟨.call 0, []⟩, ⟨.call 1, []⟩] none [constant 42, constant 99]
def missingOutput : Code := .mk [⟨.call 0, []⟩] (some 1) [constant 42]
def emptyOutput : Code := .mk [] none []
def missingCalleeOutput : Code := .mk [⟨.call 0, []⟩] none [emptyOutput]
def missingCallee : Code := .mk [⟨.call 0, []⟩] none []
def futureRegister : Code := .mk [⟨.primitive .first, [.register 0]⟩] none []

macro "reduce_code" : tactic => `(tactic| (unfold run; reduce_finite_code))

theorem lexical_exact : run lexical [] = some (.nat 42) := by
  unfold lexical constant; reduce_code
theorem registers_exact : run registers [.nat 7, .nat 9] =
    some (.pair (.pair (.nat 9) (.nat 7)) (.nat 7)) := by
  unfold registers; reduce_code
theorem unused_input_fails : run unusedInput [] = none := by
  unfold unusedInput constant; reduce_code
theorem unused_failure_retained : run eager [] = none := by
  unfold eager constant; reduce_code
theorem explicit_output_exact : run explicitOutput [] = some (.nat 42) := by
  unfold explicitOutput constant; reduce_code
theorem default_output_exact : run lastOutput [] = some (.nat 99) := by
  unfold lastOutput constant; reduce_code
theorem missing_output_fails : run missingOutput [] = none := by
  unfold missingOutput constant; reduce_code
theorem empty_output_fails : run emptyOutput [] = none := by
  unfold emptyOutput; reduce_code
theorem missing_callee_output_fails : run missingCalleeOutput [] = none := by
  unfold missingCalleeOutput emptyOutput; reduce_code
theorem missing_callee_fails : run missingCallee [] = none := by
  unfold missingCallee; reduce_code
theorem future_register_fails : run futureRegister [] = none := by
  unfold futureRegister; reduce_code
theorem pack_truncates : apply .packByte [.nat 18446744073709551615, .nat 255] =
    .ok (.nat 18446744073709551615) := by rfl
theorem byte_guard : apply .packByte [.nat 1, .nat 256] =
    .error (.byteOverflow 256) := by rfl
theorem arity_before_sort : apply .checkedAdd [.bool false] =
    .error (.arity .checkedAdd 2 1) := by rfl
theorem first_sort_before_second : apply .rem [.bool false, .bool true] =
    .error (.sort .rem 0) := by rfl
theorem select_mixed : apply .select [.bool true, .bool false, .nat 7] =
    .ok (.bool false) := by rfl
theorem binding_order_and_duplicates (s a b : Value) :
    FiniteCode.bind 2 [.external 1, .state, .external 0, .external 1] s [a, b] =
      some [b, s, a, b] := rfl
theorem binding_width_refused (s a b : Value) :
    FiniteCode.bind 1 [.state, .external 0] s [a, b] = none := rfl

/-- info: 'FiniteCodeControls.lexical_exact' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms lexical_exact
end FiniteCodeControls
