"""Byte provenance, exact extension and strict expressiveness controls."""
import gzip
import hashlib
import json
from pathlib import Path
import re
import tempfile
import unittest

from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.AcquiredBytes
set_option warningAsError true
open AcquiredBytes TypedPrograms
def literalPair : AcquiredBytes.Program := .ret (.literal (.pair (.bytes [7]) (.bytes [8])))
def inputPair : AcquiredBytes.Program :=
  .call (params := [(),()]) (s := ()) (some .pair) (.cons (.reg .here) (.cons (.reg .here) .nil)) (.ret (.reg .here))
def refused : AcquiredBytes.Program :=
  .call (params := [(),()]) (s := ()) (some .rem) (.cons (.literal (.nat 1)) (.cons (.literal (.nat 0)) .nil)) (.ret (.reg .here))
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class AcquiredBytesTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.AcquiredBytes'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-byte-provenance-') as directory:
            path = Path(directory)/'check.lean'
            path.write_text(HEADER + source)
            result = check(path, expected)
            if expected:
                errors = [d for d in result['diagnostics'] if d['severity'] == 'error']
                self.assertEqual(len(errors), 1, errors)
                self.assertTrue(errors[0]['data'].startswith(expected), errors)
            return result

    def test_axioms(self):
        names = []
        for module in ('ProgramProperties', 'AcquiredBytes'):
            names += [module+'.'+name for name in re.findall(r'^theorem (\w+)',
                (ROOT/f'VerumExternalReplay/{module}.lean').read_text(), re.M)]
        result = self.prove('\n'.join('#print axioms '+name for name in names))
        rows = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rows), len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)

    def test_leaf_boundaries_and_literals(self):
        self.prove('''
example : apply .at [.bytes [1,2],.nat 0] = some (.nat 1) := by decide_cbv
example : apply .at [.bytes [1,2],.nat 2] = none := by decide_cbv
example : apply .querySensory [.query {sensory := some ⟨[1],[2,3]⟩}] =
  some (.pair (.bytes [1]) (.bytes [2,3])) := by decide_cbv
example : apply .at [.trace [{sensory := some ⟨[1],[2]⟩}],.nat 0] =
  some (.event {sensory := some ⟨[1],[2]⟩}) := by decide_cbv
example : runOn literalPair [] = some ([7],[8]) := by decide_cbv
example : literalLeaves literalPair = [[7],[8]] := by decide_cbv
example : literalLeaves inputPair = [] := by decide_cbv
example : runOn inputPair [4,5] = some ([4,5],[4,5]) := by decide_cbv
example : runOn inputPair [] ≠ some ([],[0]) := literal_free_witness _ rfl
example : ∃ input, runOn literalPair input ≠ some (input,input ++ [0]) :=
  append_zero_not_expressible literalPair
example : decode (.pair (.nat 1) (.bytes [1])) = none := by decide_cbv
def callee : FiniteCode.Code Op Value :=
  .mk [⟨.primitive .pair,[.input 0,.literal (.bytes [7])],none⟩] (some 0) []
def caller : FiniteCode.Code Op Value := .mk [⟨.call 0,[.input 0],none⟩] (some 0) [callee]
example : FiniteCode.run apply caller [.bytes [3]] =
  some (.pair (.bytes [3]) (.bytes [7])) := by decide_cbv
example : [7] ∈ literalLeaves (FiniteCode.lower caller 1) ∨
    ∃ v ∈ [Value.bytes [3]], [7] ∈ leaves v :=
  finite_code_leaf caller [.bytes [3]] (.pair (.bytes [3]) (.bytes [7])) [7]
    (by decide_cbv) (by decide_cbv)
''')

    def test_conservative_extension_and_constructed_witness(self):
        self.prove('''
example : extendedRun sequenceApply (embedOld literalPair) [4] = runOn literalPair [4] :=
  embedding_exact sequenceApply literalPair [4]
example : extendedRun sequenceApply (embedOld refused) [] = none := by decide_cbv
example : sequenceApply .singleton [.nat 256] = none := by decide_cbv
example : sequenceApply .concat [.bytes [3],.bytes [4,5]] = some (.bytes [3,4,5]) := by decide_cbv
example (input : Bytes) : extendedRun sequenceApply sequenceWitness input =
    some (input,input ++ [0]) := sequence_realizes input
example : ¬ ∃ p : AcquiredBytes.Program, ∀ input,
    extendedRun sequenceApply sequenceWitness input = runOn p input := sequence_strict.2
''')

    def test_false_claims_fail_in_kernel(self):
        for claim in [
            'apply .at [.bytes [1,2],.nat 0] = some (.bytes [1])',
            'runOn inputPair [1] = some ([1],[1,0])',
            'literalLeaves literalPair = []',
            'extendedRun sequenceApply (embedOld refused) [] = some ([],[])',
            'sequenceApply .singleton [.nat 256] = some (.bytes [0])',
            'extendedRun sequenceApply sequenceWitness [] = runOn inputPair []',
        ]:
            with self.subTest(claim=claim):
                self.prove('example : '+claim+' := by decide_cbv', FALSE)

    def test_pinned_source_operation_roster(self):
        fixture = ROOT/'fixtures/acquired-bytes-source.json'
        receipt = json.loads(fixture.read_text())
        source = gzip.decompress(fixture.with_suffix('.rs.gz').read_bytes())
        self.assertEqual(hashlib.sha256(source).hexdigest(), receipt['source_sha256'])
        enum = re.search(r'enum Op \{([^}]+)\}', source.decode()).group(1)
        native = re.findall(r'\b[A-Z]\w*\b', enum)
        lean = (ROOT/'VerumExternalReplay/AcquiredBytes.lean').read_text()
        model = re.search(r'inductive Op where(.*?)deriving', lean, re.S).group(1)
        names = re.findall(r'\|\s*(\w+)', model)
        self.assertEqual([op[0].lower()+op[1:] for op in native], names)
        self.assertEqual(native, receipt['primitive_roster'])


if __name__ == '__main__':
    unittest.main()
