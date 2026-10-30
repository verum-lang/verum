#!/usr/bin/env python3
"""Check exact classified-event intervals, not native callbacks or forecast accuracy."""
import argparse
import json
from pathlib import Path
import re
import tempfile

from check_typed_programs import ROOT, check, require_success, run, sha256

HEADER = '''import VerumExternalReplay.ClassifiedMass
open ExactMass ClassifiedMass
set_option warningAsError true
def quarter : Fraction := unit 4 (by decide)
def world : FiniteWeights Bool where
  names := [false, true]
  distinct := by decide
  weight _ := quarter
  bounded := by decide_cbv
def split : Classification world where
  met x := !x
  unmet x := x
  disjoint x _ := by cases x <;> simp
def unknown : Classification world where
  met _ := false
  unmet _ := false
  disjoint _ _ := by simp
def onlyMet : Classification world where
  met x := !x
  unmet _ := false
  disjoint _ _ _ := rfl
def prefixWorld : FiniteWeights Bool where
  names := [false]
  distinct := by decide
  weight _ := quarter
  bounded := by decide_cbv
def prefixMet : Classification prefixWorld where
  met x := !x
  unmet _ := false
  disjoint _ _ _ := rfl
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    sources = [ROOT / 'VerumExternalReplay' / (name + '.lean')
               for name in ('ExactMass', 'ClassifiedMass')]
    sources += [Path(__file__).resolve(), ROOT / 'check_typed_programs.py',
                ROOT / 'lean-toolchain', ROOT / 'lakefile.toml']
    before = {str(path.relative_to(ROOT)): sha256(path) for path in sources}
    require_success(run('lake', 'build', 'VerumExternalReplay.ClassifiedMass'))
    names = ['ClassifiedMass.' + name for name in re.findall(r'^theorem (\w+)',
             (ROOT / 'VerumExternalReplay/ClassifiedMass.lean').read_text(), re.M)]
    equations = {
        'overlap_is_not_disjoint':
            '∀ x ∈ world.names, (true : Bool) = true → (true : Bool) = false',
        'repeated_wire_names_are_not_a_set': '([false, false] : List Bool).Nodup',
        'incorrect_met_classifier': 'Correct split (fun _ => false)',
        'incorrect_unmet_classifier': 'Correct split (fun _ => true)',
        'unknown_mass_must_not_be_renormalized': 'Equal (lower split) (unit 2 (by decide))',
        'missing_mass_is_not_zero': 'Equal (upper split) (lower split)',
        'removing_evidence_is_not_refinement': 'Refines split unknown',
        'overlapping_intervals_do_not_rank': 'Less (upper split) (lower split)',
        'equal_endpoints_do_not_strictly_rank': 'Less quarter quarter',
        'unknown_classification_has_no_completed_event_mass':
            'Equal (lower unknown) (unit 2 (by decide))',
        'completion_cannot_exceed_remaining_budget':
            'AtMost (add (sum (world.names.map world.weight)) one) one',
    }
    with tempfile.TemporaryDirectory(prefix='verum-classified-mass-') as directory:
        directory = Path(directory)
        positive = directory / 'positive.lean'
        positive.write_text(HEADER + '''
example : Equal (lower split) quarter := by decide_cbv
example : Equal (upper split) ⟨3, 4, by decide⟩ := by decide_cbv
example : Equal (lower unknown) zero ∧ Equal (upper unknown) one := by decide_cbv
example : AtMost (lower split) (add (selected world (fun x => !x)) (unit 2 (by decide))) ∧
    AtMost (add (selected world (fun x => !x)) (unit 2 (by decide))) (upper split) :=
  interval_sound split (fun x => !x) (by decide_cbv) (unit 2 (by decide)) (by decide_cbv)
example : AtMost (lower unknown) (lower onlyMet) ∧ AtMost (upper onlyMet) (upper unknown) :=
  refinement_narrows unknown onlyMet (by decide_cbv)
example : AtMost (lower onlyMet) (lower split) ∧ AtMost (upper split) (upper onlyMet) :=
  refinement_narrows onlyMet split (by decide_cbv)
example : Equal quarter (unit (2^2) (by decide)) := by decide_cbv
example : AtMost (lower prefixMet) (lower split) ∧ AtMost (upper split) (upper prefixMet) :=
  extension_narrows prefixMet split [true] rfl (by decide_cbv) (by decide_cbv) (by decide_cbv)
example (a : Classification wa) (b : Classification wb) (pa pb : Fraction)
    (ha : AtMost (lower a) pa) (hb : AtMost pb (upper b))
    (separated : Less (upper b) (lower a)) : Less pb pa :=
  interval_dominance a b pa pb ha hb separated
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
        raise RuntimeError('Classified-mass sources changed during verification')
    args.report.write_text(json.dumps({'scope': __doc__, 'sources': before,
        'declarations': names, 'positive': audit, 'negative': negatives}, indent=2) + '\n')
    print(f'Classified mass checked: {len(names)} declarations, {len(negatives)} intended rejections')


if __name__ == '__main__':
    main()
