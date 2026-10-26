#!/usr/bin/env python3
"""Audit conditional retirement/continuation laws, not native runtime conformance."""
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
    modules = ['ObserverRetirement', 'ObserverRetirementControls']
    sources = [ROOT / 'VerumExternalReplay' / (name + '.lean') for name in modules]
    sources += [Path(__file__).resolve(), ROOT / 'check_typed_programs.py',
                ROOT / 'lean-toolchain', ROOT / 'lakefile.toml']
    before = {str(p.relative_to(ROOT)): sha256(p) for p in sources}
    require_success(run('lake', 'build', 'VerumExternalReplay.ObserverRetirementControls'))
    header = '''import VerumExternalReplay.ObserverRetirementControls
open ObserverRetirement ObserverRetirementControls
set_option warningAsError true
'''
    names = [module + '.' + name for module in modules for name in re.findall(
        r'^theorem (\w+)', (ROOT / 'VerumExternalReplay' / (module + '.lean')).read_text(), re.M)]
    equations = {
        'foreign_owner': 'retire before { receipt with owner := 8 } = some after',
        'stale_cut': 'retire before { receipt with cut := 4 } = some after',
        'resource_is_not_negative': 'retire before { receipt with reason := .resource } = some after',
        'foreign_source_is_not_negative': 'retire before { receipt with reason := .foreignSource } = some after',
        'invalid_layer': 'retire before { receipt with failed := 3 } = some after',
        'no_budget_reset': 'retire before { receipt with remaining := 8 } = some after',
        'bounded_receipts': 'retire { before with capacity := 0 } receipt = some after',
        'receipt_cannot_be_reused': 'retire after receipt = some finalState',
        'committed_action_cannot_be_repeated':
            'replay before [receipt, second] = some { finalState with physical := 902 }',
        'current_observer_state_must_survive':
            'retire before receipt = some { after with layers := [⟨10, 10⟩] }',
        'suffix_must_be_removed': 'retire before receipt = some { after with layers := before.layers }',
        'epoch_must_be_restored': 'retire before receipt = some { after with language := 40 }',
        'receipts_cannot_be_discarded':
            'replay before [receipt, second] = some { finalState with receipts := [] }',
        'original_prior_cannot_be_replaced': 'edge after 5 input readNext = some (1001, 1001)',
        'undefined_prior_has_no_edge':
            'edge after 5 (fun _ _ => none) readNext = some (25, 1001)',
        'undefined_next_has_no_edge':
            'edge after 5 input (fun _ _ _ => none) = some (25, 1001)',
    }
    with tempfile.TemporaryDirectory(prefix='verum-observer-retirement-') as directory:
        directory = Path(directory)
        positive = directory / 'positive.lean'
        positive.write_text(header + '\n'.join('#print axioms ' + name for name in names) + '\n')
        audit = check(positive)
        rosters = [d['data'] for d in audit['diagnostics'] if d['severity'] == 'information']
        if len(rosters) != len(names) or any(not (r.endswith('does not depend on any axioms') or
                r.endswith('depends on axioms: [propext]') or
                r.endswith('depends on axioms: [propext, Quot.sound]')) for r in rosters):
            raise RuntimeError('Unexpected or missing theorem axiom audit')
        negative = {}
        for name, equation in equations.items():
            path = directory / (name + '.lean')
            path.write_text(header + f'example : {equation} := by rfl\n')
            negative[name] = check(path, 'Tactic `rfl` failed:')
    if before != {str(p.relative_to(ROOT)): sha256(p) for p in sources}:
        raise RuntimeError('Retirement model changed during verification')
    args.report.write_text(json.dumps({'scope': __doc__, 'sources': before,
        'theorems': names, 'positive': audit, 'negative': negative}, indent=2) + '\n')
    print(f'Retirement contract checked: {len(names)} theorems, {len(negative)} intended rejections')


if __name__ == '__main__':
    main()
