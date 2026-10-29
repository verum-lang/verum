"""Literal-free typed numerals, partial machine bounds, and false controls."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER='''import VerumExternalReplay.NumeralPrograms
open NumeralPrograms NumeralPrograms.Typed TypedPrograms
set_option warningAsError true
'''


class NumeralProgramTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.NumeralPrograms'))

    def test_typed_composition_and_exact_machine_boundary(self):
        text=(ROOT/'VerumExternalReplay/NumeralPrograms.lean').read_text()
        core,typed=text.split('namespace Typed')
        names=['NumeralPrograms.'+n for n in re.findall(r'^theorem (\w+)',core,re.M)]
        names+=['NumeralPrograms.Typed.'+n for n in re.findall(r'^theorem (\w+)',typed,re.M)]
        with tempfile.TemporaryDirectory(prefix='verum-numeral-laws-') as directory:
            path=Path(directory)/'audit.lean'
            path.write_text(HEADER+'''
example : (numeral 7).run (semantics (fun n => some (n+1))) empty = some 7 := by decide_cbv
example : (numeral 7).run (semantics (boundedSuccessor 7)) empty = some 7 := by decide_cbv
example : (numeral 8).run (semantics (boundedSuccessor 7)) empty = none := by decide_cbv
example : boundedSuccessor u64Maximum (u64Maximum-1) = some u64Maximum := by decide_cbv
example : boundedSuccessor u64Maximum u64Maximum = none := by decide_cbv
example : (numeral 0).run (semantics (boundedSuccessor 0)) empty = some 0 := by decide_cbv
example : (numeral 1).run (semantics (boundedSuccessor 0)) empty = none := by decide_cbv
example : (numeral 3).run (semantics (fun _ => none)) empty = none := by decide_cbv
'''+ '\n'.join('#print axioms '+name for name in names))
            result=check(path)
        rows=[d['data'] for d in result['diagnostics'] if d['severity']=='information']
        self.assertEqual(len(rows),len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')),row)

    def test_false_interpretations_fail_semantically(self):
        claims={
            'zero_alone_names_positive': '(numeral 1).run (semantics (fun _ => some 0)) empty = some 1',
            'overflow_wraps': 'boundedSuccessor u64Maximum u64Maximum = some 0',
            'overflow_is_mathematical_successor': 'boundedSuccessor u64Maximum u64Maximum = some (u64Maximum+1)',
            'maximum_itself_rejected': 'boundedSuccessor u64Maximum (u64Maximum-1) = none',
            'partial_failure_recovers': '(numeral 3).run (semantics (fun _ => none)) empty = some 3',
            'bounded_grammar_is_unbounded': '(numeral 8).run (semantics (boundedSuccessor 7)) empty = some 8',
        }
        with tempfile.TemporaryDirectory(prefix='verum-numeral-controls-') as directory:
            for name,claim in claims.items():
                with self.subTest(name=name):
                    path=Path(directory)/(name+'.lean')
                    path.write_text(HEADER+f'example : {claim} := by decide_cbv\n')
                    result=check(path,'`decide_cbv` failed: the proposition evaluates to `false`')
                    errors=[d for d in result['diagnostics'] if d['severity']=='error']
                    self.assertEqual(len(errors),1)
                    self.assertIn('proposition evaluates to `false`',errors[0]['data'])


if __name__=='__main__': unittest.main()
