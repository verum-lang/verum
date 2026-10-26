#!/usr/bin/env python3
"""Replay general partial observer contracts and altered evidence."""
from pathlib import Path
import certificate_runner
import generate_partial_observers
import generate_exact_transports
import generate_typed_programs
import portable_terms

if __name__ == '__main__':
    certificate_runner.run(generate_partial_observers.cases(), __doc__,
        [Path(__file__), Path(generate_partial_observers.__file__),
         Path(generate_exact_transports.__file__), Path(generate_typed_programs.__file__),
         Path(portable_terms.__file__),
         Path(certificate_runner.__file__)])
