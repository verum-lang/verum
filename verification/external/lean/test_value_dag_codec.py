"""Graph extraction controls; synthetic graphs are not native execution evidence."""
import copy
import gzip
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from acquired_export_codec import LEGACY, SENSORY
from check_typed_programs import check, require_success, run

FIXTURES = Path(__file__).parent / 'fixtures'


def native_fixture(name):
    provenance = json.loads((FIXTURES / 'value-dag-native.provenance.json').read_text())
    raw = gzip.decompress((FIXTURES / f'value-dag-native-{name}.json.gz').read_bytes())
    if hashlib.sha256(raw).hexdigest() != provenance['cases'][name]['sha256']:
        raise ValueError('Frozen native value DAG hash mismatch')
    return json.loads(raw)


def graph(nodes):
    return dict(kind='ValueDag', schema='synarc.value-dag.v1', root=0, nodes=nodes)


def fixture():
    # Includes both forward references and a backreference to a shared leaf.
    return graph([dict(kind='Pair', first=1, second=2),
                  dict(kind='Pair', first=2, second=3),
                  dict(kind='Nat', value=5), dict(kind='Unit')])


class ValueDagCodec(unittest.TestCase):
    def test_frozen_native_wires_preserve_every_node_without_expansion(self):
        for name, count in [('shared-ordered-pair', 4), ('shared-depth-10000', 10001)]:
            value = native_fixture(name)
            self.assertEqual(len(value['nodes']), count)
            self.assertIsNone(LEGACY.validate_value(value))
            self.assertEqual(LEGACY.value(value).count('let _dag'), count)

    def test_shared_children_emit_once_in_dependency_order(self):
        text = LEGACY.value(fixture())
        self.assertEqual(text.count('let _dag'), 4)
        self.assertEqual(text.count('(.nat 5)'), 1)
        self.assertIn('(.pair _dag1 _dag2)', text)
        self.assertIn('(.pair _dag2 _dag3)', text)
        self.assertLess(text.index('let _dag2'), text.index('let _dag1'))
        self.assertLess(text.index('let _dag1'), text.index('let _dag0'))
        self.assertEqual(LEGACY.value(json.loads(json.dumps(fixture()))), text)

    def test_deep_exponentially_shared_value_has_linear_extraction(self):
        depth = 10_000
        nodes = [dict(kind='Pair', first=i + 1, second=i + 1) for i in range(depth)]
        nodes.append(dict(kind='Nat', value=17))
        text = LEGACY.value(graph(nodes))
        self.assertEqual(text.count('let _dag'), depth + 1)
        self.assertEqual(text.count('(.pair '), depth)
        self.assertEqual(text.count('(.nat 17)'), 1)
        self.assertLess(len(text), 100 * len(nodes))
        self.assertIsNone(LEGACY.validate_value(graph(nodes)))

    def test_historical_structure_validation_is_separate_from_term_emission(self):
        tree = dict(kind='Nat', value=5)
        for _ in range(10_000):
            tree = dict(kind='Pair', first=tree, second=tree)
        self.assertIsNone(LEGACY.validate_value(tree))
        with self.assertRaisesRegex(ValueError, 'depth limit'): LEGACY.value(tree)
        tree['second'] = tree
        with self.assertRaisesRegex(ValueError, 'Cyclic'): LEGACY.validate_value(tree)

    def test_root_schema_and_exact_fields_are_required(self):
        for key, value in [('kind', 'DAG'), ('schema', 'synarc.value-dag.v2'),
                           ('root', True), ('root', 1), ('root', '0'), ('root', -1),
                           ('root', 0.0), ('nodes', []), ('nodes', {}), ('extra', True)]:
            bad = fixture(); bad[key] = value
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                LEGACY.value_dag(bad)
        for key in fixture():
            bad = fixture(); del bad[key]
            with self.subTest(missing=key), self.assertRaises(ValueError):
                LEGACY.value_dag(bad)

    def test_invalid_references_cycles_unreachable_and_nested_nodes_are_refused(self):
        for ref in (True, False, 1.0, '1', -1, 4, 2**64, None, {'kind': 'Unit'}):
            bad = fixture(); bad['nodes'][0]['first'] = ref
            with self.subTest(ref=ref), self.assertRaises(ValueError): LEGACY.value(bad)
            with self.subTest(validation_ref=ref), self.assertRaises(ValueError): LEGACY.validate_value(bad)
        for node, child in ((0, 0), (1, 0), (1, 1)):
            bad = fixture(); bad['nodes'][node]['first'] = child
            with self.subTest(cycle=(node, child)), self.assertRaises(ValueError):
                LEGACY.value(bad)
        for unused in (dict(kind='Unit'), dict(kind='Pair', first=4, second=4)):
            bad = fixture(); bad['nodes'].append(unused)
            with self.subTest(unreachable=unused), self.assertRaises(ValueError): LEGACY.value(bad)
        for node in (fixture(), {}, None, dict(kind='Pair', first=1),
                     dict(kind='Pair', first=1, second=2, trusted=True),
                     dict(kind='Nat', value=5, trusted=True), dict(kind='Unknown', value=0)):
            bad = fixture(); bad['nodes'][1] = node
            with self.subTest(node=node), self.assertRaises(ValueError): LEGACY.value(bad)

    def test_leaf_codec_selection_and_historical_trees_remain_exact(self):
        leaves = [dict(kind='Unit'), dict(kind='Nat', value=2**64 - 1),
                  dict(kind='Bool', value=True), dict(kind='Action', value=255),
                  dict(kind='Trace', events=[]),
                  dict(kind='Outcome', value=dict(kind='End', terminal='Completed'))]
        for leaf in leaves:
            self.assertIn(LEGACY.value(leaf), LEGACY.value(graph([leaf])))
        byte = dict(kind='Bytes', value=[0, 255])
        with self.assertRaises(ValueError): LEGACY.value(graph([byte]))
        self.assertIn('(.bytes [0, 255])', SENSORY.value(graph([byte])))
        old = dict(kind='Pair', first=dict(kind='Nat', value=5), second=dict(kind='Unit'))
        self.assertEqual(LEGACY.value(old), '(.pair (.nat 5) .unit)')
        for bad in (True, -1, 2**64, 1.0, '5'):
            with self.subTest(nat=bad), self.assertRaises(ValueError):
                LEGACY.value(graph([dict(kind='Nat', value=bad)]))


class ValueDagLean(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.AcquiredObserver'))

    def replay(self, text, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-value-dag-') as directory:
            path = Path(directory) / 'Check.lean'
            path.write_text('import VerumExternalReplay.AcquiredObserver\n'
                            'open AcquiredObserver\nset_option warningAsError true\n' + text)
            return check(path, expected, memory_mib=1024)

    def test_exact_order_sharing_and_reference_renaming(self):
        original = native_fixture('shared-ordered-pair')
        # Reindex the same graph without preserving BFS numbering.
        reordered = graph([dict(kind='Pair', first=3, second=2), dict(kind='Unit'),
                           dict(kind='Nat', value=5), dict(kind='Pair', first=2, second=1)])
        duplicated = graph([dict(kind='Pair', first=1, second=4),
                            dict(kind='Pair', first=2, second=3), dict(kind='Nat', value=5),
                            dict(kind='Unit'), dict(kind='Nat', value=5)])
        self.assertNotEqual(original, duplicated)
        reference = '(.pair (.pair (.nat 5) .unit) (.nat 5))'
        result = self.replay(
            f'theorem exact_value : ({LEGACY.value(original)} : Value) = {reference} := by rfl\n'
            f'theorem reindexed : ({LEGACY.value(reordered)} : Value) = {LEGACY.value(original)} := by rfl\n'
            f'theorem allocation_independent : ({LEGACY.value(duplicated)} : Value) = {LEGACY.value(original)} := by rfl\n'
            '#print axioms exact_value\n#print axioms reindexed\n#print axioms allocation_independent\n')
        self.assertEqual([d['data'] for d in result['diagnostics']], [
            "'exact_value' does not depend on any axioms", "'reindexed' does not depend on any axioms",
            "'allocation_independent' does not depend on any axioms"])
        changed = copy.deepcopy(original)
        changed['nodes'][0]['first'], changed['nodes'][0]['second'] = 2, 1
        result = self.replay(
            f'example : ({LEGACY.value(changed)} : Value) = {reference} := by rfl\n',
            'Tactic `rfl` failed:')
        errors = [d for d in result['diagnostics'] if d['severity'] == 'error']
        self.assertEqual(len(errors), 1)


if __name__ == '__main__':
    unittest.main()
