#!/usr/bin/env python3
"""Check finite-code lowering controls and intended semantic proof rejections."""
import argparse
import json
from pathlib import Path
import tempfile
from check_typed_programs import ROOT, check, require_success, run, sha256


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    modules = ['TypedPrograms', 'FiniteCode', 'CheckedPairs', 'FiniteCodeControls']
    sources = [ROOT / 'VerumExternalReplay' / (name + '.lean') for name in modules]
    sources += [Path(__file__).resolve(), ROOT / 'check_typed_programs.py',
                ROOT / 'lean-toolchain', ROOT / 'lakefile.toml']
    before = {str(p.relative_to(ROOT)): sha256(p) for p in sources}
    require_success(run('lake', 'build', 'VerumExternalReplay.FiniteCodeControls'))
    positive = check(ROOT / 'VerumExternalReplay/FiniteCodeControls.lean')
    header = ('import VerumExternalReplay.FiniteCodeControls\n'
              'open CheckedPairs FiniteCodeControls\nset_option warningAsError true\n')
    controls = {
        'lexical_scope': 'run lexical [] = some (.nat 99)',
        'unused_input': 'run unusedInput [] = some (.nat 42)',
        'unused_failure': 'run eager [] = some (.nat 42)',
        'explicit_output': 'run explicitOutput [] = some (.nat 99)',
        'empty_output': 'run emptyOutput [] = some (.nat 0)',
        'declared_operand_hole': 'run declaredHole [] = some (.nat 1)',
        'ignored_operand': 'run ignoredOperand [] = none',
        'packing_high_bits': 'apply .packByte [.nat 72057594037927936, .nat 0] = .ok (.nat 1)',
        'binding_order': 'FiniteCode.bind 2 [.external 1, .external 0] (Value.nat 0) [.nat 7, .nat 9] = some [.nat 7, .nat 9]',
    }
    negative = {}
    with tempfile.TemporaryDirectory(prefix='verum-finite-code-') as directory:
        for name, equation in controls.items():
            path = Path(directory) / (name + '.lean')
            path.write_text(header + f'example : {equation} := by\n  '
                            'try simp only [lexical_exact, unused_input_fails, unused_failure_retained, '
                            'explicit_output_exact, empty_output_fails, declared_hole_refused, '
                            'ignored_operand_good]\n  rfl\n')
            # Generated controls share a reduction roster; unused simp entries
            # are not proof failures and must not mask the intended rejection.
            path.write_text(path.read_text().replace('example :',
                'set_option linter.unusedSimpArgs false in\nexample :'))
            negative[name] = check(path, 'Tactic `rfl` failed:')
    if before != {str(p.relative_to(ROOT)): sha256(p) for p in sources}:
        raise RuntimeError('Proof sources changed during verification')
    args.report.write_text(json.dumps({'scope': 'Checked finite-tree lowering model controls; '
        'foreign runtime correspondence requires separate observations', 'sources': before,
        'positive': positive, 'negative': negative}, indent=2) + '\n')
    print(f'Finite-code controls checked; {len(negative)} intended rejections')


if __name__ == '__main__':
    main()
