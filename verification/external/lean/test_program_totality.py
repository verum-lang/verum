"""Exact success conditions reuse the typed evaluator and preserve partiality."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.ProgramTotality
set_option warningAsError true
set_option maxRecDepth 65536
open TypedPrograms ProgramTotality
inductive Kind where | number | bytes
abbrev Carrier : Kind → Type | .number => Nat | .bytes => List (Fin 256)
inductive Op : List Kind → Kind → Type where
  | succ : Op [.number] .number
  | cons : Op [.number,.bytes] .bytes
  | tail : Op [.bytes] .bytes
  | empty : Op [] .bytes
def limit : Nat := 2^64-1
def primitive : Op params result → HList Carrier params → Option (Carrier result)
  | .succ, .cons n .nil => if n < limit then some (n+1) else none
  | .cons, .cons n (.cons bytes .nil) => if h : n < 256 then some (⟨n,h⟩::bytes) else none
  | .tail, .cons bytes .nil => match bytes with | [] => none | _::rest => some rest
  | .empty, .nil => some []
def semantics : Semantics Op Carrier Carrier := ⟨fun x => x,primitive⟩
def input (n : Nat) : Env Carrier [.number] := (HList.cons n .nil).get
def byteInput (xs : List (Fin 256)) : Env Carrier [.bytes] :=
  (HList.cons xs .nil : HList Carrier [.bytes]).get
def advance : Program Op Carrier [.number] .number :=
  .call .succ (.cons (.reg .here) .nil) (.ret (.reg .here))
def again : Program Op Carrier [.number,.number] .number :=
  .call .succ (.cons (.reg .here) .nil) (.ret (.reg .here))
def singleton : Program Op Carrier [.number] .bytes :=
  .call .empty .nil
    (.call .cons (.cons (.reg (.there .here)) (.cons (.reg .here) .nil)) (.ret (.reg .here)))
def remove : Program Op Carrier [.bytes] .bytes :=
  .call .tail (.cons (.reg .here) .nil) (.ret (.reg .here))
def discardFailure : Program Op Carrier [.number] .number :=
  .call .succ (.cons (.reg .here) .nil) (.ret (.literal 7))
def accepted (p : Program Op Carrier [.number] t) (n : Nat) : Prop :=
  wp semantics p (fun _ => True) (input n)
instance (p : Program Op Carrier [.number] t) (n : Nat) : Decidable (accepted p n) :=
  inferInstanceAs (Decidable (wp semantics p (fun _ => True) (input n)))
def applyNat (_ : Unit) : List Nat → Option Nat
  | [n] => if n < limit then some (n+1) else none
  | _ => none
def leaf : FiniteCode.Code Unit Nat := .mk [⟨.primitive (),[.input 0],none⟩] none []
def nested : FiniteCode.Code Unit Nat := .mk
  [⟨.call 0,[.input 0],none⟩,⟨.primitive (),[.register 0],none⟩] (some 0) [leaf]
def missing : FiniteCode.Code Unit Nat := .mk [⟨.call 1,[.input 0],none⟩] none [leaf]
def badOperand : FiniteCode.Code Unit Nat := .mk [⟨.primitive (),[.register 0],none⟩] none []
def badOutput : FiniteCode.Code Unit Nat := .mk [⟨.primitive (),[.input 0],none⟩] (some 1) []
def finiteWP (code : FiniteCode.Code Unit Nat) (n : Nat) (post : Nat → Prop) : Prop :=
  wp (FiniteCode.semantics applyNat) (FiniteCode.lower code 1) post (FiniteCode.row [n]).get
instance (code : FiniteCode.Code Unit Nat) (n : Nat) (post : Nat → Prop) [DecidablePred post] :
    Decidable (finiteWP code n post) :=
  inferInstanceAs (Decidable (wp (FiniteCode.semantics applyNat) (FiniteCode.lower code 1)
    post (FiniteCode.row [n]).get))
'''
FALSE = 'Tactic `decide` proved that the proposition'
CBV_FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class ProgramTotalityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.ProgramTotality'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-program-totality-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            result = check(path, expected)
            if expected:
                errors = [d for d in result['diagnostics'] if d['severity'] == 'error']
                self.assertEqual(len(errors), 1, errors)
                self.assertTrue(errors[0]['data'].startswith(expected), errors)
                if expected == FALSE:
                    self.assertTrue(errors[0]['data'].endswith('is false'), errors)
            return result

    def test_axiom_roster(self):
        source = (ROOT / 'VerumExternalReplay/ProgramTotality.lean').read_text()
        names = re.findall(r'^theorem (\w+)', source, re.M) + ['wp','Certificate','decidableWP']
        result = self.prove('\n'.join('#print axioms ProgramTotality.' + n for n in names))
        rows = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rows), len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)

    def test_refined_primitive_and_relational_contract(self):
        self.prove('''
example : Certificate semantics advance (fun env => env .here < limit)
    (fun env value => value = env .here + 1) := by
  intro env bounded
  refine ⟨env .here+1,?_,rfl⟩
  simpa only [semantics,HList.map,Arg.eval,primitive] using ite_eq_left bounded
example (n : Nat) : accepted advance n ↔ n < limit := by
  simp only [accepted,advance,wp,semantics,HList.map,Arg.eval,input,HList.get,primitive]
  split <;> simp_all
example (n : Nat) (valid : n < 256) : wp semantics singleton
    (fun bytes => bytes.map Fin.val = [n]) (input n) := by
  exact ⟨[],rfl,⟨[⟨n,valid⟩],dite_eq_left valid,rfl⟩⟩
example : wp semantics remove (fun bytes => bytes = []) (byteInput [⟨0,by decide⟩]) := by
  exact ⟨[],rfl,rfl⟩
example : wp semantics advance (fun value => value = 43) (input 42) := by decide +kernel
''')

    def test_continuation_and_splicing(self):
        self.prove('''
example (n : Nat) : wp semantics (advance.splice again) (fun value => value = n+2) (input n) ↔
    wp semantics advance (fun value => wp semantics again (fun out => out = n+2)
      (Env.cons value (input n))) (input n) := splice_iff _ _ _ _ _
example : wp semantics (advance.splice again) (fun value => value = 44) (input 42) := by decide +kernel
example : accepted (advance.splice again) (limit-2) := by decide +kernel
example : ¬ accepted (advance.splice again) (limit-1) := by decide +kernel
example : wp semantics (.call .succ (.cons (.reg .here) .nil) (.ret (.reg .here)))
    (fun value => value > 42) (input 42) :=
  call_rule semantics .succ _ _ _ (fun value => value = 43) _ ⟨43,rfl,rfl⟩
    (by intro value equal; cases equal; decide)
''')

    def test_finite_lowering_keeps_all_eager_calls(self):
        self.prove('''
example : finiteWP nested 41 (fun value => value = 42) := by decide +kernel
example : ¬ finiteWP nested (limit-1) (fun _ => True) := by decide +kernel
example : ¬ finiteWP missing 41 (fun _ => True) := by decide +kernel
example : ¬ finiteWP badOperand 41 (fun _ => True) := by decide +kernel
example : ¬ finiteWP badOutput 41 (fun _ => True) := by decide +kernel
example (code : FiniteCode.Code Unit Nat) (n : Nat) (post : Nat → Prop) :
    finiteWP code n post ↔ ∃ value, FiniteCode.run applyNat code [n] = some value ∧ post value :=
  finite_code_iff applyNat code [n] post
''')

    def test_false_totality_claims_rejected(self):
        claims = {
            'overflow': 'accepted advance limit',
            'large_byte': 'accepted singleton 256',
            'empty_tail': 'wp semantics remove (fun _ => True) (byteInput [])',
            'discarded_failure': 'accepted discardFailure limit',
            'partial_continuation': 'accepted (advance.splice again) (limit-1)',
            'wrong_refinement': 'wp semantics advance (fun value => value = 42) (input 42)',
            'wrong_selected_output': 'finiteWP nested 41 (fun value => value = 43)',
            'dead_call_erased': 'finiteWP nested (limit-1) (fun _ => True)',
            'absent_callee': 'finiteWP missing 41 (fun _ => True)',
            'absent_operand': 'finiteWP badOperand 41 (fun _ => True)',
            'absent_output': 'finiteWP badOutput 41 (fun _ => True)',
        }
        for name, claim in claims.items():
            with self.subTest(name=name):
                tactic, error = ('decide_cbv', CBV_FALSE) if claim.startswith('finiteWP') else ('decide +kernel', FALSE)
                self.prove('example : ' + claim + ' := by ' + tactic, error)


if __name__ == '__main__':
    unittest.main()
