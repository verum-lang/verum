#!/usr/bin/env python3
"""Audit exact categorical prediction; excludes native conformance and statistical error bounds."""
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
    modules = ['ExactMass', 'CategoricalPrediction']
    sources = [ROOT / 'VerumExternalReplay' / (m + '.lean') for m in modules]
    sources += [Path(__file__).resolve(), ROOT / 'check_typed_programs.py',
                ROOT / 'lean-toolchain', ROOT / 'lakefile.toml']
    before = {str(p.relative_to(ROOT)): sha256(p) for p in sources}
    require_success(run('lake', 'build', 'VerumExternalReplay.CategoricalPrediction'))
    header = '''import VerumExternalReplay.CategoricalPrediction
open ExactMass CategoricalPrediction
set_option warningAsError true
def binary : Histogram := ⟨[0, 0], by decide⟩
def ternary : Histogram := ⟨[0, 0, 0], by decide⟩
def trained : Histogram := ⟨[3, 1], by decide⟩
def singleton : Histogram := ⟨[1000], by decide⟩
def score (h : Histogram) (labels : List Nat) : Fraction :=
  match path h labels with | none => zero | some (q, _) => q
def bank (c : Bool) : Histogram := if c then trained else binary
def choose (past : List Nat) : Bool := past.head? == some 1
'''
    names = [m + '.' + n for m in modules for n in re.findall(r'^theorem (\w+)',
        (ROOT / 'VerumExternalReplay' / (m + '.lean')).read_text(), re.M)]
    names += ['ExactMass.divide', 'CategoricalPrediction.predict', 'CategoricalPrediction.observe']
    equations = {
        'unseen_label_keeps_half_count': 'Equal (predict binary ⟨0, by decide⟩) zero',
        'probability_is_charged_before_update':
            'Equal (score binary [0]) (predict (observe binary ⟨0, by decide⟩) ⟨0, by decide⟩)',
        'selected_count_increments_once': '(observe trained ⟨1, by decide⟩).counts = [3, 3]',
        'other_count_is_unchanged': '(observe trained ⟨1, by decide⟩).counts = [4, 2]',
        'unknown_label_refused': '(path binary [2]).isSome = true',
        'later_unknown_label_not_skipped': '(path binary [0, 2, 1]).isSome = true',
        'alphabet_does_not_grow': '(observe binary ⟨0, by decide⟩).counts.length = 3',
        'held_out_counts_are_not_reset': 'Equal (score trained [1, 0]) (score binary [1, 0])',
        'path_keeps_every_factor': 'Equal (score binary [0, 0, 1]) (unit 8 (by decide))',
        'other_context_not_updated':
            '(observeContext bank false ⟨0, by decide⟩ true).counts = [4, 1]',
        'finite_future_mass_is_one': 'Equal (futureMass ternary 2) (unit 2 (by decide))',
        'alpha_cannot_be_dropped':
            'statisticalGate (unit 20 (by decide)) (unit 4 (by decide)) (unit 2 (by decide)) (unit 100 (by decide))',
        'threshold_equality_is_refusal':
            'statisticalGate (unit 20 (by decide)) (unit 4 (by decide)) (unit 2 (by decide)) (unit 160 (by decide))',
        'zero_alpha_does_not_admit': 'statisticalGate zero one one (unit 100 (by decide))',
        'large_denominators_remain_distinct':
            'Equal (unit (2^80 + 1) (by decide)) (unit (2^80) (by decide))',
    }
    with tempfile.TemporaryDirectory(prefix='verum-categorical-prediction-') as directory:
        directory = Path(directory)
        positive = directory / 'positive.lean'
        positive.write_text(header + '''
example : Equal (predict binary ⟨0, by decide⟩) (unit 2 (by decide)) := by decide_cbv
example : Equal (predict ternary ⟨1, by decide⟩) (unit 3 (by decide)) := by decide_cbv
example : Equal (predict trained ⟨0, by decide⟩) ⟨7, 10, by decide⟩ := by decide_cbv
example : Equal (score binary [0, 0]) ⟨3, 8, by decide⟩ := by decide_cbv
example : Equal (score binary [0, 1]) (unit 8 (by decide)) := by decide_cbv
example : Equal (score binary [0, 0, 1]) (unit 16 (by decide)) := by decide_cbv
example : Equal (score trained [1, 0]) ⟨7, 40, by decide⟩ := by decide_cbv
example : Equal (score singleton [0, 0, 0]) one := by decide_cbv
example : path binary [0, 2, 1] = none := by decide_cbv
example : Equal (futureMass ternary 3) one := by decide_cbv
example : Equal (contextFutureMass choose bank [] 3) one := by decide_cbv
example : (observeContext bank false ⟨0, by decide⟩ true).counts = [3, 1] := by decide_cbv
example : statisticalGate (unit 20 (by decide)) (unit 4 (by decide))
    (unit 2 (by decide)) (unit 200 (by decide)) := by decide_cbv
example : codePreference (unit 4 (by decide)) (unit 2 (by decide))
    (unit 100 (by decide)) := by decide_cbv
example : Less (unit (2^80 + 1) (by decide)) (unit (2^80) (by decide)) := by decide_cbv
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
        raise RuntimeError('Categorical prediction model changed during verification')
    args.report.write_text(json.dumps({'scope': __doc__, 'sources': before,
        'declarations': names, 'positive': audit, 'negative': negative}, indent=2) + '\n')
    print(f'Categorical prediction checked: {len(names)} declarations, {len(negative)} intended rejections')


if __name__ == '__main__':
    main()
