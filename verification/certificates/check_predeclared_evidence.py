#!/usr/bin/env python3
"""Check reservation/buffer structural laws through native kernels and Lean replay."""
from pathlib import Path
import re
import certificate_runner
import generate_predeclared_evidence
import portable_terms

if __name__ == '__main__':
    source = Path(__file__).resolve().parents[1]/'external/lean/VerumExternalReplay/PredeclaredEvidence.lean'
    cases = generate_predeclared_evidence.cases()
    portable = source.read_text().split('namespace Exact')[0]
    if [name for name, _, expected in cases if expected] != re.findall(r'^theorem (\w+)', portable, re.M):
        raise RuntimeError('Portable theorem roster differs from the Lean source')
    certificate_runner.run(cases, __doc__, [Path(__file__), source,
        Path(generate_predeclared_evidence.__file__), Path(portable_terms.__file__), Path(certificate_runner.__file__)])
