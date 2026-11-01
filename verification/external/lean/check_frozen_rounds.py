#!/usr/bin/env python3
"""Check paid frozen-round visitation laws; excludes native refinement and VM termination."""
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
    model = ROOT / 'VerumExternalReplay/FrozenRounds.lean'
    sources = [model, Path(__file__).resolve(), ROOT / 'check_typed_programs.py',
               ROOT / 'lean-toolchain', ROOT / 'lakefile.toml']
    before = {str(p.relative_to(ROOT)): sha256(p) for p in sources}
    require_success(run('lake', 'build', 'VerumExternalReplay.FrozenRounds'))
    names = ['FrozenRounds.' + name for name in re.findall(r'^theorem (\w+)', model.read_text(), re.M)]
    header = '''import VerumExternalReplay.FrozenRounds
open FrozenRounds
set_option warningAsError true
'''
    equations = {
        'unpaid_advance': 'handoff ⟨1, 3⟩ 0 0 = some (⟨2, 3⟩, 0)',
        'midround_reset': 'beginRound ⟨1, 3⟩ 100 = ⟨0, 100⟩',
        'midround_extension': '(beginRound ⟨1, 3⟩ 100).stop = 100',
        'skipped_slot': 'handoff ⟨1, 3⟩ 1 0 = some (⟨3, 3⟩, 0)',
        'unspent_routing_unit': 'handoff ⟨1, 3⟩ 0 1 = some (⟨2, 3⟩, 1)',
        'empty_round_visit': 'handoff ⟨0, 0⟩ 1 1 = some (⟨1, 0⟩, 1)',
        'append_changes_old_identity': '(([10, 11] ++ [12] : List Nat)[1]?) = some 12',
        'unpaid_restart': '(attempt ⟨3, 3⟩ 3 0 0).1 = ⟨0, 3⟩',
        'unpaid_restart_with_arrivals': '(attempt ⟨3, 3⟩ 4 0 0).1 = ⟨0, 4⟩',
        'routing_restart_without_debit': 'attempt ⟨3, 3⟩ 4 0 1 = (⟨1, 4⟩, 1)',
    }
    with tempfile.TemporaryDirectory(prefix='verum-frozen-rounds-') as directory:
        directory = Path(directory)
        positive = directory / 'positive.lean'
        positive.write_text(header + '''
example : handoff ⟨1, 3⟩ 1 0 = some (⟨2, 3⟩, 0) := by decide_cbv
example : handoff ⟨1, 3⟩ 0 1 = some (⟨2, 3⟩, 0) := by decide_cbv
example : handoff ⟨1, 3⟩ 0 0 = none := by decide_cbv
example : beginRound ⟨1, 3⟩ 100 = ⟨1, 3⟩ := by decide_cbv
example : beginRound ⟨3, 3⟩ 100 = ⟨0, 100⟩ := by decide_cbv
example : attempt ⟨3, 3⟩ 4 0 0 = (⟨3, 3⟩, 0) := by decide_cbv
example : attempt ⟨3, 3⟩ 4 1 0 = (⟨1, 4⟩, 0) := by decide_cbv
example : attempt ⟨3, 3⟩ 4 0 1 = (⟨1, 4⟩, 0) := by decide_cbv
example : attempt ⟨0, 0⟩ 0 0 1 = (⟨0, 0⟩, 1) := by decide_cbv
example : handoff ⟨2^64 - 2, 2^64 - 1⟩ 1 0 =
    some (⟨2^64 - 1, 2^64 - 1⟩, 0) := by decide_cbv
theorem trace : Visits ⟨2, 5⟩ [2, 3, 4] ⟨5, 5⟩ :=
  .cons (after := ⟨3, 5⟩) (paid := 1) (grant := 0) (rest := 0) (by decide_cbv)
    (.cons (after := ⟨4, 5⟩) (paid := 0) (grant := 1) (rest := 0) (by decide_cbv)
      (.cons (after := ⟨5, 5⟩) (paid := 2) (grant := 0) (rest := 0) (by decide_cbv) (.nil _)))
example : 4 ∈ ([2, 3, 4] : List Nat) := fixed_slot_bound trace 4 (by decide) (by decide)
example : (5 : Nat) = 5 := frozen_round_completed trace (by decide) (by decide)
''' + '\n'.join('#print axioms ' + name for name in names) + '\n')
        audit = check(positive)
        rosters = [d['data'] for d in audit['diagnostics'] if d['severity'] == 'information']
        allowed = ('does not depend on any axioms', 'depends on axioms: [propext]',
                   'depends on axioms: [propext, Quot.sound]')
        if len(rosters) != len(names) or any(not r.endswith(allowed) for r in rosters):
            raise RuntimeError('Unexpected or missing theorem axiom audit: ' + repr(rosters))
        negative = {}
        for name, equation in equations.items():
            path = directory / (name + '.lean')
            path.write_text(header + f'example : {equation} := by decide_cbv\n')
            negative[name] = check(path, '`decide_cbv` failed: the proposition evaluates to `false`')
    if before != {str(p.relative_to(ROOT)): sha256(p) for p in sources}:
        raise RuntimeError('Frozen-round inputs changed during verification')
    args.report.write_text(json.dumps({'scope': __doc__, 'sources': before,
        'declarations': names, 'positive': audit, 'negative': negative}, indent=2) + '\n')
    print(f'Frozen rounds checked: {len(names)} declarations, {len(negative)} intended rejections')


if __name__ == '__main__':
    main()
