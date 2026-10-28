"""Original frozen transfers and causal corruptions; no simulated producer result."""
import gzip
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from check_transfer_export import emit
from observer_export_replay import ROOT
from check_typed_programs import check, require_success, run

FIXTURES=ROOT/'fixtures'

def original(phase=0):
    raw=gzip.decompress((FIXTURES/f'recipient-transfer-v2-{phase}.json.gz').read_bytes())
    provenance=json.loads((FIXTURES/f'recipient-transfer-v2-{phase}.provenance.json').read_text())
    if hashlib.sha256(raw).hexdigest()!=provenance['export_sha256']:raise AssertionError('Changed original fixture')
    return json.loads(raw)

class TransferExportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.CategoricalScore','VerumExternalReplay.AcquiredObserver',
                            'VerumExternalReplay.ObserverInstallation','VerumExternalReplay.ObserverOwnership','VerumExternalReplay.ExactNames'))

    def test_originals_account_for_all_rows(self):
        for phase in (0,3):
            data=original(phase);_,counts=emit(data)
            self.assertEqual(counts['reading_replays'],29)
            self.assertEqual(counts['exact_scores'],3)
            self.assertEqual(counts['history_events'],36)
            self.assertFalse(data['validation']['purchased']);self.assertTrue(data['confirmed'])

    def test_original_installation_snapshot_mismatch_is_rejected(self):
        # Keep the entire original proof: every earlier obligation must pass,
        # and the retained snapshot inconsistency must remain an explicit failure.
        for phase in (0,3):
            source,counts=emit(original(phase))
            with tempfile.TemporaryDirectory() as d:
                p=Path(d)/'original_mismatch.lean';p.write_text(source)
                result=check(p,'Tactic `rfl` failed:')
            errors=[x for x in result['diagnostics'] if x['severity']=='error']
            self.assertEqual(len(errors),1)
            rosters=[x['data'] for x in result['diagnostics'] if x['severity']=='information']
            self.assertEqual(len(rosters),counts['claims'])
            for roster in rosters[:-1]:
                self.assertTrue(roster.endswith('does not depend on any axioms') or
                    roster.endswith('depends on axioms: [propext]') or
                    roster.endswith('depends on axioms: [propext, Quot.sound]'),roster)
            self.assertIn('lifeAge := 35', errors[0]['data'])
            self.assertIn('lifeAge := 36', errors[0]['data'])

    def test_extraction_refuses_changed_assumptions(self):
        edits=[
            lambda d:d.update(schema='synarc-recipient-code-transfer-v1'),
            lambda d:d['validation']['history']['origin']['lineage'].update(authority=999),
            lambda d:d['validation']['history']['events'][0]['values'][0]['value'].update(input=999),
            lambda d:d['validation']['rows'][0].update(cut=20),
            lambda d:d['training']['rows'][0]['query']['interface']['owner'].update(authority=999),
            lambda d:d['candidate'].update(syntax_indices=[True,False,False]),
            lambda d:d['candidate'].update(syntax_indices=[0,0,1]),
            lambda d:d['recipient_definition']['read_binding'].update(slots=[{'External':0},'State']),
            lambda d:d['training']['alphabets'][0]['symbols'].pop(),
            lambda d:d['validation']['profile'].update(risk_denominator='21'),
            lambda d:d.update(confirmation_rule='fresh_purchased'),
        ]
        for edit in edits:
            data=original();edit(data)
            with self.subTest(edit=edits.index(edit)),self.assertRaises(ValueError):emit(data)

    def test_kernel_refuses_wrong_reading_score_decision_and_state(self):
        mutations=[
            (lambda d:d['training']['rows'][0].update(reading={'kind':'Nat','value':123456789}), 'some (.nat 123456789)', 'Tactic `rfl` failed:'),
            (lambda d:d['joint_exact_score']['predictive'].update(numerator='1'), 'CategoricalScore.agrees joint_contexts', 'Tactic `decide` proved that the proposition'),
            (lambda d:d['validation'].update(purchased=True), 'CategoricalScore.purchased fresh_score', 'Tactic `decide` proved that the proposition'),
            (lambda d:d.update(confirmed=False), 'CategoricalScore.confirmed training_score', 'Tactic `decide` proved that the proposition'),
            (lambda d:d['installed_cut'].update(value={'kind':'Nat','value':123456789}), '= some (.nat 123456789)', 'Tactic `rfl` failed:'),
        ]
        with tempfile.TemporaryDirectory() as directory:
            for i,(edit,target,error) in enumerate(mutations):
                data=original();edit(data);source,_=emit(data)
                # Keep exactly the relevant closed claim and every declaration.
                blocks=source.strip().split('\n\n')
                claim=next(b for b in blocks if b.startswith('theorem ') and target in b)
                prelude=[b for b in blocks if b.startswith(('import ','open ','namespace ','set_option ','def '))]
                p=Path(directory)/f'mutation{i}.lean';p.write_text('\n\n'.join(prelude+[claim,'end FrozenObserverExport']))
                with self.subTest(target=target):check(p,error)

if __name__=='__main__':unittest.main()
