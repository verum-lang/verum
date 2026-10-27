#!/usr/bin/env python3
"""Audit conditional finite error bounds; excludes native, causal and infinite-horizon conformance."""
import argparse
import json
from pathlib import Path
import re
import tempfile
from check_typed_programs import ROOT, check, require_success, run, sha256


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    modules = ['ExactMass', 'CategoricalPrediction', 'FiniteAdmission']
    sources = [ROOT / 'VerumExternalReplay' / (m + '.lean') for m in modules]
    sources += [Path(__file__).resolve(), ROOT / 'check_typed_programs.py',
                ROOT / 'lean-toolchain', ROOT / 'lakefile.toml']
    before = {str(p.relative_to(ROOT)): sha256(p) for p in sources}
    require_success(run('lake', 'build', 'VerumExternalReplay.FiniteAdmission'))
    header = '''import VerumExternalReplay.FiniteAdmission
open ExactMass CategoricalPrediction FiniteAdmission
set_option warningAsError true
def coin : Experiment Bool where
  outcomes := [false, true]
  distinct := by decide
  nullMass b := if b then ⟨3, 4, by decide⟩ else unit 4 (by decide)
  normalized := by decide_cbv
def focus (label : Bool) (weight : Fraction) : Test coin where
  allocation := weight
  predictive b := if b == label then one else zero
  parentMaximum := coin.nullMass
  proper := by cases label <;> decide_cbv
  dominates _ _ := le_refl _
def half : Fraction := unit 2 (by decide)
def small : Fraction := unit 10 (by decide)
def alpha : Fraction := ⟨4, 5, by decide⟩
def paid : List (Test coin) := [focus false half, focus true half]
def unpaid : List (Test coin) := [focus false one, focus true one]
def badParent (_ : Bool) : Bool := decide (statisticalGate small one half zero)
def badPredictor (b : Bool) : Bool := decide (statisticalGate alpha one one (coin.nullMass b))
def mdl (b : Bool) : Bool := decide (codePreference half ((focus false half).predictive b) (coin.nullMass b))
'''
    names = [m + '.' + n for m in modules for n in re.findall(r'^theorem (\w+)',
        (ROOT / 'VerumExternalReplay' / (m + '.lean')).read_text(), re.M)]
    names += ['ExactMass.divide', 'CategoricalPrediction.predict', 'CategoricalPrediction.observe']
    equations = {
        'paid_allocations_cannot_be_free': 'AtMost (sum (unpaid.map Test.allocation)) one',
        'unpaid_union_can_exceed_alpha': 'AtMost (eventMass coin (anyReject alpha unpaid)) alpha',
        'predictor_must_be_subprobability': 'AtMost (sum (coin.outcomes.map (fun _ => one))) one',
        'unnormalized_predictor_breaks_bound': 'AtMost (eventMass coin badPredictor) alpha',
        'parent_must_dominate_null': 'AtMost (coin.nullMass false) zero',
        'underestimated_parent_breaks_bound': 'AtMost (eventMass coin badParent) small',
        'mdl_preference_has_no_alpha_bound': 'AtMost (eventMass coin mdl) small',
        'threshold_equality_is_not_rejection': 'rejects (unit 4 (by decide)) (focus false one) false = true',
        'overlap_is_not_disjoint_addition':
            'Equal (eventMass coin (anyReject alpha [focus false half, focus false half])) '
            '(add (eventMass coin (rejects alpha (focus false half))) (eventMass coin (rejects alpha (focus false half))))',
        'one_paid_test_does_not_cover_arbitrary_selection':
            'AtMost (eventMass coin (fun _ => true)) (mul alpha half)',
        'nonempty_error_need_not_be_zero': 'Equal (eventMass coin (anyReject alpha paid)) zero',
        'probability_atoms_cannot_be_duplicated': '([false, false, true] : List Bool).Nodup',
    }
    with tempfile.TemporaryDirectory(prefix='verum-finite-admission-') as directory:
        directory = Path(directory)
        positive = directory / 'positive.lean'
        positive.write_text(header + '''
example : AtMost (sum (paid.map Test.allocation)) one := by decide_cbv
example : Equal (eventMass coin (anyReject alpha paid)) (unit 4 (by decide)) := by decide_cbv
example : AtMost (eventMass coin (anyReject alpha paid)) alpha :=
  paid_family_error coin alpha paid (by decide_cbv)
example : Equal (eventMass coin (anyReject alpha unpaid)) one := by decide_cbv
example : Equal (eventMass coin (anyReject alpha [focus false half, focus false half]))
    (unit 4 (by decide)) := by decide_cbv
example : Equal (eventMass coin (anyReject zero paid)) zero := by decide_cbv
example : Equal (eventMass coin (anyReject alpha ([] : List (Test coin)))) zero := by decide_cbv
example : Equal (eventMass coin (rejects alpha (focus false zero))) zero := by decide_cbv
example : Equal (eventMass coin badParent) one := by decide_cbv
example : Equal (eventMass coin badPredictor) one := by decide_cbv
example : Equal (eventMass coin mdl) (unit 4 (by decide)) := by decide_cbv
example (e : Experiment Bool) (a : Fraction) (ts : List (Test e))
    (budget : AtMost (sum (ts.map Test.allocation)) one) :
    AtMost (eventMass e (anyReject a ts)) a := paid_family_error e a ts budget
''' + '\n'.join('#print axioms ' + n for n in names) + '\n')
        audit = check(positive)
        rosters = [d['data'] for d in audit['diagnostics'] if d['severity'] == 'information']
        if len(rosters) != len(names) or any(not (r.endswith('does not depend on any axioms') or
                r.endswith('depends on axioms: [propext]') or
                r.endswith('depends on axioms: [propext, Quot.sound]')) for r in rosters):
            raise RuntimeError('Unexpected or missing theorem axiom audit: ' + repr(rosters))
        negative = {}
        for name, equation in equations.items():
            path = directory / (name + '.lean')
            path.write_text(header + f'example : {equation} := by decide_cbv\n')
            negative[name] = check(path, '`decide_cbv` failed: the proposition evaluates to `false`')
    if before != {str(p.relative_to(ROOT)): sha256(p) for p in sources}:
        raise RuntimeError('Finite admission model changed during verification')
    args.report.write_text(json.dumps({'scope': __doc__, 'sources': before,
        'declarations': names, 'positive': audit, 'negative': negative}, indent=2) + '\n')
    print(f'Finite admission checked: {len(names)} declarations, {len(negative)} intended rejections')


if __name__ == '__main__':
    main()
