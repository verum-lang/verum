"""Synthetic extractor controls, explicitly not native execution evidence."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from check_lifecycle_export import emit, value
from observer_export_replay import batches
from check_typed_programs import check, require_success, run


def model_fixture():
    nat = lambda n: {'kind': 'Nat', 'value': n}
    interface = lambda shape: dict(epoch='1', raw_space='1', owner=None, shape=shape)
    binding = lambda shape: dict(interface=interface(shape), width=1, slots=['State', {'External': 0}])
    program = lambda op: dict(steps=[dict(operation={'Primitive': op}, arguments=[{'Input': 0}, {'Input': 1}])], output=None, library=[])
    budget = lambda remaining: dict(dispatch_limit=None, work_remaining=remaining, dispatches=0, poisoned=False)
    cut = lambda owner, state, n: dict(definition=0, owner=owner, state=state, value=nat(n))
    before, after, fork = cut(0, 0, 2), cut(0, 1, 5), cut(1, 0, 2)
    prepared = dict(before=before, after=dict(state=1, value=nat(5)))
    execution = dict(stop='Complete', registers=[nat(5)], dispatch_attempts=1, expanded_attempts=1, call_attempts=0, halt=None)
    ownership = [
        dict(name='synthetic-prepare', kind='prepare', current_before=before, current_after=before,
             input=dict(interface=interface('Event'), values=[nat(3)]), budget_before=budget(100),
             budget_after={**budget(90), 'dispatches': 1}, result=dict(kind='Ok', prepared=prepared, execution=execution)),
        dict(name='synthetic-fork', kind='fork', before=before, after=fork),
        dict(name='synthetic-foreign', kind='commit', current_before=fork, current_after=fork, prepared=prepared,
             result=dict(kind='Err', reason='ForeignInstance', prepared=prepared)),
        dict(name='synthetic-commit', kind='commit', current_before=before, current_after=after, prepared=prepared,
             result=dict(kind='Ok', receipt=dict(before=before, after=after))),
        dict(name='synthetic-stale', kind='commit', current_before=after, current_after=after, prepared=prepared,
             result=dict(kind='Err', reason='StaleState', prepared=prepared)),
    ]
    read_report = {**execution, 'registers': [{'kind': 'Pair', 'first': nat(2), 'second': nat(3)}]}
    ownership.append(dict(name='synthetic-read', kind='read', current_before=before, current_after=before,
        input=dict(interface=interface('Query'), values=[nat(3)]), budget_before=budget(100),
        budget_after={**budget(90), 'dispatches': 1}, result=dict(kind='Ok', cut=before,
            value=read_report['registers'][0], execution=read_report)))
    ns = lambda entries: dict(owner=0, epoch='9', capacity=1, entries=entries)
    entry = dict(input=7, value=nat(257))
    naming = [dict(name='synthetic-name', before=ns([]), input=7, value=nat(257), budget_before=budget(100),
                   after=ns([entry]), budget_after=budget(99), result=dict(kind='Ok', name=0)),
              dict(name='synthetic-name-full', before=ns([entry]), input=7, value=nat(1), budget_before=budget(99),
                   after=ns([entry]), budget_after=budget(97), result=dict(kind='Err', reason=dict(kind='NameCapacity', epoch='9')))]
    history = dict(owner=0, origin=dict(lineage=dict(authority=0, birth=0), epoch='1', ordinal=0),
                   events=[dict(interface=interface('Event'), values=[nat(3)])])
    installation = [dict(name='synthetic-install', kind='success', admission=dict(kind='Validated', owner=0, definition=0),
        history=history, cut=1, known_histories=[], installed=cut(2, 2, 5), replay=cut(3, 3, 5))]
    refusals = [dict(name='synthetic-class', kind='classification', domain='canonical', reason_kind='Work',
        stop=None, meter_error=True, counter_capacity=False, primitive_error=None, primitive_refusal=None,
        classification='Resource', budget_before=budget(0), budget_after=budget(0))]
    return dict(schema='synarc-observer-lifecycle-v1', definitions=[dict(identity=0, initial=nat(2),
        update=program('Add'), read=program('Pair'), update_binding=binding('Event'), read_binding=binding('Query'),
        semantics=dict(kind='AcquiredV1', max_trace=16))], ownership=ownership, naming=naming,
        installation=installation, refusals=refusals)


def event_value(pose):
    return dict(kind='Event', value=dict(epoch='1', raw_space='1', owner=None,
        raw=7, input=8, action=9, episode=10, life_age=11, pose=pose))


def pose_read_fixture(operation, argument, result, arity=1):
    data = model_fixture()
    definition = data['definitions'][0]; definition['semantics']['kind'] = 'AcquiredV2'
    definition['initial'] = argument
    definition['read']['steps'] = [dict(operation=dict(Primitive=operation),
        arguments=[dict(Input=0)] if arity else [])]
    row = data['ownership'][-1]
    row['current_before']['value'] = argument
    row['result']['value'] = result; row['result']['execution']['registers'] = [result]
    # Four binding units, one frame unit, two instruction units plus operands,
    # then one root-output unit. This is a synthetic accounting control.
    row['budget_after']['work_remaining'] = 92 - arity
    data['ownership'] = [row]
    for key in ('installation', 'naming', 'refusals'): data[key] = []
    return data


class LifecycleExtractionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.AcquiredObserver',
                            'VerumExternalReplay.ExactNames', 'VerumExternalReplay.ObserverOwnership',
                            'VerumExternalReplay.ObserverInstallation'))

    def replay(self, data, rejected=False):
        source, counts = emit(data)
        self.assertGreater(counts['claims'], 0)
        with tempfile.TemporaryDirectory(prefix='verum-lifecycle-extraction-') as directory:
            if rejected:
                p = Path(directory) / 'wrong.lean'; p.write_text(source)
                check(p, 'Tactic `rfl` failed:')
            else:
                for i, (part, _) in enumerate(batches(source)):
                    p = Path(directory) / f'part{i}.lean'; p.write_text(part); check(p)

    def test_model_fixture_replays(self):
        self.replay(model_fixture())

    def test_pose_adapter_primitives(self):
        unit = dict(kind='Unit'); nat = lambda n: dict(kind='Nat', value=n)
        event = event_value(255)
        query = dict(kind='Query', value={**event['value'], 'pose': 0, 'action': None})
        pair = dict(kind='Pair', first=event, second=query)
        for op, arg, out, arity in [
            ('Unit', unit, unit, 0), ('Zero', unit, nat(0), 0),
            ('EmptyTrace', unit, dict(kind='Trace', events=[]), 0),
            ('Identity', pair, pair, 1), ('EventPose', event, nat(255), 1),
            ('QueryPose', query, nat(0), 1),
        ]:
            with self.subTest(operation=op): self.replay(pose_read_fixture(op, arg, out, arity))
        wrong = pose_read_fixture('EventPose', event, nat(254))
        self.replay(wrong, True)

    def test_adapter_versions_are_explicit(self):
        data = pose_read_fixture('EventPose', event_value(None), dict(kind='Nat', value=0))
        self.replay(data, True)  # Missing pose cannot manufacture zero.
        del data['definitions'][0]['initial']['value']['pose']
        with self.assertRaisesRegex(ValueError, 'pose fields'): emit(data)
        data = pose_read_fixture('EventPose', event_value(5), dict(kind='Nat', value=5))
        data['definitions'][0]['semantics']['kind'] = 'AcquiredV1'
        with self.assertRaises(ValueError): emit(data)
        data = pose_read_fixture('Zero', dict(kind='Unit'), dict(kind='Nat', value=0), 0)
        data['definitions'][0]['semantics']['kind'] = 'AcquiredV1'
        with self.assertRaisesRegex(ValueError, 'requires AcquiredV2'): emit(data)
        data['definitions'][0]['semantics']['kind'] = 'AcquiredV2'
        data['definitions'][0]['read']['steps'][0]['operation']['Primitive'] = 'FuturePrimitive'
        with self.assertRaisesRegex(ValueError, 'Unknown variant'): emit(data)
        for pose in (-1, 256, True, '0'):
            with self.subTest(pose=pose), self.assertRaises(ValueError): value(event_value(pose))

    def test_pose_is_part_of_exact_namespace_key(self):
        data = model_fixture(); data['definitions'][0]['semantics']['kind'] = 'AcquiredV2'
        data['ownership'] = []; data['installation'] = []; data['refusals'] = []
        a, b = data['naming']; first, second = event_value(None), event_value(0)
        a['value'] = first; a['after']['entries'][0]['value'] = first
        b['value'] = second; b['before']['entries'][0]['value'] = first
        b['after']['entries'] = [dict(input=7, value=first), dict(input=7, value=second)]
        for row in (a, b):
            row['before']['capacity'] = row['after']['capacity'] = 2
        b['result'] = dict(kind='Ok', name=1)
        self.replay(data)
        b['result']['name'] = 0
        self.replay(data, True)

    def test_frozen_refusal_fixes(self):
        data = json.loads((Path(__file__).resolve().parent /
                          'fixtures/observer-lifecycle-v1.json').read_text())
        _, counts = emit(data)
        self.assertEqual(counts, dict(definitions=1, ownership=14, naming=5,
                                      installation=4, refusals=4, claims=189))
        data['ownership'] = []; data['naming'] = []; data['installation'] = []
        self.replay(data)
        for name in ('canonical_zero_work', 'canonical_zero_dispatch', 'acquired_trace_capacity'):
            wrong = copy.deepcopy(data)
            next(row for row in wrong['refusals'] if row['name'] == name)['classification'] = 'Undefined'
            with self.subTest(refusal=name): self.replay(wrong, True)

    def test_model_corruptions_rejected(self):
        # Independent observations are changed; expected model behavior is not.
        for change in ('commit', 'retained_payload', 'read', 'name', 'install', 'classification'):
            data = model_fixture()
            if change == 'commit': data['ownership'][3]['current_after']['state'] = 9
            elif change == 'retained_payload':
                data['ownership'][2]['result']['prepared'] = copy.deepcopy(data['ownership'][2]['result']['prepared'])
                data['ownership'][2]['result']['prepared']['after']['value']['value'] = 8
            elif change == 'read': data['ownership'][-1]['result']['value'] = {'kind': 'Nat', 'value': 99}
            elif change == 'name': data['naming'][0]['result']['name'] = 1
            elif change == 'install': data['installation'][0]['installed']['value']['value'] = 6
            else: data['refusals'][0]['classification'] = 'Undefined'
            with self.subTest(change=change): self.replay(data, True)

    def test_receiver_replay_multiple_updates(self):
        data = model_fixture(); row = data['installation'][0]
        row['history']['events'].append(copy.deepcopy(row['history']['events'][0]))
        row['cut'] = 2
        row['installed']['value']['value'] = row['replay']['value']['value'] = 8
        self.replay(data)

    def test_structural_refusal_order(self):
        data = model_fixture(); row = data['installation'][0]
        del row['installed']; del row['replay']
        known = copy.deepcopy(row['history']); known['owner'] = 1
        row['history']['events'] = [{'kind': 'Gap'}, {'kind': 'Gap'}]
        known['events'] *= 2
        row.update(kind='failure', cut=2, known_histories=[dict(phase='training', history=known)],
                   reason=dict(kind='HistoryChanged', event=0), incomplete=True,
                   current_before=data['ownership'][0]['current_before'],
                   current_after=data['ownership'][0]['current_after'])
        row['request'] = {k: copy.deepcopy(row[k]) for k in ('admission', 'history', 'cut')}
        self.replay(data)
        for reason in (dict(kind='HistoryChanged', event=1), dict(kind='Gap', event=0)):
            wrong = copy.deepcopy(data); wrong['installation'][0]['reason'] = reason
            self.replay(wrong, True)
        row['known_histories'] = []; row['reason'] = dict(kind='Gap', event=0)
        self.replay(data)
        row['reason']['event'] = 1
        self.replay(data, True)

    def test_shared_namespace_cannot_forget_entries(self):
        data = model_fixture(); row = data['naming'][1]
        # This is a locally valid fresh insertion, but violates the retained
        # owner's append-only table from the previous observation.
        row['before']['entries'] = []
        row['after']['entries'] = [dict(input=7, value=row['value'])]
        row['result'] = dict(kind='Ok', name=0)
        row['budget_after']['work_remaining'] = 98
        self.replay(data, True)

    def test_fresh_allocations_cannot_reuse_live_tokens(self):
        data = model_fixture()
        sibling = copy.deepcopy(data['ownership'][0]); sibling['name'] = 'synthetic-sibling'
        data['ownership'].insert(1, sibling)
        self.replay(data, True)
        data = model_fixture()
        sibling = copy.deepcopy(data['ownership'][1]); sibling['name'] = 'synthetic-fork-sibling'
        data['ownership'].insert(2, sibling)
        self.replay(data, True)

    def test_retained_history_order_is_strict(self):
        data = model_fixture(); row = data['installation'][0]
        row['known_histories'] = [dict(phase=phase, history=copy.deepcopy(row['history']))
                                  for phase in ('fresh', 'training')]
        with self.assertRaisesRegex(ValueError, 'native installation order'): emit(data)
        row['known_histories'] = row['known_histories'][:1]
        row['admission']['kind'] = 'Accepted'
        with self.assertRaisesRegex(ValueError, 'native installation order'): emit(data)

    def test_ambiguous_or_unknown_data_refused(self):
        for corrupted in ({'kind': 'Nat', 'value': True}, {'kind': 'Action', 'value': 256},
                          {'kind': 'Nat', 'value': -1}, {'kind': 'Hash', 'value': 1}):
            with self.subTest(value=corrupted), self.assertRaises(ValueError): value(corrupted)
        data = model_fixture(); data['definitions'][0]['semantics']['kind'] = 'Unknown'
        with self.assertRaises(ValueError): emit(data)
        data = model_fixture(); data['ownership'][0]['input']['interface']['epoch'] = '01'
        with self.assertRaises(ValueError): emit(data)

    def test_retained_request_is_not_substituted(self):
        data = model_fixture(); row = data['installation'][0]
        del row['installed']; del row['replay']
        row.update(kind='failure', cut=2, reason=dict(kind='CutBeyondHistory', cut=2, events=1),
                   incomplete=True, current_before=data['ownership'][0]['current_before'],
                   current_after=data['ownership'][0]['current_after'])
        row['request'] = {k: copy.deepcopy(row[k]) for k in ('admission', 'history', 'cut')}
        row['request']['history']['owner'] = 999
        with self.assertRaisesRegex(ValueError, 'retain its request'): emit(data)


if __name__ == '__main__':
    unittest.main()
