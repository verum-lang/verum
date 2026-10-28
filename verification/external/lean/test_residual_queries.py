"""Synthetic controls for exact adaptive residual query replay; not native evidence."""
import copy
from fractions import Fraction
import hashlib
import json
from pathlib import Path
import re
import sys
import tempfile
import unittest

from check_residual_queries import FORECAST_LAW, FORMAT, emit, load_jsonl
from check_typed_programs import ROOT, check, require_success, run
from observer_export_replay import batches


FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


def rational(value):
    return dict(numerator=str(value.numerator), denominator=str(value.denominator))


def fixture(rows=None, context=None, mask=None):
    """Small synthetic packets, with Fraction arithmetic independent of Lean."""
    if rows is None:
        rows = [dict(context=[9], mask=[], target=[]),
                dict(context=[9], mask=[255, 17], target=[0]),
                dict(context=[1], mask=[0], target=[1, 2]),
                # Keep one residual bit asymmetric so mask/context changes matter.
                dict(context=[9], mask=[255], target=[254])]
    context = [9] if context is None else context
    mask = [255] if mask is None else mask
    lengths, residual, reference = {}, {}, []
    for row in rows:
        length = len(row['target'])
        lengths[str(length)] = lengths.get(str(length), 0) + 1
        if not length:
            continue
        child = residual.setdefault(tuple(row['context']), [])
        while len(child) < 8 * length:
            child.append([0, 0])
        while len(reference) < 8 * length:
            reference.append([0, 0])
        for position in range(8 * length):
            byte, offset = divmod(position, 8)
            target = (row['target'][byte] >> offset) & 1
            predicted = (row['mask'][byte] >> offset) & 1 if byte < len(row['mask']) else 0
            child[position][target ^ predicted] += 1
            reference[position][target] += 1
    queries = []
    for target in ([], [0], [255], [1, 2], [3, 0, 1]):
        for prefix in (False, True):
            length = len(target)
            count = sum(c for n, c in lengths.items() if int(n) >= length) if prefix else lengths.get(str(length), 0)
            base = Fraction(1, 2 ** (length + (not prefix)))
            candidate = raw = (count + base) / (len(rows) + 1)
            for position in range(8 * length):
                byte, offset = divmod(position, 8)
                actual = (target[byte] >> offset) & 1
                predicted = (mask[byte] >> offset) & 1 if byte < len(mask) else 0
                child = residual.get(tuple(context), [])
                c = child[position] if position < len(child) else [0, 0]
                r = reference[position] if position < len(reference) else [0, 0]
                candidate *= Fraction(2 * c[actual ^ predicted] + 1, 2 * sum(c) + 2)
                raw *= Fraction(2 * r[actual] + 1, 2 * sum(r) + 2)
            queries.append(dict(prefix=prefix, target=target,
                                candidate=rational(candidate), reference=rational(raw)))
    return dict(format=FORMAT, phase='synthetic', rows=rows,
                forecast=dict(forecast_law=FORECAST_LAW,
                              prediction=dict(context=context, mask=mask),
                              lengths=dict(total=len(rows), counts=lengths),
                              reference=reference,
                              residual=[dict(context=list(c), counts=counts)
                                        for c, counts in sorted(residual.items())]),
                queries=queries)


class ResidualQueryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.AdaptiveResidual'))

    def prove(self, data, expected=None):
        source, counts = emit(data)
        with tempfile.TemporaryDirectory(prefix='verum-query-synthetic-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(source)
            result = check(path, expected)
        if expected is not None:
            self.assert_only_false(result)
        return source, counts, result

    def assert_only_false(self, result):
        errors = [item['data'] for item in result['diagnostics'] if item['severity'] == 'error']
        self.assertTrue(errors)
        self.assertTrue(all(error.startswith(FALSE) for error in errors), errors)

    def test_mixed_lengths_initial_and_unseen_context_packets(self):
        data = dict(packets=[fixture(), fixture([]), fixture(context=[42], mask=[])])
        source, counts, _ = self.prove(data)
        self.assertEqual(counts['rows'], 8)
        self.assertEqual(counts['queries'], 30)
        self.assertEqual(counts['claims'], 77)
        self.assertEqual(sum(len(names) for _, names in batches(source)), counts['claims'])
        names = re.findall(r'^theorem (\w+)', source, re.M)
        self.assertEqual(len(names), counts['claims'])
        self.assertEqual(len(set(names)), len(names))
        with tempfile.TemporaryDirectory(prefix='verum-query-axioms-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(source + '\n'.join('#print axioms FrozenObserverExport.' + n for n in names))
            result = check(path)
        rosters = [item['data'] for item in result['diagnostics'] if item['severity'] == 'information']
        self.assertEqual(len(rosters), counts['claims'])
        for roster in rosters:
            self.assertTrue(roster.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), roster)

    def test_false_probabilities_histograms_and_banks_reach_kernel_rejection(self):
        for change in ('candidate', 'reference', 'prefix', 'target', 'mask', 'context',
                       'residual', 'raw_bank', 'histogram', 'histogram_total', 'strict_tail',
                       'bit_length', 'typed_renormalization'):
            packet = fixture()
            forecast, queries = packet['forecast'], packet['queries']
            if change in ('candidate', 'reference'):
                value = queries[2][change]
                value['numerator'] = str(int(value['numerator']) + 1)
            elif change == 'prefix': queries[0]['prefix'] = True
            elif change == 'target': queries[2]['target'] = [1]
            elif change == 'mask': forecast['prediction']['mask'] = [254]
            elif change == 'context': forecast['prediction']['context'] = [42]
            elif change == 'residual': forecast['residual'][0]['counts'][0][0] += 1
            elif change == 'raw_bank': forecast['reference'][0][0] += 1
            elif change == 'histogram': forecast['lengths']['counts'].update({'1': 1, '2': 2})
            elif change == 'histogram_total': forecast['lengths']['counts']['0'] += 1
            else:
                query = queries[3 if change == 'strict_tail' else 2]
                value = query['candidate']
                original = Fraction(int(value['numerator']), int(value['denominator']))
                scale = {'strict_tail': Fraction(3, 7),
                         'bit_length': Fraction(2 * 512 + 1, 512) / Fraction(9, 4),
                         'typed_renormalization': Fraction(2)}[change]
                query['candidate'] = rational(original * scale)
            with self.subTest(change=change):
                self.prove(dict(packets=[packet]), FALSE)

    def test_trace_hint_cannot_change_the_supplied_row_fold(self):
        source, _ = emit(dict(packets=[fixture()]))
        # Isolate the first transition: a rejected hint must not be reused in
        # later opaque folds, where it would also cause unrelated stuck terms.
        blocks = source.strip().split('\n\n')
        first = next(i for i, block in enumerate(blocks)
                     if block.startswith('@[cbv_eval]\ntheorem packet_0_transition_0'))
        source = '\n\n'.join(blocks[:first + 1] + ['end FrozenObserverExport']) + '\n'
        forged = source.replace('def packet_0_state_1 : State := ⟨1, 0,',
                                'def packet_0_state_1 : State := ⟨1, 1,')
        self.assertNotEqual(source, forged)
        with tempfile.TemporaryDirectory(prefix='verum-query-forged-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(forged)
            self.assert_only_false(check(path, FALSE))

    def test_strict_schema_rejects_ambiguous_and_incomplete_data(self):
        for change in ('format', 'phase', 'law', 'prefix', 'byte', 'boolean_byte', 'float_byte',
                       'negative_byte', 'fraction_number', 'fraction_decimal', 'denominator',
                       'total', 'length_key', 'length_bound', 'zero_count', 'boolean_count',
                       'extra_length', 'missing_length', 'order', 'duplicate_context',
                       'empty_context', 'missing_cell', 'extra_cell', 'pair_width', 'count_bound',
                       'queries', 'field', 'row_field', 'prediction_field'):
            packet = fixture()
            forecast, query = packet['forecast'], packet['queries'][0]
            if change == 'format': packet['format'] = 'synarc-residual-queries-v1'
            elif change == 'phase': packet['phase'] = False
            elif change == 'law': forecast['forecast_law'] = '2^-(n+1)'
            elif change == 'prefix': query['prefix'] = 1
            elif change in ('byte', 'boolean_byte', 'float_byte', 'negative_byte'):
                query['target'] = [{'byte': 256, 'boolean_byte': True, 'float_byte': 0.0, 'negative_byte': -1}[change]]
            elif change == 'fraction_number': query['candidate']['numerator'] = 1
            elif change == 'fraction_decimal': query['candidate']['numerator'] = '01'
            elif change == 'denominator': query['reference']['denominator'] = '0'
            elif change == 'total': forecast['lengths']['total'] = 3
            elif change == 'length_key': forecast['lengths']['counts']['01'] = 1
            elif change == 'length_bound': forecast['lengths']['counts'][str(2**64)] = 1
            elif change == 'zero_count': forecast['lengths']['counts']['0'] = 0
            elif change == 'boolean_count': forecast['lengths']['counts']['0'] = True
            elif change == 'extra_length': forecast['lengths']['counts']['3'] = 1
            elif change == 'missing_length': del forecast['lengths']['counts']['0']
            elif change == 'order': forecast['residual'].reverse()
            elif change == 'duplicate_context': forecast['residual'].append(copy.deepcopy(forecast['residual'][0]))
            elif change == 'empty_context': forecast['residual'][0]['counts'] = []
            elif change == 'missing_cell': forecast['residual'][0]['counts'].pop()
            elif change == 'extra_cell': forecast['reference'].append([0, 0])
            elif change == 'pair_width': forecast['reference'][0] = [0, 0, 0]
            elif change == 'count_bound': forecast['reference'][0] = [2**64, 0]
            elif change == 'queries': packet['queries'] = []
            elif change == 'field': packet['trusted'] = True
            elif change == 'row_field': packet['rows'][0]['trusted'] = True
            elif change == 'prediction_field': forecast['prediction']['target'] = []
            with self.subTest(change=change), self.assertRaises(ValueError):
                emit(dict(packets=[packet]))
        for data in ({'packets': []}, {'packets': [None]}, {'packets': 'not an array'}, {}):
            with self.subTest(data=data), self.assertRaises(ValueError): emit(data)

    def test_jsonl_retains_every_packet_and_rejects_invalid_lines(self):
        packets = [fixture(), fixture([])]
        valid = '\n'.join(json.dumps(packet) for packet in packets) + '\n'
        with tempfile.TemporaryDirectory(prefix='verum-query-jsonl-') as directory:
            path = Path(directory) / 'synthetic.jsonl'
            path.write_text(valid)
            self.assertEqual(load_jsonl(path), dict(packets=packets))
            for malformed in ('', valid + '\n', '\n' + valid, valid + '{',
                              '{"format":1,"format":2}\n',
                              '{"nested":{"0":1,"0":2}}\n', '{"value":NaN}\n'):
                path.write_text(malformed)
                with self.subTest(malformed=malformed[:80]), self.assertRaises(ValueError):
                    load_jsonl(path)

    def test_synthetic_cli_uses_original_jsonl_hash_and_private_replay(self):
        with tempfile.TemporaryDirectory(prefix='verum-query-cli-synthetic-') as directory:
            root = Path(directory)
            export, provenance = root / 'synthetic.jsonl', root / 'provenance.json'
            output, report = root / 'proof.lean', root / 'report.json'
            packets = [fixture(), fixture([])]
            for packet in packets:
                packet['queries'] = packet['queries'][:2]
            export.write_text('\n'.join(json.dumps(packet) for packet in packets) + '\n')
            digest = hashlib.sha256(export.read_bytes()).hexdigest()
            provenance.write_text(json.dumps({'export_sha256': digest, 'kind': 'synthetic-test-only'}))
            require_success(run(sys.executable, str(ROOT / 'check_residual_queries.py'),
                                '--export', str(export), '--sha256', digest,
                                '--provenance', str(provenance), '--output', str(output),
                                '--report', str(report)))
            receipt = json.loads(report.read_text())
            self.assertEqual(receipt['status'], 'verified')
            self.assertEqual(receipt['export_sha256'], digest)
            self.assertEqual(receipt['reported_rust_provenance']['kind'], 'synthetic-test-only')
            self.assertEqual(receipt['counts']['claims'], 18)
            self.assertEqual(receipt['model_build'], 'private immutable source snapshot')
            self.assertIn('check_residual_queries.py', receipt['source_sha256'])
            self.assertIn('VerumExternalReplay/AdaptiveResidual.lean', receipt['source_sha256'])


if __name__ == '__main__':
    unittest.main()
