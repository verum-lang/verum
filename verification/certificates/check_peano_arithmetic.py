#!/usr/bin/env python3
"""Replay universal constructive natural arithmetic and polynomial certificates."""
from pathlib import Path
import certificate_runner
import generate_peano_arithmetic
import peano_arithmetic
import portable_terms

if __name__ == '__main__':
    certificate_runner.run(generate_peano_arithmetic.cases(), __doc__,
        [Path(module.__file__) for module in
         (generate_peano_arithmetic, peano_arithmetic, portable_terms)] + [Path(__file__)])
