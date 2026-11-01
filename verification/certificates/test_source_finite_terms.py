"""Lean's kernel checks the exact certificate emitted from an actual source body."""
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import test_arithmetic_terms as replay


ROOT = Path(__file__).resolve().parents[2]
DOCUMENTED = '''fn finish(ready: Bool, accepted: Bool) -> Bool
    requires ready
    ensures result == accepted
{
    ready && accepted
}

fn publish(ready: Bool, accepted: Bool) -> Bool
    ensures result == (ready && accepted)
{
    if ready { finish(ready, accepted) } else { false }
}'''


def cases():
    worker = os.environ['VERUM_SOURCE_REPLAY']
    source = (ROOT / 'crates/verum_source_proof/tests/fixtures/finite_transition.vr').read_text()
    contracted = (ROOT / 'crates/verum_source_proof/tests/fixtures/contracted_transition.vr').read_text()
    rows = []
    for name, text in [('step', source), ('publish', DOCUMENTED), ('publish', contracted)]:
        request = dict(profile='verum-source-finite/1', source=text, declaration=name)
        result = subprocess.run([worker, '--worker'], input=json.dumps(request),
                                text=True, capture_output=True, timeout=60, check=True)
        if result.stderr:
            raise AssertionError(result.stderr)
        report = json.loads(result.stdout)
        if (report['status'] != 'verified' or report['profile'] != request['profile']
                or report['source']['declaration'] != name
                or report['source']['sha256'] != hashlib.sha256(text.encode()).hexdigest()
                or report['native']['accepted'] is not True):
            raise AssertionError(report)
        judgment = report['judgment']
        if judgment['statement'] != judgment['certificate']['claimed_type']:
            raise AssertionError('source statement differs from certificate')
        rows.append((name, judgment['certificate'], True))
    # The conjunction must retain supporting helper and call proofs. A caller
    # proof alone cannot inhabit the complete bundle's claimed type.
    missing = copy.deepcopy(rows[-1][1])
    missing['term'] = missing['term']['Pair'][1]
    rows.append(('missing-supporting-obligations', missing, False))
    corrupt = copy.deepcopy(rows[0][1])
    # A reflexive type equality is not a proof of the universally quantified
    # contract. The target kernel must reject it semantically, not time out.
    corrupt['term'] = {'Refl': {'Universe': {'Concrete': 0}}}
    rows.append(('corrupted-transition-proof', corrupt, False))
    return rows


class SourceFiniteTerms(replay.ArithmeticTerms):
    CASES = staticmethod(cases)
