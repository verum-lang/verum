"""Exact callback composition and paid outer visits, independent of native exports."""
import re
import unittest
from lean_test_fixture import IsolatedLeanTest

HEADER = '''import VerumExternalReplay.WorkDrain
set_option warningAsError true
open WorkDrain
structure Job where
  left : Nat
  phase : Nat
  failed : Bool := false
  deriving DecidableEq, Repr
def advance (s : Job) (work : Nat) : Observation Job Nat Nat :=
  if s.phase = 0 then
    if s.left = 0 then ⟨s,work,.ready,[9]⟩
    else ⟨{s with phase := 2},work-1,.pending,[10+s.left]⟩
  else if s.phase = 2 then ⟨{s with phase := 1},work,.pending,[20+s.left]⟩
  else if s.phase = 1 then
    if s.failed then ⟨{s with phase := 3},work,.refused 7,[7]⟩
    else ⟨{s with left := s.left-1,phase := 0},work,.pending,[30+s.left]⟩
  else ⟨s,work,.refused 8,[8]⟩
def machine : Machine Job Nat Nat where
  step := advance
  rank := Job.phase
  bounded s work := by
    by_cases p0 : s.phase = 0 <;> by_cases p2 : s.phase = 2 <;> by_cases p1 : s.phase = 1 <;>
      by_cases empty : s.left = 0 <;> cases failed : s.failed <;> simp_all [advance]
  progress s work positive pending sameWork := by
    by_cases p0 : s.phase = 0 <;> by_cases p2 : s.phase = 2 <;> by_cases p1 : s.phase = 1 <;>
      by_cases empty : s.left = 0 <;> cases failed : s.failed <;> simp_all [advance] <;> omega
def start : Job := ⟨2,0,false⟩
def whole := drain machine start 3
def first := drain machine start 1
def second := drain machine first.state 1
def third := drain machine second.state 1
def broken := drain machine ⟨2,0,true⟩ 3
def yielding : Machine Nat Unit Nat where
  step s work := ⟨s+1,work,.yielded,[s]⟩
  rank _ := 0
  bounded _ _ := Nat.le_refl _
  progress _ _ _ pending _ := by cases pending
'''
class WorkDrainTests(IsolatedLeanTest):
    MODULES = ('WorkDrain',)

    def prove(self, body, failure=False):
        return super().prove(HEADER + body, failure=failure)

    def test_handoffs_and_slicing_preserve_full_result(self):
        self.prove('''
example : whole = ⟨⟨0,0,false⟩,1,.ready,[12,22,32,11,21,31,9]⟩ := by decide_cbv
example : first = ⟨⟨2,2,false⟩,0,.pending,[12]⟩ := by decide_cbv
example : second = ⟨⟨1,2,false⟩,0,.pending,[22,32,11]⟩ := by decide_cbv
example : third.state = whole.state ∧ third.remaining = whole.remaining ∧
    third.status = whole.status ∧ first.events ++ second.events ++ third.events = whole.events := by decide_cbv
example : (drain machine start 2).status = .pending := by decide_cbv
example : (drain machine start 2).state = ⟨1,2,false⟩ := by decide_cbv
example : (drain machine ⟨0,0,false⟩ 0).events = [] := by decide_cbv
''')

    def test_refusal_retains_changed_state_and_all_prior_effects(self):
        self.prove('''
example : broken = ⟨⟨2,3,true⟩,2,.refused 7,[12,22,7]⟩ := by decide_cbv
example : (drain machine broken.state 1) = ⟨broken.state,1,.refused 8,[8]⟩ := by decide_cbv
example : (drain machine broken.state 0) = ⟨broken.state,0,.pending,[]⟩ := by decide_cbv
example : drain yielding 4 9 = ⟨5,9,.yielded,[4]⟩ := by decide_cbv
example : Execution machine start 3 whole := drain_execution _ _ _
example : ¬ Execution machine start 3 first := by
  rw [execution_iff]
  decide_cbv
''')

    def test_zero_cost_cycles_cannot_be_certified(self):
        self.prove('''
example : ¬ ∃ m : Machine Unit Unit Unit,
    ∀ s work, m.step s work = ⟨s,work,.pending,[]⟩ := by
  rintro ⟨m,stuck⟩
  have no := no_pending_stutter m () 1 (by decide)
    (congrArg Observation.remaining (stuck () 1))
    (congrArg Observation.state (stuck () 1))
  exact no (congrArg Observation.status (stuck () 1))
example : ¬ ∃ m : Machine Bool Unit Unit,
    ∀ s work, m.step s work = ⟨!s,work,.pending,[]⟩ := by
  rintro ⟨m,cycle⟩
  have down (s : Bool) : m.rank (!s) < m.rank s := by
    have h := m.progress s 1 (by decide)
      (congrArg Observation.status (cycle s 1))
      (congrArg Observation.remaining (cycle s 1))
    simpa only [cycle] using h
  have a := down true
  have b := down false
  simp only [Bool.not_true,Bool.not_false] at a b
  omega
''')

    def test_false_driver_claims_rejected(self):
        claims = {
            'early_stop': '(drain machine start 3).state = first.state',
            'hidden_outer_charge': '(drain machine start 3).remaining = 0',
            'erased_handoffs': 'whole.events = [12,11,9]',
            'reordered_effects': 'whole.events = [12,32,22,11,21,31,9]',
            'zero_grant_calls_service': '(drain machine ⟨0,0,false⟩ 0).status = .ready',
            'exhaustion_is_success': '(drain machine start 2).status = .ready',
            'unpaid_handoff_at_zero': '(drain machine start 2).state = ⟨0,0,false⟩',
            'refusal_is_success': 'broken.status = .ready',
            'refusal_state_rolled_back': 'broken.state = start',
            'refusal_effect_erased': 'broken.events = [12,22]',
            'paid_work_refunded': 'broken.remaining = 3',
            'sliced_handoffs_rebilled': 'third.remaining = 0',
            'yield_is_success': '(drain yielding 4 9).status = .ready',
            'yield_spends_work': '(drain yielding 4 9).remaining = 8',
        }
        for name, claim in claims.items():
            with self.subTest(name=name):
                self.prove('example : ' + claim + ' := by decide_cbv', True)

    def test_constructive_axiom_roster(self):
        names = re.findall(r'^theorem ([\w.]+)', self.sources['WorkDrain'], re.M)
        names += ['Machine', 'drain', 'Execution', 'paidVisit', 'PaidVisits']
        names = ['WorkDrain.' + n for n in names] + ['machine']
        self.assertEqual(len(names), 31)
        self.assert_axioms(self.prove('\n'.join('#print axioms ' + n for n in names)), names)

    def test_paid_visit_exact_cost_and_unsigned_boundaries(self):
        self.prove('''
example : paidVisit 9 9 = 8 := by rfl
example : paidVisit 9 8 = 8 := by rfl
example : paidVisit 9 4 = 4 := by rfl
example : paidVisit 9 0 = 0 := by rfl
example : paidVisit 1 1 = 0 := by rfl
example : paidVisit 1 0 = 0 := by rfl
example : paidVisit 18446744073709551615 18446744073709551615 = 18446744073709551614 := by rfl
example : 18446744073709551615 - paidVisit 18446744073709551615 0 = 18446744073709551615 := by rfl
example (b a : Nat) (positive : 0 < b) (bounded : a ≤ b) :
    b - paidVisit b a = max 1 (b-a) := paid_visit_cost positive bounded
example (b a : Nat) (positive : 0 < b) (bounded : a ≤ b) :
    a - paidVisit b a = 1 ↔ b-a = 0 := paid_visit_administrative_iff positive bounded
''')

    def test_finite_round_conservation_and_composition(self):
        self.prove('''
theorem initialRound : PaidVisits 10 [1, 3] 6 :=
  .visit (afterNative := 10) (by decide) (by decide)
    (.visit (afterNative := 6) (by decide) (by decide) (.nil _))
theorem finalRound : PaidVisits 6 [1, 5] 0 :=
  .visit (afterNative := 6) (by decide) (by decide)
    (.visit (afterNative := 0) (by decide) (by decide) (.nil _))
example : PaidVisits 10 [1, 3, 1, 5] 0 := initialRound.append finalRound
example : [1, 3, 1, 5].sum + 0 = 10 := (initialRound.append finalRound).accounting
example : [1, 3, 1, 5].sum ≤ 10 ∧ [1, 3, 1, 5].length ≤ 10 :=
  (initialRound.append finalRound).grant_bound
example (grant final : Nat) (costs : List Nat) (run : PaidVisits grant costs final) :
    costs.sum + final = grant ∧ costs.length ≤ grant :=
  ⟨run.accounting, run.grant_bound.2⟩
example : ¬ PaidVisits 0 [1] 0 := by
  intro run
  have := run.zero_inert
  simp at this
example : ¬ PaidVisits 1 [1, 1] 0 := by
  intro run
  have := run.grant_bound
  simp at this
example : ¬ PaidVisits 1 [0] 1 := by
  intro run
  have := run.visit_bound
  simp at this
''')

    def test_outer_visit_preserves_inner_execution_accounting(self):
        self.prove('''
example : PaidVisits 3 [2] 1 := by
  have remaining : (drain machine start 3).remaining = 1 := by decide_cbv
  simpa [remaining, paidVisit] using (drain_execution machine start 3).paid_visit (by decide)
example : PaidVisits 3 [1] 2 := by
  have remaining : (drain machine ⟨2,0,true⟩ 3).remaining = 2 := by decide_cbv
  simpa [remaining, paidVisit] using (drain_execution machine ⟨2,0,true⟩ 3).paid_visit (by decide)
example : PaidVisits 9 [1] 8 := by
  have remaining : (drain yielding 4 9).remaining = 9 := by decide_cbv
  simpa [remaining, paidVisit] using (drain_execution yielding 4 9).paid_visit (by decide)
example : (drain yielding 4 9).remaining = 9 := by decide_cbv
example : broken.events = [12,22,7] := by decide_cbv
''')

    def test_false_paid_visit_claims_rejected(self):
        claims = {
            'free_handoff': 'paidVisit 7 7 = 7',
            'precharged_callback': 'paidVisit 7 2 = 1',
            'rebilled_unit_native_work': 'paidVisit 7 6 = 5',
            'consumed_grant_refunded': 'paidVisit 7 0 = 1',
            'spent_native_cost_erased': '7 - paidVisit 7 2 = 1',
            'native_visit_charges_admin': '2 - paidVisit 7 2 = 1',
            'idle_visit_has_no_admin': '7 - paidVisit 7 7 = 0',
            'zero_is_positive_visit': '1 ≤ 0 - paidVisit 0 0',
            'nonmonotone_native_is_admissible': 'paidVisit 3 4 < 3',
            'grant_partitions_preserve_admin_cost': 'paidVisit 2 2 = paidVisit 1 1 + paidVisit 1 1',
        }
        for name, claim in claims.items():
            with self.subTest(claim=name):
                self.prove('example : ' + claim + ' := by decide_cbv', True)


if __name__ == '__main__':
    unittest.main()
