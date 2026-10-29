"""Synthetic kernel controls; these fixtures are not native execution evidence."""
import copy
from pathlib import Path
import re
import tempfile
import unittest

from check_signature_catalogue_export import emit
from check_typed_programs import ROOT, check, require_success, run


def fixture(predictive=False):
    source = dict(epoch='1', raw_space='2', owner=dict(authority=3, birth=4), shape='Event')
    code = dict(steps=[dict(operation={'Primitive': 'Unit'}, arguments=[])], output=0, library=[])
    binding = dict(interface=source, width=1, slots=['State', {'External': 0}])
    definition = dict(identity=0, semantics=dict(kind='AcquiredV2', max_trace=64), initial={'kind': 'Unit'},
        update=code, read=code, update_binding=binding,
        read_binding={**binding, 'interface': {**source, 'shape': 'Query'}})
    signature = dict(identity=0, version=1, parent=None, dependency=None, validated_owner=0,
        definition=0, origin=dict(lineage=source['owner'], epoch='1', ordinal=0), parts=[0, 1, 2],
        candidate={}, definition_receipt={}, initialization={})
    empty = dict(next=0, pending=None, phase_order=['initial', 'update', 'read'],
        calls=[[], [], []], frozen=None, catalogue_versions=[[], [], []], cells=1, taken=False, selections=[])
    included = [True, not predictive, not predictive]
    frozen = [[dict(arity=0 if i == 0 else 2, body=i)] if included[i] else [] for i in range(3)]
    final = dict(next=None, pending=None, phase_order=['initial', 'update', 'read'],
        calls=[[], [], []], frozen=frozen, catalogue_versions=[[1] if b else [] for b in included],
        cells=9 + 2 * sum(included), taken=False,
        selections=[dict(version=1, included=included, exclusion='Ports' if predictive else None)])
    event = dict(name='projection', kind='LibraryAdvance',
        input=dict(source=source, max_trace=64, predictive=predictive, capacity=64),
        before=empty, after=final, result=dict(kind='Ok', complete=True), work_before=64, work_after=58)
    return dict(schema='synarc-acquired-signature-lifecycle-v1', scope='synthetic control',
        identity_scope='test-local nominal owners', catalogue_mapping='initial/update/read',
        definitions=[definition], signatures=[signature],
        bodies=[dict(identity=i, program=code) for i in range(3)], events=[event])


class SignatureCataloguesTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.SignatureCatalogues'))

    def prove(self, data, expected=None):
        source, counts = emit(data)
        with tempfile.TemporaryDirectory(prefix='verum-signature-catalogue-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(source)
            result = check(path, expected)
        if expected is None:
            rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
            self.assertEqual(len(rosters), counts['claims'])
            self.assertTrue(all('sorryAx' not in row for row in rosters))
        return counts

    def test_actual_shaped_projection_and_ports(self):
        for predictive in (False, True):
            with self.subTest(predictive=predictive): self.prove(fixture(predictive))

    def test_zero_work_and_capacity_keep_the_exact_owner(self):
        for grant, capacity in [(0, 64), (1, 0)]:
            data = fixture(); event = data['events'][0]
            event.update(after=copy.deepcopy(event['before']), work_before=grant, work_after=0)
            event['input']['capacity'] = capacity
            event['result'] = dict(kind='Ok', complete=False) if grant == 0 else dict(
                kind='Err', reason='signature catalogue retention capacity')
            self.prove(data)

    def test_universal_axiom_inventory(self):
        names = re.findall(r'^theorem (\w+)',
            (ROOT / 'VerumExternalReplay/SignatureCatalogues.lean').read_text(), re.M)
        with tempfile.TemporaryDirectory(prefix='verum-signature-laws-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text('import VerumExternalReplay.SignatureCatalogues\n' +
                '\n'.join('#print axioms SignatureCatalogues.' + n for n in names))
            result = check(path)
        rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rosters), len(names))
        for row in rosters:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)

    def test_forged_correspondence_is_rejected_by_kernel(self):
        cases = []
        data = fixture(); data['events'][0]['after']['frozen'][0][0]['body'] = 1
        cases.append(('equal_program_different_owner', data))
        data = fixture(); data['events'][0]['after']['frozen'][2][0]['arity'] = 1
        cases.append(('wrong_arity', data))
        data = fixture(); data['events'][0]['work_after'] = 59
        cases.append(('unpaid_step', data))
        data = fixture(True); after = data['events'][0]['after']
        after.update(frozen=[[], [], []], catalogue_versions=[[], [], []], cells=9)
        after['selections'][0]['included'] = [False, False, False]
        cases.append(('ports_erases_initial', data))
        data = fixture(); data['events'][0]['input']['max_trace'] = 63
        cases.append(('changed_semantics', data))
        data = fixture(); data['events'][0]['input']['source'] = {
            **data['events'][0]['input']['source'], 'raw_space': '9'}
        cases.append(('foreign_source', data))
        # Two signatures may both be version one. Equal printed code and a
        # matching version cannot replace the selected node's body identities.
        data = fixture(); node = copy.deepcopy(data['signatures'][0])
        node.update(identity=1, parts=[3, 4, 5]); data['signatures'].append(node)
        data['bodies'] += [dict(identity=i, program=copy.deepcopy(data['bodies'][0]['program'])) for i in (3, 4, 5)]
        data['events'][0]['before']['next'] = 1
        cases.append(('same_version_other_chain', data))
        for name, data in cases:
            with self.subTest(name=name):
                self.prove(data, '`decide_cbv` failed: the proposition evaluates to `false`')

    def test_malformed_or_unobserved_ownership_is_rejected(self):
        data = fixture(); data['bodies'].append(copy.deepcopy(data['bodies'][0]))
        with self.assertRaisesRegex(ValueError, 'Duplicate nominal'): emit(data)
        data = fixture(); data['signatures'][0]['definition'] = False
        with self.assertRaises(ValueError): emit(data)
        data = fixture(); event = data['events'][0]
        event['before'] = copy.deepcopy(event['after'])
        with self.assertRaisesRegex(ValueError, 'Unobserved or rebound'): emit(data)
        data = fixture(); data['events'][0]['result'] = dict(kind='Err', reason='signature call allocation')
        with self.assertRaisesRegex(ValueError, 'Unmodeled library failure'): emit(data)


if __name__ == '__main__': unittest.main()
