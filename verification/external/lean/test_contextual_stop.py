"""Constructive STOP/BIT normalization, cylinders, tails and frozen row counts."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.ContextualStop
set_option warningAsError true
open ExactMass EnumerableMass CausalCoding StoppedProcess ContextualStop
def fixed : Selector Nat := fun _ _ => ⟨0,false⟩
def emptyBank : Bank Nat := fun _ _ => 0
def uniform := model fixed emptyBank 2 (by decide)
def row : List (List Bool) := [[false,false],[false,true]]
def causal : Selector Nat := fun _ past => ⟨if past.isEmpty then 0 else 1,past.headD false⟩
def learned : Bank Nat := fun (kind,key) bit => match kind, key, bit with
  | .stop, 0, false => 3 | .stop, 0, true => 1
  | .stop, _, false => 0 | .stop, _, true => 2
  | .bit, 0, false => 2 | .bit, 0, true => 0
  | .bit, _, false => 1 | .bit, _, true => 3
def contextual := model causal learned 2 (by decide)
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class ContextualStopTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.ContextualStop'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-contextual-stop-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            result = check(path, expected)
            if expected:
                errors = [d for d in result['diagnostics'] if d['severity'] == 'error']
                self.assertEqual(len(errors), 1, errors)
                self.assertTrue(errors[0]['data'].startswith(expected), errors)
            return result

    def test_axiom_roster(self):
        names = ['ExactMass.le_less_trans']
        for module, definitions in [('StoppedProcess', ['lengthLaw']), ('ContextualStop', ['law'])]:
            source = (ROOT / f'VerumExternalReplay/{module}.lean').read_text()
            names += [module + '.' + name for name in re.findall(r'^theorem (\w+)', source, re.M) + definitions]
        result = self.prove('\n'.join('#print axioms ' + n for n in names))
        rows = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rows), len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)

    def test_exact_point_prefix_and_complete_units(self):
        self.prove('''
example : Equal (point uniform [] []) ⟨1,2,by decide⟩ := by decide_cbv
example : Equal (cylinder uniform [] [[false,false]]) ⟨1,8,by decide⟩ := by decide_cbv
example : Equal (point uniform [] [[false,false]]) ⟨1,16,by decide⟩ := by decide_cbv
example : Equal (point uniform [] row) ⟨1,128,by decide⟩ := by decide_cbv
example : (score uniform [] [[false]]).isNone = true := by decide_cbv
example : (score uniform [] [[false,false,false]]).isNone = true := by decide_cbv
example : (score uniform [] [[]]).isNone = true := by decide_cbv
example : score uniform [] [] = some (point uniform [] []) := by decide_cbv
example : Equal (point (model fixed emptyBank 8 (by decide)) [] [List.replicate 8 false])
  ⟨1,1024,by decide⟩ := by decide_cbv
example (past future : List (List Bool)) : Equal (point uniform [] (past++future))
  (mul (cylinder uniform [] past) (point uniform (endpoint uniform [] past) future)) :=
  point_extension uniform [] past future
''')

    def test_causal_selection_and_inversion(self):
        self.prove('''
example : Equal (probability causal learned .stop [] true) ⟨3,10,by decide⟩ := by decide_cbv
example : Equal (probability causal learned .stop [true] true) ⟨1,6,by decide⟩ := by decide_cbv
example : Equal (point contextual [] [[true,false]]) ⟨49,720,by decide⟩ := by decide_cbv
example : Equal (cylinder contextual [] [[true,false]]) ⟨49,600,by decide⟩ := by decide_cbv
example : endpoint contextual [] [[true,false]] = [false,true] := by decide_cbv
example : rowVisits causal [] [[true,false]] =
  [((.stop,0),false),((.bit,0),true),((.bit,1),true),((.stop,1),true)] := by decide_cbv
example (blocks : List (List Bool)) : Equal (eventMass learned (rowVisits causal [] blocks))
  (point contextual [] blocks) := row_visits_mass causal learned 2 (by decide) [] blocks
''')

    def test_finite_cuts_and_uniform_tail(self):
        self.prove('''
example : Equal (slice uniform 0 []) ⟨1,2,by decide⟩ := by decide_cbv
example : Equal (slice uniform 1 []) ⟨1,4,by decide⟩ := by decide_cbv
example : Equal (tail uniform 2 []) ⟨1,4,by decide⟩ := by decide_cbv
example : Equal (add (partialSum (fun n => slice uniform n []) 2) (tail uniform 2 [])) one := by decide_cbv
example (n : Nat) : Equal (add (partialSum (fun k => slice contextual k []) n) (tail contextual n [])) one :=
  normalized_prefix contextual [] n
example (n : Nat) (past : List (List Bool)) : Equal
  (add (mul (cylinder contextual [] past) (partialSum (fun k => slice contextual k (endpoint contextual [] past)) n))
    (mul (cylinder contextual [] past) (tail contextual n (endpoint contextual [] past))))
  (cylinder contextual [] past) := cylinder_partition contextual [] past n
example (c : Nat) (e : Fraction) (positive : 0 < e.numerator) (n : Nat)
  (large : (c+1)*e.denominator ≤ n) : Less (decay c n) e := decay_vanishes c e positive n large
example : AtMost (unit (4*(2^64-1)+2) (by decide))
  (probability fixed (fun _ bit => if bit then 0 else 2^64-1) .stop [] true) := by decide_cbv
example : 2^64 < 4*(2^64-1)+2 := by decide_cbv
''')

    def test_frozen_counts_and_multiplicity(self):
        self.prove('''
example : publish fixed emptyBank [] row (.stop,0) false = 2 := by decide_cbv
example : publish fixed emptyBank [] row (.stop,0) true = 1 := by decide_cbv
example : publish fixed emptyBank [] row (.bit,0) false = 3 := by decide_cbv
example : publish fixed emptyBank [] row (.bit,0) true = 1 := by decide_cbv
example : publish fixed emptyBank [] [] (.stop,0) true = 1 := by decide_cbv
example : publish causal learned [] [[true,false]] (.bit,1) true = 4 := by decide_cbv
example : publish fixed emptyBank [] row (.bit,1) false = 0 := by decide_cbv
example : Equal (eventMass emptyBank (rowVisits fixed [] row)) ⟨1,128,by decide⟩ := by decide_cbv
''')

    def test_wrong_claims_reach_kernel_false(self):
        claims = {
            'prefix_includes_stop': 'Equal (cylinder uniform [] [[false,false]]) (point uniform [] [[false,false]])',
            'empty_row_unscored': 'Equal (point uniform [] []) one',
            'length_factored_twice': 'Equal (point uniform [] row) ⟨1,256,by decide⟩',
            'partial_byte_accepted': '(score uniform [] [[false]]).isSome = true',
            'oversized_byte_accepted': '(score uniform [] [[false,false,false]]).isSome = true',
            'prefix_order_reversed': 'endpoint contextual [] [[true,false]] = [true,false]',
            'inversion_ignored': 'Equal (probability causal learned .stop [true] true) ⟨5,6,by decide⟩',
            'context_ignored': 'Equal (point contextual [] [[true,false]]) (point uniform [] [[true,false]])',
            'stop_deduplicated': 'publish fixed emptyBank [] row (.stop,0) false = 1',
            'bit_deduplicated': 'publish fixed emptyBank [] row (.bit,0) false = 1',
            'final_stop_omitted': 'publish fixed emptyBank [] row (.stop,0) true = 0',
            'inversion_training_ignored': 'publish causal learned [] [[true,false]] (.bit,1) false = 2',
            'postrow_scoring': 'Equal (point uniform [] row) (point (model fixed (publish fixed emptyBank [] row) 2 (by decide)) [] row)',
            'cut_already_normalized': 'Equal (partialSum (fun n => slice uniform n []) 2) one',
            'strict_geometric_tail': 'Less (decay 1 2) ⟨1,4,by decide⟩',
        }
        for name, claim in claims.items():
            with self.subTest(name=name):
                self.prove('example : ' + claim + ' := by decide_cbv', FALSE)


if __name__ == '__main__':
    unittest.main()
