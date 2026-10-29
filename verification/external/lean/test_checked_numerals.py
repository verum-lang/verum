"""Checked native successor decoding, execution, and mathematical correspondence."""
from pathlib import Path
import re
import tempfile
import unittest
from acquired_export_codec import LEGACY,SENSORY
from check_typed_programs import ROOT,check,require_success,run

HEADER='''import VerumExternalReplay.CheckedNumerals
open AcquiredObserver CheckedNumerals
set_option warningAsError true
deriving instance DecidableEq for Except
'''
FALSE='`decide_cbv` failed: the proposition evaluates to `false`'


def numeral(n):
    return dict(steps=[dict(operation=dict(Primitive='Zero'),arguments=[])]+
        [dict(operation=dict(Primitive='Succ'),arguments=[dict(Register=i)]) for i in range(n)],
        output=n,library=[])


class CheckedNumeralTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.CheckedNumerals'))

    def prove(self,source,expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-checked-numeral-') as directory:
            path=Path(directory)/'check.lean';path.write_text(HEADER+source)
            return check(path,expected)

    def test_decoded_closed_code_and_library_use_checked_successor(self):
        code=numeral(17)
        wrapper=dict(steps=[dict(operation=dict(Call=0),arguments=[])],output=0,library=[code])
        self.prove(f'''
def closed : FiniteCode.Code Op Value := {SENSORY.code(code)}
def caller : FiniteCode.Code Op Value := {SENSORY.code(wrapper)}
example : FiniteCode.run (fun op args => (apply 0 op args).toOption) closed [] = some (.nat 17) := by decide_cbv
example : FiniteCode.run (fun op args => (apply 0 op args).toOption) caller [] = some (.nat 17) := by decide_cbv
example : apply 0 .succ [.nat 18446744073709551614] = .ok (.nat 18446744073709551615) := by decide_cbv
example : apply 0 .succ [.nat 18446744073709551615] = .error .overflow := by decide_cbv
example : apply 0 .succ [] = .error .arity := by decide_cbv
example : apply 0 .succ [.unit] = .error .sort := by decide_cbv
example : primitiveClass .overflow = .undefined := rfl
''')
        with self.assertRaisesRegex(ValueError,'Unknown variant'): LEGACY.code(code)
        unknown=numeral(1);unknown['steps'][1]['operation']['Primitive']='UncheckedSucc'
        with self.assertRaisesRegex(ValueError,'Unknown variant'): SENSORY.code(unknown)

    def test_universal_correspondence_axioms(self):
        source=(ROOT/'VerumExternalReplay/CheckedNumerals.lean').read_text()
        names=re.findall(r'^theorem (\w+)',source,re.M)
        result=self.prove('\n'.join('#print axioms '+name for name in names))
        rows=[d['data'] for d in result['diagnostics'] if d['severity']=='information']
        self.assertEqual(len(rows),len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms','depends on axioms: [propext]',
                'depends on axioms: [propext, Quot.sound]')),row)

    def test_wrong_execution_results_refuse_semantically(self):
        for claim in [
            'apply 0 .succ [.nat 18446744073709551615] = .ok (.nat 0)',
            'apply 0 .succ [.nat 18446744073709551614] = .error .overflow',
            'apply 0 .succ [] = .error .sort',
            'apply 0 .succ [.unit] = .ok (.nat 1)',
            'primitiveClass .overflow = .resource',
            'naturalResult (apply 0 .succ [.nat 7]) = some 7',
        ]:
            with self.subTest(claim=claim):
                result=self.prove('example : '+claim+' := by decide_cbv',FALSE)
                errors=[d for d in result['diagnostics'] if d['severity']=='error']
                self.assertEqual(len(errors),1)
                self.assertIn('proposition evaluates to `false`',errors[0]['data'])


if __name__=='__main__':unittest.main()
