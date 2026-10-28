"""Position non-interference, complete row semantics and strict export controls."""
import copy
from pathlib import Path
import re
import tempfile
import unittest

from check_residual_export import emit, FORECAST_LAW
from check_typed_programs import ROOT, check, require_success, run
from observer_export_replay import batches, load

FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'
HEADER = '''import VerumExternalReplay.ResidualPrediction
open ExactMass ResidualPrediction
set_option warningAsError true
def training : Row := ⟨[1], [0], [0]⟩
def fresh : Row := ⟨[1], [255], [255]⟩
def runtime : Row := ⟨[2], [0], [1]⟩
def complete := fold [training, fresh, runtime] initial
'''


def fixture():
    """Small, explicitly synthetic example; never substituted for native evidence."""
    rows = [dict(context=c, mask=[m], target=[t], native_read=m, native_state=0,
                 ordinal=i, origin=7, phase=p)
            for i, (c, m, t, p) in enumerate((([1], 0, 0, 'training'),
                                             ([1], 255, 255, 'fresh'),
                                             ([2], 0, 1, 'runtime')))]
    def frontier(n, ratio, reference, residual):
        return dict(samples=n, bits=8*n, ratio=dict(numerator=str(ratio), denominator='1'),
                    reference=reference, residual=residual)
    return dict(format='synarc-residual-native-v1', bit_order='lsb0', length_unit='byte',
                length_law='2^-(n+1)', rows=rows, frontiers=[
        frontier(1, 1, [[1, 0]]*8, [dict(context=[1], counts=[[1, 0]]*8)]),
        frontier(2, 6561, [[1, 1]]*8, [dict(context=[1], counts=[[2, 0]]*8)]),
        frontier(3, 6561, [[1, 2]]+[[2, 1]]*7, [dict(context=[1], counts=[[2, 0]]*8),
                                                  dict(context=[2], counts=[[0, 1]]+[[1, 0]]*7)])])


def adaptive_fixture():
    """Synthetic learned-bank stream including an empty completed training row."""
    data = fixture()
    data.update(format='synarc-residual-native-v2', forecast_law=FORECAST_LAW,
                length_law='(count(n)+2^-(n+1))/(N+1)')
    data['rows'].insert(0, dict(context=[99], mask=[255], target=[], native_read=0,
                               native_state=0, ordinal=0, origin=8, phase='training'))
    for f in data['frontiers']:
        n = f['samples']
        f.update(samples=n+1, forecast_law=FORECAST_LAW,
                 lengths=dict(total=n+1, counts={'0': 1, '1': n}))
    return data


class ResidualPredictionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.AdaptiveResidual'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-residual-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(source)
            return check(path, expected)

    def test_model_axioms(self):
        names = []
        for module in ('PositionPrediction', 'ResidualPrediction', 'AdaptiveResidual'):
            names += [module+'.'+name for name in re.findall(r'^theorem (\w+)',
                (ROOT / f'VerumExternalReplay/{module}.lean').read_text(), re.M)]
        result = self.prove('import VerumExternalReplay.AdaptiveResidual\n' + HEADER +
                            '\n'.join('#print axioms '+n for n in names))
        rows = [r['data'] for r in result['diagnostics'] if r['severity'] == 'information']
        self.assertEqual(len(rows), len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)

    def test_full_counts_masks_and_empty_bodies(self):
        self.prove(HEADER + '''
example : complete.samples = 3 ∧ complete.bits = 24 := by decide_cbv
example : Equal complete.ratio ⟨6561, 1, by decide⟩ := by decide_cbv
example : (PositionPrediction.lookup complete.residual ([1], 0)).val.counts = [2, 0] := by decide_cbv
example : (PositionPrediction.lookup complete.residual ([2], 0)).val.counts = [0, 1] := by decide_cbv
example : (PositionPrediction.lookup complete.reference 0).val.counts = [1, 2] := by decide_cbv
example : (PositionPrediction.lookup complete.reference 1).val.counts = [2, 1] := by decide_cbv
example : row complete ⟨[9], [255], []⟩ = {complete with samples := 4} := by decide_cbv
example : row complete ⟨[1], [], [4, 8]⟩ = row complete ⟨[1], [0, 0], [4, 8]⟩ := by decide_cbv
example : row complete ⟨[1], [5, 17], [4]⟩ = row complete ⟨[1], [5], [4]⟩ := by decide_cbv
example : byteBits 1 = [true, false, false, false, false, false, false, false] := by decide_cbv
''')

    def test_no_reset_skips_or_noninjective_forecasts(self):
        controls = [
            'Equal complete.ratio (fold [fresh, runtime] initial).ratio',
            'complete.bits = 16',
            '(PositionPrediction.lookup complete.residual ([2], 1)).val.counts = [2, 1]',
            'Equal (CausalCoding.likelihood (PositionPrediction.kernel (fun _ : Nat => 0)) '
            '⟨0, []⟩ [false, false]) (PositionPrediction.frozen (fun _ : Nat => 0) [] 0 [false, false])',
        ]
        for claim in controls:
            with self.subTest(claim=claim):
                self.prove(HEADER + f'example : {claim} := by decide_cbv', FALSE)

    def test_export_and_duplicate_adoption_frontier(self):
        data = fixture()
        data['frontiers'].insert(2, copy.deepcopy(data['frontiers'][1]))
        source, counts = emit(data)
        self.assertEqual(counts['claims'], 4)
        self.assertEqual(sum(len(ns) for _, ns in batches(source)), 4)
        names = ['complete_stream'] + [f'transition_{i}' for i in range(3)]
        source += '\n' + '\n'.join('#print axioms FrozenObserverExport.'+n for n in names)
        result = self.prove(source)
        rows = [r['data'] for r in result['diagnostics'] if r['severity'] == 'information']
        self.assertEqual(len(rows), 4)
        self.assertTrue(all(r.endswith(('depends on axioms: [propext, Quot.sound]',
            'depends on axioms: [propext]', 'does not depend on any axioms')) for r in rows), rows)

    def test_false_native_claims_are_mathematically_rejected(self):
        for change in ('ratio', 'residual', 'reference', 'bits', 'mask', 'target', 'reset', 'drop'):
            data = fixture()
            if change == 'ratio': data['frontiers'][-1]['ratio']['numerator'] = '6562'
            elif change == 'residual': data['frontiers'][-1]['residual'][0]['counts'][0] = [1, 1]
            elif change == 'reference': data['frontiers'][-1]['reference'][0] = [2, 1]
            elif change == 'bits': data['frontiers'][-1]['bits'] = 23
            elif change in ('mask', 'target'): data['rows'][1][change] = [254]
            elif change == 'reset': data['frontiers'][-1]['ratio']['numerator'] = '1'
            else: data['rows'][1]['target'] = []
            with self.subTest(change=change): self.prove(emit(data)[0], FALSE)

    def test_trace_hints_cannot_be_forged(self):
        source, _ = emit(fixture())
        changed = source.replace('def state_1 : State := ⟨1, 8,', 'def state_1 : State := ⟨1, 7,')
        self.assertNotEqual(source, changed)
        self.prove(changed, FALSE)

    def test_adaptive_completed_histograms(self):
        data = adaptive_fixture()
        source, counts = emit(data)
        self.assertEqual(counts['length_frontiers'], 3)
        self.assertEqual(counts['claims'], 8)
        self.prove(source)
        for change in ('count', 'ignored_empty', 'swapped_counts'):
            data = adaptive_fixture()
            cell = data['frontiers'][-1]['lengths']['counts']
            if change == 'count': cell['1'] = 4
            elif change == 'ignored_empty': cell['0'] = 2
            else: cell.update({'0': 3, '1': 1})
            with self.subTest(change=change):
                result = self.prove(emit(data)[0], FALSE)
                self.assertEqual(sum(d['severity'] == 'error' for d in result['diagnostics']), 1)

    def test_adaptive_schema_refuses_ambiguous_lengths(self):
        for change in ('total', 'missing_empty', 'extra', 'zero', 'key', 'law', 'frontier_law', 'unit'):
            data = adaptive_fixture()
            cell = data['frontiers'][-1]['lengths']['counts']
            if change == 'total': data['frontiers'][-1]['lengths']['total'] = 3
            elif change == 'missing_empty': del cell['0']
            elif change == 'extra': cell['8'] = 1
            elif change == 'zero': cell['0'] = 0
            elif change == 'key': cell['01'] = cell.pop('1')
            elif change == 'law': data['forecast_law'] = 'geometric'
            elif change == 'frontier_law': data['frontiers'][-1]['forecast_law'] = 'geometric'
            else: data['length_unit'] = 'bit'
            with self.subTest(change=change), self.assertRaises(ValueError): emit(data)

    def test_storage_order_does_not_change_context_identity(self):
        data = fixture()
        for row in data['rows']:
            row['context'] = [9] if row['context'] == [1] else [1]
        for frontier in data['frontiers']:
            for entry in frontier['residual']:
                entry['context'] = [9] if entry['context'] == [1] else [1]
            frontier['residual'].sort(key=lambda entry: entry['context'])
        self.prove(emit(data)[0])

    def test_strict_decoder(self):
        for change in ('boolean', 'float', 'byte', 'denominator', 'decimal', 'order', 'context',
                       'missing_cell', 'extra_cell', 'phase_cut', 'phase_order', 'identity', 'field', 'empty'):
            data = fixture()
            if change == 'boolean': data['rows'][0]['native_state'] = True
            elif change == 'float': data['rows'][0]['target'][0] = 0.0
            elif change == 'byte': data['rows'][0]['mask'][0] = 256
            elif change == 'denominator': data['frontiers'][0]['ratio']['denominator'] = '0'
            elif change == 'decimal': data['frontiers'][0]['ratio']['numerator'] = '01'
            elif change == 'order': data['frontiers'][-1]['residual'].reverse()
            elif change == 'context': data['frontiers'][-1]['residual'].append(data['frontiers'][-1]['residual'][0])
            elif change == 'missing_cell': data['frontiers'][0]['residual'][0]['counts'].pop()
            elif change == 'extra_cell': data['frontiers'][0]['reference'].append([0, 0])
            elif change == 'phase_cut': del data['frontiers'][0]
            elif change == 'phase_order': data['rows'][0]['phase'] = 'runtime'
            elif change == 'identity': data['rows'][1]['ordinal'] = 0
            elif change == 'field': data['rows'][0]['trusted'] = True
            else: data['frontiers'] = []
            with self.subTest(change=change), self.assertRaises(ValueError): emit(data)
        with tempfile.TemporaryDirectory(prefix='verum-duplicate-json-') as directory:
            path = Path(directory)/'duplicate.json'; path.write_text('{"rows": [], "rows": []}')
            with self.assertRaisesRegex(ValueError, 'Duplicate JSON key'): load(path)


if __name__ == '__main__':
    unittest.main()
