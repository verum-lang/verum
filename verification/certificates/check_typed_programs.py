#!/usr/bin/env python3
"""Replay native typed-program macro preservation and its exact-result corollaries."""
from pathlib import Path
import certificate_runner
import generate_typed_programs
import portable_terms

if __name__ == '__main__':
    certificate_runner.run(generate_typed_programs.cases(), __doc__,
        [Path(__file__), Path(generate_typed_programs.__file__),
         Path(portable_terms.__file__), Path(certificate_runner.__file__)])
