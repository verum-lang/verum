"""Historical native support replay; semantic mutations must reach Lean rejection."""
import gzip
import hashlib
import json
from pathlib import Path
import re
import shutil
import tempfile
import unittest
from check_domain_support_export import emit
from check_typed_programs import ROOT, check, require_success, run

FALSE = 'Tactic `decide` proved that the proposition'

def fixture():
    raw = gzip.decompress((ROOT/'fixtures/domain-support-native.json.gz').read_bytes())
    provenance = json.loads((ROOT/'fixtures/domain-support-native.provenance.json').read_text())
    if hashlib.sha256(raw).hexdigest() != provenance['export_sha256']:
        raise ValueError('Native fixture identity changed')
    return json.loads(raw)

class DomainSupport(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix='verum-domain-support-')
        cls.root = Path(cls.temporary.name)
        for file in ['lakefile.toml', 'lean-toolchain', 'VerumExternalReplay/DomainSupport.lean']:
            target = cls.root/file; target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT/file, target)
        target = cls.root/'.lake/build/lib/lean/VerumExternalReplay/DomainSupport.olean'
        target.parent.mkdir(parents=True, exist_ok=True)
        require_success(run('lake','env','lean','-M','1024','-o',str(target),
            'VerumExternalReplay/DomainSupport.lean',cwd=cls.root,timeout=60))

    @classmethod
    def tearDownClass(cls): cls.temporary.cleanup()

    def replay(self, data, expected=None):
        text, counts = emit(data)
        path = self.root/'claims.lean'; path.write_text(text)
        result = check(path,expected,cwd=self.root,memory_mib=1024)
        return result, counts

    def test_frozen_native_steps_costs_refusals_and_cache_lifetimes(self):
        result, counts = self.replay(fixture())
        self.assertTrue(result['accepted'])
        self.assertEqual(counts['events'],185)

    def test_universal_laws_and_exact_full_width_support(self):
        source = (self.root/'VerumExternalReplay/DomainSupport.lean').read_text()
        names = re.findall(r'^theorem (\w+)',source,re.M)
        self.assertEqual(names,['support_extensional','intersection','scan_append','reuse','zero_work'])
        path=self.root/'audit.lean'
        path.write_text('import VerumExternalReplay.DomainSupport\nopen DomainSupport\n'+
            '\n'.join(f'#print axioms DomainSupport.{name}' for name in names)+'''
example : mask [9223372036854775808, 0, 18446744073709551615] 9223372036854775808 = 5 := by decide
example : mask [3, 0, 2, 1] 0 = 0 := by decide
example : mask [3, 0, 2, 1] 18446744073709551615 = 13 := by decide
example : mask (List.replicate 64 1) 1 = 18446744073709551615 := by decide
''')
        result=check(path,cwd=self.root,memory_mib=1024)
        self.assertEqual(len(result['diagnostics']),len(names))
        for item in result['diagnostics']:
            text=item['data']
            self.assertTrue(text.endswith('does not depend on any axioms') or
                            text.endswith('depends on axioms: [propext]'),text)
        # An output may witness two different domain bits. Domain intersection
        # is not the intersection of separately computed case supports.
        path.write_text('''import VerumExternalReplay.DomainSupport
open DomainSupport
example : mask [3] (1 &&& 2) = (mask [3] 1 &&& mask [3] 2) := by decide
''')
        check(path,FALSE,cwd=self.root,memory_mib=1024)

    def test_semantic_mutations_reach_kernel_rejection(self):
        for mutation in ('scan','domain','hit','work_state','capacity_charge','capacity_room',
                         'release','resume','owner','cost_total','control_mask','foreign_owner'):
            data=fixture(); stream=data['streams'][0]; rows=stream['events']
            def event(kind,result='advanced'):
                return next(r for r in rows if r['kind']==kind and r['result']==result)
            if mutation=='scan': event('scan')['after']['mask'] ^= 1
            elif mutation=='domain': event('domain')['after']['domain'] = 2
            elif mutation=='hit': event('hit')['after']['mask'] ^= 1
            elif mutation=='work_state': event('domain','work_refusal')['after']['domain'] = 1
            elif mutation=='capacity_charge': event('cache_publish','capacity_refusal')['charged'] = 0
            elif mutation=='capacity_room':
                row=event('cache_publish'); row['capacity']=row['cells_before']
            elif mutation=='release': event('release')['cells_after'] += 1
            elif mutation=='resume': event('cache_publish')['resumed'] = False
            elif mutation=='owner': rows[0]['before']['language_owner'] = data['streams'][1]['language_owner']
            elif mutation=='cost_total': stream['support_charged_total'] += 1
            elif mutation=='control_mask': data['controls'][1]['first_mask'] ^= 1
            elif mutation=='foreign_owner':
                row=data['controls'][0]; row['attempted_language_owner']=row['source_language_owner']
            with self.subTest(mutation=mutation): self.replay(data,FALSE)

    def test_malformed_or_truncated_exports_are_refused(self):
        mutations=[lambda d:d.update(extra=True),
            lambda d:d['streams'][0]['events'].pop(),
            lambda d:d['streams'][0]['events'][0]['before'].update(case=True),
            lambda d:d['streams'][0]['events'][0]['before'].update(anchor=1),
            lambda d:d['streams'][0]['events'][0]['before'].update(outputs=[1]),
            lambda d:d['streams'][0]['events'][0]['before'].update(phase=9),
            lambda d:d['streams'][0]['events'][0].update(grant=-1),
            lambda d:d['streams'][0]['events'][0].update(cells_before=2**64),
            lambda d:d['streams'][0]['events'][0]['before'].update(child_owner='not-an-address'),
            lambda d:d['streams'][1].update(language_owner=d['streams'][0]['language_owner'])]
        for i,mutate in enumerate(mutations):
            data=fixture(); mutate(data)
            with self.subTest(i=i), self.assertRaises(ValueError): emit(data)

if __name__ == '__main__': unittest.main()
