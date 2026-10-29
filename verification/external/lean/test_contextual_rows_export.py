"""Native declared-selector replay, sparse support soundness, and strict controls."""
import copy
import gzip
import hashlib
import json
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run
from check_contextual_rows_export import emit
from observer_export_replay import declarations, unique_object

FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'
FIXTURE = ROOT / 'fixtures/contextual-rows-native-v1.json.gz'


def fixture():
    raw = gzip.decompress(FIXTURE.read_bytes())
    provenance = json.loads(FIXTURE.with_name('contextual-rows-native-v1.provenance.json').read_text())
    assert hashlib.sha256(raw).hexdigest() == provenance['export_sha256']
    assert hashlib.sha256(FIXTURE.read_bytes()).hexdigest() == provenance['compressed_sha256']
    return json.loads(raw, object_pairs_hook=unique_object)


def prefix():
    """A smaller exact prefix of the archived native trace for rejection controls."""
    data = fixture()
    data['cases'] = data['cases'][:1]
    case = data['cases'][0]
    case['rows'] = case['rows'][:2]
    end = case['rows'][-1]['steps'][-1]['after']
    case['final_score'] = {k: end[k] for k in ('bits','samples','forecast_law','ratio')}
    case['final_score']['bank'] = end['staged_bank']
    return data


class ContextualRowsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.ContextualRows'))

    def prove(self, source, false=False):
        with tempfile.TemporaryDirectory(prefix='verum-contextual-rows-') as directory:
            path = Path(directory) / 'check.lean'; path.write_text(source)
            result = check(path, FALSE if false else None)
        if false:
            errors = [d['data'] for d in result['diagnostics'] if d['severity'] == 'error']
            self.assertEqual(errors, [FALSE])
        return result

    def reject_claim(self, data, name):
        source, _ = emit(data)
        claims = [b for b in source.split('\n\n') if b.startswith('theorem ' + name + ' :')]
        self.assertEqual(len(claims),1)
        self.prove(declarations(source)+'\nnamespace FrozenObserverExport\n'+claims[0]+
                   '\nend FrozenObserverExport\n',True)

    def test_axiom_roster_and_sparse_soundness(self):
        names = []
        source = (ROOT/'VerumExternalReplay/ContextualRows.lean').read_text()
        for namespace, body in re.findall(r'namespace (\w+)\n(.*?)\nend \1',source,re.S):
            names += [namespace+'.'+n for n in re.findall(r'^theorem (\w+)',body,re.M)]
        names += ['ContextualRows.attemptAgrees','ContextualRows.candidate','ContextualRows.reference']
        result = self.prove('''import VerumExternalReplay.ContextualRows
set_option warningAsError true
open SparseCounts
example : agrees [(7,true),(7,true)] [⟨7,0,2⟩] = true := by decide_cbv
example : agrees [(7,true),(7,true)] [⟨7,0,1⟩] = false := by decide_cbv
example : agrees [(7,true)] [] = false := by decide_cbv
example : agrees ([] : List (Nat × Bool)) [⟨99,1,0⟩] = false := by decide_cbv
example (events : List (Nat × Bool)) (cells : List (Cell Nat))
    (checked : agrees events cells = true) (key : Nat) (bit : Bool) :
    decode cells key bit = FrozenTraining.collect events (fun _ _ => 0) key bit :=
  agrees_exact events cells checked key bit
'''+ '\n'.join('#print axioms '+n for n in names))
        rows = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rows),len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms','depends on axioms: [propext]',
                                         'depends on axioms: [propext, Quot.sound]')),row)

    def test_frozen_native_prefix(self):
        source,counts = emit(prefix())
        self.assertEqual(counts['rows'],2)
        self.assertEqual(counts['attempts'],sum(len(r['steps']) for r in prefix()['cases'][0]['rows']))
        self.prove(source)

    def test_entire_native_packet_has_no_unchecked_attempts(self):
        source,counts = emit(fixture())
        self.assertEqual((counts['cases'],counts['rows'],counts['attempts']),(2,10,728))
        self.assertEqual(len(re.findall(r'^theorem ',source,re.M)),counts['claims'])
        self.assertEqual(counts['claims'],770)
        # Every complete native claim is replayed by the immutable CLI in CI.
        # This test checks the producer's exhaustive roster without a second run.

    def test_corrupted_native_observations_reach_false(self):
        base = prefix(); steps = base['cases'][0]['rows'][1]['steps']
        selected = next(i for i,s in enumerate(steps) if s['after']['pending'] and
            s['after']['pending']['cursor']['selected'] and
            s['after']['pending']['cursor']['prefix_bits_newest_first'][:2] == [0,1])
        mutations = {
            'free_progress': (0,lambda s:s['after']['pending']['cursor'].update(selecting=True)),
            'false_work_credit': (0,lambda s:s.update(paid=1)),
            'wrong_result_class': (0,lambda s:s.update(status='advanced')),
            'early_publication': (0,lambda s:s['after'].update(pending=None)),
            'counter_publication': (0,lambda s:s['after'].update(samples=2)),
            'frozen_contamination': (1,lambda s:s['after']['pending']['frozen_bank']['joint'][0]['counts'].__setitem__(1,2)),
            'raw_prefix_order': (selected,lambda s:s['after']['pending']['cursor']['prefix_bits_newest_first'].reverse()),
            'context_from_latest_bit': (selected,lambda s:s['after']['pending']['cursor']['selected'].update(context=[1,0])),
            'wrong_inversion': (selected,lambda s:s['after']['pending']['cursor']['selected'].update(invert=True)),
            'deduplicated_visits': (len(steps)-1,lambda s:s['after']['staged_bank']['joint'][-1]['counts'].__setitem__(0,1)),
            'wrong_reference_cell': (len(steps)-1,lambda s:s['after']['staged_bank']['reference'][0].__setitem__(0,1)),
            'wrong_score': (len(steps)-1,lambda s:s['after']['ratio'].update(numerator='4')),
        }
        for name,(index,mutate) in mutations.items():
            with self.subTest(name=name):
                data=copy.deepcopy(base); mutate(data['cases'][0]['rows'][1]['steps'][index])
                self.reject_claim(data,f'c0_r1_step{index}')
        data=copy.deepcopy(base); data['cases'][0]['rows'][0]['point']['n']='1'; data['cases'][0]['rows'][0]['point']['d']='1'
        self.reject_claim(data,'c0_r0_point')
        data=copy.deepcopy(base); data['cases'][0]['rows'][1]['steps'].pop()
        self.reject_claim(data,'c0_r1_complete')

    def test_schema_and_scope_rejections(self):
        mutations = {
            'implicit_totality': lambda d:d['cases'][0]['rows'][0]['forecast'].update(totality='verified'),
            'acquired_selector': lambda d:d['cases'][0]['selector'].update(declared_fixture='native_vm'),
            'boolean_grant': lambda d:d['cases'][0]['rows'][0]['steps'][0].update(grant=True),
            'extra_field': lambda d:d['cases'][0]['rows'][0]['steps'][0].update(ignored=1),
            'poison_ignored': lambda d:d['cases'][0]['rows'][0]['steps'][0]['after'].update(poisoned=True),
            'refusal_ignored': lambda d:d['cases'][0]['rows'][0]['steps'][0]['after'].update(invalid='CountOverflow'),
            'wrong_law': lambda d:d.update(law='other'),
            'non_byte': lambda d:d['cases'][0]['rows'][0].update(target=[256]),
            'zero_denominator': lambda d:d['cases'][0]['rows'][0]['point'].update(d='0'),
            'duplicate_cell': lambda d:d['cases'][0]['rows'][1]['forecast']['bank']['joint'].append(
                copy.deepcopy(d['cases'][0]['rows'][1]['forecast']['bank']['joint'][0])),
            'wrong_total': lambda d:d['cases'][0]['rows'][1]['forecast']['bank']['lengths'].update(total=2),
        }
        for name,mutate in mutations.items():
            with self.subTest(name=name):
                data=prefix();mutate(data)
                with self.assertRaises(ValueError):emit(data)


if __name__ == '__main__':
    unittest.main()
