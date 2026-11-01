"""Independent controls for exact native-v4 observations, not Rust refinement."""
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
            if type(value) is str and key in ('owner', 'child_owner', 'children'):
                return children[value]
            if type(value) is dict: return {k: walk(v, k) for k, v in value.items()}
            if type(value) is list: return [walk(v, key) for v in value]
            return value
        stream.update(walk(stream))
    return json.dumps(data, sort_keys=True, separators=(',', ':'))


def fixture():
    files = ROOT / 'fixtures'
    provenance = json.loads((files / 'append-handoff-native-v4.provenance.json').read_text())
    raw = gzip.decompress((files / 'append-handoff-native-v4.json.gz').read_bytes())
    receipt_raw = gzip.decompress((files / 'append-handoff-native-v4.receipt.json.gz').read_bytes())
    if hashlib.sha256(raw).hexdigest() != provenance['export_sha256']:
        raise ValueError('Changed frozen native export')
    if hashlib.sha256(receipt_raw).hexdigest() != provenance['receipt_sha256']:
        raise ValueError('Changed frozen native receipt')
    receipt = json.loads(receipt_raw)
    if receipt['binary'] != provenance['native_binary']:
        raise ValueError('Changed reported binary identity')
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

    @unittest.skipUnless(os.environ.get('VERUM_SYNARC_BINARY'), 'requires the frozen native binary')
    def test_exact_frozen_native_producer(self):
        binary = Path(os.environ['VERUM_SYNARC_BINARY'])
        provenance = json.loads((ROOT / 'fixtures/append-handoff-native-v4.provenance.json').read_text())
        before = hashlib.sha256(binary.read_bytes()).hexdigest()
        self.assertEqual(before, provenance['native_binary']['sha256'])
        export = self.root / 'native-export.json'
        name = ('construction::family::syntax::structural::priority::streaming_courts::'
                'bounded_native_causal_join_export_preserves_owners_and_resumptions')
        output = require_success(run('env', f'SYNARC_DOMAIN_SUPPORT_EXPORT={export}', str(binary),
            name, '--exact', '--nocapture', '--test-threads=1', cwd=self.root, timeout=30))
        self.assertEqual(re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', output),
                         [('1', '0', '0')])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), before)
        self.assertTrue(normalized_owners(json.loads(export.read_text())) == normalized_owners(fixture()),
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


if __name__ == '__main__':
    unittest.main()
