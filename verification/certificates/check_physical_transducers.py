#!/usr/bin/env python3
"""Check universal physical-history schedule erasure and semantic rejections."""
from pathlib import Path
import re
import certificate_runner
import generate_physical_transducers
import portable_terms


if __name__ == '__main__':
    cases = generate_physical_transducers.cases()
    source = Path(__file__).resolve().parents[1] / 'external/lean/VerumExternalReplay/PhysicalTransducers.lean'
    if [name for name, _, expected in cases if expected] != re.findall(r'^theorem (\w+)', source.read_text(), re.M):
        raise RuntimeError('Native theorem roster differs from the Lean source')
    certificate_runner.run(cases, __doc__, [Path(__file__), source,
        Path(generate_physical_transducers.__file__), Path(portable_terms.__file__),
        Path(certificate_runner.__file__)])
