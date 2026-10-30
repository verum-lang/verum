import copy
import gzip
import hashlib
import json
from pathlib import Path
import unittest
from check_periodic_routing_export import check

FIXTURES = Path(__file__).parent / 'fixtures'


def payload(name):
    return gzip.decompress((FIXTURES / ('periodic-routing-' + name + '.json.gz')).read_bytes())


class PeriodicRouting(unittest.TestCase):
    def test_frozen_native_exports(self):
        provenance = json.loads((FIXTURES / 'periodic-routing-v1.provenance.json').read_text())
        for name, expected in provenance['cases'].items():
            with self.subTest(name=name):
                raw = payload(name)
                self.assertEqual(hashlib.sha256(raw).hexdigest(), expected['sha256'])
                self.assertEqual(check(json.loads(raw)), expected['summary'])

    def test_discriminating_mutations(self):
        cases = [
            ('zero-entitlement', ['legacy_owner'], True),
            ('zero-entitlement', ['legacy_pending'], True),
            ('zero-entitlement', ['agenda', 'source_owner'], 77),
            ('zero-entitlement', ['agenda', 'current', 'identity'], 77),
            ('zero-entitlement', ['sittings'], 3),
            ('zero-entitlement', ['fresh'], 1),
            ('zero-entitlement', ['floors'], 1),
            ('zero-entitlement', ['ledger', 'reserved'], '1'),
            ('zero-entitlement', ['ledger', 'settled_receipts', -1, 'lane'], 'Legacy'),
            ('zero-entitlement', ['ledger', 'settled_receipts', -1, 'spent'], 1),
            ('zero-entitlement', ['ledger', 'settled_receipts', -1, 'request'], '11'),
            ('zero-entitlement', ['ledger', 'spent'], '-1'),
            ('sticky-retention-refusal', ['agenda', 'halted'], False),
            ('sticky-retention-refusal', ['agenda', 'first_unretained'], None),
            ('paused-materializer', ['agenda', 'current', 'materializer', 'processed'], 1),
        ]
        for name, path, value in cases:
            with self.subTest(name=name, path=path):
                data = json.loads(payload(name))
                target = data['events'][-1]['after']
                for key in path[:-1]: target = target[key]
                target[path[-1]] = value
                with self.assertRaises(ValueError): check(data)
        data = json.loads(payload('zero-entitlement'))
        data['events'][-1]['physical_calls_after'] = 13
        with self.assertRaises(ValueError): check(data)
        data = json.loads(payload('zero-entitlement'))
        ledger = data['events'][-1]['after']['ledger']
        receipt = copy.deepcopy(ledger['settled_receipts'][-1])
        receipt['request'] = '13'
        ledger['settled_receipts'].append(receipt)
        for key in ['requests', 'settled_requests', 'quoted']: ledger[key] = '13'
        with self.assertRaisesRegex(ValueError, 'periodic second lease'): check(data)
        data = json.loads(payload('zero-entitlement'))
        data['schema'] = 'synarc.native-periodic-routing.v0'
        with self.assertRaises(ValueError): check(data)


if __name__ == '__main__': unittest.main()
