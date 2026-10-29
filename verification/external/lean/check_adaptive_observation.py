#!/usr/bin/env python3
"""Audit universal adaptive-observation laws, their foundations and false controls.

This checks mathematical source proofs, not runtime correspondence or the
soundness of the executable native/reference checkers.
"""
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
    module = ROOT / 'VerumExternalReplay/AdaptiveObservation.lean'
    sources = [module, Path(__file__).resolve(), ROOT / 'check_typed_programs.py',
        ROOT / 'lean-toolchain', ROOT / 'lakefile.toml']
    before = {str(path.relative_to(ROOT)): sha256(path) for path in sources}
    require_success(run('lake', 'build', 'VerumExternalReplay.AdaptiveObservation'))
    positive = check(module)
    names = re.findall(r'^theorem (\w+)', module.read_text(), re.M)
    foundations = {
        'lift_initial_readout': '[propext]',
        'liftBits_involutive': '[Quot.sound]',
        'liftBits_initial_readout': '[propext, Quot.sound]',
    }
    expected = [f"'AdaptiveObservation.{name}' " +
        ('depends on axioms: ' + foundations[name] if name in foundations else 'does not depend on any axioms')
        for name in names]
    observed = [item['data'] for item in positive['diagnostics'] if item['severity'] == 'information']
    if observed != expected:
        raise RuntimeError('Missing, changed or additional theorem foundation audit')
    controls = {
        'false_erased_lift': '''
def erasedLift (f : Bool → Bool) (s : Bool × Bool) : Bool × Bool :=
  (false, Bool.xor s.2 (f s.1))
example : erasedLift (fun b => b) (erasedLift (fun b => b) (true, false)) =
    (true, false) := by decide
''',
        'false_blind_readout': '''
def blindReadout (_ : Unit) : Bool := false
example : ∀ b : Bool, blindReadout () = b := by decide
''',
    }
    negative = {}
    with tempfile.TemporaryDirectory(prefix='verum-adaptive-observation-') as directory:
        for name, source in controls.items():
            path = Path(directory) / (name + '.lean')
            path.write_text('import VerumExternalReplay.AdaptiveObservation\nset_option warningAsError true\n' + source)
            result = check(path, 'Tactic `decide` proved that the proposition')
            if len([item for item in result['diagnostics'] if item['severity'] == 'error']) != 1:
                raise RuntimeError('Control had an additional error: ' + name)
            negative[name] = result
    if before != {str(path.relative_to(ROOT)): sha256(path) for path in sources}:
        raise RuntimeError('Sources changed during replay')
    prefix = Path(require_success(run('lake', 'env', 'lean', '--print-prefix')))
    report = {'scope': __doc__, 'sources': before, 'theorems': names,
        'lean': {'version': require_success(run('lake', 'env', 'lean', '--version')),
            'sha256': sha256(prefix / 'bin/lean')},
        'positive': positive, 'negative': negative}
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(f'Adaptive observation: {len(names)} universal proofs, {len(negative)} semantic false controls')


if __name__ == '__main__':
    main()
