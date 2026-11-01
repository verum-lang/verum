"""Exact closed-prefix diagnostics; finite checks never certify universal ML."""
import copy
import unittest
from check_closed_evidence_export import emit
from lean_test_fixture import IsolatedLeanTest
from observer_export_replay import batches
import test_prefix_export as prefix_tests


def fixture():return prefix_tests.frozen('closed-evidence-native-v1')


class ClosedEvidenceReplay(IsolatedLeanTest):
    MODULES=('ClosedEvidence',)

    def replay(self,data):
        source,counts=emit(data)
        self.assertFalse(counts['universal_ml_proof'])
        self.assertEqual(counts['prefixes'],len(data['ordered_rows'])+1)
        for part,names in batches(source):
            self.assert_axioms(self.prove(part),['FrozenObserverExport.'+n for n in names])

    def test_frozen_native_history_and_prices(self):
        self.replay(fixture())

    def test_zero_count_symbols_are_retained(self):
        data=fixture();context=data['context_histograms'][1]
        absent=copy.deepcopy(context['histogram'][0]);absent['count']=0;absent['outcome']['value']['key']=99
        context['histogram'].append(absent);context['alphabet_size']=2
        self.replay(data)

    def test_empty_history_and_single_symbol_likelihood_are_one(self):
        self.prove('''import VerumExternalReplay.ClosedEvidence
open ClosedEvidence FixedKTNative
set_option warningAsError true
def singleton : List Declaration := [⟨[0], ["x"]⟩]
example : prefixLowerBound singleton [] 1 0 = true := by decide_cbv
example : prefixLowerBound singleton [⟨[0],"x",7,7⟩,⟨[0],"x",8,8⟩] 1 2 = true := by decide_cbv
example : histogram [⟨[0],[]⟩] [] = none := by decide_cbv
example : histogram [⟨[0],["x","x"]⟩] [] = none := by decide_cbv
example : histogram [⟨[0],["x"]⟩,⟨[0],["y"]⟩] [] = none := by decide_cbv
example : ingestOutcome singleton [[0]] [0] "absent" = none := by decide_cbv
''')

    def test_corrupted_frozen_evidence_fails_in_the_kernel(self):
        for change in ('count','row','alphabet','risk','threshold','minimum','boundary','reordered'):
            data=fixture()
            if change=='count':data['context_histograms'][0]['histogram'][0]['count']+=1
            elif change=='row':data['ordered_rows'][0]['outcome']['value']['key']=0
            elif change=='alphabet':data['maximum_alphabet']=1
            elif change=='risk':data['risk_denominator']='1'
            elif change=='threshold':data['threshold_denominator']='10485760' # actual charge cannot replace header bound
            elif change=='minimum':data['minimum_eligible_rows']=21
            elif change=='boundary':data['boundary']['context']=[1,0,0]
            else:data['ordered_rows'].reverse()
            source,_=emit(data)
            match={'count':'histogram declarations observations = some capturedCounts',
                'row':'histogram declarations observations = some capturedCounts',
                'alphabet':'.foldl max 0','risk':'ClosedEvidence.headerBits',
                'threshold':'ClosedEvidence.headerBits','minimum':'(2621440 : Nat) < 2^21',
                'boundary':'declarations.all (fun d => d.context !=',
                'reordered':'observations.zip (observations.drop 1)'}[change]
            with self.subTest(change=change):
                selected=[part for part,names in batches(source,size=1)
                          if any(line.startswith('theorem ') and match in line for line in part.splitlines())]
                self.assertEqual(len(selected),1,match)
                self.prove(selected[0],failure=True,
                           expected_error=('Tactic `decide` proved that the proposition'
                                           if change=='minimum' else 'Tactic `rfl` failed:'))


class ClosedEvidenceIntake(unittest.TestCase):
    def test_embedded_source_does_not_weaken_standalone_schema(self):
        from check_prefix_export import emit as emit_prefix
        source=fixture()['source']
        self.assertNotIn('library_origin',source)
        with self.assertRaises(ValueError):emit_prefix(source,source['language_versions'])

    def test_missing_source_or_ambiguous_boundary_is_refused(self):
        changes=[lambda d:d['boundary'].update(same_native_owner=False),
                 lambda d:d.update(profile='Unverified'),
                 lambda d:d['context_histograms'][0].update(alphabet_size=3),
                 lambda d:d.update(look_index='1'),
                 lambda d:d.update(retained_rows=100000),
                 lambda d:d.update(comparison='K^N < threshold')]
        for change in changes:
            data=fixture();change(data)
            with self.assertRaises(ValueError):emit(data)


if __name__=='__main__':unittest.main()
