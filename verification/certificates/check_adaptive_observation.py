#!/usr/bin/env python3
"""Replay universal adaptive-observation proofs and semantic false controls."""
from pathlib import Path
import re
import certificate_runner
import generate_adaptive_observation
import portable_terms


if __name__ == '__main__':
    cases = generate_adaptive_observation.cases()
    source = Path(__file__).resolve().parents[1] / 'external/lean/VerumExternalReplay/AdaptiveObservation.lean'
    names = re.findall(r'^theorem (\w+)', source.read_text(), re.M)
    if [name for name, _, expected in cases if expected] != names:
        raise RuntimeError('Native universal theorem roster differs from the Lean source')
    certificate_runner.run(cases, __doc__,
        [Path(__file__), Path(generate_adaptive_observation.__file__),
         Path(portable_terms.__file__), Path(certificate_runner.__file__), source],
        foundations=generate_adaptive_observation.foundations)
