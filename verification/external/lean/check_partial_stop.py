#!/usr/bin/env python3
"""Check partial STOP/BIT finite subprobability, not native selector semantics."""
import argparse
import json
from pathlib import Path
import re
import tempfile

from check_typed_programs import ROOT, check, require_success, run, sha256

HEADER = '''import VerumExternalReplay.PartialStop
open ExactMass ContextualStop PartialStop
set_option warningAsError true
def bank : Bank Unit := fun _ _ => 0
def cell : Selection Unit := ⟨(), false⟩
def total : PartialStop.Selector Unit := fun _ _ => some cell
def absent : PartialStop.Selector Unit := fun _ _ => none
def noBits : PartialStop.Selector Unit
  | .stop, _ => some cell
  | .bit, _ => none
def partialSelect : PartialStop.Selector Unit
  | .stop, true :: _ => none
  | _, _ => some cell
def fullModel := contextual total bank
def partialModel := contextual partialSelect bank
def noBitModel := contextual noBits bank
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    sources = [ROOT / 'VerumExternalReplay' / (name + '.lean') for name in
               ('ExactMass', 'CausalCoding', 'ContextualStop', 'ClassifiedMass', 'PartialStop')]
    sources += [Path(__file__).resolve(), ROOT / 'check_typed_programs.py',
                ROOT / 'lean-toolchain', ROOT / 'lakefile.toml']
    before = {str(path.relative_to(ROOT)): sha256(path) for path in sources}
    require_success(run('lake', 'build', 'VerumExternalReplay.PartialStop'))
    names = ['PartialStop.' + name for name in re.findall(r'^theorem (\w+)',
             (ROOT / 'VerumExternalReplay/PartialStop.lean').read_text(), re.M)]
    equations = {
        'undefined_selector_is_not_normalized':
            'Equal (completed (contextual absent bank) 1 2 []) one',
        'undefined_bit_does_not_preserve_body_mass':
            'Equal (body noBitModel 1 [] (fun _ => one)) one',
        'defined_survivors_are_not_renormalized':
            'Equal (completed partialModel 1 1 []) one',
        'lost_mass_is_not_redistributed':
            'Equal (rowMass partialModel [] [[false]]) (unit 4 (by decide))',
        'prefix_bound_is_not_completed_mass':
            'Equal (rowMass fullModel [] [[false]]) (unit 4 (by decide))',
        'future_conditioned_branches_are_not_a_kernel':
            'AtMost (add (⟨3,4,by decide⟩ : Fraction) ⟨3,4,by decide⟩) one',
        'malformed_blocks_are_not_leaves': '[[false]] ∈ rows 2 1',
        'past_horizon_is_not_a_completed_leaf': '[[false],[false]] ∈ rows 1 1',
        'duplicate_wire_names_are_not_a_carrier': '([[],[]] : List (List Bool)).Nodup',
        'zero_width_names_are_not_injective':
            '(List.flatten ([] : List (List Bool)) = List.flatten [[]]) → ([] : List (List Bool)) = [[]]',
        'unvisited_mass_cannot_be_classified_as_met':
            'Equal (completed fullModel 1 0 []) (completed fullModel 1 1 [])',
    }
    with tempfile.TemporaryDirectory(prefix='verum-partial-stop-') as directory:
        directory = Path(directory)
        positive = directory / 'positive.lean'
        positive.write_text(HEADER + '''
example : Equal (completed fullModel 1 1 []) ⟨3,4,by decide⟩ := by decide_cbv
example : Equal (completed partialModel 1 1 []) ⟨5,8,by decide⟩ := by decide_cbv
example : Equal (completed noBitModel 1 2 []) (unit 2 (by decide)) := by decide_cbv
example : Equal (completed (contextual absent bank) 1 2 []) zero := by decide_cbv
example : Equal (rowMass partialModel [] [[true]]) zero := by decide_cbv
example : Equal (rowMass partialModel [] [[false]]) (unit 8 (by decide)) := by decide_cbv
example : (rows 1 3).Nodup := rows_nodup 1 3
example : AtMost (sum ((rows 1 3).map (rowMass partialModel []))) one :=
  (weights partialModel 1 3 []).bounded
example (m : PartialStop.Model S) (width depth : Nat) (s : S) :
    AtMost (completed m width depth s) one := completed_bounded m width depth s
example (m : PartialStop.Model S) (width depth : Nat) (s : S) :
    AtMost (completed m width depth s) (completed m width (depth+1) s) :=
  completed_mono m width depth s
example (a b : List (List Bool)) (ha : ∀ x ∈ a, x.length = 8)
    (hb : ∀ x ∈ b, x.length = 8) (same : a.flatten = b.flatten) : a = b :=
  flatten_injective 8 (by decide) a b ha hb same
example (c : ClassifiedMass.Classification (weights partialModel 1 3 []))
    (goal : List (List Bool) → Bool) (correct : ClassifiedMass.Correct c goal) :
    AtMost (ClassifiedMass.lower c) (add (ClassifiedMass.selected (weights partialModel 1 3 []) goal) zero) :=
  (ClassifiedMass.interval_sound c goal correct zero
    (le_trans (eq_le (add_zero _)) (weights partialModel 1 3 []).bounded)).1
''' + '\n'.join('#print axioms ' + name for name in names) + '\n')
        audit = check(positive)
        rosters = [item['data'] for item in audit['diagnostics'] if item['severity'] == 'information']
        allowed = ('does not depend on any axioms', 'depends on axioms: [propext]',
                   'depends on axioms: [propext, Quot.sound]')
        if len(rosters) != len(names) or any(not line.endswith(allowed) for line in rosters):
            raise RuntimeError('Unexpected or missing theorem axiom audit')
        negatives = {}
        for name, proposition in equations.items():
            path = directory / (name + '.lean')
            path.write_text(HEADER + f'example : {proposition} := by decide_cbv\n')
            negatives[name] = check(path, '`decide_cbv` failed: the proposition evaluates to `false`')
    if before != {str(path.relative_to(ROOT)): sha256(path) for path in sources}:
        raise RuntimeError('Partial-stop sources changed during verification')
    args.report.write_text(json.dumps({'scope': __doc__, 'sources': before,
        'declarations': names, 'positive': audit, 'negative': negatives}, indent=2) + '\n')
    print(f'Partial STOP/BIT checked: {len(names)} declarations, {len(negatives)} intended rejections')


if __name__ == '__main__':
    main()
