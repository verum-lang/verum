#!/usr/bin/env python3
"""Replay frozen contextual banks and every supplied declared-selector cursor step.

The fixed two-stage FirstBit selector belongs to the native export fixture.
Neither its totality nor its phase schedule is assumed for acquired VM programs.
"""
import json
from pathlib import Path
from check_joint_export import decimal
from observer_export_replay import (fields, natural, boolean, sequence, array,
    require, module_closure, main as replay_main)

LAW = 'acquired-stop-bit-frozen-kt-v3'
SCOPE = 'actual Rust cursor/banks, declared test selector; not acquired native VM refinement'
MODELS = module_closure(['ContextualRows'])


def wire(value):
    return array(natural(x, 255) for x in sequence(value))


def fraction(value, short=False):
    numerator, denominator = ('n', 'd') if short else ('numerator', 'denominator')
    fields(value, numerator + ' ' + denominator)
    n, d = decimal(value[numerator]), decimal(value[denominator])
    require(d != '0', 'Zero denominator')
    return f'⟨{n},{d},by decide⟩'


def kind(value):
    require(type(value) is str and value in ('Stop', 'Bit'), 'Unknown decision kind')
    return '.stop' if value == 'Stop' else '.bit'


def pair(value):
    require(len(sequence(value)) == 2, 'Expected two binary counts')
    return natural(value[0]), natural(value[1])


def emit(data):
    fields(data, 'schema law scope cases')
    require(data['schema'] == 'synarc-joint-frozen-rows-v1' and
            data['law'] == LAW and data['scope'] == SCOPE, 'Unsupported contextual export')
    lines = ['import VerumExternalReplay.ContextualRows',
             'open ExactMass ContextualStop ContextualRows',
             'set_option warningAsError true', 'set_option maxRecDepth 65536',
             'set_option maxHeartbeats 30000000', 'set_option cbv.maxSteps 10000000',
             'namespace FrozenObserverExport']
    claims, cache, row_count, attempts = [], {}, 0, 0

    def claim(name, body):
        require(name not in claims, 'Duplicate claim name')
        claims.append(name)
        lines.append(f'theorem {name} : {body} := by decide_cbv')

    def intern(category, value, encode):
        key = (category, json.dumps(value, sort_keys=True, separators=(',', ':')))
        if key not in cache:
            expression = encode(value)
            name = f'{category}_{len(cache)}'
            cache[key] = name
            lines.append(f'def {name} : {category} := {expression}')
        return cache[key]

    def bank(value):
        def encode(value):
            fields(value, 'joint reference lengths')
            cells, keys = [], []
            for c in sequence(value['joint']):
                fields(c, 'kind context counts')
                k, context = kind(c['kind']), wire(c['context'])
                key = (c['kind'], tuple(c['context']))
                require(key not in keys, 'Duplicate joint cell'); keys.append(key)
                z, o = pair(c['counts'])
                require(int(z)+int(o) > 0, 'Empty allocated joint cell')
                cells.append(f'⟨({k},{context}),{z},{o}⟩')
            reference = [f'⟨{n},{z},{o}⟩' for n, counts in enumerate(sequence(value['reference']))
                         for z, o in [pair(counts)]]
            lengths = value['lengths']; fields(lengths, 'counts total')
            require(type(lengths['counts']) is dict, 'Expected length histogram')
            counts = []
            for n, count in lengths['counts'].items():
                decimal(n); natural(int(n)); natural(count)
                require(count > 0, 'Empty allocated length cell')
                counts.append(f'({n},{count})')
            require(int(natural(lengths['total'])) == sum(lengths['counts'].values()),
                    'Inconsistent histogram total')
            return f'⟨{array(cells)},{array(reference)},{array(counts)}⟩'
        return intern('ContextualRows.Bank', value, encode)

    def cursor(value):
        fields(value, 'kind prefix_bits_newest_first done poisoned selecting selected')
        require(value['poisoned'] is False, 'Poisoned cursor outside this fixture contract')
        past = array('true' if int(natural(b, 1)) else 'false'
                     for b in sequence(value['prefix_bits_newest_first']))
        selected = value['selected']
        if selected is None:
            selected = 'none'
        else:
            fields(selected, 'context invert')
            selected = f'some ({wire(selected["context"])},{boolean(selected["invert"])})'
        return f'⟨{kind(value["kind"])},{past},{boolean(value["done"])},' + \
               f'{boolean(value["selecting"])},{selected}⟩'

    def frontier(value):
        def encode(value):
            fields(value, 'forecast_law samples bits ratio staged_bank pending poisoned invalid')
            require(value['forecast_law'] == LAW, 'Wrong frontier law')
            require(value['poisoned'] is False and value['invalid'] is None,
                    'Invalid accumulator outside this fixture contract')
            pending = value['pending']
            if pending is None:
                pending = 'none'
            else:
                fields(pending, 'cursor target position frozen_bank')
                pending = f'some ⟨{cursor(pending["cursor"])},{wire(pending["target"])},' + \
                          f'{natural(pending["position"])},{bank(pending["frozen_bank"])}⟩'
            return f'⟨{natural(value["samples"])},{natural(value["bits"])},{fraction(value["ratio"])},' + \
                   f'{bank(value["staged_bank"])},{pending}⟩'
        return intern('ContextualRows.Frontier', value, encode)

    cases = sequence(data['cases']); require(cases, 'No native cases')
    inversions = []
    for ci, case in enumerate(cases):
        fields(case, 'selector rows final_score')
        fields(case['selector'], 'declared_fixture inversion')
        require(case['selector']['declared_fixture'] == 'first_raw_bit_or_empty_context',
                'Unsupported selector: native VM refinement requires a separate contract')
        inversion = boolean(case['selector']['inversion'])
        require(inversion not in inversions, 'Repeated selector case'); inversions.append(inversion)
        select = f'select_{ci}'
        lines.append(f'def {select} : Selector (List Nat) := fun kind past =>\n'
                     f'  ⟨match kind,past.getLast? with\n'
                     f'    | .bit,none => [0]\n'
                     f'    | .bit,some b => [1,if b then 1 else 0]\n'
                     f'    | .stop,_ => [],{inversion}⟩')
        rows = sequence(case['rows']); require(rows, 'No native rows')
        history, previous = [], 'ExactMass.one'
        for ri, row in enumerate(rows):
            row_count += 1
            fields(row, 'target forecast point reference steps')
            target, prior = wire(row['target']), array(history)
            f = row['forecast']; fields(f, 'bank forecast_law prediction totality')
            require(f['forecast_law'] == LAW and f['totality'] == 'conditional_on_total_pure_selector',
                    'Unsupported forecast or unjustified totality promotion')
            fields(f['prediction'], 'context mask')
            require(f['prediction']['context'] == [] and f['prediction']['mask'] == [],
                    'Fixture requires the declared empty prediction carrier')
            prefix = f'c{ci}_r{ri}'
            claim(prefix+'_forecast', f'bankAgrees {select} {prior} [] {prior} {bank(f["bank"])} = true')
            claim(prefix+'_point', f'Equal (candidate {select} {prior} {target}) {fraction(row["point"],True)}')
            claim(prefix+'_reference', f'Equal (reference {prior} {target}) {fraction(row["reference"],True)}')
            progress = 0
            steps = sequence(row['steps']); require(steps, 'Missing native attempts')
            for si, step in enumerate(steps):
                attempts += 1
                fields(step, 'before after grant paid status')
                grant, paid = natural(step['grant']), natural(step['paid'])
                require(step['status'] in ('advanced','work_refused'), 'Unknown advance result')
                refused = 'true' if step['status'] == 'work_refused' else 'false'
                claim(prefix+f'_step{si}', f'attemptAgrees {select} 2 {prior} {target} {previous} '
                      f'{progress} {grant} {paid} {refused} {frontier(step["before"])} {frontier(step["after"])} = true')
                progress += int(step['grant'] > 0)
            claim(prefix+'_complete', f'({progress} : Nat) = (rowQuestions {target}).length*4+1')
            # This carrier was checked against the previous score and exact
            # candidate/reference masses by the final attempt, not trusted here.
            last_previous, last_prior, last_target = previous, prior, target
            previous = fraction(steps[-1]['after']['ratio'])
            history.append(target)
        score = case['final_score']; fields(score, 'bank bits forecast_law ratio samples')
        snapshot = dict(forecast_law=score['forecast_law'], samples=score['samples'], bits=score['bits'],
                        ratio=score['ratio'], staged_bank=score['bank'], pending=None, poisoned=False, invalid=None)
        claim(f'c{ci}_final', f'frontierAgrees {select} 2 {last_prior} {last_target} '
              f'{last_previous} {progress} {frontier(snapshot)} = true')
    lines.append('end FrozenObserverExport')
    return '\n\n'.join(lines)+'\n', dict(cases=len(cases), rows=row_count, attempts=attempts,
        claims=len(claims), retained_views=len(cache))


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODELS,
        producers=[Path(__file__).resolve(), Path(__file__).with_name('check_joint_export.py')],
        scope='Every supplied declared FirstBit fixture attempt, work refusal, raw prefix, selected context, '
              'frozen/staged count bank, complete-row publication and exact point/reference score. '
              'No arbitrary acquired selector totality, native VM refinement, allocation or ownership proof.',
        description=__doc__)
