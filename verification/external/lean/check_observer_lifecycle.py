#!/usr/bin/env python3
"""Check ownership/name model laws; do not label these as foreign runtime evidence."""
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
    modules = ['ObserverOwnership', 'ExactNames', 'ObserverLifecycleControls']
    sources = [ROOT / 'VerumExternalReplay' / (name + '.lean') for name in modules]
    sources += [Path(__file__).resolve(), ROOT / 'check_typed_programs.py',
                ROOT / 'lean-toolchain', ROOT / 'lakefile.toml']
    before = {str(p.relative_to(ROOT)): sha256(p) for p in sources}
    require_success(run('lake', 'build', 'VerumExternalReplay.ObserverLifecycleControls'))
    positive = check(ROOT / 'VerumExternalReplay/ObserverLifecycleControls.lean')
    header = '''import VerumExternalReplay.ObserverLifecycleControls
open ObserverOwnership ObserverLifecycleControls
set_option warningAsError true
'''
    equations = {
        'state_value_cannot_replace_identity': '(commit advanced prepared).receipt = .ok ⟨before, advanced⟩',
        'foreign_guard_precedes_stale': '(commit foreign prepared).receipt = .error (.staleState, prepared)',
        'foreign_commit_cannot_change_state': '(commit foreign prepared).current = advanced',
        'fork_cannot_commit_source': '(commit before ⟨fork before 2, 11⟩).receipt = .ok ⟨before, advanced⟩',
        'old_preparation_cannot_commit_twice': '(commit (commit before prepared).current prepared).receipt = .ok ⟨before, advanced⟩',
        'failed_prepare_is_not_success': 'prepare before (.error 3) = .ok prepared',
        'full_value_is_not_truncated': 'forkNamed.2 = some 0',
        'input_is_part_of_key': '(ExactNames.intern 4 named.1 (6, 300)).2 = some 0',
        'old_names_do_not_move': '(ExactNames.intern 4 forkNamed.1 (5, 300)).2 = some 1',
        'capacity_does_not_erase_existing': '(ExactNames.intern 1 named.1 (5, 300)).2 = none',
        'capacity_does_not_admit_new': '(ExactNames.intern 1 named.1 (5, 556)).2 = some 1',
        'independent_fork_tables_are_not_one_namespace': '(ExactNames.intern 4 ([] : List (Nat × Nat)) (5, 556)).2 = some 1',
    }
    negative = {}
    with tempfile.TemporaryDirectory(prefix='verum-observer-lifecycle-') as tmp:
        for name, equation in equations.items():
            path = Path(tmp) / (name + '.lean')
            path.write_text(header + f'example : {equation} := by rfl\n')
            negative[name] = check(path, 'Tactic `rfl` failed:')
        # Audit all theorem dependencies, including the generic laws, rather
        # than only the closed controls that instantiate them.
        names = [module + '.' + name for module in modules for name in re.findall(
            r'^theorem (\w+)', (ROOT / 'VerumExternalReplay' / (module + '.lean')).read_text(), re.M)]
        path = Path(tmp) / 'axioms.lean'
        path.write_text(header + '\n'.join('#print axioms ' + name for name in names) + '\n')
        audit = check(path)
        rosters = [d['data'] for d in audit['diagnostics'] if d['severity'] == 'information']
        if len(rosters) != len(names) or any(not (r.endswith('does not depend on any axioms') or
                r.endswith('depends on axioms: [propext]') or
                r.endswith('depends on axioms: [propext, Quot.sound]')) for r in rosters):
            raise RuntimeError('Unexpected or missing theorem axiom audit')
    if before != {str(p.relative_to(ROOT)): sha256(p) for p in sources}:
        raise RuntimeError('Lifecycle model changed during verification')
    args.report.write_text(json.dumps({
        'scope': 'Universal ownership-guard and exact shared-namespace model laws; '
                 'not frozen Rust lifecycle conformance or affine-memory verification',
        'sources': before, 'positive': positive, 'axioms': audit,
        'theorems': names, 'negative': negative}, indent=2) + '\n')
    print(f'Lifecycle model checked: {len(names)} theorems, {len(negative)} intended rejections')


if __name__ == '__main__':
    main()
