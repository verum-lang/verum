#!/usr/bin/env python3
"""Check quantified diagnostic-fibre laws and altered-evidence rejections."""
from pathlib import Path
import certificate_runner
import generate_diagnostic_fibres as generate
import portable_terms

if __name__ == '__main__':
    certificate_runner.run(generate.cases(), __doc__, [Path(__file__),
        Path(generate.__file__), Path(portable_terms.__file__), Path(certificate_runner.__file__)])
