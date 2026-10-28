"""Native boundary replay and strict malformed/forged-evidence controls."""
import gzip
import hashlib
import json
import unittest

from check_expressiveness_export import emit
from check_typed_programs import ROOT, require_success, run
from replay_assertions import ReplayAssertions

FIXTURE = ROOT/'fixtures/expressiveness-native-v1.jsonl.gz'


def fixture():
    return dict(packets=[json.loads(line) for line in gzip.decompress(FIXTURE.read_bytes()).splitlines()])


class ExpressivenessExportTests(ReplayAssertions, unittest.TestCase):
    emitter = staticmethod(emit)
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.OutcomeTransitions',
                            'VerumExternalReplay.PredictiveObservation'))

    def test_frozen_native_replay(self):
        counts = self.prove(fixture())
        self.assertEqual(counts['outcome_histories'],2)
        self.assertEqual(counts['outcome_rows'],4)
        self.assertEqual(counts['probability_queries'],12)
        self.assertEqual(counts['training_transitions'],8)

    def test_native_source_identity(self):
        receipt = json.loads((ROOT/'fixtures/expressiveness-native-v1.provenance.json').read_text())
        exported = gzip.decompress(FIXTURE.read_bytes())
        self.assertEqual(hashlib.sha256(exported).hexdigest(),receipt['export_sha256'])
        native = json.loads(gzip.decompress((ROOT/'fixtures/expressiveness-native-v1.receipt.json.gz').read_bytes()))
        self.assertEqual(native['head'],receipt['native_source_commit'])
        self.assertEqual(native['kan_binary']['sha256'],receipt['native_binary_sha256'])
        for item in receipt['retained_files']:
            raw = gzip.decompress((ROOT/'fixtures'/item['file']).read_bytes())
            self.assertEqual(hashlib.sha256(raw).hexdigest(),item['sha256'])
            if 'source_path' in item:
                self.assertEqual(item['sha256'],native['source'][item['source_path']]['sha256'])
        extraction = receipt['extraction']; lines = []
        for name in extraction['ordered_logs']:
            text = gzip.decompress((ROOT/'fixtures'/name).read_bytes()).decode()
            lines.extend(line.split(extraction['marker'],1)[1] for line in text.splitlines()
                         if extraction['marker'] in line)
        self.assertEqual(len(lines),extraction['packets'])
        self.assertEqual(('\n'.join(lines)+'\n').encode(),exported)

    def test_forged_native_values_fail_in_kernel(self):
        for kind in ('program','target','bank','owner_alias','query','conditional','conditioning','witness'):
            data = fixture()
            if kind in ('program','target','bank','owner_alias'): data['packets'] = data['packets'][:1]
            else: data['packets'] = data['packets'][1:2]
            packet = data['packets'][0]
            if kind == 'program':
                packet['native_owner_metadata']['tables']['definitions'][0]['update']['steps'][0]['arguments'][1] = {'literal':1}
                select = 'p0_left_rows.map OutcomeTransitions.predictiveRow'
            elif kind == 'target':
                packet['returned_target_wires'][0][0][0] = 2
                select = '([2, 0, 0, 0, 0, 0, 0, 0] : List Nat)'
            elif kind == 'bank':
                packet['native_owner_metadata']['tables']['scores'][1]['frontier']['ratio']['numerator'] = '1'
                select = 'p0_left_rows.map OutcomeTransitions.predictiveRow'
            elif kind == 'owner_alias':
                packet['native_owner_metadata']['left_final']['cut']['owner'] = 0
                select = '([0, 0, 2] : List Nat).Nodup'
            elif kind == 'query':
                packet['points'][0] = ['1','2']
                select = 'AdaptiveResidual.candidate'
                # Only the selected target's query assertion is needed for this control.
                source,_ = emit(data)
                select = next(b.split(' : ',1)[1].split(' := by',1)[0] for b in source.split('\n\n')
                              if b.startswith('theorem ') and select in b and '⟨1, 2, by decide⟩' in b)
            elif kind == 'conditional':
                packet['conditionals'][0] = ['1','3']; select = 'ExactMass.mul ⟨1, 3, by decide⟩'
            elif kind == 'conditioning':
                packet['conditioning']['high_six_bits'] = 1; select = '(List.range 4)'
            else:
                packet['strict_positive_witness'] = [['1','4'] for _ in range(4)]
                select = '¬ ConditionalExpressiveness.RankOne p0_witness'
            with self.subTest(kind=kind): self.prove(data, select)

    def test_incomplete_or_ambiguous_evidence_refused(self):
        for kind in ('unknown','unknown_format','missing_rows','duplicate_query','missing_target','extra_event',
                     'unknown_owner','changed_definition','unknown_codec','missing_probability'):
            data = fixture(); first, second = data['packets'][:2]
            if kind == 'unknown': first['kind'] = 'unrecognized'
            elif kind == 'unknown_format': second['format'] = 'unknown'
            elif kind == 'missing_rows': del second['rows']
            elif kind == 'duplicate_query': second['query_targets'][1] = second['query_targets'][0]
            elif kind == 'missing_target': first['returned_target_wires'][0].pop()
            elif kind == 'extra_event': first['native_owner_metadata']['tables']['events'] = [{}]
            elif kind == 'unknown_owner': first['native_owner_metadata']['left_final']['score'] = 999
            elif kind == 'changed_definition': first['native_owner_metadata']['left_final']['cut']['definition'] = 999
            elif kind == 'unknown_codec': first['native_owner_metadata']['tables']['wire_protocol']['target']['encoding'] = 'host'
            else: second['points'].pop()
            with self.subTest(kind=kind), self.assertRaises(ValueError): emit(data)


if __name__ == '__main__': unittest.main()
