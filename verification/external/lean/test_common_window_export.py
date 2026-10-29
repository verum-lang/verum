"""Native common-window scoring replay and strict semantic corruption controls."""
import copy
from fractions import Fraction
from pathlib import Path
import tempfile
import unittest
from check_common_window_export import MODULES, emit
from check_typed_programs import ROOT, check, require_success, run, sha256
from observer_export_replay import load

FIXTURE = ROOT / 'fixtures/common-window-native-v1.json'
SHA = 'f5b665781cbffcd16d86c5096d6fa7644aa4c441980e4a81f515e4975305d088'
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


def packet():
    if sha256(FIXTURE) != SHA: raise RuntimeError('Frozen common-window fixture changed')
    return load(FIXTURE)


def rescore(data):
    """Independent arithmetic oracle for explicitly synthetic varied-support cases."""
    def score(side):
        counts = {}; result = Fraction(1)
        for row in data['ordered_rows']:
            context = str(row['context']); label = row['outcome']
            key = (context, row[side + '_reading']) if side else context
            histogram = counts.setdefault(key, {})
            result *= Fraction(2 * histogram.get(label, 0) + 1,
                               2 * sum(histogram.values()) + len(data['support'][context]))
            histogram[label] = histogram.get(label, 0) + 1
        return result
    parent, old, new = score(None), score('old'), score('new')
    for key, mass in [('parent_kt', parent), ('old_child_kt', old), ('new_child_kt', new),
                      ('old_native_compression_ratio', old / parent),
                      ('new_native_compression_ratio', new / parent), ('child_ratio', new / old)]:
        data[key] = dict(numerator=str(mass.numerator), denominator=str(mass.denominator))


class CommonWindowExportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', *('VerumExternalReplay.' + m for m in MODULES)))

    def prove(self, data, failure=False):
        source, counts = emit(data)
        with tempfile.TemporaryDirectory(prefix='verum-common-window-native-') as directory:
            path = Path(directory) / 'claims.lean'; path.write_text(source)
            result = check(path, FALSE if failure else None)
        if failure:
            errors = [d for d in result['diagnostics'] if d['severity'] == 'error']
            self.assertEqual(len(errors), 1, errors)
            self.assertTrue(errors[0]['data'].startswith(FALSE), errors)
        return counts

    def test_native_readings_and_declared_support(self):
        self.assertEqual(self.prove(packet()), dict(rows=96, contexts=2, conditions=22, claims=1))

    def test_semantic_corruptions_reach_kernel(self):
        controls = {
            'different_window': lambda d: d.update(same_window_owner=False),
            'invent_admission': lambda d: d.update(descriptive_only=False),
            'missing_context': lambda d: d['ordered_rows'][0].update(context=8),
            'foreign_label': lambda d: d['ordered_rows'][0].update(outcome=8),
            'rename_old_cell': lambda d: d['ordered_rows'][0].update(old_reading=d['ordered_rows'][2]['old_reading']),
            'merge_new_cells': lambda d: d['ordered_rows'][0].update(new_reading=2),
            'lost_row': lambda d: d['ordered_rows'].pop(),
            'empty_window': lambda d: d.update(ordered_rows=[]),
            'duplicate_sample': lambda d: d['ordered_rows'][1].update(ordinal=0),
            'wrong_parent': lambda d: d['parent_kt'].update(numerator='1'),
            'zero_child': lambda d: d['old_child_kt'].update(numerator='0'),
            'wrong_compression': lambda d: d['new_native_compression_ratio'].update(denominator='1'),
            'wrong_child_ratio': lambda d: d['child_ratio'].update(numerator='1'),
            'drop_name_factor': lambda d: d['new_original_price'].update(mass_denominator='1'),
            'invent_support': lambda d: d['support']['0'].append(9),
        }
        for name, mutate in controls.items():
            with self.subTest(control=name):
                data = packet(); mutate(data); self.prove(data, True)

    def test_original_paid_look_coherence(self):
        data = packet()
        data['new_original_price'].update(authority='JointPaidLook', paid_look='7', mass_denominator='144')
        self.prove(data)
        data['new_original_price']['mass_denominator'] = '140'
        self.prove(data, True)

    def test_varied_declared_alphabets_and_unobserved_context(self):
        data = packet()
        data['support'] = {'7': [11, 13], '19': [2, 3, 5], '31': [20, 21, 22, 23]}
        data['old_training_support'] = copy.deepcopy(data['support'])
        data['new_training_support'] = copy.deepcopy(data['support'])
        data['ordered_rows'] = [dict(origin=0, ordinal=i, cut=i, context=context, outcome=label,
            query_interface=1, query_values=[i], old_reading=cell, new_reading=i % 2)
            for i, (context, label, cell) in enumerate([(7,11,4), (19,2,4), (7,13,5),
                (19,3,4), (7,11,4), (19,2,5), (7,13,4)])]
        rescore(data); self.prove(data)
        # Removing an unobserved label changes each relevant KT denominator.
        data['support']['19'].remove(5)
        data['old_training_support']['19'].remove(5)
        data['new_training_support']['19'].remove(5)
        self.prove(data, True)

    def test_readings_remain_an_explicit_input_boundary(self):
        data = packet()
        data['old_definition']['read'] = 'Opaque program description; no executable semantics'
        data['ordered_rows'][0]['query_values'] = [42]
        # Scores cannot establish that the supplied reading came from this
        # query/program. Accepting it documents the deliberately limited scope.
        self.prove(data)

    def test_malformed_schema_refused_before_proof(self):
        controls = [
            lambda d: d.update(schema='future-unimplemented-schema'),
            lambda d: d.update(unreplayed=True),
            lambda d: d['ordered_rows'][0].update(context=True),
            lambda d: d['ordered_rows'][0].update(old_reading=-1),
            lambda d: d['ordered_rows'][0].update(context=256),
            lambda d: d['ordered_rows'][0].update(query_values=[2**64]),
            lambda d: d['parent_kt'].update(numerator='1; axiom forged : False'),
            lambda d: d['parent_kt'].update(denominator='0'),
            lambda d: d['parent_kt'].update(numerator='01'),
            lambda d: d['parent_kt'].update(numerator=1),
            lambda d: d['support']['0'].append(1),
            lambda d: d['support'].update({'01': [0,1]}),
            lambda d: d['new_original_price'].update(authority='Unknown'),
            lambda d: d['new_original_price'].update(paid_look='7'),
        ]
        for index, mutate in enumerate(controls):
            with self.subTest(control=index):
                data = packet(); mutate(data)
                with self.assertRaises(ValueError): emit(data)


if __name__ == '__main__': unittest.main()
