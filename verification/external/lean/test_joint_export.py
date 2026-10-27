"""Synthetic joint-export controls; never native producer acceptance."""
import copy
from pathlib import Path
import tempfile
import unittest
from check_joint_export import emit, exact_integer
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


def structural_fixture(*, kraft=False, large=False):
    data = joint_fixture(); candidate = data['candidate']
    exact = lambda n: n if n <= 2**53 else {'exact_decimal': str(n)}
    count = None if kraft else (2**90 + 7 if large else 6)
    look = 2**70 + 3 if large else 2
    ordinal = 2**80 + 1 if large else 0
    words = [[1], [1, 0], [1, 1]]
    denominator = 2**sum(map(len, words)) if kraft else count
    comparison = denominator * (look + 1) * (look + 2)
    prior = 'KraftProgramV1 { bits: 5 }' if kraft else f'Uniform {{ original_cardinality: {count} }}'
    origin = dict(owner=0, authority=0, count=None if kraft else exact(count),
                  index=exact(ordinal), look=exact(look), comparison_count=exact(comparison), name_prior=prior)
    candidate.update(index=exact(ordinal), family_count=None if kraft else exact(count),
                     evidence_look=exact(look), comparison_count=exact(comparison),
                     structural_code_words=words, provenance=origin)
    candidate['grammar'].update(count=candidate['family_count'], component_counts=[None]*3,
        source_order='structural-odometer-v1', admission_profile='ExactPredictiveV1 { risk: Mass { denominator: 20 } }')
    data['definition_receipt'] = copy.deepcopy(origin)
    return data


class JointExtractionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.AcquiredObserver',
                            'VerumExternalReplay.ObserverInstallation', 'VerumExternalReplay.ObserverOwnership',
                            'VerumExternalReplay.ExactNames', 'VerumExternalReplay.StructuralNames'))

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

    def test_current_uniform_and_kraft_naming(self):
        for kraft, large in ((False, False), (False, True), (True, True)):
            with self.subTest(kraft=kraft, large=large):
                self.replay(structural_fixture(kraft=kraft, large=large))

    def test_exact_integer_wire_format(self):
        for value in (True, 1.5, 2**53+1, {'exact_decimal': '01'}, {'exact_decimal': '-1'},
                      {'exact_decimal': '1'}, {'exact_decimal': '9007199254740993', 'trusted': True}):
            with self.subTest(value=value), self.assertRaises(ValueError): exact_integer(value)
        self.assertEqual(exact_integer({'exact_decimal': str(2**128+1)}), str(2**128+1))

    def test_current_naming_mutations(self):
        for change in ('paid', 'prior', 'risk', 'look', 'kraft_length', 'kraft_word', 'large_receipt'):
            data = structural_fixture(kraft=change.startswith('kraft'), large=change=='large_receipt')
            c = data['candidate']
            if change == 'paid': c['comparison_count'] += 1
            elif change == 'prior': data['definition_receipt']['name_prior'] = 'Uniform { original_cardinality: 7 }'
            elif change == 'risk': c['grammar']['admission_profile'] = 'ExactPredictiveV1 { risk: Mass { denominator: 0 } }'
            elif change == 'look': c['evidence_look'] += 1
            elif change == 'kraft_length':
                c['provenance']['name_prior'] = data['definition_receipt']['name_prior'] = 'KraftProgramV1 { bits: 4 }'
            elif change == 'kraft_word': c['structural_code_words'][0].append(0)
            else:
                d = data['definition_receipt']['index']['exact_decimal']
                data['definition_receipt']['index']['exact_decimal'] = str(int(d)+1)
            with self.subTest(change=change):
                self.replay(data, 'Tactic `decide` proved that the proposition' if change=='risk' else 'Tactic `rfl` failed:')

    def test_current_naming_schema_is_strict(self):
        for change in ('order', 'profile', 'prior', 'bit', 'word', 'count', 'indexed_words', 'missing'):
            data = structural_fixture(); c = data['candidate']
            if change == 'order': c['grammar']['source_order'] = 'unknown'
            elif change == 'profile': c['grammar']['admission_profile'] = 'trusted'
            elif change == 'prior': c['provenance']['name_prior'] = 'Uniform { original_cardinality: 06 }'
            elif change == 'bit': c['structural_code_words'][0][0] = 2
            elif change == 'word': c['structural_code_words'][0] = []
            elif change == 'count': c['grammar']['component_counts'] = [1, 2, 3]
            elif change == 'indexed_words': c['grammar']['source_order'] = 'legacy-indexed-v1'
            else: del c['provenance']['name_prior']
            with self.subTest(change=change), self.assertRaises(ValueError): emit(data)

    def test_structural_words_are_retained_without_claiming_decode(self):
        data = structural_fixture(); source, counts = emit(data)
        self.assertIn('def joint_structural_words : List (List Bool) := [[true], [true, false], [true, true]]', source)
        self.assertEqual(counts['structural_program_bindings'], 0)

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
