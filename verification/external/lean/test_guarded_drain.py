"""Checked progress guards and finite native rank embeddings, without native claims."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.GuardedDrain
set_option warningAsError true
open WorkDrain GuardedDrain
def stalled (s work : Nat) : Observation Nat Nat Nat := ⟨s+1,work,.pending,[s]⟩
def yielding := machine (fun _ => 0) stalled (fun _ _ => Nat.le_refl _)
def handoff (s work : Nat) : Observation Nat Nat Nat :=
  if s = 0 then ⟨s,work,.ready,[0]⟩ else ⟨s-1,work,.pending,[s]⟩
def free := machine id handoff (by intro s work; simp only [handoff]; split <;> exact Nat.le_refl _)
def growing (s work : Nat) : Observation Nat Nat Nat := ⟨s+1,work+1,.pending,[s]⟩
def terminal (status : Status Nat) (s work : Nat) : Observation Nat Nat Nat :=
  ⟨s+1,work+1,status,[s]⟩
def before : Rank (2^64) 7 (2^64) := ⟨⟨8,by decide⟩,⟨3,by decide⟩,⟨4,by decide⟩⟩
def nextRow : Rank (2^64) 7 (2^64) := ⟨⟨9,by decide⟩,⟨6,by decide⟩,⟨2^64-1,by decide⟩⟩
def nextLevel : Rank (2^64) 7 (2^64) := ⟨⟨8,by decide⟩,⟨2,by decide⟩,⟨2^64-1,by decide⟩⟩
def nextPhase : Rank (2^64) 7 (2^64) := ⟨⟨8,by decide⟩,⟨3,by decide⟩,⟨3,by decide⟩⟩
def previousRow : Rank (2^64) 7 (2^64) := ⟨⟨7,by decide⟩,⟨0,by decide⟩,⟨0,by decide⟩⟩
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class GuardedDrainTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.GuardedDrain'))

    def prove(self, body, failure=False):
        with tempfile.TemporaryDirectory(prefix='verum-guarded-drain-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + body)
            result = check(path, FALSE if failure else None)
            if failure:
                errors = [d for d in result['diagnostics'] if d['severity'] == 'error']
                self.assertEqual(len(errors), 1, errors)
                self.assertTrue(errors[0]['data'].startswith(FALSE), errors)
            return result

    def test_checked_callback_derives_total_driver(self):
        self.prove('''
example : drain yielding 7 5 = ⟨8,5,.yielded,[7]⟩ := by decide_cbv
example : drain free 3 1 = ⟨0,1,.ready,[3,2,1,0]⟩ := by decide_cbv
example : drain free 3 0 = ⟨3,0,.pending,[]⟩ := by decide_cbv
example : checked id growing 3 5 = ⟨4,6,.refused .workIncreased,[3]⟩ := by decide_cbv
example : classify (E := Nat) 5 0 4 100 = fun
    | .pending => .pending | .yielded => .yielded | .ready => .ready
    | .refused e => .refused (.callback e) := by
  funext s; cases s <;> simp [classify]
''')

    def test_callback_result_precedes_progress_guard(self):
        self.prove('''
example : checked id (terminal .ready) 3 5 = ⟨4,6,.ready,[3]⟩ := by decide_cbv
example : checked id (terminal (.refused 17)) 3 5 = ⟨4,6,.refused (.callback 17),[3]⟩ := by decide_cbv
example : checked id (terminal .yielded) 3 5 = ⟨4,6,.yielded,[3]⟩ := by decide_cbv
example : classify (E := Nat) 5 3 5 2 .pending = .pending := by decide_cbv
example : classify (E := Nat) 5 3 5 3 .pending = .yielded := by decide_cbv
example : classify (E := Nat) 5 3 5 4 .pending = .yielded := by decide_cbv
''')

    def test_rank_embeds_bounded_native_order_without_overflow(self):
        self.prove('''
example : nextRow.value < before.value := by decide_cbv
example : nextLevel.value < before.value := by decide_cbv
example : nextPhase.value < before.value := by decide_cbv
example : before.value < previousRow.value := by decide_cbv
example : nextRow.value > 2^64 := by decide_cbv
example : ¬ nextRow.value = nextLevel.value := by decide_cbv
example (a b : Rank ordinals levels phases) (same : a.value = b.value) : a = b :=
  (Rank.value_injective a b).mp same
''')

    def test_strict_corruptions_rejected(self):
        claims = {
            'stutter_keeps_spinning': '(drain yielding 7 5).status = .pending',
            'guard_rolls_back_state': '(drain yielding 7 5).state = 7',
            'guard_discards_effect': '(drain yielding 7 5).events = []',
            'guard_invents_charge': '(drain yielding 7 5).remaining = 4',
            'free_handoff_is_charged': '(drain free 3 1).remaining = 0',
            'ready_overwritten': '(checked id (terminal .ready) 3 5).status = .refused .workIncreased',
            'refusal_overwritten': '(checked id (terminal (.refused 17)) 3 5).status = .refused .workIncreased',
            'bad_meter_clipped': '(checked id growing 3 5).remaining = 5',
            'increasing_meter_continues': '(checked id growing 3 5).status = .pending',
            'rank_may_reset_without_payment': 'classify (E := Nat) 5 3 5 4 .pending = .pending',
            'paid_rank_may_not_reset': 'classify (E := Nat) 5 3 4 100 .pending = .yielded',
            'ordinal_not_reversed': 'before.value < nextRow.value',
            'phase_overrides_ordinal': 'nextRow.value > nextPhase.value',
            'native_multiplication_wrap': 'nextRow.value < 2^64',
            'rank_collides': 'nextRow.value = nextLevel.value',
        }
        for name, claim in claims.items():
            with self.subTest(name=name):
                self.prove('example : ' + claim + ' := by decide_cbv', True)

    def test_constructive_axiom_roster(self):
        source = (ROOT / 'VerumExternalReplay/GuardedDrain.lean').read_text()
        names = re.findall(r'^theorem ([\w.]+)', source, re.M) + ['classify','checked','machine','Rank.value']
        result = self.prove('\n'.join('#print axioms GuardedDrain.' + n for n in names))
        rows = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rows), len(names))
        for row in rows:
            if 'depends on axioms: [' in row:
                axioms = row.split('depends on axioms: [')[1].removesuffix(']').split(', ')
                self.assertLessEqual(set(axioms), {'propext','Quot.sound'}, row)
            else:
                self.assertTrue(row.endswith('does not depend on any axioms'), row)


if __name__ == '__main__':
    unittest.main()
