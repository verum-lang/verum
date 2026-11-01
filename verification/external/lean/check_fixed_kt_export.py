#!/usr/bin/env python3
"""Replay frozen diagnostic rows and native exact fractions in Lean's kernel.

The consumer supplies candidate coverage independently. Counts are reconstructed
from ordered typed IDs, never inferred from equal totals or display labels.
Source provenance and a finite arithmetic replay are not a Rust refinement proof.
"""
import argparse
import json
from math import gcd
from pathlib import Path
import uuid

from observer_export_replay import (array, atomic_report, fields,
    require, sequence, verify)


def natural(value):
    require(type(value) is int and 0 <= value < 2**64, 'Invalid native natural coordinate')
    return value


def magnitude(value, *, positive=False):
    require(type(value) is str and value.isascii() and value.isdecimal() and len(value) <= 4096,
            'Expected bounded decimal magnitude')
    n = int(value)
    require(str(n) == value and (not positive or n > 0), 'Noncanonical or nonpositive magnitude')
    return n


def fraction(value):
    fields(value, 'numerator denominator')
    n = magnitude(value['numerator'], positive=True)
    d = magnitude(value['denominator'], positive=True)
    require(gcd(n, d) == 1, 'Native fraction is not reduced')
    return f'(⟨{n}, {d}⟩ : Fraction)'


def assessment(value):
    fields(value, 'contexts ordered_rows child_kt compression predictive')
    contexts, rows = sequence(value['contexts']), sequence(value['ordered_rows'])
    require(0 < len(contexts) <= 1024 and 0 < len(rows) <= 10000, 'Finite replay size outside policy')
    declarations, actual = [], []
    for c in contexts:
        fields(c, 'context outcome_labels cell_labels parent_counts child_counts')
        require(type(c['context']) is list and len(c['context']) == 3, 'Invalid context coordinates')
        identity = tuple(magnitude(x) for x in c['context'])
        labels, cells = sequence(c['outcome_labels']), sequence(c['cell_labels'])
        require(labels and cells and all(type(x) is str for x in labels + cells), 'Missing coordinate labels')
        require(len(labels) == len(set(labels)) and len(cells) == len(set(cells)), 'Ambiguous display labels')
        width = len(labels)
        parent = [natural(n) for n in sequence(c['parent_counts'])]
        children = [[natural(n) for n in sequence(child)] for child in sequence(c['child_counts'])]
        require(len(parent) == width and len(children) == len(cells) and all(len(r) == width for r in children),
                'Changed declared alphabet width')
        require(all(sum(r[y] for r in children) == parent[y] for y in range(width)), 'Per-outcome conservation failed')
        declarations.append((identity, tuple(labels)))
        actual.append([[0] * width for _ in children])
    require(len({c[0] for c in declarations}) == len(declarations), 'Duplicate context identity')
    identity_rows = []
    for expected, row in enumerate(rows):
        require(type(row) is list and len(row) == 5, 'Malformed native row')
        ordinal, ci, cell, outcome, cut = map(natural, row)
        require(ordinal == expected, 'Noncontiguous row ordinals')
        require(ci < len(actual) and cell < len(actual[ci]) and outcome < len(actual[ci][cell]), 'Foreign row coordinate')
        actual[ci][cell][outcome] += 1
        identity_rows.append((ordinal, ci, outcome, cut))
    require(all(counts == c['child_counts'] for counts, c in zip(actual, contexts)), 'Ordered rows/counts disagree')
    require(all(identity_rows[i][3] < identity_rows[i+1][3] for i in range(len(rows)-1)), 'Repeated or reversed cut')
    for field in ('child_kt', 'compression', 'predictive'):
        fraction(value[field])
    return tuple(declarations), tuple(identity_rows)


def emit(data, expected_names):
    fields(data, 'schema scope window_rows incumbent incumbent_mass_denominator family_count look '
                 'envelope_child_kt envelope_predictive envelope_gain candidates')
    require(data['schema'] == 'synarc-fixed-window-envelope-v1', 'Unsupported frozen schema')
    require(type(data['scope']) is str, 'Missing diagnostic scope')
    expected = tuple(magnitude(name) for name in expected_names)
    require(expected and len(set(expected)) == len(expected), 'Expected coverage must be independently fixed and unique')
    family = magnitude(data['family_count'], positive=True)
    look = magnitude(data['look'])
    old_price = magnitude(data['incumbent_mass_denominator'], positive=True)
    candidates = sequence(data['candidates'])
    require(len(candidates) == len(expected), 'Candidate coverage changed')
    require(all(name < family for name in expected), 'Expected candidate outside declared family')
    baseline = assessment(data['incumbent'])
    require(natural(data['window_rows']) == len(baseline[1]), 'Window row count changed')
    for candidate, name in zip(candidates, expected):
        fields(candidate, 'numeral original_codes original_name mass_denominator native_assessment priced_envelope')
        natural(candidate['numeral'])
        codes = sequence(candidate['original_codes'])
        require(len(codes) == 3 and all(type(code) is list and len(code) <= 4096 and
                all(type(bit) is int and bit in (0, 1) for bit in code) for code in codes), 'Invalid recorded code bits')
        require(magnitude(candidate['original_name']) == name, 'Candidate identity or order changed')
        require(magnitude(candidate['mass_denominator'], positive=True) == family*(look+1)*(look+2), 'Original price changed')
        require(assessment(candidate['native_assessment']) == baseline, 'Window or outcome coordinates changed')
        fraction(candidate['priced_envelope'])
    for key in ('envelope_child_kt', 'envelope_predictive', 'envelope_gain'):
        fraction(data[key])

    lines = ['import VerumExternalReplay.FixedKTNative', 'open FixedKTNative',
             'set_option warningAsError true', 'set_option maxRecDepth 8192',
             'namespace FrozenObserverExport']
    claims = []
    def claim(proposition):
        name = f'c{len(claims)}'; claims.append(name)
        lines.append(f'theorem {name} : {proposition} := by decide_cbv')
    all_assessments = [data['incumbent'], *[c['native_assessment'] for c in candidates]]
    for i, value in enumerate(all_assessments):
        contexts = array(f'⟨{len(c["outcome_labels"])}, {array(map(str,c["parent_counts"]))}, '
                         f'{array(array(map(str,r)) for r in c["child_counts"])}⟩' for c in value['contexts'])
        rows = array('⟨' + ', '.join(map(str, r)) + '⟩' for r in value['ordered_rows'])
        lines += [f'def contexts{i} : List Context := {contexts}', f'def rows{i} : List Row := {rows}']
        claim(f'intakeAgrees contexts{i} rows{i} = true')
        claim(f'(rows{i}.map Row.ordinal) = List.range {data["window_rows"]}')
        claim(f'rowIdentity rows{i} = rowIdentity rows0')
        claim(f'(childScore contexts{i}).same {fraction(value["child_kt"])}')
        claim(f'((childScore contexts{i}).div (parentCode contexts{i})).same {fraction(value["compression"])}')
        claim(f'((childScore contexts{i}).div (parentLikelihood contexts{i})).same {fraction(value["predictive"])}')
        claim(f'(childScore contexts{i}).le (ceiling contexts0)')
        # Actual observed factor traversal, including zero origin, is checked
        # against the closed denominator for every parent and child count row.
        for ci, c in enumerate(value['contexts']):
            counts = array([array(map(str, c['parent_counts'])), *[array(map(str, r)) for r in c['child_counts']]])
            claim(f'({counts} : List (List Nat)).all (fun ns => counterDen {len(c["outcome_labels"])} ns 0 == '
                  f'rise2 {len(c["outcome_labels"])} ns.sum) = true')
    claim(f'(ceiling contexts0).same {fraction(data["envelope_child_kt"])}')
    claim(f'((ceiling contexts0).div (parentLikelihood contexts0)).same {fraction(data["envelope_predictive"])}')
    claim(f'((ceiling contexts0).div (childScore contexts0)).same {fraction(data["envelope_gain"])}')
    price = family*(look+1)*(look+2)
    for candidate in candidates:
        claim(f'(((ceiling contexts0).div (childScore contexts0)).mul ⟨{old_price}, {price}⟩).same '
              f'{fraction(candidate["priced_envelope"])}')
    lines.append('end FrozenObserverExport')
    return '\n\n'.join(lines)+'\n', {'claims': len(claims), 'assessments': len(all_assessments),
        'rows_per_assessment': len(baseline[1]), 'contexts': len(baseline[0]), 'expected_names': list(expected_names)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('export', 'provenance', 'output', 'report'):
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--sha256', required=True)
    parser.add_argument('--expected-names', nargs='+', required=True)
    args = parser.parse_args()
    invocation = {'run_id': uuid.uuid4().hex, 'requested_export_sha256': args.sha256,
                  'expected_names': args.expected_names}
    require(len({p.resolve() for p in (args.export, args.provenance, args.output, args.report)}) == 4, 'Output collision')
    atomic_report(args.report, {**invocation, 'status': 'running'})
    try:
        report = verify(args, emit=lambda data: emit(data, args.expected_names), modules=['FixedKTNative'],
            producers=[Path(__file__).resolve()],
            scope='Finite frozen row/count/fraction correspondence only. Universal KT algebra is separately checked in '
                  'Verum and Lean. No Rust/BigUint/Clone/Ord refinement, native program behavior, predictor, admission, '
                  'pruning, repricing, statistical guarantee or constant-cost resource bound.')
    except Exception as error:
        atomic_report(args.report, {**invocation, 'status': 'failed', 'error': str(error)[:8192]})
        raise
    atomic_report(args.report, {**invocation, 'status': 'verified', **report})
    print(json.dumps(report['counts']))


if __name__ == '__main__':
    main()
