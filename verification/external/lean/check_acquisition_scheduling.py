#!/usr/bin/env python3
"""Check declaration, fixed-round, persistent-frontier and paid-opening laws; not native scheduler conformance."""
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
def search : SearchState Nat Nat := ⟨41, 7, some 0, true⟩
def revisions : List (SearchAction Nat) :=
  [.revision 0, .declaration, .revision 0, .suspended, .declaration]
def movingOwners (i : Nat) : List Nat := if i = 0 then [1, 2] else [9, 2, 1]
-- Deliberately broken policy: each revision forces another revisit turn.
def resetStep (s : SearchState Nat Nat) (action : SearchAction Nat) : SearchState Nat Nat :=
  match action with
  | .revision start => { s with revisit := some start, revisitTurn := true }
  | _ => searchStep Nat.succ s action
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
        'new_owner_cannot_join_active_round': 'roundSelections [1, 2] movingOwners 2 = some [1, 9]',
        'swaps_cannot_reassign_nominal_owner': 'roundSelect [2, 1] [1, 2] = some (1, [1])',
        'missing_owner_cannot_be_substituted': 'roundSelect [2, 1] [1, 9] = some (9, [1])',
        'missing_owner_cannot_be_skipped': 'roundSelect [2, 1] [1, 9] = some (1, [])',
        'round_cannot_repeat_after_exhaustion': 'roundSelections [1, 2] movingOwners 3 = some [1, 2, 1]',
        'revision_cannot_reset_frontier': '(searchStep Nat.succ search (.revision 0)).frontier = 0',
        'revision_cannot_reset_turn': '(searchStep Nat.succ search (.revision 0)).revisitTurn = false',
        'suspension_cannot_count_as_declaration': 'completedDeclarations ([.suspended, .suspended] : List (SearchAction Nat)) = 2',
        'suspension_cannot_advance_frontier': '(searchStep Nat.succ search .suspended).frontier = 8',
        'retained_owner_cannot_change': '(searchTrace Nat.succ search revisions).owner = 42',
        'revisions_cannot_starve_frontier': '(searchTrace Nat.succ search revisions).frontier = 7',
        'not_every_declaration_advances_frontier': '(searchStep Nat.succ search .declaration).frontier = 8',
        'frontier_fallback_cannot_skip': '(searchStep Nat.succ { search with revisit := none } .declaration).frontier = 9',
        'reset_policy_has_no_progress_guarantee': '7 < (revisions.foldl resetStep search).frontier',
    }
    with tempfile.TemporaryDirectory(prefix='verum-acquisition-scheduling-') as directory:
        directory = Path(directory)
        positive = directory / 'positive.lean'
        positive.write_text(header + '''
example : (List.range 6).map addressAt = [⟨0,0⟩, ⟨0,1⟩, ⟨1,0⟩, ⟨0,2⟩, ⟨1,1⟩, ⟨2,0⟩] := by decide_cbv
example : some 7 ∈ selected [1, 2, 7] busy 3 := by decide_cbv
example : (replay 3 events) = (5, [⟨8, 3⟩, ⟨7, 4⟩]) := by decide_cbv
example : negative (.assessed false) = true := by rfl
example : roundSelections [1, 2] movingOwners 2 = some [1, 2] := by decide_cbv
example : roundSelect [2, 1] [1, 9] = none := by decide_cbv
example : searchTrace Nat.succ search revisions = ⟨41, 8, some 0, true⟩ := by decide_cbv
example : (revisions.foldl resetStep search).frontier = 7 := by decide_cbv
example : completedDeclarations revisions = 2 := by decide_cbv
example : (searchStep Nat.succ { search with revisit := none } .declaration).frontier = 8 := by decide_cbv
example : (searchStep Nat.succ search .declaration).revisit = some 1 := by decide_cbv
example (a : Address) : addressAt (index a) = a := reaches_exact_address a
example (target : Nat) (ahead tail : List Nat) (plan : Nat → Turn Nat) :
  some target ∈ selected (ahead ++ target :: tail) plan (ahead.length + 1) :=
  fifo_service_bound target ahead tail plan
example (s : SearchState Nat Address) (trace : List (SearchAction Address)) :
  index s.frontier + completedDeclarations trace / 2 ≤ index (searchTrace next s trace).frontier :=
  completed_declarations_advance_frontier next index next_index s trace
example (s : SearchState Nat Nat) (trace : List (SearchAction Nat))
    (completed : completedDeclarations trace = 2) :
  s.frontier < (searchTrace Nat.succ s trace).frontier :=
  two_declarations_advance_frontier Nat.succ id (fun _ => rfl) s trace completed
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
            errors = [d for d in negative[name]['diagnostics'] if d['severity'] == 'error']
            if len(errors) != 1:
                raise RuntimeError(f'Control has additional proof or elaboration failures: {name}')
    if before != {str(p.relative_to(ROOT)): sha256(p) for p in sources}:
        raise RuntimeError('Scheduling model changed during verification')
    args.report.write_text(json.dumps({'scope': __doc__, 'sources': before,
        'declarations': names, 'positive': audit, 'negative': negative}, indent=2) + '\n')
    print(f'Acquisition scheduling checked: {len(names)} declarations, {len(negative)} intended rejections')


if __name__ == '__main__':
    main()
