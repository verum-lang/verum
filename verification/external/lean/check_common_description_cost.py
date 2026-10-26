#!/usr/bin/env python3
"""Audit exact cost cancellation; no floating-point or native admission claim."""
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
    module = ROOT / 'VerumExternalReplay/CommonDescriptionCost.lean'
    sources = [module, Path(__file__).resolve(), ROOT / 'check_typed_programs.py',
               ROOT / 'lean-toolchain', ROOT / 'lakefile.toml']
    before = {str(p.relative_to(ROOT)): sha256(p) for p in sources}
    require_success(run('lake', 'build', 'VerumExternalReplay.CommonDescriptionCost'))
    header = '''import VerumExternalReplay.CommonDescriptionCost
open CommonDescriptionCost
set_option warningAsError true
def a : Candidate Int := ⟨20, 10, 0, 0⟩
def b : Candidate Int := ⟨20, 15, 0, 0⟩
'''
    names = ['CommonDescriptionCost.' + n for n in re.findall(r'^theorem (\w+)', module.read_text(), re.M)]
    equations = {
        'ranking_direction': 'full 100 a < full 100 b ↔ net a < net b',
        'different_parent_baselines': 'full 100 a < full 0 b ↔ net b < net a',
        'changed_fee_is_not_outside_code': 'full 100 { a with nameFee := 3 } = full 100 a',
        'domain_fee_must_be_paid': 'full 100 { a with domainFee := 3 } = 90',
        'admission_gain_cannot_replace_common_gain': 'full 100 a = (100 : Int) - (net a + 1)',
    }
    with tempfile.TemporaryDirectory(prefix='verum-common-cost-') as directory:
        directory = Path(directory)
        positive = directory / 'positive.lean'
        positive.write_text(header + '''
example : full 100 a = 90 := by rfl
example : full 100 b = 95 := by rfl
example : full 100 { a with nameFee := 3 } = 93 := by rfl
example : full 107 a - full 107 b = full 100 a - full 100 b := by rfl
example (baseline : Rat) (left right : Candidate Rat) :
    full baseline left < full baseline right ↔ net right < net left := ranking_lt _ _ _
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
            path.write_text(header + f'example : {equation} := by decide\n')
            negative[name] = check(path, 'Tactic `decide` proved that the proposition')
    if before != {str(p.relative_to(ROOT)): sha256(p) for p in sources}:
        raise RuntimeError('Cost model changed during verification')
    args.report.write_text(json.dumps({'scope': __doc__, 'sources': before,
        'theorems': names, 'positive': audit, 'negative': negative}, indent=2) + '\n')
    print(f'Exact common cost checked: {len(names)} theorems, {len(negative)} intended rejections')


if __name__ == '__main__':
    main()
