"""Synthetic controls for the retirement wire contract, not native evidence."""
import copy
from pathlib import Path
import tempfile
import unittest
from check_retirement_export import emit
from check_typed_programs import check, require_success, run
from observer_export_replay import batches
from test_physical_read_export import fixture as physical_fixture


def fixture():
    physical = physical_fixture(); definition = physical['definitions'][0]
    # An actual AcquiredV2 Rem-by-zero execution, including its typed refusal.
    definition['read'] = dict(steps=[dict(operation=dict(Primitive='Rem'),
        arguments=[dict(Literal=dict(kind='Nat', value=1)), dict(Literal=dict(kind='Nat', value=0))])],
        output=None, library=[])
    query = copy.deepcopy(physical['cases'][0]['native'][0]['query'])
    cut = dict(definition=0, owner=7, state=9, value=dict(kind='Nat', value=0))
    reason = dict(kind='Execution', phase='Read', classification='Undefined', meter_error=False,
        counter_capacity=False, primitive_refusal='Undefined', report=dict(stop='Undefined', registers=[],
        dispatch_attempts=1, expanded_attempts=1, call_attempts=0,
        halt=dict(path=[0], reason=dict(kind='Primitive', error='Missing'))))
    lineage = dict(authority=0, birth=0)
    lower = dict(input_epoch='1', output_epoch=None, cut=dict(kind='Canonical', value=dict(
        definition=0, owner=0, state=1, value=dict(kind='Nat', value=12))))
    failing = dict(input_epoch='1', output_epoch='3', cut=dict(kind='Acquired', value=cut))
    pending = dict(fault=dict(index=1, cut=cut, input=query, refusal=reason), boundary='BeforeAct', actual=None)
    before = dict(epoch='3', highest_epoch='3', history=dict(lineage=lineage, length=0, events=[]),
        physical=dict(raw=7, pose=1, episode=2, life_age=3, attempts=4), floors=[lower, failing],
        tiers=[], corpus=[], pending=pending, journal=[], used=0, limit=100, work=20)
    pre = copy.deepcopy(before); pre['work'] = 17
    receipt = dict(floor=dict(lineage=lineage, input_epoch='1', output_epoch='3'), ordinal=0,
        history=copy.deepcopy(before['history']), **copy.deepcopy(pending),
        current_prefix=dict(epoch='1', floors=[{k: lower[k] for k in ('input_epoch', 'cut')}], tiers=[]),
        retired_floors=[copy.deepcopy(failing)], retired_tiers=[], corpus=[],
        work_before=20, work_after=15, journal_slots=3)
    after = copy.deepcopy(before); after.update(epoch='1', floors=[copy.deepcopy(lower)], pending=None,
                                               journal=[receipt], used=3, work=15)
    return dict(schema='synarc-observer-retirement-v1', definitions=[definition], cases=[dict(
        name='current-prefix-survives', attempt=before, precommit=pre, after=after, result=dict(kind='Committed'))])


class RetirementExportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.ObserverRetirement',
            'VerumExternalReplay.AcquiredObserver', 'VerumExternalReplay.ObserverOwnership',
            'VerumExternalReplay.ExactNames', 'VerumExternalReplay.ObserverInstallation',
            'VerumExternalReplay.TraceReduction'))

    def replay(self, data, rejected=False):
        source, counts = emit(data)
        self.assertGreater(counts['claims'], 0)
        with tempfile.TemporaryDirectory(prefix='retirement-export-') as tmp:
            if rejected:
                p = Path(tmp) / 'false.lean'; p.write_text(source)
                check(p, 'Tactic `rfl` failed:')
            else:
                for i, (part, names) in enumerate(batches(source)):
                    p = Path(tmp) / f'part{i}.lean'; p.write_text(part)
                    audit = check(p)
                    rosters = [d['data'] for d in audit['diagnostics'] if d['severity'] == 'information']
                    self.assertEqual(len(rosters), len(names))
                    for r in rosters:
                        self.assertTrue(r.endswith(('does not depend on any axioms',
                            'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), r)

    def test_current_prefix_and_resource_refusal(self):
        self.replay(fixture())
        for spent in (0, 4):
            data = fixture(); c = data['cases'][0]
            c.update(precommit=None, after=copy.deepcopy(c['attempt']), result=dict(kind='Refused', resource=True))
            c['after']['work'] -= spent
            self.replay(data)

    def test_corrupt_boundary(self):
        for mutation in ('current_state', 'owner', 'output_epoch', 'work_reset', 'work_charge', 'history',
                         'actual_action', 'retired_suffix', 'journal_used', 'receipt_work', 'fault_ports'):
            data = fixture(); c = data['cases'][0]; a = c['after']; r = a['journal'][0]
            if mutation == 'current_state': a['floors'][0]['cut']['value']['value']['value'] = 11
            elif mutation == 'owner': r['fault']['cut']['owner'] = 8
            elif mutation == 'output_epoch': r['floor']['output_epoch'] = '4'
            elif mutation == 'work_reset': a['work'] = 21
            elif mutation == 'work_charge': c['precommit']['work'] = 18
            elif mutation == 'history': a['history']['lineage']['birth'] = 2
            elif mutation == 'actual_action': a['physical']['attempts'] += 1
            elif mutation == 'retired_suffix': r['retired_floors'] = []
            elif mutation == 'journal_used': a['used'] = 4
            elif mutation == 'receipt_work': r['work_after'] = 14
            else:
                for value in (c['attempt']['pending'], c['precommit']['pending'], r):
                    value['fault']['input']['values'][0]['value']['raw'] = 99
            with self.subTest(mutation=mutation): self.replay(data, True)

    def test_actual_after_act_outcome(self):
        data = fixture(); c = data['cases'][0]; r = c['after']['journal'][0]
        cut = dict(lineage=dict(authority=0,birth=0), ordinal=0, epoch='1', raw_space='1',
            raw=6,situation=6,action=0,episode=2,life_age=2,pose=1,inputs=[['1',6]])
        actual = dict(cut=cut,outcome=dict(kind='Outcome',value=dict(kind='Next',key=7)))
        history = dict(lineage=dict(authority=0,birth=0),length=1,events=[cut])
        corpus = [dict(key=6,rows=[dict(tick=dict(key=6,age=2,events=0,obs=True,tower=0,theta=1),
            cut=cut,outcome=actual['outcome'],ordinal=0,raw_top=6)])]
        for s in (c['attempt'],c['precommit'],c['after']): s['history']=copy.deepcopy(history)
        for p in (c['attempt']['pending'],c['precommit']['pending'],r):
            p.update(boundary='AfterAct',actual=copy.deepcopy(actual))
        for s in (c['attempt'],c['precommit'],r):s['corpus']=copy.deepcopy(corpus)
        r.update(history=copy.deepcopy(history),ordinal=1,journal_slots=4,work_after=14)
        c['precommit']['work']=16;c['after'].update(used=4,work=14)
        self.replay(data)
        for p in (c['attempt']['pending'],c['precommit']['pending'],r):p['actual']['outcome']['value']['key']=8
        for s in (c['attempt'],c['precommit'],r):s['corpus'][0]['rows'][0]['outcome']['value']['key']=8
        self.replay(data,True)

    def test_missing_independent_snapshot(self):
        data = fixture(); data['cases'][0]['precommit'] = None
        with self.assertRaises(ValueError): emit(data)


if __name__ == '__main__': unittest.main()
