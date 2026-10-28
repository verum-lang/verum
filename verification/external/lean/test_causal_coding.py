"""Exact causal body laws; reject look-ahead and silently discarded wire mass."""
from pathlib import Path
import re
import tempfile
import unittest

from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.CausalCoding
open ExactMass CausalCoding
set_option warningAsError true

def persistent : Kernel Bool where
  probability state bit := if state == bit then ⟨3, 4, by decide⟩ else unit 4 (by decide)
  next _ bit := bit
  normalized state := by cases state <;> decide_cbv

def deterministic : Kernel Unit where
  probability _ bit := if bit then zero else one
  next _ _ := ()
  normalized _ := by decide_cbv

-- A total parser of one-bit aliases still has only one canonical encoding.
def singleton : Codec Unit where
  encode _ := [false]
  decode body := if body.length == 1 then some () else none
  roundtrip _ := rfl
'''


class CausalCodingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.CausalCoding'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-causal-coding-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            return check(path, expected)

    def test_axiom_roster(self):
        names = re.findall(r'^theorem (\w+)',
            (ROOT / 'VerumExternalReplay/CausalCoding.lean').read_text(), re.M)
        self.assertIn('selected_normalized_iff', names)
        result = self.prove('\n'.join('#print axioms CausalCoding.' + name for name in names))
        rows = [row['data'] for row in result['diagnostics'] if row['severity'] == 'information']
        self.assertEqual(len(rows), len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)

    def test_causal_scoring_and_arbitrary_mask_length(self):
        self.prove('''
example : Equal (likelihood persistent false [false, true]) ⟨3, 16, by decide⟩ := by decide_cbv
example : advance persistent false [true, false, true] = true := by decide_cbv
example : Equal (likelihood persistent false ([true] ++ [false, true]))
    (mul (likelihood persistent false [true]) (likelihood persistent true [false, true])) :=
  likelihood_append persistent false [true] [false, true]
example : Equal (sum ((words 0).map (likelihood persistent true))) one := body_normalized _ _ _
example : Equal (sum ((words 4).map (likelihood persistent false))) one := by decide_cbv
example : Equal (sum ((words 3).map (fun b => likelihood persistent true (residual [true] b)))) one := by decide_cbv
example : Equal (sum ((words 2).map (fun b => likelihood persistent false (residual [true, false, true] b)))) one := by decide_cbv
example : residual [true] [false, true, false] = [true, true, false] := by decide_cbv
example : residual [true, false, true] [false] = [true] := by decide_cbv
example (n : Nat) : (words n).Nodup := words_nodup n
example (past future mask : List Bool) : residual mask (past ++ future) =
    residual mask past ++ residual (mask.drop past.length) future := residual_append mask past future
example (n : Nat) : Equal (sum ((words n).map (fun b =>
    likelihood persistent false (residual [true, false] ([false] ++ b)))))
    (likelihood persistent false (residual [true, false] [false])) :=
  prefix_marginal persistent false [true, false] [false] n
''')

    def test_lossless_encoding_and_exact_missing_mass(self):
        self.prove('''
example : singleton.decode [true] = some () := by decide_cbv
example : canonical singleton [true] = false := by decide_cbv
example : canonical singleton [false] = true := by decide_cbv
example : Equal (selectedMass persistent false [] 1 (canonical singleton)) ⟨3, 4, by decide⟩ := by decide_cbv
example : Equal (selectedMass persistent false [] 1 (fun b => !canonical singleton b)) (unit 4 (by decide)) := by decide_cbv
example : Equal (selectedMass deterministic () [] 1 (canonical singleton)) one := by decide_cbv
example (n : Nat) : AtMost (selectedMass persistent false [true] n (canonical singleton)) one :=
  selected_bounded _ _ _ _ _
''')

    def test_weakened_claims_rejected(self):
        claims = {
            'future_body_chosen_mask':
                'Equal (sum ((words 1).map (fun b => likelihood persistent false (residual b b)))) one',
            'future_body_chosen_initial_state':
                'Equal (sum ((words 1).map (fun b => likelihood persistent (b.headD false) b))) one',
            'post_update_probability':
                'Equal (likelihood persistent false [true]) (persistent.probability (persistent.next false true) true)',
            'lossless_is_not_surjective':
                'Equal (selectedMass persistent false [] 1 (canonical singleton)) one',
            'accepted_alias_is_not_canonical': 'canonical singleton [true] = true',
            'residual_does_not_repair_image':
                'Equal (selectedMass persistent false [true] 1 (canonical singleton)) one',
            'empty_body_mass_not_omitted': 'Equal (likelihood persistent false []) zero',
            'mask_truncates_to_body_length': '(residual [true, true] [false]).length = 2',
            'length_dependent_kernel_breaks_prefix_consistency':
                'Equal (sum ((words 1).map (fun b => likelihood persistent true ([false] ++ b)))) '
                '(likelihood persistent false [false])',
        }
        for name, claim in claims.items():
            with self.subTest(name=name):
                self.prove(f'example : {claim} := by decide_cbv\n',
                           '`decide_cbv` failed: the proposition evaluates to `false`')


if __name__ == '__main__':
    unittest.main()
