#!/usr/bin/env python3
"""Exercise exact general certificates and causal controls through real checkers."""
from pathlib import Path
import generate_conservative_refinement as generate
import certificate_runner
import portable_terms


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
    cases = [('finite_execution_preserved', generate.certificate(), True),
             ('finite_observations_preserved', generate.certificate(True), True)]
    for mutation in ['continue-state', 'stop-outcome', 'refusal-outcome', 'tail-admissibility']:
        cases.append((mutation, generate.certificate(mutation=mutation), False))
    cases.append(('observation-map', generate.certificate(True, 'observation-map'), False))
    for refusal in [False, True]:
        for changed in [False, True]:
            name = ('refusal' if refusal else 'terminal')+'-payload-'+('changed' if changed else 'identity')
            cases.append((name, payload_control(refusal, changed), not changed))
    certificate_runner.run(cases, __doc__, [Path(__file__), Path(generate.__file__), Path(portable_terms.__file__), Path(certificate_runner.__file__)])


if __name__ == '__main__': main()
