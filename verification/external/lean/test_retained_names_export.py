"""Synthetic strict-decoder controls, separate from actual frozen native evidence."""
import copy
from pathlib import Path
import tempfile
import unittest
from check_retained_names_export import emit
from check_typed_programs import check, require_success, run
from observer_export_replay import batches


def fixture():
    nat = lambda n: dict(kind='Nat', value=n)
    first_keys = [dict(input=0, value=nat(257))]
    appended = first_keys + [dict(input=0, value=nat(1))]
    final = appended + [dict(input=1, value=nat(1))]
    def namespace(keys, capacity):
        return dict(owner=0, epoch='9', capacity=capacity, entries=copy.deepcopy(keys))
    def cursor(owner, input_, keys, capacity, scanned, finished=False, fork=False):
        q = dict(interface=dict(epoch='1', raw_space='2', owner=None, shape='Query'), values=[dict(
            kind='Query', value=dict(epoch='1', raw_space='2', owner=None, raw=11, input=input_,
                episode=3, life_age=4, pose=None, action=None))])
        return dict(owner=owner, input_owner=owner, namespace=namespace(keys, capacity),
            expected_namespace=namespace(keys, capacity), same_namespace=True, query=q,
            cut=dict(definition=0, owner=int(fork), state=0, value=nat(7)), input=input_, value=nat(1),
            scanned=scanned, finished=finished, poisoned=False)
    def case(name, before, after, result, grant=1):
        return dict(name=name, before=before, after=after, grant=grant, remaining=0, result=result)
    rows = [
        case('denied', cursor(0, 0, first_keys, 3, 0), cursor(0, 0, first_keys, 3, 0), dict(kind='Work'), 0),
        case('compare', cursor(0, 0, first_keys, 3, 0), cursor(0, 0, first_keys, 3, 1), dict(kind='Work')),
        case('found', cursor(0, 0, appended, 3, 1), cursor(0, 0, appended, 3, 1, True), dict(kind='Named', name=1)),
        case('full', cursor(1, 1, appended, 2, 0), cursor(1, 1, appended, 2, 1), dict(kind='Work')),
        case('full', cursor(1, 1, appended, 2, 1), cursor(1, 1, appended, 2, 2), dict(kind='Work')),
        case('full', cursor(1, 1, appended, 2, 2), cursor(1, 1, appended, 2, 2), dict(kind='Capacity', epoch='9')),
        case('grant', cursor(1, 1, appended, 3, 2), cursor(1, 1, final, 3, 2, True), dict(kind='Named', name=2)),
    ]
    append = dict(before_case=2, before=namespace(first_keys, 3), after=namespace(appended, 3),
        same_namespace=True, name=1, target_cursor=cursor(0, 0, appended, 3, 1),
        sibling_cursor=cursor(2, 0, appended, 3, 1, True, True))
    grants = [dict(before_case=3, before=namespace(appended, 3), after=namespace(appended, 2)),
              dict(before_case=6, before=namespace(appended, 2), after=namespace(appended, 3))]
    return dict(schema='synarc-retained-names-v1', cases=rows, sibling_append=append, capacity_grants=grants)


class RetainedExportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.RetainedNames', 'VerumExternalReplay.AcquiredObserver'))

    def replay(self, data, expected=None):
        source, counts = emit(data)
        self.assertGreater(counts['claims'], 0)
        with tempfile.TemporaryDirectory(prefix='verum-retained-export-') as directory:
            if expected:
                p = Path(directory) / 'wrong.lean'; p.write_text(source); check(p, expected)
            else:
                for i, (part, _) in enumerate(batches(source)):
                    p = Path(directory) / f'part{i}.lean'; p.write_text(part); check(p)

    def test_actual_shaped_chronology(self):
        self.replay(fixture())

    def test_corrupted_observations_fail(self):
        mutations = {
            'skip_new_entry': lambda d: d['cases'][2]['before'].update(scanned=2),
            'replace_input_owner': lambda d: d['cases'][2]['after'].update(input_owner=99),
            'wrong_namespace': lambda d: d['cases'][1]['before'].update(same_namespace=False),
            'wrong_name': lambda d: d['cases'][2]['result'].update(name=0),
            'work_refunded': lambda d: d['cases'][1].update(remaining=1),
            'early_finish': lambda d: d['cases'][1]['after'].update(finished=True),
            'poisoned': lambda d: d['cases'][1]['after'].update(poisoned=True),
            'wrong_cut': lambda d: d['cases'][1]['after']['cut'].update(state=9),
            'changed_value': lambda d: d['cases'][1]['after'].update(value=dict(kind='Nat', value=257)),
            'changed_query': lambda d: d['cases'][1]['after']['query']['values'][0]['value'].update(raw=99),
            'wrong_input_port': lambda d: d['cases'][0]['before']['query']['values'][0]['value'].update(input=99),
            'wrong_query_source': lambda d: d['cases'][0]['before']['query']['interface'].update(epoch='99'),
            'sibling_value': lambda d: d['sibling_append']['sibling_cursor'].update(value=dict(kind='Nat', value=257)),
            'sibling_namespace': lambda d: d['sibling_append'].update(same_namespace=False),
            'capacity_epoch': lambda d: d['cases'][5]['result'].update(epoch='10'),
            'unrecorded_capacity': lambda d: d['capacity_grants'].pop(0),
        }
        for name, mutate in mutations.items():
            data = fixture(); mutate(data)
            with self.subTest(name=name): self.replay(data, 'Tactic `rfl` failed:')
        data = fixture(); data['sibling_append']['sibling_cursor']['cut']['owner'] = 0
        self.replay(data, 'Tactic `decide` proved that the proposition')

    def test_invalid_schema_rejected(self):
        mutations = {
            'unknown_version': lambda d: d.update(schema='unknown'),
            'wide_grant': lambda d: d['cases'][0].update(grant=2),
            'boolean_grant': lambda d: d['cases'][0].update(grant=True),
            'negative_remaining': lambda d: d['cases'][0].update(remaining=-1),
            'pending': lambda d: d['cases'][0].update(result=dict(kind='Pending')),
            'extra': lambda d: d['cases'][0]['before'].update(fabricated_report={}),
            'event_order': lambda d: d['capacity_grants'].reverse(),
            'event_collision': lambda d: d['capacity_grants'][0].update(before_case=2),
            'trailing_event': lambda d: d['capacity_grants'][0].update(before_case=7),
            'missing_cursor': lambda d: d['sibling_append']['target_cursor'].update(owner=99),
            'overwide_name': lambda d: d['cases'][2]['result'].update(name=2**56),
            'missing_pose': lambda d: d['cases'][0]['before']['query']['values'][0]['value'].pop('pose'),
        }
        for name, mutate in mutations.items():
            data = fixture(); mutate(data)
            with self.subTest(name=name), self.assertRaises(ValueError): emit(data)


if __name__ == '__main__': unittest.main()
