"""Native grammar/provenance authority binding without conflating identity namespaces."""
import gzip
import copy
import hashlib
import json
from pathlib import Path
import unittest
from check_dyadic_ownership_export import emit
import test_dyadic_ownership_export as baseline
from test_dyadic_ownership_export import FALSE, visit


def fixture():
    raw = gzip.decompress((Path(__file__).resolve().parent / 'fixtures/dyadic-ownership-v3.json.gz').read_bytes())
    assert hashlib.sha256(raw).hexdigest() == '4d2b86c9e0bd0b44bc408274dff1521010faeba31a2b606a9845bcc9be729967'
    return json.loads(raw)


class DyadicOwnershipV3Tests(unittest.TestCase):
    setUpClass = classmethod(baseline.DyadicOwnershipTests.setUpClass.__func__)
    replay = baseline.DyadicOwnershipTests.replay

    def test_exact_frozen_authorities(self):
        self.assertEqual(self.replay(fixture()),
                         dict(events=6150, segments=4, snapshots=811, claims=669))

    def test_identity_namespaces_are_independent(self):
        data = fixture()
        def rename(o):
            for field, delta in [('source_owner', 100), ('source_authority', 200),
                                 ('provenance_authority', 200), ('initializer_owner', 300)]:
                if o.get(field) is not None: o[field] += delta
            if 'owner' in o and 'enumeration' in o: o['owner'] += 400
            if 'authority' in o: o['authority'] += 500  # paid-look namespace
        visit(data, rename)
        self.replay(data)
        # Different immutable grammars may share an authority. The map is
        # functional, not assumed injective and not identified with grammar IDs.
        data = fixture()
        def shared(o):
            for field in ('source_authority', 'provenance_authority'):
                if o.get(field) is not None: o[field] = 0
        visit(data, shared)
        self.replay(data)

    def test_foreign_authorities_reach_kernel_rejection(self):
        for mutation in ('candidate_grammar', 'candidate_authority', 'candidate_provenance',
                         'materializer_provenance', 'snapshot_authority', 'grammar_rebinding'):
            data = fixture()
            def change(o):
                candidate = o.get('original_name') == '0' and 'programs_ready' not in o
                if mutation == 'candidate_grammar' and candidate: o['source_owner'] += 100
                elif mutation == 'candidate_authority' and candidate:
                    o['source_authority'] += 100; o['provenance_authority'] += 100
                elif mutation == 'candidate_provenance' and candidate: o['provenance_authority'] += 100
                elif mutation == 'materializer_provenance' and o.get('original_name') == '0' and 'programs_ready' in o:
                    if o['provenance_authority'] is not None: o['provenance_authority'] += 100
                elif mutation == 'snapshot_authority' and o.get('owner') == 0 and 'enumeration' in o:
                    o['source_authority'] += 100
                elif mutation == 'grammar_rebinding' and 'source_owner' in o: o['source_owner'] = 0
            visit(data, change)
            with self.subTest(mutation=mutation): self.replay(data, FALSE)

    def test_missing_or_partial_authority_never_falls_back(self):
        for mutation in ('candidate_grammar', 'candidate_provenance', 'materializer_authority',
                         'partial_provenance', 'initialized_without_provenance',
                         'boolean_authority', 'legacy_schema', 'unknown_schema'):
            data = fixture()
            def change(o):
                candidate = 'original_name' in o and 'programs_ready' not in o
                if mutation == 'candidate_grammar' and candidate: o.pop('source_owner')
                elif mutation == 'candidate_provenance' and candidate: o['provenance_authority'] = None
                elif mutation == 'materializer_authority' and 'programs_ready' in o: o.pop('source_authority')
                elif mutation == 'partial_provenance' and 'programs_ready' in o: o['provenance_authority'] = None
                elif mutation == 'initialized_without_provenance' and 'programs_ready' in o:
                    o['provenance_authority'] = None; o['provenance_name'] = None
                elif mutation == 'boolean_authority' and 'source_authority' in o: o['source_authority'] = True
            if mutation == 'legacy_schema': data['schema'] = 'synarc-dyadic-cumulative-ownership-v2'
            elif mutation == 'unknown_schema': data['schema'] = 'synarc-dyadic-cumulative-ownership-v4'
            else: visit(data, change)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): emit(data)

    def test_learned_provenance_cannot_be_erased_before_initialization(self):
        data = fixture(); data['events'] = data['events'][:101]
        row = data['events'][-1]
        self.assertIsNotNone(row['before']['materializing']['provenance_authority'])
        self.assertIsNone(row['before']['materializing']['initializer_owner'])
        row['after'] = copy.deepcopy(row['before'])
        row['after']['materializing']['provenance_authority'] = None
        row['after']['materializing']['provenance_name'] = None
        row['returned'] = {'kind':'Advanced'}
        with self.assertRaisesRegex(ValueError, 'Erased learned provenance authority'): emit(data)


if __name__ == '__main__':
    unittest.main()
