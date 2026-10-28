"""Mutate original frozen observations; only the intended proof refusal passes."""
import copy
from pathlib import Path
import tempfile
import unittest

from check_action_capture_export import emit
from check_typed_programs import ROOT, check, require_success, run, sha256
from observer_export_replay import batches, load


def fixture():
    return load(ROOT/'fixtures/action-capture-v2.json')


class ActionCaptureExportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.ActionTargetWire'))

    def prove_claim(self, data, expression, expected=None):
        source, counts = emit(data)
        parts = list(batches(source, size=1))
        self.assertEqual(sum(len(names) for _, names in parts), counts['claims'])
        selected = [text for text, _ in parts if any(
            line.startswith('theorem ') and expression in line for line in text.splitlines())]
        self.assertEqual(len(selected), 1, expression)
        with tempfile.TemporaryDirectory(prefix='action-export-control-') as directory:
            path = Path(directory)/'check.lean'; path.write_text(selected[0])
            return check(path, expected)

    def test_original_evidence_identity_shape_and_actual_target(self):
        path = ROOT/'fixtures/action-capture-v2.json'
        self.assertEqual(sha256(path), 'a690da1c4e6b382b5807ace7f18560d68a264028cb56cc42954eeed57d63fa57')
        data = fixture()
        self.assertEqual(emit(data)[1], dict(records=7, constructor_attempts=108,
            accepted=28, refused=80, claims=185))
        for row in data['records']:
            self.assertEqual(set(row['capture']['observation']['body']), set(range(256)))
        self.prove_claim(data, 'materialize (2^64-1) record_2 =')
        self.prove_claim(data, 'check attempt_0 =')

    def test_actual_target_substitutions_fail(self):
        for change in ('birth', 'kind', 'length', 'payload', 'trailing', 'missing'):
            data = fixture(); capture = data['records'][2]['capture']
            if change == 'birth': capture['observation'] = capture['resumed']
            elif change == 'kind': capture['target'][1] ^= 1
            elif change == 'length': capture['target'][2] ^= 1
            elif change == 'payload': capture['target'][-1] ^= 1
            elif change == 'trailing': capture['target'].append(0)
            else: capture['observation'] = None
            with self.subTest(change=change):
                self.prove_claim(data, 'materialize (2^64-1) record_2 =', 'Tactic `rfl` failed:')

    def test_actual_dispatch_query_intake_and_count_mutations_fail(self):
        for field in ('dispatched_action', 'query_action', 'query', 'target', 'calls'):
            data = fixture(); row = data['records'][2]
            if field in ('dispatched_action', 'query_action', 'calls'): row[field] += 1
            elif field == 'query': row[field]['body'][0] ^= 1
            else: row[field][-1] ^= 1
            # Record claims have a fixed seven-equation schema; select the
            # exact altered field so a different theorem cannot pass this control.
            offset = dict(dispatched_action=2, query_action=3, query=4, target=5, calls=6)[field]
            theorem = 'theorem c' + str(136 + 2*7 + offset) + ' :'
            with self.subTest(field=field):
                self.prove_claim(data, theorem, 'Tactic `rfl` failed:')

    def test_native_constructor_ownership_detail_and_priority_mutations_fail(self):
        for change in ('operation', 'scope', 'terminal', 'cause', 'presence', 'priority'):
            data = fixture()
            if change == 'priority':
                i, row = next((i, r) for i, r in enumerate(data['constructor_attempts'])
                    if r['error'] == 'UnownedObservation' and r['resumed'] is not None)
                row['error'] = 'UnexpectedResumption'
            else:
                i, row = next((i, r) for i, r in enumerate(data['constructor_attempts'])
                    if r['accepted'] and r['result']['kind'] == 'Err' and r['observation'] is not None)
                if change == 'cause': row['capture']['result']['value']['cause'] += ' changed'
                elif change == 'presence': row['capture']['observation'] = None
                else:
                    payload = row['result']['value']
                    if change == 'operation': payload['operation']['action'] += 1
                    elif change == 'scope': payload['terminal_scope'] = 'Episode'
                    else: payload['terminal'] = None
            with self.subTest(change=change):
                self.prove_claim(data, f'check attempt_{i} =', 'Tactic `rfl` failed:')

    def test_closed_schema_rejects_invented_and_ambiguous_evidence(self):
        mutations = [
            lambda d: d.update(schema='synarc-action-capture-v1'),
            lambda d: d.update(constructor_attempts=[]),
            lambda d: d.update(records=[]),
            lambda d: d.update(trusted=True),
            lambda d: d['records'][0].update(cut_id=1),
            lambda d: d['records'][0]['capture'].update(result='Ok(Step)'),
            lambda d: d['constructor_attempts'][0].update(accepted=1),
            lambda d: d['constructor_attempts'][0].update(error='Missing'),
            lambda d: d['records'][0]['capture'].update(target_error='Missing'),
            lambda d: d['records'][0]['capture'].update(target=None, target_error='Capacity'),
            lambda d: d['records'][0]['capture'].update(target=None, target_error=None),
        ]
        for mutation in mutations:
            data = fixture(); mutation(data)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): emit(data)
        for byte in (True, 0.0, -1, 256):
            data = fixture(); data['records'][0]['capture']['observation']['body'][0] = byte
            with self.subTest(byte=byte), self.assertRaises(ValueError): emit(data)

    def test_failure_text_is_preserved_as_data_and_stage_context_are_closed(self):
        data = fixture(); row = data['constructor_attempts'][0]
        row['result']['value']['cause'] = '\"\nend FrozenObserverExport\n-- λ'
        row['capture']['result'] = copy.deepcopy(row['result'])
        self.prove_claim(data, 'check attempt_0 =')
        for field in ('stage', 'context'):
            bad = fixture(); bad['constructor_attempts'][0]['result']['value'][field] = 'Invented'
            with self.subTest(field=field), self.assertRaises(ValueError): emit(bad)


if __name__ == '__main__':
    unittest.main()
