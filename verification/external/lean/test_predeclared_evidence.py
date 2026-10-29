"""Exact reservation/risk/owned-buffer laws and semantic false controls."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.PredeclaredEvidence
open PredeclaredEvidence PredeclaredEvidence.Exact ExactMass EnumerableMass
set_option warningAsError true
deriving instance DecidableEq for Except
def declaration : Declaration Nat Nat Nat Nat := ⟨7, 0, 10, 2, 3, 4⟩
def initial : OwnedBuffer Nat Nat Nat Nat Bool := ((5, declaration), [])
def buffered : OwnedBuffer Nat Nat Nat Nat Bool := appendBatch initial [(11, true)]
'''


class PredeclaredEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.PredeclaredEvidence'))

    def test_universal_axioms_and_exact_transition(self):
        source = (ROOT/'VerumExternalReplay/PredeclaredEvidence.lean').read_text()
        core, extended = source.split('namespace Exact')
        names = ['PredeclaredEvidence.'+n for n in re.findall(r'^theorem (\w+)', core, re.M)]
        names += ['PredeclaredEvidence.Exact.'+n for n in re.findall(r'^theorem (\w+)', extended, re.M)]
        with tempfile.TemporaryDirectory(prefix='verum-reservation-laws-') as directory:
            path = Path(directory)/'audit.lean'
            path.write_text(HEADER+'''
example : receive 2 initial 7 11 true = .ok buffered := by decide
example : receive 2 buffered 7 12 false = .ok (appendBatch buffered [(12, false)]) := by decide
example : receive 2 initial 8 11 true = .error initial := by decide
example : receive 0 initial 7 11 true = .error initial := by decide
example : receive 2 initial 7 10 true = .error initial := by decide
example : receive 2 buffered 7 11 false = .error buffered := by decide
example : Equal (partialSum (fun j => if j == 1 || j == 3 then lookWeight j else zero) 4)
  ⟨13, 60, by decide⟩ := by decide_cbv
example : (complete 2 (reserve 0 true).2 (some 3)).2 = some ((0, true), 3) := rfl
example : (complete 2 (reserve 1 false).2 (some 4)).2 = some ((1, false), 4) := rfl
example : (beginValidation (R := Bool) 9 13).2 = (13, []) := rfl
example : receiveValidation 2 12 initial 7 12 true = .ok (appendBatch initial [(12, true)]) := by decide
example : receiveValidation 2 12 initial 7 11 true = .error initial := by decide
example : receiveValidation 2 12 initial 8 12 true = .error initial := by decide
example : receiveValidation 0 12 initial 7 12 true = .error initial := by decide
'''+ '\n'.join('#print axioms '+name for name in names))
            result = check(path)
        rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rosters), len(names))
        for row in rosters:
            allowed = ('depends on axioms: [propext, Quot.sound]',) if any(name in row for name in
                ('skipped_reservations_bounded', 'risk_prefix_balance')) else (
                'does not depend on any axioms', 'depends on axioms: [propext]')
            self.assertTrue(row.endswith(allowed), row)

    def test_false_claims_rejected_by_kernel(self):
        claims = {
            'refusal_refunds_ordinal': '(complete (reserve 0 true).1 (reserve 0 true).2 (none : Option Nat)).1 = 0',
            'completion_reprices_ordinal': '(complete 2 (0, true) (some 7)).2 = some ((2, true), 7)',
            'replacement_declaration': '(reserve 0 true).2.2 = false',
            'buffered_rows_erased': '(appendBatch buffered [(12, false)]).2 = [(12, false)]',
            'buffer_order_reversed': '(appendBatch buffered [(12, false)]).2 = [(12, false), (11, true)]',
            'refusal_changes_owner': 'receive 2 initial 8 11 true = .error ((5, { declaration with source := 8 }), [])',
            'training_reused_as_validation': '(beginValidation (R := Bool) 9 13).2.2 = [true]',
            'old_rows_accepted': 'receive 2 buffered 7 10 false = .ok (appendBatch buffered [(10, false)])',
            'capacity_ignored': 'receive 1 buffered 7 12 false = .ok (appendBatch buffered [(12, false)])',
            'selection_boundary_exclusive': 'receiveValidation 2 12 initial 7 12 true = .error initial',
            'preselection_row_accepted': 'receiveValidation 2 12 initial 7 11 true = .ok (appendBatch initial [(11, true)])',
        }
        with tempfile.TemporaryDirectory(prefix='verum-reservation-controls-') as directory:
            for name, claim in claims.items():
                with self.subTest(name=name):
                    path = Path(directory)/(name+'.lean')
                    path.write_text(HEADER+f'example : {claim} := by decide\n')
                    result = check(path, 'Tactic `decide` proved that the proposition')
                    self.assertEqual(sum(d['severity'] == 'error' for d in result['diagnostics']), 1)
            # Exact arithmetic rejects repeated allocation and renormalization.
            for name, claim in {
                'reservation_reused': 'AtMost (add (add (lookWeight 0) (lookWeight 0)) (lookWeight 0)) one',
                'skipped_mass_refunded': 'Equal (partialSum (fun j => if j == 1 then lookWeight j else zero) 4) one',
            }.items():
                path = Path(directory)/(name+'.lean')
                path.write_text(HEADER+f'example : {claim} := by decide_cbv\n')
                check(path, '`decide_cbv` failed: the proposition evaluates to `false`')


if __name__ == '__main__': unittest.main()
