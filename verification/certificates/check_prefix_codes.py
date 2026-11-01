#!/usr/bin/env python3
"""Replay constructive prefix-code and Kraft bounds with exact refusal controls."""
from pathlib import Path
import certificate_runner
import generate_prefix_codes
import peano_arithmetic
import positive_fractions
import portable_terms

if __name__ == '__main__':
    certificate_runner.run(generate_prefix_codes.cases(), __doc__,
        [Path(m.__file__) for m in (generate_prefix_codes, peano_arithmetic,
         positive_fractions, portable_terms)] + [Path(__file__)])
