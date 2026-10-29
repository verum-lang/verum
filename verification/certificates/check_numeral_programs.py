#!/usr/bin/env python3
"""Replay partial iteration and natural expressibility in native and Lean kernels."""
from pathlib import Path
import re
import certificate_runner
import generate_numeral_programs
import portable_terms

if __name__ == '__main__':
    source=Path(__file__).resolve().parents[1]/'external/lean/VerumExternalReplay/NumeralPrograms.lean'
    cases=generate_numeral_programs.cases()
    portable=source.read_text().split('namespace Typed')[0]
    if [name for name,_,accepted in cases if accepted] != re.findall(r'^theorem (\w+)',portable,re.M):
        raise RuntimeError('Portable theorem roster differs from the Lean source')
    certificate_runner.run(cases,__doc__,[Path(__file__),source,Path(generate_numeral_programs.__file__),
        Path(portable_terms.__file__),Path(certificate_runner.__file__)])
