"""Independent controls for exact native-v4/v5 observations, not Rust refinement."""
import copy
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import tempfile
import unittest

from check_append_handoff_export import emit, MODELS
from check_typed_programs import ROOT, check, require_success, run
from observer_export_replay import batches, declarations

FALSE = 'Tactic `decide` proved that the proposition'


def normalized_owners(data):
    """Only allocation addresses may differ between exact native executions."""
    data = copy.deepcopy(data)
    languages = {s['language_owner']: f'language-{i}' for i, s in enumerate(data['streams'])}
    for stream in data['streams']:
        children = {}
        for state in stream['states']:
            if state['indexer'] is not None:
                indexer = state['indexer']
                name = f'child-{indexer["arrival"]}'
                previous = children.setdefault(indexer['child_owner'], name)
                if previous != name or indexer['child_owner'] in languages:
                    raise ValueError('Captured native owners alias')
        def walk(value, key=None):
            if type(value) is str and key in ('language_owner', 'source', 'attempted'):
                return languages[value]
            if type(value) is str and key in ('owner', 'child_owner', 'children', 'pending_owner'):
                return children[value]
            if type(value) is dict: return {k: walk(v, k) for k, v in value.items()}
            if type(value) is list: return [walk(v, key) for v in value]
            return value
        stream.update(walk(stream))
    return json.dumps(data, sort_keys=True, separators=(',', ':'))


def fixture(version='v4'):
    files = ROOT / 'fixtures'
    prefix = f'append-handoff-native-{version}'
    provenance = json.loads((files / f'{prefix}.provenance.json').read_text())
    raw = gzip.decompress((files / f'{prefix}.json.gz').read_bytes())
    receipt_raw = gzip.decompress((files / f'{prefix}.receipt.json.gz').read_bytes())
    if hashlib.sha256(raw).hexdigest() != provenance['export_sha256']:
        raise ValueError('Changed frozen native export')
    if hashlib.sha256(receipt_raw).hexdigest() != provenance['receipt_sha256']:
        raise ValueError('Changed frozen native receipt')
    receipt = json.loads(receipt_raw)
    if receipt['binary'] != provenance['native_binary']:
        raise ValueError('Changed reported binary identity')
    if version == 'v5':
        build_raw = gzip.decompress((files / f'{prefix}.build-receipt.json.gz').read_bytes())
        build = json.loads(build_raw)
        if (hashlib.sha256(build_raw).hexdigest() != provenance['build_receipt_sha256'] or
                build['binary'] != receipt['binary'] or build['source'] != receipt['source']):
            raise ValueError('Changed frozen build provenance')
    for source in provenance['sources']:
        retained = gzip.decompress((files / source['fixture']).read_bytes())
        if (hashlib.sha256(retained).hexdigest() != source['sha256'] or
                len(retained) != source['bytes'] or
                receipt['source'][source['path']]['sha256'] != source['sha256']):
            raise ValueError('Changed frozen source identity')
    return json.loads(raw)


class AppendHandoff(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix='verum-append-handoff-')
        cls.root = Path(cls.temporary.name)
        for name in ['lakefile.toml', 'lean-toolchain'] + [f'VerumExternalReplay/{m}.lean' for m in MODELS]:
            path = cls.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, path)
        for module in MODELS:
            target = cls.root / f'.lake/build/lib/lean/VerumExternalReplay/{module}.olean'
            target.parent.mkdir(parents=True, exist_ok=True)
            require_success(run('lake', 'env', 'lean', '-M', '1024', '-o', str(target),
                                f'VerumExternalReplay/{module}.lean', cwd=cls.root, timeout=60))

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def test_general_handoff_laws_and_exact_axiom_roster(self):
        source = (self.root / 'VerumExternalReplay/AppendHandoff.lean').read_text()
        names = re.findall(r'^theorem (\w+)', source, re.M)
        self.assertEqual(names, ['snapshot_equality_preserves_bytes', 'handoff_committed',
                                 'emitted_pending_identity'])
        path = self.root / 'audit.lean'
        path.write_text('import VerumExternalReplay.AppendHandoff\n' +
                        '\n'.join('#print axioms AppendHandoff.' + name for name in names))
        result = check(path, cwd=self.root, memory_mib=1024)
        self.assertEqual(len(result['diagnostics']), len(names))
        for name, row in zip(names, result['diagnostics']):
            self.assertEqual(row['severity'], 'information')
            self.assertEqual(row['data'], f"'AppendHandoff.{name}' depends on axioms: [propext]")

    def test_wake_pass_laws_and_exact_axiom_roster(self):
        names = ['ready_completed', 'wake_preserves_retained', 'visits_exact', 'completed_visits']
        source = (self.root / 'VerumExternalReplay/WakeHandoff.lean').read_text()
        self.assertEqual(re.findall(r'^theorem (\w+)', source, re.M), names)
        path = self.root / 'wake-audit.lean'
        path.write_text('import VerumExternalReplay.WakeHandoff\n' +
                        '\n'.join('#print axioms WakeHandoff.' + name for name in names))
        result = check(path, cwd=self.root, memory_mib=1024)
        self.assertTrue(result['accepted'])
        self.assertEqual(len(result['diagnostics']), len(names))
        for name, row in zip(names, result['diagnostics']):
            self.assertEqual(row['severity'], 'information')
            expected = ('depends on axioms: [propext]' if name == 'ready_completed'
                        else 'does not depend on any axioms')
            self.assertEqual(row['data'], f"'WakeHandoff.{name}' {expected}")

    def check_batches(self, text, negative=False):
        shared = self.root / 'FrozenWakeDeclarations.lean'
        shared.write_text(declarations(text))
        require_success(run('lake', 'env', 'lean', '-M', '1024', '-o',
            str(self.root / '.lake/build/lib/lean/FrozenWakeDeclarations.olean'), str(shared),
            cwd=self.root, timeout=60))
        rejected = 0
        for i, (part, names) in enumerate(batches(text, shared_module='FrozenWakeDeclarations')):
            path = self.root / f'wake-part-{i}.lean'
            path.write_text(part)
            result = run('lake', 'env', 'lean', '-M', '1024', '--json', str(path), cwd=self.root, timeout=60)
            self.assertFalse(result.stderr)
            diagnostics = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            errors = [d for d in diagnostics if d['severity'] == 'error']
            if errors:
                self.assertTrue(negative, str(errors)[:1200])
                self.assertEqual(result.returncode, 1)
                self.assertTrue(all(e['data'].startswith(FALSE) for e in errors), str(errors)[:1200])
                rejected += 1
            else:
                self.assertEqual(result.returncode, 0)
                rosters = [d['data'] for d in diagnostics if d['severity'] == 'information']
                self.assertEqual(len(rosters), len(names))
                for name, roster in zip(names, rosters):
                    self.assertIn(roster, [f"'FrozenObserverExport.{name}' {suffix}" for suffix in (
                        'does not depend on any axioms', 'depends on axioms: [propext]',
                        'depends on axioms: [propext, Quot.sound]')])
        self.assertEqual(rejected > 0, negative)

    @unittest.skipUnless(os.environ.get('VERUM_SYNARC_BINARY'), 'requires the frozen native binary')
    def test_exact_frozen_native_producer(self):
        self.check_native('v4', 'VERUM_SYNARC_BINARY')

    @unittest.skipUnless(os.environ.get('VERUM_SYNARC_V5_BINARY'), 'requires the frozen v5 native binary')
    def test_exact_frozen_v5_native_producer(self):
        self.check_native('v5', 'VERUM_SYNARC_V5_BINARY')

    def check_native(self, version, variable):
        binary = Path(os.environ[variable])
        provenance = json.loads((ROOT / f'fixtures/append-handoff-native-{version}.provenance.json').read_text())
        before = hashlib.sha256(binary.read_bytes()).hexdigest()
        self.assertEqual(before, provenance['native_binary']['sha256'])
        export = self.root / f'native-export-{version}.json'
        name = ('construction::family::syntax::structural::priority::streaming_courts::'
                'bounded_native_causal_join_export_preserves_owners_and_resumptions')
        output = require_success(run('env', f'SYNARC_DOMAIN_SUPPORT_EXPORT={export}', str(binary),
            name, '--exact', '--nocapture', '--test-threads=1', cwd=self.root, timeout=30))
        self.assertEqual(re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', output),
                         [('1', '0', '0')])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), before)
        self.assertTrue(normalized_owners(json.loads(export.read_text())) == normalized_owners(fixture(version)),
                        'Native export differs after renaming only captured allocation addresses')

    def test_frozen_data_and_actual_rejections(self):
        baseline = fixture()
        text, counts = emit(baseline)
        self.assertEqual(counts['events'], 1253)
        self.assertEqual(counts['publications'], 9)
        path = self.root / 'claims.lean'
        path.write_text(text)
        self.assertTrue(check(path, cwd=self.root, memory_mib=1024)['accepted'])
        for mutation in ['uncommitted', 'not_ready', 'wrong_arrival', 'wrong_owner',
                         'changed_outputs', 'ordinary_cursor', 'constructor_cursor',
                         'lost_emission', 'zero_work_mutation', 'capacity_mutation']:
            with self.subTest(mutation=mutation):
                data = copy.deepcopy(baseline)
                data['streams'] = data['streams'][:1]
                stream = data['streams'][0]
                event = next(e for e in stream['events'] if e['result'] == 'expression')
                if mutation == 'uncommitted':
                    stream['states'][event['before']]['indexed_entries'] -= 1
                elif mutation == 'not_ready':
                    stream['states'][event['before']]['append_ready'] = False
                elif mutation in ['wrong_arrival', 'wrong_owner', 'changed_outputs']:
                    publication = stream['emitted'][event['publication']]
                    if mutation == 'wrong_arrival': publication['arrival'] -= 1
                    if mutation == 'wrong_owner': publication['owner'] = '1'
                    if mutation == 'changed_outputs': publication['outputs'][0] ^= 1
                elif mutation in ['ordinary_cursor', 'constructor_cursor']:
                    key = 'next_ordinary_lane' if mutation == 'ordinary_cursor' else 'constructor_cursor'
                    stream['states'][event['after']][key] += 1
                elif mutation == 'lost_emission':
                    stream['states'][event['before']]['pending_emission'] = False
                else:
                    outcome = 'work_refusal' if mutation == 'zero_work_mutation' else 'capacity_refusal'
                    event = next(e for e in stream['events'] if e['result'] == outcome)
                    after = copy.deepcopy(stream['states'][event['after']])
                    # A non-projected field must still participate in exact snapshot equality.
                    after['plan']['requested_domain'] += 1
                    event['after'] = len(stream['states'])
                    stream['states'].append(after)
                text, _ = emit(data)
                path.write_text(text)
                result = check(path, FALSE, cwd=self.root, memory_mib=1024)
                errors = [r for r in result['diagnostics'] if r['severity'] == 'error']
                self.assertTrue(errors)
                self.assertTrue(all(r['data'].startswith(FALSE) for r in errors), errors)

    def test_schema_cannot_coerce_or_erase_inputs(self):
        for mutation in ['old_schema', 'extra_field', 'bool_index', 'noncanonical_owner']:
            data = fixture()
            if mutation == 'old_schema': data['schema'] = 'synarc-causal-constructor-native-v3'
            elif mutation == 'extra_field': data['streams'][0]['states'][0]['wake_cursor'] = 0
            elif mutation == 'bool_index': data['streams'][0]['states'][0]['indexed_entries'] = False
            else: data['streams'][0]['language_owner'] = '01'
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                emit(data)

    def test_v5_wake_completion_and_pending_owners(self):
        baseline = fixture('v5')
        text, counts = emit(baseline)
        self.assertEqual((counts['events'], counts['commits'], counts['wake_visits'],
                          counts['completed_wake_passes'], counts['publications']), (1253, 13, 39, 13, 9))
        self.check_batches(text)
        for mutation in ['early_handoff', 'skipped_visit', 'repeated_visit', 'missing_reset',
                         'wrong_pending_owner', 'missing_constructor_wake', 'frame_mutation',
                         'wrong_posting_lookup', 'zero_work_wake_mutation']:
            with self.subTest(mutation=mutation):
                data = copy.deepcopy(baseline)
                data['streams'] = data['streams'][:1]
                stream = data['streams'][0]
                states = stream['states']
                wake = next(e for e in stream['events'] if e['result'] == 'advanced' and
                    states[e['before']]['pending_index'] is not None and
                    states[e['before']]['indexed_entries'] == states[e['before']]['catalogue_entries'] and
                    states[e['before']]['wake_cursor'] < len(states[e['before']]['constructors']) and
                    states[e['before']]['constructors'][states[e['before']]['wake_cursor']]['phase'] == 'Idle')
                before, after = states[wake['before']], states[wake['after']]
                if mutation == 'early_handoff':
                    e = next(e for e in stream['events'] if e['result'] == 'expression')
                    states[e['before']]['wake_cursor'] -= 1
                elif mutation == 'skipped_visit': after['wake_cursor'] += 1
                elif mutation == 'repeated_visit': after['wake_cursor'] -= 1
                elif mutation == 'missing_reset':
                    e = next(e for e in stream['events'] if
                        states[e['after']]['indexed_entries'] > states[e['before']]['indexed_entries'])
                    states[e['after']]['wake_cursor'] = 1
                elif mutation == 'wrong_pending_owner': after['pending_owner'] = '1'
                elif mutation == 'missing_constructor_wake':
                    after['constructors'][before['wake_cursor']]['phase'] = 'Idle'
                elif mutation == 'wrong_posting_lookup':
                    after['constructors'][before['wake_cursor']]['axes'][0]['posting'] = [0, 0]
                elif mutation == 'zero_work_wake_mutation':
                    e = next(e for e in stream['events'] if e['result'] == 'work_refusal' and
                             states[e['before']]['pending_index'] is not None)
                    changed = copy.deepcopy(states[e['before']])
                    changed['wake_cursor'] = 1 if changed['wake_cursor'] == 0 else 0
                    e['after'] = len(states)
                    states.append(changed)
                else: after['cells'] += 1
                text, _ = emit(data)
                self.check_batches(text, negative=True)

    def test_v5_requires_the_raw_fields_without_coercion(self):
        for mutation in ['missing_cursor', 'missing_owner', 'boolean_cursor', 'orphan_owner']:
            data = fixture('v5')
            state = data['streams'][0]['states'][0]
            if mutation == 'missing_cursor': del state['wake_cursor']
            elif mutation == 'missing_owner': del state['pending_owner']
            elif mutation == 'boolean_cursor': state['wake_cursor'] = False
            else: state['pending_owner'] = '1'
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                emit(data)


if __name__ == '__main__':
    unittest.main()
