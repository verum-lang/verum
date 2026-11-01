"""Strict coordinate/coverage controls and kernel-checked frozen arithmetic."""
import copy
import unittest

from check_fixed_kt_export import emit
from lean_test_fixture import IsolatedLeanTest


def ratio(n, d):
    return {'numerator': str(n), 'denominator': str(d)}


def fixture():
    assessment = {'contexts': [{'context': ['1', '0', '0'], 'outcome_labels': ['x', 'y'],
        'cell_labels': ['left', 'right'], 'parent_counts': [1, 1], 'child_counts': [[1, 0], [0, 1]]}],
        'ordered_rows': [[0, 0, 0, 0, 10], [1, 0, 1, 1, 11]],
        'child_kt': ratio(1, 4), 'compression': ratio(2, 1), 'predictive': ratio(1, 1)}
    return {'schema': 'synarc-fixed-window-envelope-v1', 'scope': 'diagnostic', 'window_rows': 2,
        'incumbent': assessment, 'incumbent_mass_denominator': '1', 'family_count': '100', 'look': '0',
        'envelope_child_kt': ratio(1, 4), 'envelope_predictive': ratio(1, 1), 'envelope_gain': ratio(1, 1),
        'candidates': [{'numeral': 1, 'original_name': '3', 'original_codes': [[0], [1], [0]],
                        'mass_denominator': '200', 'native_assessment': copy.deepcopy(assessment),
                        'priced_envelope': ratio(1, 200)}]}


class FrozenKT(IsolatedLeanTest):
    MODULES = ('FixedKTNative',)

    def test_exact_frozen_arithmetic(self):
        source, counts = emit(fixture(), ['3'])
        self.assertEqual(counts['assessments'], 2)
        self.assertGreater(counts['claims'], 15)
        self.prove(source)
        self.assert_axioms(self.prove('import VerumExternalReplay.FixedKTNative\n'
            '#print axioms FixedKTNative.factor_range\n'
            '#print axioms FixedKTNative.counter_range\n'
            '#print axioms FixedKTNative.counter_zero_origin\n'),
            ['FixedKTNative.factor_range', 'FixedKTNative.counter_range', 'FixedKTNative.counter_zero_origin'])

    def test_changed_native_fraction_is_false_in_lean(self):
        data = fixture()
        data['incumbent']['child_kt'] = ratio(1, 5)
        source, _ = emit(data, ['3'])
        self.prove(source, failure=True)

    def test_undefined_fractions_cannot_compare(self):
        for relation in ('same', 'le'):
            for numerator, denominator in ((0, 0), (0, 1), (1, 0)):
                with self.subTest(relation=relation, numerator=numerator, denominator=denominator):
                    self.prove('import VerumExternalReplay.FixedKTNative\nopen FixedKTNative\n'
                        f'example : (Fraction.mk {numerator} {denominator}).{relation} ⟨1, 2⟩ := by decide_cbv\n',
                        failure=True)

    def test_lean_intake_refuses_foreign_coordinates(self):
        for context, cell, outcome in ((1, 0, 0), (0, 1, 0), (0, 0, 2)):
            with self.subTest(context=context, cell=cell, outcome=outcome):
                self.prove('import VerumExternalReplay.FixedKTNative\nopen FixedKTNative\n'
                    f'example : ingest [⟨2, [0, 0], [[0, 0]]⟩] ⟨0, {context}, {cell}, {outcome}, 0⟩ = none := by decide_cbv\n')

    def test_structural_corruptions_are_refused(self):
        changes = [
            lambda d: d['candidates'].clear(),
            lambda d: d['candidates'].append(copy.deepcopy(d['candidates'][0])),
            lambda d: d['candidates'][0].update(original_name='4'),
            lambda d: d['candidates'][0].update(mass_denominator='201'),
            lambda d: d['incumbent']['ordered_rows'][1].__setitem__(0, 0),
            lambda d: d['incumbent']['ordered_rows'][1].__setitem__(2, 8),
            lambda d: d['incumbent']['ordered_rows'][1].__setitem__(4, 10),
            lambda d: d['incumbent']['contexts'][0].update(parent_counts=[2, 0]),
            lambda d: d['incumbent']['contexts'][0]['outcome_labels'].append('unused'),
            lambda d: d['incumbent']['contexts'][0].update(outcome_labels=['x', 'x']),
            lambda d: d['candidates'][0]['native_assessment']['contexts'][0].update(context=['2', '0', '0']),
            lambda d: d['candidates'][0]['original_codes'][0].append(2),
        ]
        for i, change in enumerate(changes):
            data = fixture(); change(data)
            with self.subTest(mutation=i), self.assertRaises(ValueError):
                emit(data, ['3'])


if __name__ == '__main__':
    unittest.main()
