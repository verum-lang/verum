#!/usr/bin/env python3
"""Check partial-program resource transfer through both native kernels and Lean."""
from pathlib import Path
import re
import certificate_runner
import generate_resource_transfer
import portable_terms

if __name__ == '__main__':
    cases = generate_resource_transfer.cases()
    source = Path(__file__).resolve().parents[1]/'external/lean/VerumExternalReplay/ResourceTransfer.lean'
    if [name for name,_,expected in cases if expected] != re.findall(r'^theorem (\w+)',source.read_text(),re.M):
        raise RuntimeError('Native theorem roster differs from the Lean source')
    certificate_runner.run(cases,__doc__,[Path(__file__),source,Path(generate_resource_transfer.__file__),
        Path(portable_terms.__file__),Path(certificate_runner.__file__)],foundations=generate_resource_transfer.foundations)
