"""Synthetic controls for partial-bank and observed publication correspondence."""
import copy
import gzip
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from check_residual_boundaries import emit
from check_typed_programs import ROOT, check, require_success, run
from observer_export_replay import batches
from test_residual_queries import fixture as forecast_fixture, FALSE


def fixture():
    rows = [dict(context=[9], mask=[], target=[]), dict(context=[9], mask=[5], target=[5])]
    packets = [forecast_fixture(rows[:n], context=[9], mask=[5]) for n in range(3)]
    before = copy.deepcopy(packets[2]['forecast'])
    before['lengths'] = copy.deepcopy(packets[1]['forecast']['lengths'])
    last = copy.deepcopy(before)
    before['reference'] = before['reference'][:7]
    before['residual'][0]['counts'] = before['residual'][0]['counts'][:7]
    committed = copy.deepcopy(packets[2]['forecast'])
    del committed['prediction']
    committed.update(samples=2, bits=8, ratio=dict(numerator='1', denominator='1'))
    return dict(format='synarc-residual-boundaries-v2', queries=packets, committed=committed,
                partial=dict(before_refusal=before, after_refusal=copy.deepcopy(before),
                    after_last_bit=last, positions=[7,7,8], length_total=1, length_totals=[1,1,1],
                    partial_finish_refused=True, last_bit_finish_refused=True, zero_work_refused=True,
                    empty_finish_refused=True, empty_zero_work_refused=True))


class BoundaryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.AdaptiveResidual'))

    def prove(self, data, expected=None):
        source, counts = emit(data)
        with tempfile.TemporaryDirectory(prefix='verum-boundary-control-') as directory:
            path = Path(directory)/'control.lean'
            path.write_text(source)
            result = check(path, expected)
            if expected:
                errors = [d for d in result['diagnostics'] if d['severity'] == 'error']
                self.assertTrue(errors)
                self.assertTrue(all(d['data'].startswith(FALSE) for d in errors), errors)
        return source, counts

    def test_complete_boundary_capture(self):
        source, counts = self.prove(fixture())
        self.assertEqual(counts['boundary_claims'], 16)
        self.assertEqual(sum(len(names) for _, names in batches(source)), counts['claims'])

    def test_frozen_native_export_identities(self):
        for stem, suffix in [('residual-native-v2', '.json.gz'),
                             ('residual-queries-v2', '.jsonl.gz'),
                             ('residual-boundaries-v2', '.json')]:
            root = ROOT/'fixtures'
            data = (root/(stem+suffix)).read_bytes()
            if suffix.endswith('.gz'): data = gzip.decompress(data)
            provenance = json.loads((root/(stem+'.provenance.json')).read_text())
            self.assertEqual(hashlib.sha256(data).hexdigest(), provenance['export_sha256'])

    def test_altered_observations_are_rejected_by_kernel(self):
        changes = ['before', 'after_refusal', 'last', 'early_length', 'totals', 'length_total',
                   'commit_bits', 'commit_ratio', 'commit_samples', 'frozen']
        changes += ['partial_finish_refused', 'last_bit_finish_refused', 'zero_work_refused',
                    'empty_finish_refused', 'empty_zero_work_refused']
        for change in changes:
            data = fixture()
            partial = data['partial']
            if change in ('before', 'after_refusal', 'last'):
                key = {'before':'before_refusal','after_refusal':'after_refusal','last':'after_last_bit'}[change]
                partial[key]['reference'][0] = [1,0]
            elif change == 'early_length': partial['after_last_bit']['lengths']['counts']['0'] = 2
            elif change == 'totals': partial['length_totals'][-1] = 2
            elif change == 'length_total': partial['length_total'] = 2
            elif change == 'commit_bits': data['committed']['bits'] = 7
            elif change == 'commit_ratio': data['committed']['ratio']['numerator'] = '2'
            elif change == 'commit_samples': data['committed']['samples'] = 1
            elif change == 'frozen': data['queries'][0]['queries'][0]['candidate']['numerator'] = '2'
            else: partial[change] = False
            with self.subTest(change=change): self.prove(data, FALSE)

    def test_ambiguous_or_incomplete_boundary_is_refused(self):
        for change in ('format', 'history', 'row_order', 'positions', 'totals', 'flag_type',
                       'law', 'prediction', 'context', 'cells', 'extra', 'missing'):
            data = fixture()
            partial = data['partial']
            if change == 'format': data['format'] = 'synarc-residual-boundaries-v1'
            elif change == 'history': data['queries'][1]['rows'] = []
            elif change == 'row_order': data['queries'][2]['rows'].reverse()
            elif change == 'positions': partial['positions'] = [7,8]
            elif change == 'totals': partial['length_totals'] = [1,1]
            elif change == 'flag_type': partial['zero_work_refused'] = 1
            elif change == 'law': partial['before_refusal']['forecast_law'] = 'old'
            elif change == 'prediction': partial['before_refusal']['prediction']['mask'] = []
            elif change == 'context': partial['before_refusal']['residual'][0]['context'] = [8]
            elif change == 'cells': partial['before_refusal']['reference'].pop()
            elif change == 'extra': partial['trusted'] = True
            else: del partial['after_refusal']
            with self.subTest(change=change), self.assertRaises(ValueError): emit(data)


if __name__ == '__main__':
    unittest.main()
