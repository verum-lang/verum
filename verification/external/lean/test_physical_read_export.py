"""Synthetic extraction controls; these are not native execution evidence."""
import copy
from pathlib import Path
import tempfile
import unittest
from check_physical_read_export import emit
from observer_export_replay import batches
from check_typed_programs import check, require_success, run


def fixture(terminal=False):
    nat = lambda n: dict(kind='Nat', value=n)
    pair = lambda a, b: dict(kind='Pair', first=nat(a), second=nat(b))
    interface = lambda shape: dict(epoch='1', raw_space='1', owner=None, shape=shape)
    binding = lambda shape: dict(interface=interface(shape), width=1, slots=['State', {'External': 0}])
    update = dict(steps=[dict(operation=dict(Primitive='EventPose'), arguments=[dict(Input=1)])], output=None, library=[])
    read = dict(steps=[dict(operation=dict(Primitive='QueryPose'), arguments=[dict(Input=1)]),
        dict(operation=dict(Primitive='Pair'), arguments=[dict(Input=0), dict(Register=0)])], output=None, library=[])
    definition = dict(identity=0, initial=nat(0), update=update, read=read,
        update_binding=binding('Event'), read_binding=binding('Query'), semantics=dict(kind='AcquiredV2', max_trace=16))
    cut = lambda state, value: dict(definition=0, owner=0, state=state, value=nat(value))
    old, new = cut(0, 0), cut(1, 1)
    ports = lambda raw, pose, age, episode: dict(raw=raw, pose=pose, life_age=age, episode=episode,
                                               runtime_raw=raw, runtime_pose=pose)
    prior = ports(7, 1, 3, 2)
    successor = ports(8, 2, 0 if terminal else 4, 3 if terminal else 2)
    entries = [dict(input=7, value=pair(0, 1))]
    names = lambda es: dict(owner=0, epoch='9', capacity=100, entries=es)
    snapshot = lambda p, c, es, attempts, ordinal: dict(physical=p, cut=c, namespace=names(es), attempts=attempts, ordinal=ordinal)
    report = lambda values: dict(stop='Complete', registers=values, dispatch_attempts=len(values),
        expanded_attempts=len(values), call_attempts=0, halt=None)
    def observation(p, c, name):
        q = dict(epoch='1', raw_space='1', owner=None, raw=p['raw'], input=p['raw'],
                 episode=p['episode'], life_age=p['life_age'], pose=p['pose'], action=None)
        result = pair(c['value']['value'], p['pose'])
        return dict(kind='Read', view={k: v for k, v in dict(**q, under=None).items()
            if k in ('raw', 'input', 'episode', 'life_age', 'pose', 'under')},
            query=dict(interface=interface('Query'), values=[dict(kind='Query', value=q)]),
            cut=c, value=result, execution=report([nat(p['pose']), result]), name=name)
    event = dict(epoch='1', raw_space='1', owner=None, raw=7, input=7, episode=2, life_age=3, pose=1, action=0)
    commit = dict(kind='Commit', input=dict(interface=interface('Event'), values=[dict(kind='Event', value=event)]),
                  receipt=dict(before=old, after=new), execution=report([nat(1)]))
    return dict(schema='synarc-physical-observer-read-v1', definitions=[definition], cases=[dict(
        before=snapshot(prior, old, entries, 4, 6),
        after=snapshot(successor, new, entries + [dict(input=8, value=pair(1, 2))], 5, 7),
        action=0, native=[observation(prior, old, 0), commit, observation(successor, new, 1)],
        witness=dict(ordinal=6, lineage=dict(authority=0, birth=0), raw=7, situation=0,
            action=0, episode=2, life_age=3, pose=1, epoch='9', inputs=[['1', 7]]),
        outcome=dict(kind='Outcome', value=dict(kind='End', terminal='Time') if terminal else dict(kind='Next', key=8)),
        table=dict(source=0, action=0, successor=None if terminal else 1))])


class PhysicalReadTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.AcquiredObserver',
            'VerumExternalReplay.ObserverOwnership', 'VerumExternalReplay.ObserverInstallation',
            'VerumExternalReplay.ExactNames', 'VerumExternalReplay.TraceReduction'))

    def replay(self, data, rejected=False):
        source, counts = emit(data)
        self.assertGreater(counts['claims'], 0)
        with tempfile.TemporaryDirectory(prefix='verum-physical-read-') as directory:
            if rejected:
                path = Path(directory) / 'false.lean'; path.write_text(source)
                check(path, 'Tactic `rfl` failed:')
            else:
                for i, (part, _) in enumerate(batches(source)):
                    path = Path(directory) / f'part{i}.lean'; path.write_text(part)
                    result = check(path)
                    rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
                    self.assertEqual(len(rosters), len(_))
                    for roster in rosters:
                        self.assertTrue(roster.endswith(('does not depend on any axioms',
                            'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), roster)

    def test_successor_and_terminal(self):
        self.replay(fixture())
        self.replay(fixture(True))

    def test_long_retained_state_is_checked_without_recursive_simplification(self):
        data = fixture(); row = data['cases'][0]
        event = row['native'][1]['input']['values'][0]['value']
        trace = dict(kind='Trace', events=[copy.deepcopy(event) for _ in range(600)])
        data['definitions'][0]['initial'] = trace
        row['before']['cut']['value'] = trace
        row['native'][0]['value']['first'] = trace
        row['before']['namespace']['entries'][0]['value']['first'] = trace
        self.replay(data)
        row['native'][0]['cut'] = copy.deepcopy(row['native'][0]['cut'])
        row['native'][0]['cut']['value']['events'][300]['raw'] += 1
        self.replay(data, rejected=True)

    def test_hypothetical_queries_do_not_substitute_for_physical_evidence(self):
        data = fixture(); row = data['cases'][0]
        probe = copy.deepcopy(row['native'][0])
        def change_query(observation, raw, pose, name):
            observation['view'].update(raw=raw, input=raw, pose=pose)
            observation['query']['values'][0]['value'].update(raw=raw, input=raw, pose=pose)
            observation['value']['second']['value'] = pose
            observation['execution']['registers'][0]['value'] = pose
            observation['name'] = name
        change_query(probe, 99, 9, 1)
        row['native'].insert(1, probe)
        row['native'][-1]['name'] = 2
        row['table']['successor'] = 2
        row['after']['namespace']['entries'].insert(1, dict(input=99, value=copy.deepcopy(probe['value'])))
        self.replay(data)
        # A coherent VM/name report about hypothetical ports still cannot prove
        # the actual successor world was read.
        change_query(row['native'][-1], 88, 9, 2)
        row['after']['namespace']['entries'][-1] = dict(input=88, value=copy.deepcopy(row['native'][-1]['value']))
        self.replay(data, rejected=True)

    def test_port_and_receipt_corruption(self):
        for corruption in ('witness', 'runtime', 'query', 'view', 'report', 'state', 'name', 'table', 'input', 'outcome'):
            data = fixture(); row = data['cases'][0]
            if corruption == 'witness': row['witness']['pose'] = 2
            elif corruption == 'runtime': row['after']['physical']['runtime_pose'] = 1
            elif corruption == 'query': row['native'][2]['query']['values'][0]['value']['pose'] = 1
            elif corruption == 'view': row['native'][2]['view']['life_age'] = 3
            elif corruption == 'report': row['native'][2]['execution']['expanded_attempts'] = 1
            elif corruption == 'state': row['native'][2]['cut'] = copy.deepcopy(row['before']['cut'])
            elif corruption == 'name': row['native'][2]['name'] = 0
            elif corruption == 'table': row['table']['successor'] = 2
            elif corruption == 'input': row['native'][1]['input']['values'][0]['value']['input'] = 9
            else: row['outcome']['value']['key'] = row['table']['successor']
            with self.subTest(corruption=corruption): self.replay(data, rejected=True)

    def test_missing_and_unknown_evidence_refused(self):
        for corruption in ('read', 'commit', 'terminal', 'unknown', 'adapter', 'inputs'):
            data = fixture(); row = data['cases'][0]
            if corruption == 'read': row['native'].pop()
            elif corruption == 'commit': row['native'].pop(1)
            elif corruption == 'terminal': row['outcome']['value'] = dict(kind='End', terminal='Time')
            elif corruption == 'unknown': row['native'][0]['trusted'] = True
            elif corruption == 'adapter': data['definitions'][0]['semantics']['kind'] = 'AcquiredV1'
            else: row['witness']['inputs'] = []
            with self.subTest(corruption=corruption), self.assertRaises(ValueError): emit(data)


if __name__ == '__main__':
    unittest.main()
