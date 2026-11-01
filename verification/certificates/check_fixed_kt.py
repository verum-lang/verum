#!/usr/bin/env python3
"""Replay exact fixed-histogram KT bounds, attainment and refusal controls."""
from pathlib import Path
import certificate_runner
import generate_fixed_kt
import peano_arithmetic
import positive_fractions
import portable_terms

if __name__ == '__main__':
    certificate_runner.run(generate_fixed_kt.cases(), __doc__,
        [Path(module.__file__) for module in
         (generate_fixed_kt, peano_arithmetic, positive_fractions, portable_terms)] + [Path(__file__)])
