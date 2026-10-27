#!/usr/bin/env python3
"""Check finite declaration/FIFO/paid-opening laws; not Rust scheduler or infinite-memory conformance."""
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
    modules = ['ExactMass', 'EnumerableMass', 'AcquisitionScheduling']
    sources = [ROOT / 'VerumExternalReplay' / (m + '.lean') for m in modules]
    sources += [Path(__file__).resolve(), ROOT / 'check_typed_programs.py',
                ROOT / 'lean-toolchain', ROOT / 'lakefile.toml']
    before = {str(p.relative_to(ROOT)): sha256(p) for p in sources}
    require_success(run('lake', 'build', 'VerumExternalReplay.AcquisitionScheduling'))
    header = '''import VerumExternalReplay.AcquisitionScheduling
open AcquisitionScheduling
set_option warningAsError true
def busy (_ : Nat) : Turn Nat := ⟨true, [8, 9]⟩
def events : List (Event Nat) := [.resource 7, .opening 8, .service, .resource 7, .opening 7]
'''
    names = ['AcquisitionScheduling.' + n for n in re.findall(r'^theorem (\w+)',
        (ROOT / 'VerumExternalReplay/AcquisitionScheduling.lean').read_text(), re.M)]
    names += ['AcquisitionScheduling.declarations']
    equations = {
        'diagonal_cannot_skip_pair': 'addressAt 4 = ⟨0, 3⟩',
        'index_is_not_paid_look': '(pay 12 (.opening (addressAt 4))).2.map Charge.look = some 4',
        'triangle_has_half_factor': 'index ⟨1, 1⟩ = 7',
        'depth_is_positive': '(addressAt 0).offset + 1 = 0',
        'no_unfunded_declaration': 'EnumerableMass.reached declarations (fun _ => 0) 0 ⟨0, 0⟩ = true',
        'no_early_service_claim': 'some 7 ∈ selected [1, 2, 7] busy 2',
        'no_service_without_turn': 'some 7 ∈ selected [7] busy 0',
        'arrivals_cannot_jump_head': 'service [1, 2] ⟨false, [7]⟩ = [7, 2]',
        'unfinished_cannot_jump_head': 'service [1, 2] ⟨true, []⟩ = [1, 2]',
        'unfinished_not_discarded': 'service [1, 2] ⟨true, []⟩ = [2]',
        'resource_not_charged': '(pay 3 (.resource (7 : Nat))).1 = 4',
        'service_not_charged': '(pay 3 (Event.service : Event Nat)).1 = 4',
        'deferred_price_not_reserved': '(replay 3 events).2 = [⟨8, 3⟩, ⟨7, 3⟩]',
        'resource_is_not_negative': 'negative .resource = true',
        'no_infinite_fixed_storage': '(service [1, 2] ⟨true, [3, 4]⟩).length ≤ 2',
    }
    with tempfile.TemporaryDirectory(prefix='verum-acquisition-scheduling-') as directory:
        directory = Path(directory)
        positive = directory / 'positive.lean'
        positive.write_text(header + '''
example : (List.range 6).map addressAt = [⟨0,0⟩, ⟨0,1⟩, ⟨1,0⟩, ⟨0,2⟩, ⟨1,1⟩, ⟨2,0⟩] := by decide_cbv
example : some 7 ∈ selected [1, 2, 7] busy 3 := by decide_cbv
example : (replay 3 events) = (5, [⟨8, 3⟩, ⟨7, 4⟩]) := by decide_cbv
example : negative (.assessed false) = true := by rfl
example (a : Address) : addressAt (index a) = a := reaches_exact_address a
example (target : Nat) (ahead tail : List Nat) (plan : Nat → Turn Nat) :
  some target ∈ selected (ahead ++ target :: tail) plan (ahead.length + 1) :=
  fifo_service_bound target ahead tail plan
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
        raise RuntimeError('Scheduling model changed during verification')
    args.report.write_text(json.dumps({'scope': __doc__, 'sources': before,
        'declarations': names, 'positive': audit, 'negative': negative}, indent=2) + '\n')
    print(f'Acquisition scheduling checked: {len(names)} declarations, {len(negative)} intended rejections')


if __name__ == '__main__':
    main()
