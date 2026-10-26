#!/usr/bin/env python3
"""Check general candidate-support locality and causal witness controls."""
from pathlib import Path
import certificate_runner
import generate_candidate_locality as generate
import portable_terms

if __name__ == '__main__':
    certificate_runner.run(generate.cases(), __doc__, [Path(__file__),
        Path(generate.__file__), Path(portable_terms.__file__), Path(certificate_runner.__file__)])
