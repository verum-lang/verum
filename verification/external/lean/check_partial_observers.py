#!/usr/bin/env python3
"""Check observer model theorems and causal mathematical counterexamples."""
import argparse
import json
from pathlib import Path
import tempfile
from check_typed_programs import ROOT, check, require_success, run, sha256


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    names = ['PartialObservers', 'PartialObserverControls', 'TypedPrograms',
             'ConservativeRefinement', 'ExactTransports', 'DiagnosticFactorization']
    sources = [ROOT / 'VerumExternalReplay' / (name + '.lean') for name in names]
    sources += [Path(__file__).resolve(), ROOT / 'check_typed_programs.py', ROOT / 'lean-toolchain', ROOT / 'lakefile.toml']
    before = {str(path.relative_to(ROOT)): sha256(path) for path in sources}
    require_success(run('lake', 'build', 'VerumExternalReplay.PartialObserverControls'))
    positive = {name: check(ROOT / 'VerumExternalReplay' / (name + '.lean'))
                for name in ['PartialObservers', 'PartialObserverControls']}
    header = 'import VerumExternalReplay.PartialObserverControls\nopen PartialObservers PartialObserverControls\nset_option warningAsError true\n'
    equations = {
        'history_order': 'ordered.observe [1, 2] = ordered.observe [2, 1]',
        'invented_zero_origin': 'ordered.replay [] = some 0',
        'pending_product': '(ordered.product pending).observe [1, 2] = some (8, 0)',
        'unused_undefined_update': 'guarded.replay [1, 0, 2] = some 3',
        'independent_lives_are_not_additive': '(ordered.replay [1]).getD 0 + (ordered.replay [2]).getD 0 = (ordered.replay [1, 2]).getD 0',
    }
    negative = {}
    with tempfile.TemporaryDirectory(prefix='verum-observer-controls-') as directory:
        directory = Path(directory)
        for name, equation in equations.items():
            path = directory / (name + '.lean')
            path.write_text(header + 'example : ' + equation + ' := by rfl\n')
            negative[name] = check(path, 'Tactic `rfl` failed:')
        path = directory / 'omitted_read_diagram.lean'
        path.write_text(header + '''example (s t : Observer Nat Nat Nat)
    (initial : t.initial = s.initial)
    (update : ∀ z e, t.update z e = (s.update z e).map id) :
    Transport s t id id id := by
  refine ⟨initial, update, ?_⟩
  intro z
  rfl
''')
        negative[path.stem] = check(path, 'Tactic `rfl` failed:')
    after = {str(path.relative_to(ROOT)): sha256(path) for path in sources}
    if before != after:
        raise RuntimeError('Proof sources changed during verification')
    lean_prefix = Path(require_success(run('lake', 'env', 'lean', '--print-prefix')))
    binaries = [lean_prefix / 'bin/lean'] + sorted(path for path in (lean_prefix / 'lib/lean').iterdir()
        if path.is_file() and path.suffix in {'.dylib', '.so', '.dll'})
    args.report.write_text(json.dumps({'scope': 'General pure partial observer model and typed-program specialization; actual runtime/adapter correspondence is separate',
        'sources': before, 'checker_files': {str(path): sha256(path) for path in binaries},
        'positive': positive, 'negative': negative}, indent=2) + '\n')
    print(f'checked observer model modules and {len(negative)} intended proof rejections')


if __name__ == '__main__':
    main()
