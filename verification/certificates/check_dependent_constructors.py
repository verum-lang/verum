#!/usr/bin/env python3
"""Replay general dependent-constructor elimination and altered evidence."""
from pathlib import Path
import certificate_runner
import portable_terms
from portable_terms import (
    app, bind, construct, data, dependent_field, identity, induct, lam,
    lower, pi, refl, universe, var,
)


def cases():
    a, x, y, record, value, proof = map(var, ('A', 'x', 'y', 'record', 'value', 'proof'))
    evidence = data('Evidence', [
        [dependent_field('value', a), dependent_field('proof', identity(a, x, value))]
    ])
    read = lam('record', evidence, induct(record, lam('subject', evidence, a),
        bind(lam, [('value', a), ('proof', identity(a, x, value))], value)))
    parameters = [('A', universe()), ('x', a), ('record', evidence)]
    goal = bind(pi, parameters, identity(a, x, app(read, record)))
    motive = lam('subject', evidence, identity(a, x, app(read, var('subject'))))
    elimination = bind(lam, parameters, induct(record, motive,
        bind(lam, [('value', a), ('proof', identity(a, x, value))], proof)))
    forged = bind(lam, parameters, induct(record, motive,
        bind(lam, [('value', a), ('proof', identity(a, x, value))], refl(value))))
    wrong_branch = bind(lam, parameters, induct(record, motive,
        bind(lam, [('value', a), ('proof', identity(a, value, value))], proof)))
    inputs = [('A', universe()), ('x', a), ('y', a)]
    constructor_goal = bind(pi, inputs, evidence)
    valid = bind(lam, inputs, construct(evidence, 0, x, refl(x)))
    wrong_value = bind(lam, inputs, construct(evidence, 0, y, refl(x)))
    wrong_proof = bind(lam, inputs, construct(evidence, 0, x, refl(y)))
    rows = [
        ('general-dependent-record-elimination', elimination, goal, True),
        ('general-dependent-record-construction', valid, constructor_goal, True),
        ('discard-dependent-evidence', forged, goal, False),
        ('replace-dependent-branch-premise', wrong_branch, goal, False),
        ('change-dependent-constructor-value', wrong_value, constructor_goal, False),
        ('change-dependent-constructor-proof', wrong_proof, constructor_goal, False),
    ]
    return [(name, {'term': lower(term), 'claimed_type': lower(goal), 'metadata': {}}, accepted)
            for name, term, goal, accepted in rows]


if __name__ == '__main__':
    certificate_runner.run(cases(), __doc__, [Path(__file__),
        Path(portable_terms.__file__), Path(certificate_runner.__file__)])
