"""Corruption controls derived from preserved original native observations."""
import copy
import gzip
import json
from pathlib import Path
import tempfile
import unittest
from check_typed_programs import check, require_success, run
from check_retained_invocation_export import emit


class NativeInvocationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.RetainedNative',
                            'VerumExternalReplay.ObserverAdmission'))
        cls.original = json.loads(gzip.decompress((Path(__file__).parent /
            'fixtures/shared-invocation-native-v1.json.gz').read_bytes()))

    def case(self, name):
        data = copy.deepcopy(self.original)
        data['cases'] = [next(c for c in data['cases'] if c['id'] == name)]
        data['associations'] = []
        return data, data['cases'][0]

    def event(self, case, kind, event):
        return next(e for s in case['services'] for e in s['trace']
                    if e['kind'] == kind and e['event']['kind'] == event)

    def proof(self, data, expected=None):
        source, counts = emit(data)
        with tempfile.TemporaryDirectory(prefix='verum-invocation-control-') as directory:
            path = Path(directory) / 'control.lean'; path.write_text(source)
            check(path, expected)
        return counts

    def test_all_original_events_accounted_for(self):
        _, counts = emit(self.original)
        self.assertEqual(counts, {'cases': 11, 'associations': 2, 'services': 216,
            'native_events': 5288, 'claims': 1093, 'nonpoisoned_terminal_retries': 1})

    def test_root_body_survives_denied_selection(self):
        data, _ = self.case('root_output_work')
        self.proof(data)

    def test_numeric_literals_cannot_impersonate_booleans(self):
        for number in (0, 0.0):
            with self.subTest(number=number):
                data, case = self.case('read_whole')
                case['services'][0]['budgetAfter']['poisoned'] = number
                with self.assertRaisesRegex(ValueError, 'Native scalar'): emit(data)
        data, case = self.case('read_whole')
        case['services'][0]['returned']['result']['sameCut'] = 1
        with self.assertRaisesRegex(ValueError, 'Native scalar'): emit(data)

    def test_owned_state_corruptions_rejected(self):
        edits = [
            ('call', 'service_active', ['cut', 'state'], '999999'),
            ('call', 'service_active', ['input', 'owner'], '999999'),
            ('call', 'service_active', ['definition', 'readCode'], '999999'),
            ('call', 'service_active', ['definition', 'semantics'], '999999'),
            ('call', 'service_active', ['originalLimit', 'max'], '999999'),
            ('call', 'input_inspected', ['inspected'], '2'),
            ('invocation', 'service_enter', ['retainedInput', 'owner'], '999999'),
            ('invocation', 'service_active', ['poisoned'], False),
            ('invocation', 'body_complete', ['body', 'registers'], []),
        ]
        for kind, event, path, value in edits:
            with self.subTest(event=event, path=path):
                data, case = self.case('read_whole')
                target = self.event(case, kind, event)['state']
                for key in path[:-1]: target = target[key]
                target[path[-1]] = value
                with self.assertRaises(ValueError): emit(data)

    def test_budget_body_classification_and_unwind_corruptions_rejected(self):
        def delta(case): case['services'][1]['dispatchDelta'] = '1'
        def body(case): case['services'][1]['before']['invocation']['body'] = None
        def refund(case):
            s = case['services'][0]; s['meter']['after'] = str(int(s['meter']['after']) + 1)
        def unpoison(case): case['services'][0]['budgetAfter']['poisoned'] = False
        def classification(case):
            self.event(case, 'invocation', 'classification_result')['event']['classification'] = 'Undefined'
        def duplicate(case):
            s = case['services'][-1]; s['trace'].insert(1, copy.deepcopy(s['trace'][0]))
        for name, edit in [
            ('started_instruction_zero_new_dispatch', delta),
            ('root_output_work', body), ('root_output_work', duplicate),
            ('primitive_resource', classification),
            ('primitive_unwind_fresh_budget_refused', refund),
            ('primitive_unwind_fresh_budget_refused', unpoison),
        ]:
            with self.subTest(case=name, edit=edit.__name__):
                data, case = self.case(name); edit(case)
                with self.assertRaises(ValueError): emit(data)

    def test_wrong_root_value_rejected_by_kernel(self):
        data, case = self.case('read_whole')
        self.event(case, 'invocation', 'root_output_result')['event']['value'] = '999'
        case['services'][-1]['returned']['result']['value'] = '999'
        self.proof(data, 'Tactic `rfl` failed:')

    def test_wrong_binding_rejected_by_kernel(self):
        data, _ = self.case('read_whole')
        # Keep all repeated definition observations consistent while changing
        # the source binding. Only replay against the actual bound values can
        # distinguish this from a structurally valid extraction.
        def change(value):
            if isinstance(value, dict):
                if 'readBinding' in value: value['readBinding']['slots'][1] = {'state': True}
                for child in value.values(): change(child)
            elif isinstance(value, list):
                for child in value: change(child)
        change(data)
        self.proof(data, 'Tactic `rfl` failed:')


if __name__ == '__main__': unittest.main()
