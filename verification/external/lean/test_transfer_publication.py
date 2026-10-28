"""Original receiver commit/fork/read chain and mutations of its native receipts."""
from pathlib import Path
import tempfile
import unittest
import transfer_publication
from check_transfer_export import emit
from check_typed_programs import check
from test_transfer_export import original
import test_transfer_export as transfer_tests

class PublicationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        transfer_tests.TransferExportTests.setUpClass()
        source,_=emit(original(version=4))
        cls.prelude=[b for b in source.strip().split('\n\n')
                     if b.startswith(('import ','open ','namespace ','set_option ','def '))]

    def proof(self, data, error=None):
        claims=[]; names=[]
        def claim(equation, proof='rfl'):
            name='publication_'+str(len(names));names.append(name)
            claims.append(f'theorem {name} : {equation} := by\n  {proof}')
        d=data['recipient_definition']
        counts=transfer_publication.emit(data,d,'d'+str(d['identity']),str(d['semantics']['max_trace']),claim)
        source='\n\n'.join(self.prelude+claims+['#print axioms '+n for n in names]+['end FrozenObserverExport'])
        with tempfile.TemporaryDirectory() as directory:
            p=Path(directory)/'publication.lean';p.write_text(source);result=check(p,error)
        if error is None:
            rows=[r['data'] for r in result['diagnostics'] if r['severity']=='information']
            self.assertEqual(len(rows),len(names))
            self.assertTrue(all(r.endswith('does not depend on any axioms') or
                r.endswith('depends on axioms: [propext]') or
                r.endswith('depends on axioms: [propext, Quot.sound]') for r in rows), rows)
        return counts

    def test_original_actual_chain_and_reports(self):
        for phase in (0,3):
            data=original(phase,4)
            # Each phase has a different recipient lineage; retain its actual
            # declarations instead of borrowing the first phase's environment.
            source,all_counts=emit(data)
            self.prelude=[b for b in source.strip().split('\n\n')
                          if b.startswith(('import ','open ','namespace ','set_option ','def '))]
            self.assertEqual(all_counts['claims'],94)
            counts=self.proof(data)
            self.assertEqual(counts,dict(native_owner_transition_witnesses=2,
                publication_reads=1,publication_commits=1,publication_events=3))

    def test_missing_reordered_and_duplicated_events_rejected(self):
        edits=[lambda e:e.pop(0),lambda e:e.pop(1),lambda e:e.pop(2),
               lambda e:e.reverse(),lambda e:e.insert(2,e[1])]
        for edit in edits:
            data=original(version=4);edit(data['publication_events'])
            with self.subTest(edit=edits.index(edit)),self.assertRaises(ValueError):emit(data)

    def test_native_receipt_value_and_report_corruptions_rejected(self):
        edits=[
            (lambda e:e[0]['receipt']['before'].update(state=999),'Tactic `rfl` failed:'),
            (lambda e:e[0]['receipt']['after'].update(owner=999),'Tactic `rfl` failed:'),
            (lambda e:e[1]['after'].update(owner=e[1]['before']['owner']),'Tactic `decide` proved that the proposition'),
            (lambda e:e[1]['after'].update(state=999),'Tactic `rfl` failed:'),
            (lambda e:e[2].update(value={'kind':'Nat','value':999}),'Tactic `rfl` failed:'),
            (lambda e:e[2]['execution'].update(dispatch_attempts=2),'Tactic `rfl` failed:'),
            (lambda e:e[0]['execution'].update(expanded_attempts=2),'Tactic `rfl` failed:'),
            (lambda e:e[0]['input']['values'][0]['value'].update(input=999),'Tactic `rfl` failed:'),
        ]
        for edit,error in edits:
            data=original(version=4);edit(data['publication_events'])
            with self.subTest(edit=edits.index((edit,error))):self.proof(data,error)

    def test_foreign_payload_and_incomplete_report_refused(self):
        edits=[lambda e:e[2]['query']['values'][0]['value']['owner'].update(authority=999),
               lambda e:e[0]['execution'].update(stop='Budget')]
        for edit in edits:
            data=original(version=4);edit(data['publication_events'])
            with self.assertRaises(ValueError):emit(data)

if __name__=='__main__':unittest.main()
