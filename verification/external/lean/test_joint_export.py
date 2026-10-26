"""Synthetic joint-export controls; never native producer acceptance."""
import copy
from pathlib import Path
import tempfile
import unittest
from check_joint_export import emit
from observer_export_replay import batches
from check_typed_programs import check, require_success, run
from test_lifecycle_export import model_fixture, event_value


def joint_fixture():
    base = model_fixture(); definition = base['definitions'][0]
    definition['semantics']['kind'] = 'AcquiredV2'
    nat = lambda n: dict(kind='Nat', value=n)
    definition['initial'] = nat(0)
    definition['update']['steps'] = [
        dict(operation=dict(Primitive='EventInput'), arguments=[dict(Input=1)]),
        dict(operation=dict(Primitive='Add'), arguments=[dict(Input=0), dict(Register=0)])]
    origin = dict(owner=0, authority=0, count=6, index=0, look=2, comparison_count=72)
    cut = lambda owner, state, n: dict(definition=0, owner=owner, state=state, value=nat(n))
    row = base['installation'][0]['history']['events'][0]
    event = event_value(3); event['value']['input'] = 3
    row['values'] = [event]
    history = base['installation'][0]['history']; history['events'] = [copy.deepcopy(row), copy.deepcopy(row)]
    return dict(schema='synarc-joint-observer-v1', definition=definition,
        candidate=dict(owner=0, definition=0, grammar=dict(owner=0, authority=0, count=6, component_counts=[1, 2, 3]),
            index=0, syntax_indices=[0, 0, 0], family_count=6, evidence_look=2,
            comparison_count=72, provenance=origin), definition_receipt=copy.deepcopy(origin),
        initialization=dict(program=dict(steps=[dict(operation=dict(Primitive='Zero'), arguments=[])],
            output=None, library=[]), report=dict(stop='Complete', registers=[nat(0)],
            dispatch_attempts=1, expanded_attempts=1, call_attempts=0, halt=None), observation=None),
        transition=dict(input=copy.deepcopy(row), before=cut(0, 0, 3), fork_before=cut(1, 0, 3),
            fork_after=cut(1, 1, 6), live_after=cut(0, 2, 6)), history=history, replay=cut(2, 3, 6))


class JointExtractionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.AcquiredObserver',
                            'VerumExternalReplay.ObserverInstallation', 'VerumExternalReplay.ObserverOwnership',
                            'VerumExternalReplay.ExactNames'))

    def replay(self, data, rejection=None):
        source, counts = emit(data); self.assertGreater(counts['claims'], 0)
        with tempfile.TemporaryDirectory(prefix='verum-joint-extraction-') as directory:
            if rejection:
                p = Path(directory) / 'wrong.lean'; p.write_text(source); check(p, rejection)
            else:
                for i, (part, _) in enumerate(batches(source)):
                    p = Path(directory) / f'part{i}.lean'; p.write_text(part); check(p)

    def test_joint_model_replays(self):
        self.replay(joint_fixture())

    def test_long_history_composes_every_transition(self):
        data = joint_fixture()
        row = data['history']['events'][0]
        data['history']['events'] = [copy.deepcopy(row) for _ in range(300)]
        for key in ('before', 'fork_before'):
            data['transition'][key]['value']['value'] = 897
        for key in ('fork_after', 'live_after'):
            data['transition'][key]['value']['value'] = 900
        data['replay']['value']['value'] = 900
        for changed in (False, True):
            altered = copy.deepcopy(data)
            if changed:
                # A middle event contributes to both prefix and complete replay;
                # checking only the final event would miss this mutation.
                altered['history']['events'][150]['values'][0]['value']['input'] = 4
            source, _ = emit(altered)
            with tempfile.TemporaryDirectory(prefix='verum-long-joint-replay-') as directory:
                # These closed batches contain all fold claims. The earlier
                # metadata checks already have independent extraction controls.
                for i, (part, _) in enumerate(list(batches(source))[-2:]):
                    path = Path(directory) / f'fold{i}.lean'; path.write_text(part)
                    affected = ': joint_before = ' in part or ': joint_replay = ' in part
                    check(path, 'Tactic `rfl` failed:' if changed and affected else None)

    def test_changed_observations_rejected(self):
        for change in ('initial', 'report', 'receipt', 'look', 'prefix', 'transition', 'fork', 'live', 'replay'):
            data = joint_fixture()
            if change == 'initial': data['definition']['initial']['value'] = 1
            elif change == 'report': data['initialization']['report']['dispatch_attempts'] = 2
            elif change == 'receipt': data['definition_receipt']['owner'] = 99
            elif change == 'look': data['candidate']['evidence_look'] = 0
            elif change == 'prefix': data['history']['events'][0]['values'][0]['value']['input'] = 4
            elif change == 'transition': data['transition']['input']['values'][0]['value']['input'] = 4
            elif change == 'fork': data['transition']['fork_after']['value']['value'] = 7
            elif change == 'live': data['transition']['live_after']['value']['value'] = 7
            else: data['replay']['value']['value'] = 7
            with self.subTest(change=change): self.replay(data, 'Tactic `rfl` failed:')

    def test_no_fresh_owner_or_state_alias(self):
        data = joint_fixture(); data['replay']['owner'] = data['transition']['before']['owner']
        self.replay(data, 'Tactic `decide` proved that the proposition')

    def test_unknown_or_incomplete_evidence_rejected(self):
        for change in ('legacy', 'observation', 'receipt', 'gap', 'field'):
            data = joint_fixture()
            if change == 'legacy': data['definition']['semantics']['kind'] = 'AcquiredV1'
            elif change == 'observation': data['initialization']['observation'] = {}
            elif change == 'receipt': data['definition_receipt'] = None
            elif change == 'gap': data['history']['events'][0] = {'kind': 'Gap'}
            else: data['candidate']['trusted'] = True
            with self.subTest(change=change), self.assertRaises(ValueError): emit(data)


if __name__ == '__main__':
    unittest.main()
