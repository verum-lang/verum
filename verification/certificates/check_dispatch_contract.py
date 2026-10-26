#!/usr/bin/env python3
"""Check general owned-dispatch settlement laws and corrupted-policy controls."""
from pathlib import Path
import certificate_runner
import generate_dispatch_contract as generate
import generate_dispatch_envelope as envelope
import portable_terms as terms

if __name__ == '__main__':
    certificate_runner.run(generate.cases() + envelope.cases(), __doc__, [Path(__file__),
        Path(generate.__file__), Path(envelope.__file__), Path(terms.__file__), Path(certificate_runner.__file__)])
