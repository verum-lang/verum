#!/usr/bin/env python3
"""Replay frozen native Shift.apply over every valid 64-bit input in Lean.

This proves the selected sequential Rust arithmetic under the explicit MIR
frontend trust boundary. It does not prove callers, ownership or world descent.
"""
import argparse
import json
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT.parent / 'kani'))
from check_native_clock import archive
from rust_replay import load, read, require, sha, write
from mir_to_lean import translate

from runtime import RUST_VERSION, execute, runtime_identity
SOURCE = 'rust/crates/synarc-kan/src/organism/observer_acquired/finite_realization/source_transport.rs'
HEADER = 'import VerumExternalReplay.BoundedLasso\nset_option warningAsError true\nopen BoundedLasso\n'
PROOF = '''
theorem implementation_exact (size closing capped residue index : Nat)
    (width : size < 18446744073709551616) (closed : closing < size)
    (inside : index < size) (_cap : capped ≤ size) (cycle : residue < size-closing) :
    extracted size closing capped residue index =
      some (transport size closing ⟨capped,residue⟩ index) := by
  have remBound := Nat.mod_lt (size-index) (show 0 < size-closing by omega)
  have nonzero : size-closing ≠ 0 := by omega
  by_cases before : capped < size-index
  · have room : index+capped < 18446744073709551616 := by omega
    simp [extracted, transport, inside, Nat.le_of_lt inside, before, room]
  · by_cases borrow : (size-index) % (size-closing) ≤ residue
    · have room : closing + (residue - (size-index) % (size-closing)) < 18446744073709551616 := by omega
      simp [extracted, transport, subResidue, inside, Nat.le_of_lt inside,
        Nat.le_of_lt closed, nonzero, before, borrow, room]
    · have lower : residue ≤ (size-index) % (size-closing) := by omega
      have safe : (size-index) % (size-closing) - residue ≤ size-closing := by omega
      have room : closing + (size-closing - ((size-index) % (size-closing) - residue)) < 18446744073709551616 := by omega
      simp [extracted, transport, subResidue, inside, Nat.le_of_lt inside,
        Nat.le_of_lt closed, nonzero, before, borrow, lower, safe, room]

-- The same machine result denotes arbitrarily long logical execution, while
-- both stored accumulators remain bounded. No logical time bound is imposed.
theorem elapsed_exact (size closing count index : Nat)
    (width : size < 18446744073709551616) (closed : closing < size) (inside : index < size) :
    extracted size closing (elapsed size closing count).capped
      (elapsed size closing count).residue index = some (iterate size closing count index) := by
  rw [implementation_exact size closing _ _ index width closed inside
      (elapsed_bounded size closing count closed).1 (elapsed_bounded size closing count closed).2,
    transport_exact size closing count index closed inside]

example : extracted 18446744073709551615 0 18446744073709551614 0 0 =
    some 18446744073709551614 := by decide_cbv
example : extracted 18446744073709551615 18446744073709551614 18446744073709551615 0 0 =
    some 18446744073709551614 := by decide_cbv
example : extracted 1 0 1 0 0 = some 0 := by decide_cbv
example : extracted 5 2 5 0 0 = some 3 := by decide_cbv
example : extracted 5 2 5 2 0 = some 2 := by decide_cbv
example : extracted 5 2 1 0 5 = none := by decide_cbv
#print axioms implementation_exact
#print axioms elapsed_exact
'''



NATIVE_CONTROL = b"""
fn main() {
    assert_eq!(std::mem::size_of::<usize>(), 8);
    for states in 1..24 {
        for start in 0..states {
            for index in 0..states {
                let mut shift = Shift::new(states, start);
                let mut expected = index;
                for _ in 0..3*states {
                    assert_eq!(shift.apply(index), expected);
                    shift.advance();
                    expected = if expected + 1 < states { expected + 1 } else { start };
                }
            }
        }
    }
    let maximum = usize::MAX;
    assert_eq!(Shift { states: maximum, start: 0, capped: maximum-1, residue: 0 }.apply(0), maximum-1);
    assert_eq!(Shift { states: maximum, start: maximum-1, capped: maximum, residue: 0 }.apply(0), maximum-1);
    assert_eq!(Shift { states: maximum, start: maximum-2, capped: maximum, residue: 0 }.apply(0), maximum-1);
    assert!(std::panic::catch_unwind(|| Shift::new(5,2).apply(5)).is_err());
}
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rustc', type=Path, required=True)
    parser.add_argument('--lean', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    report = {'status': 'inconclusive', 'scope': __doc__}
    write(out/'receipt.json', report)
    try:
        _, blobs = archive(ROOT.parent/'lean/fixtures', 'source-cycle-native-v1')
        provenance = load(blobs['reviewed-sources.json'])[SOURCE]
        receipt = load(blobs['native-receipt.json'])
        source = blobs[provenance['file']]
        require(sha(source) == provenance['sha256'] == receipt['source'][SOURCE]['sha256'],
                'Frozen Rust source is not the actual native build input')
        (out/'source_transport.rs').write_bytes(source)
        (out/'native-receipt.json').write_bytes(blobs['native-receipt.json'])
        fragment = source[474:1476]
        require(sha(fragment) == 'e696f2259b666e7dbc4370385c3c1bf1dd0492df146973a3c71ebada63bd374e',
                'Reviewed Shift declaration or field layout changed')
        report.update(source_sha256=sha(source), native_run_status=receipt['status'], selected_bytes=[474,1476],
                      fragment_sha256=sha(fragment), native_receipt_sha256=sha(blobs['native-receipt.json']))
        identity = runtime_identity(args.rustc)
        lean_identity = runtime_identity(args.lean)
        write(out/'lean-toolchain.json', lean_identity)
        report['lean_toolchain_sha256'] = sha(read(out/'lean-toolchain.json'))
        write(out/'rust-toolchain.json', identity)
        report['rust_toolchain_sha256'] = sha(read(out/'rust-toolchain.json'))
        code, version, _ = execute([str(args.rustc.resolve()), '--version'], out, out/'rust-version.log')
        require(code == 0 and version.strip() == RUST_VERSION, 'Unreviewed Rust MIR compiler version')
        code, cfg, _ = execute([str(args.rustc.resolve()), '--print', 'cfg'], out, out/'target.log')
        require(code == 0 and 'target_pointer_width="64"' in cfg.splitlines(), 'Expected a 64-bit target')
        code, version, _ = execute([str(args.lean.resolve()), '--version'], out, out/'lean-version.log')
        require(code == 0 and version.startswith('Lean (version 4.35.0,'), 'Unreviewed Lean version')
        model = out/'VerumExternalReplay/BoundedLasso.lean'
        model.parent.mkdir()
        model_source = (ROOT.parent/'lean/VerumExternalReplay/BoundedLasso.lean').read_bytes()
        model.write_bytes(model_source)
        env = {k:v for k,v in os.environ.items() if not k.startswith('LEAN_')}
        env['LEAN_PATH'] = str(out)
        lean = [str(args.lean.resolve()), '-M', '1024']
        code, _, _ = execute([*lean, '-o', str(model.with_suffix('.olean')), str(model)], out, out/'model.log', env)
        require(code == 0, 'Model failed to compile')
        native_source = fragment + NATIVE_CONTROL
        (out/'native_control.rs').write_bytes(native_source)
        code, _, _ = execute([str(args.rustc.resolve()), '--edition', '2021', '-C', 'overflow-checks=yes',
                              'native_control.rs', '-o', 'native_control'], out, out/'native-compile.log')
        require(code == 0, 'Native control failed to compile')
        code, _, _ = execute([str(out/'native_control')], out, out/'native-run.log')
        require(code == 0 and (out/'native_control.rs').read_bytes() == native_source,
                'Native arithmetic controls failed or changed')
        report['native_control_source_sha256'] = sha(native_source)
        cases = [
            ('exact', fragment, None),
            ('wrong_modulus', fragment.replace(b'until_wrap % length', b'until_wrap % self.states'),
             'extracted 5 2 5 0 0 = some 3'),
            ('wrong_wrap', fragment.replace(b'self.start + residue', b'self.states + residue'),
             'extracted 18446744073709551615 18446744073709551613 18446744073709551615 0 0 = some 18446744073709551614'),
            ('off_by_one_prefix', fragment.replace(b'index + self.capped', b'index + self.capped + 1'),
             'extracted 5 2 1 0 0 = some 1'),
        ]
        results = []
        for name, data, false_claim in cases:
            case = out/name
            case.mkdir()
            (case/'shift.rs').write_bytes(data)
            command = [str(args.rustc.resolve()), '--crate-type', 'lib', '--edition', '2021',
                       '--emit', 'mir', '-C', 'overflow-checks=yes', 'shift.rs', '-o', 'shift.mir']
            code, _, _ = execute(command, case, case/'compiler.log')
            require(code == 0, 'Rust compilation failed')
            mir = (case/'shift.mir').read_text()
            generated = HEADER + translate(mir, 'apply')
            if name == 'wrong_wrap':
                generated += '\nexample : extracted 18446744073709551615 18446744073709551613 18446744073709551615 0 0 = none := by decide_cbv\n'
            generated += PROOF if false_claim is None else '\nexample : '+false_claim+' := by decide_cbv\n'
            proof = case/'Replay.lean'
            proof.write_text(generated)
            code, stdout, stderr = execute([*lean, '--json', str(proof)], out, case/'lean.log', env)
            require(not stderr, 'Unexpected Lean stderr')
            diagnostics = [json.loads(line) for line in stdout.splitlines() if line.strip()]
            errors = [d for d in diagnostics if d['severity'] == 'error']
            require(not any(d['severity'] == 'warning' for d in diagnostics), 'Lean warning')
            if false_claim is None:
                require(code == 0 and not errors, 'Universal implementation proof failed')
                infos = [d['data'] for d in diagnostics if d['severity'] == 'information']
                require(len(infos) == 2, 'Missing theorem axiom audit')
                for theorem, info in zip(('implementation_exact','elapsed_exact'), infos):
                    require(info in [f"'{theorem}' does not depend on any axioms",
                        f"'{theorem}' depends on axioms: [propext]",
                        f"'{theorem}' depends on axioms: [propext, Quot.sound]"], 'Unapproved proof axiom')
            else:
                require(code == 1 and len(errors) == 1 and errors[0]['data'].startswith(
                    '`decide_cbv` failed: the proposition evaluates to `false`'),
                    'Mutation did not reach a semantic counterexample')
            require((case/'shift.rs').read_bytes() == data and (case/'shift.mir').read_text() == mir
                    and proof.read_text() == generated, 'Compilation or proof input changed')
            results.append({'case': name, 'source_sha256': sha(data), 'mir_sha256': sha(mir.encode()),
                            'proof_sha256': sha(generated.encode()), 'diagnostics': diagnostics})
            print(name, 'passed', flush=True)
        require(model.read_bytes() == model_source, 'Mathematical model changed during replay')
        require(runtime_identity(args.rustc) == identity, 'Compiler changed during replay')
        require(runtime_identity(args.lean) == lean_identity, 'Lean runtime changed during replay')
        report.update(status='verified', results=results, model_sha256=sha(model.read_bytes()),
                      translator_sha256=sha(read(ROOT/'mir_to_lean.py')), runner_sha256=sha(read(Path(__file__))),
                      runtime_sha256=sha(read(ROOT/'runtime.py')))
    except BaseException as error:
        report['error'] = repr(error)
        raise
    finally:
        write(out/'receipt.json', report)


if __name__ == '__main__':
    main()
