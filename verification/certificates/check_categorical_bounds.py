#!/usr/bin/env python3
"""Replay universal categorical KT bounds without assuming ML optimality."""
from pathlib import Path
import certificate_runner
import generate_categorical_bounds
import generate_fixed_kt
import peano_arithmetic
import positive_fractions
import portable_terms

if __name__=='__main__':
    certificate_runner.run(generate_categorical_bounds.cases(),__doc__,
        [Path(m.__file__) for m in (generate_categorical_bounds,generate_fixed_kt,
         peano_arithmetic,positive_fractions,portable_terms)]+[Path(__file__)])
