#!/usr/bin/env python3
"""Check operational observation laws and reject loss of refusal/accounting evidence."""
import argparse
import json
from pathlib import Path
import tempfile
from check_typed_programs import ROOT, check, require_success, run, sha256


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    modules = ['TypedPrograms', 'FiniteCode', 'CheckedPairs', 'FiniteExecution',
               'ObserverAdmission', 'ObserverInterfaces', 'FiniteExecutionProjection', 'FiniteExecutionControls']
    sources = [ROOT / 'VerumExternalReplay' / (m + '.lean') for m in modules]
    sources += [Path(__file__).resolve(), ROOT / 'check_typed_programs.py',
                ROOT / 'lean-toolchain', ROOT / 'lakefile.toml']
    before = {str(p.relative_to(ROOT)): sha256(p) for p in sources}
    require_success(run('lake', 'build', 'VerumExternalReplay.FiniteExecutionControls',
                        'VerumExternalReplay.ObserverInterfaces'))
    positive = check(ROOT / 'VerumExternalReplay/FiniteExecutionControls.lean')
    projection = check(ROOT / 'VerumExternalReplay/FiniteExecutionProjection.lean')
    header = '''import VerumExternalReplay.FiniteExecutionControls
open CheckedPairs FiniteExecution ObserverAdmission FiniteExecutionControls
set_option warningAsError true
'''
    controls = {
        'budget_stop_does_not_imply_undefined': ('FiniteCode.run (FiniteExecutionProjection.partialApply apply) (add 2 3) [] = none', 'budget_does_not_refute_value.2'),
        'budget_is_not_undefined': ('(execute apply (add 18446744073709551615 1) ⟨some 0, 1000⟩ []).stop = .undefined', 'dispatch_refusal'),
        'refused_dispatch_is_not_counted': ('(execute apply (add 18446744073709551615 1) ⟨some 0, 1000⟩ []).usage.dispatch = 1', 'dispatch_refusal'),
        'exact_callee_path': ('(execute apply noCalleeOutput ⟨none, 1000⟩ []).halt = some ⟨[0], .missingCalleeOutput⟩', 'child_output_position'),
        'operand_before_callee': ('(execute apply precedence ⟨none, 1000⟩ []).halt = some ⟨[0], .missingCallee⟩', 'argument_before_callee'),
        'missing_operand_is_not_primitive_arity': ('(execute apply hole ⟨none, 1000⟩ []).halt = some ⟨[0], .primitive (.arity .checkedAdd 2 1)⟩', 'declared_operand_hole'),
        'shared_allowance_not_reset': ('secondUpdate.budget.dispatches = 2', 'cumulative_accounting'),
        'binding_and_entry_work_not_refunded': ('secondUpdate.budget.workUsed = 11', 'cumulative_accounting'),
        'root_selection_is_not_free': ('(evaluate apply .read pack binding (.nat 0) 7 [.nat 1, .nat 2] ⟨none, 10, 0, 0, false⟩).result = .ok (.nat 258, ⟨.complete, [.nat 258], ⟨10, 1, 1, 0⟩, none, none⟩)', 'root_work_is_separate'),
    }
    negative = {}
    with tempfile.TemporaryDirectory(prefix='verum-operational-controls-') as tmp:
        for name, (equation, theorem) in controls.items():
            path = Path(tmp) / (name + '.lean')
            path.write_text(header + f'example : {equation} := by\n  rw [{theorem}]\n  rfl\n')
            negative[name] = check(path, 'Tactic `rfl` failed:')
    if before != {str(p.relative_to(ROOT)): sha256(p) for p in sources}:
        raise RuntimeError('Operational model sources changed during verification')
    args.report.write_text(json.dumps({'scope': 'Finite operational and observer admission model controls; '
        'foreign implementation conformance requires frozen actual observations',
        'sources': before, 'positive': positive, 'universal_projection': projection,
        'negative': negative}, indent=2) + '\n')
    print(f'Operational model checked; {len(negative)} intended proof rejections')


if __name__ == '__main__':
    main()
