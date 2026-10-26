#!/usr/bin/env python3
"""Exercise exact general certificates and causal controls through real checkers."""
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path
import generate_conservative_refinement as generate


def payload_control(refusal, changed):
    # The wrong terminal/reason is another inhabitant of the SAME payload type.
    # Rejection must come from the equality, not a field-type or scope mistake.
    payload = generate.data('Payload', [[], []])
    state = generate.unit()
    result = generate.data('Outcome', [[generate.value_field(state)],
        [generate.value_field(state), generate.value_field(payload)], [generate.value_field(payload)]])
    def outcome(bit):
        value = generate.construct(payload, bit)
        return generate.construct(result, 2, value) if refusal else generate.construct(result, 1, generate.tt(), value)
    left, right = outcome(0), outcome(1 if changed else 0)
    return {'term': generate.lower(generate.refl(left)),
            'claimed_type': generate.lower(generate.identity(result, left, right)), 'metadata': {}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native', required=True, type=Path)
    parser.add_argument('--lean', required=True, type=Path)
    parser.add_argument('--report', required=True, type=Path)
    args = parser.parse_args()
    cases = [('finite_execution_preserved', generate.certificate(), True),
             ('finite_observations_preserved', generate.certificate(True), True)]
    for mutation in ['continue-state', 'stop-outcome', 'refusal-outcome', 'tail-admissibility']:
        cases.append((mutation, generate.certificate(mutation=mutation), False))
    cases.append(('observation-map', generate.certificate(True, 'observation-map'), False))
    for refusal in [False, True]:
        for changed in [False, True]:
            name = ('refusal' if refusal else 'terminal')+'-payload-'+('changed' if changed else 'identity')
            cases.append((name, payload_control(refusal, changed), not changed))
    report = {'schema_version': 1, 'cases': []}
    with tempfile.TemporaryDirectory(prefix='verum-refinement-') as directory:
        battery = {'schema_version': 1, 'certificates': [dict(id=name, term=cert['term'], claimed_type=cert['claimed_type']) for name, cert, _ in cases]}
        battery_path = Path(directory)/'battery.json'
        battery_path.write_text(json.dumps(battery, separators=(',', ':')))
        lean_run = subprocess.run([str(args.lean.resolve()), str(battery_path)], capture_output=True, text=True, timeout=60)
        if lean_run.returncode: raise RuntimeError('independent replay failed: '+lean_run.stderr[:1000])
        verdicts = json.loads(lean_run.stdout)['verdicts']
        if [row['id'] for row in verdicts] != [name for name, _, _ in cases]:
            raise RuntimeError('independent replay changed or omitted certificate identities')
        for (name, cert, expected), lean in zip(cases, verdicts):
            judgment = {'profile': 'verum-native/1', 'statement': cert['claimed_type'], 'certificate': cert}
            encoded = json.dumps(judgment, separators=(',', ':')).encode()
            native_run = subprocess.run([str(args.native.resolve())], input=encoded, capture_output=True, timeout=10)
            native = json.loads(native_run.stdout)
            if native_run.returncode != (0 if native['accepted'] else 1):
                raise RuntimeError(name+': native exit status disagrees with verdict')
            if native['accepted'] != expected or lean['ok'] != expected:
                raise RuntimeError(f'{name}: expected {expected}, native={native}, independent={lean}')
            if not expected and native['diagnostic']['stage'] not in ('substitution', 'normalization'):
                raise RuntimeError(name+': rejection did not reach proof checking: '+str(native['diagnostic']))
            report['cases'].append({'id': name, 'expected': expected, 'native': native, 'independent_lean': lean,
                'judgment_sha256': hashlib.sha256(encoded).hexdigest()})
    args.report.write_text(json.dumps(report, indent=2)+'\n')
    print(f"checked {len(cases)} exact cases through bounded native admission and independent Lean replay")

if __name__ == '__main__': main()
