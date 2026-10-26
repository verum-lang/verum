#!/usr/bin/env python3
"""Audit exact enumerable-mass contracts; not native prefix grammar or scheduler conformance."""
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
    modules = ['ExactMass', 'EnumerableMass']
    sources = [ROOT / 'VerumExternalReplay' / (m + '.lean') for m in modules]
    sources += [Path(__file__).resolve(), ROOT / 'check_typed_programs.py',
               ROOT / 'lean-toolchain', ROOT / 'lakefile.toml']
    before = {str(p.relative_to(ROOT)): sha256(p) for p in sources}
    require_success(run('lake', 'build', 'VerumExternalReplay.EnumerableMass'))
    header = '''import VerumExternalReplay.EnumerableMass
open ExactMass EnumerableMass
set_option warningAsError true
def numbers : Enumeration Nat := ⟨some, id, fun _ => rfl⟩
def work (index : Nat) := index + 2
def bit : PrefixCodec Bool where
  encode b := [b]
  decode bits := match bits with | [] => none | b :: tail => some (b, tail)
  exact _ _ := rfl
'''
    names = [m + '.' + n for m in modules for n in re.findall(r'^theorem (\w+)',
        (ROOT / 'VerumExternalReplay' / (m + '.lean')).read_text(), re.M)]
    names += ['EnumerableMass.dyadic', 'EnumerableMass.paidLooks', 'EnumerableMass.PrefixCodec.product']
    equations = {
        'program_mass_cannot_replace_joint_mass': 'Equal (allocation dyadic paidLooks 0 1) (mass dyadic 0)',
        'look_zero_cannot_be_reused': 'Equal (mass paidLooks 1) (mass paidLooks 0)',
        'unused_mass_is_not_renormalized':
            'Equal (partialSum (fun i => partialSum (fun j => if i == 0 && j == 0 then allocation dyadic paidLooks i j else zero) 4) 3) one',
        'repeated_names_are_not_distinct_mass': 'AtMost (add (add (lookWeight 0) (lookWeight 0)) (lookWeight 0)) one',
        'exact_look_denominator': 'Equal (lookWeight 3) (unit 16 (by decide))',
        'mass_is_not_free_work': 'reached numbers work 11 2 = true',
        'name_visits_are_not_free': 'spent (fun _ => 0) 4 = 0',
        'incomplete_decoder_is_not_complete': '(some 0 : Option Nat) = some 2',
        'prefix_decoder_preserves_suffix': 'bit.decode [false, true] = some (false, [])',
        'distinct_names_cannot_collide': 'bit.encode false = bit.encode true',
        'concatenated_lengths_add': 'Equal (half (3 + 5 + 7)) (unit 128 (by decide))',
    }
    with tempfile.TemporaryDirectory(prefix='verum-enumerable-mass-') as directory:
        directory = Path(directory)
        positive = directory / 'positive.lean'
        positive.write_text(header + '''
example : Equal (mass dyadic 0) (unit 2 (by decide)) := by decide_cbv
example : Equal (mass paidLooks 1) (unit 6 (by decide)) := by decide_cbv
example : Equal (partialSum (mass paidLooks) 4) ⟨4, 5, by decide⟩ := by decide_cbv
example : Equal (allocation dyadic paidLooks 2 3) (unit 160 (by decide)) := by decide_cbv
example : Equal (partialSum (fun i => partialSum (allocation dyadic paidLooks i) 4) 3) ⟨7, 10, by decide⟩ := by decide_cbv
example : Equal (half (3 + 5 + 7)) (unit 32768 (by decide)) := by decide_cbv
example : reached numbers work 12 2 = true := by rfl
example : (bit.product bit).decode [true, false, true] = some ((true, false), [true]) := by rfl
example (code : Nat) : ∃ threshold, ∀ budget, threshold ≤ budget →
    reached numbers work budget code = true ∧ numbers.decode (numbers.index code) = some code :=
  every_code_reachable numbers work code
''' + '\n'.join('#print axioms ' + n for n in names) + '\n')
        audit = check(positive)
        rosters = [d['data'] for d in audit['diagnostics'] if d['severity'] == 'information']
        if len(rosters) != len(names) or any(not (r.endswith('does not depend on any axioms') or
                r.endswith('depends on axioms: [propext]') or
                r.endswith('depends on axioms: [propext, Quot.sound]')) for r in rosters):
            raise RuntimeError('Unexpected or missing theorem axiom audit')
        negative = {}
        for name, equation in equations.items():
            path = directory / (name + '.lean')
            path.write_text(header + f'example : {equation} := by decide_cbv\n')
            negative[name] = check(path, '`decide_cbv` failed: the proposition evaluates to `false`')
    if before != {str(p.relative_to(ROOT)): sha256(p) for p in sources}:
        raise RuntimeError('Enumerable-mass model changed during verification')
    args.report.write_text(json.dumps({'scope': __doc__, 'sources': before,
        'declarations': names, 'positive': audit, 'negative': negative}, indent=2) + '\n')
    print(f'Enumerable mass checked: {len(names)} declarations, {len(negative)} intended rejections')


if __name__ == '__main__':
    main()
