"""Bounded delayed transport denotes the exact lasso action, not a history."""
import re
import unittest
from lean_test_fixture import IsolatedLeanTest

HEADER = '''import VerumExternalReplay.BoundedLasso
open BoundedLasso
set_option warningAsError true
'''


class BoundedLassoTests(IsolatedLeanTest):
    MODULES = ('BoundedLasso',)

    def test_constructive_axioms(self):
        names = ['BoundedLasso.'+n for n in re.findall(r'^theorem (\w+)', self.sources['BoundedLasso'], re.M)]
        self.assertEqual(len(names), 14)
        self.assert_axioms(self.prove(HEADER+'\n'.join('#print axioms '+n for n in names)), names)

    def test_transient_arbitrary_member_and_wrapped_cycle(self):
        self.prove(HEADER+'''
example : (List.range 10).map (fun k => transport 5 2 (elapsed 5 2 k) 0) =
    [0,1,2,3,4,2,3,4,2,3] := by decide_cbv
example : (List.range 7).map (fun k => transport 5 2 (elapsed 5 2 k) 3) =
    [3,4,2,3,4,2,3] := by decide_cbv
example : (List.range 6).map (fun k => transport 3 0 (elapsed 3 0 k) 1) =
    [1,2,0,1,2,0] := by decide_cbv
example : transport 1 0 (elapsed 1 0 99999999999999999999999999999) 0 = 0 := by decide_cbv
example (k i : Nat) (inside : i < 5) :
    transport 5 2 (elapsed 5 2 k) i = iterate 5 2 k i :=
  transport_exact 5 2 k i (by decide) inside
''')

    def test_increment_and_machine_boundary_intermediates(self):
        self.prove(HEADER+'''
def maximum : Nat := 18446744073709551615
example : push maximum 0 ⟨maximum,maximum-1⟩ = ⟨maximum,0⟩ := by decide_cbv
example : push maximum 0 ⟨maximum-1,maximum-1⟩ = ⟨maximum,0⟩ := by decide_cbv
example : subResidue maximum 0 (maximum-1) = 1 := by decide_cbv
example : 1 = subResidue maximum 0 (maximum-1) :=
  subResidue_unique maximum 0 (maximum-1) 1 (by decide) (by decide)
example : maximum-2 = subResidue maximum (maximum-1) 1 :=
  subResidue_unique maximum (maximum-1) 1 (maximum-2) (by decide) (by decide)
example : transport maximum 0 (elapsed maximum 0 maximum) (maximum-1) = maximum-1 := by decide_cbv
example : (List.range 30).foldl (fun summary _ => push 5 2 summary) (elapsed 5 2 0) =
    elapsed 5 2 30 := by decide_cbv
example (k : Nat) : transport 5 2 (push 5 2 (elapsed 5 2 k)) 1 =
    next 5 2 (transport 5 2 (elapsed 5 2 k) 1) := transport_push 5 2 k 1 (by decide) (by decide)
''')

    def test_equal_transformations_do_not_authenticate_commit_counts(self):
        self.prove(HEADER+'''
example : elapsed 5 2 8 = elapsed 5 2 11 := by decide_cbv
example : elapsed 1 0 1 = elapsed 1 0 2 := by decide_cbv
example : (8:Nat) ≠ 11 := by decide
''')

    def test_semantic_mutations_refused(self):
        claims = {
            'early_saturation': 'transport 5 2 ⟨min 5 (5-2),5%(5-2)⟩ 0 = iterate 5 2 5 0',
            'forgotten_transient': '2 + ((1+1) % (5-2)) = iterate 5 2 1 1',
            'wrong_modulus': 'transport 5 2 ⟨5,6%5⟩ 0 = iterate 5 2 6 0',
            'wrong_wrap_target': 'next 5 0 4 = next 5 2 4',
            'unmodded_subtraction': '((6%(5-2) - (5%(5-2))) % (5-2)) = subResidue 3 (6%3) (5%3)',
            'skipped_commit': 'transport 5 2 (elapsed 5 2 7) 0 = iterate 5 2 8 0',
            'doubled_commit': 'transport 5 2 (push 5 2 (push 5 2 (elapsed 5 2 7))) 0 = iterate 5 2 8 0',
            'after_threshold': 'transport 5 2 (elapsed 5 2 5) 0 = 5',
            'history_is_injective': 'elapsed 5 2 8 ≠ elapsed 5 2 11',
            'query_phase_from_age': 'transport 5 2 (elapsed 5 2 1) 3 = 1%3',
        }
        for name, claim in claims.items():
            with self.subTest(claim=name):
                self.prove(HEADER+'example : '+claim+' := by decide_cbv', failure=True)


if __name__ == '__main__':
    unittest.main()
