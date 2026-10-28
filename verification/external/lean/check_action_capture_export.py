#!/usr/bin/env python3
"""Replay frozen typed action constructors, retained queries and complete target wires."""
from pathlib import Path
from acquired_export_codec import SENSORY, option, tag
import check_lifecycle_export as lifecycle
from observer_export_replay import fields, natural, sequence, boolean, require, main as replay_main

MODELS = lifecycle.MODELS + ['SensoryPort', 'ActionCapture', 'ActionTargetWire']
TERMINALS = ['Death', 'Time', 'Completed', 'Ended']
TYPE = 'Capture UInt8 (List UInt8 × List UInt8 × List UInt8)'


class Encoder:
    def __init__(self, action_maximum=255):
        self.definitions = []
        self.bytes = {}
        self.action_maximum = action_maximum

    def wire(self, value):
        encoded = SENSORY.bytes(value)  # Strict integers, never Boolean/float byte aliases.
        key = tuple(value)
        if key not in self.bytes:
            name = 'bytes_' + str(len(self.bytes))
            self.bytes[key] = name
            self.definitions.append(f'def {name} : List UInt8 := {encoded}')
        return self.bytes[key]

    def sensory(self, value):
        fields(value, 'schema body')
        return f'⟨{self.wire(value["schema"])}, {self.wire(value["body"])}⟩'

    def text(self, value):
        require(type(value) is str, 'Expected exact failure text')
        return self.wire(list(value.encode('utf-8')))

    def result(self, value):
        fields(value, 'kind value')
        payload = value['value']
        if value['kind'] == 'Ok':
            require(type(payload) is dict and 'kind' in payload, 'Missing observed result')
            if payload['kind'] == 'Terminal':
                fields(payload, 'kind terminal resumption')
                observed = f'(.terminal {tag(payload["terminal"], TERMINALS)} {tag(payload["resumption"], ["Resumed", "Stopped"])})'
            else:
                fields(payload, 'kind')
                observed = tag(payload['kind'], ['Step', 'Refused'])
            return '(.ok ' + observed + ')'
        require(value['kind'] == 'Err', 'Unknown action result')
        fields(payload, 'operation terminal terminal_scope stage context cause')
        operation = payload['operation']
        require(type(operation) is dict and 'kind' in operation, 'Missing failure operation')
        if operation['kind'] == 'Act':
            fields(operation, 'kind action')
            operation = '(.act ' + natural(operation['action'], self.action_maximum) + ')'
        else:
            fields(operation, 'kind')
            operation = tag(operation['kind'], ['Begin', 'Checkpoint', 'Restore'])
        tag(payload['stage'], ['Budget', 'Dispatch', 'Receive', 'Decode', 'Restart', 'Replay'])
        tag(payload['context'], ['Live', 'Imagined'])
        detail = '(' + ', '.join(self.text(payload[k]) for k in ('stage', 'context', 'cause')) + ')'
        terminal = option(payload['terminal'], lambda t: tag(t, TERMINALS))
        return f'(.error ⟨{operation}, {terminal}, {tag(payload["terminal_scope"], ["Episode", "Action"])}, {detail}⟩)'

    def capture(self, value, full=False):
        fields(value, 'action result observation resumed' + (' target target_error' if full else ''))
        return '⟨' + ', '.join((natural(value['action'], self.action_maximum), self.result(value['result']),
            option(value['observation'], self.sensory), option(value['resumed'], self.sensory))) + '⟩'

    def target_result(self, value):
        if value['target'] is not None:
            require(value['target_error'] is None, 'Target has both bytes and an error')
            return '(.ok ' + self.wire(value['target']) + ')'
        require(value['target_error'] in ('Unobserved', 'Missing'),
                'Missing target reason or unsupported allocator-capacity observation')
        return '(.error ' + tag(value['target_error'], ['Unobserved', 'Missing']) + ')'


def emit(data):
    fields(data, 'schema constructor_attempts records')
    require(data['schema'] == 'synarc-action-capture-v2', 'Unsupported action capture schema')
    encoder = Encoder()
    claims = []
    accepted = 0

    def claim(equation):
        name = 'c' + str(len(claims))
        claims.append(f'theorem {name} : {equation} := by rfl')

    def capture(name, value, full=False):
        encoded = encoder.capture(value, full)
        encoder.definitions.append(f'def {name} : {TYPE} := {encoded}')
        return name

    attempts = sequence(data['constructor_attempts'])
    records = sequence(data['records'])
    require(attempts and records, 'Missing constructor or actual execution observations')
    for i, row in enumerate(attempts):
        fields(row, 'action result observation resumed accepted error capture')
        boolean(row['accepted'])
        original = capture('attempt_' + str(i), {k: row[k] for k in ('action', 'result', 'observation', 'resumed')})
        if row['accepted']:
            accepted += 1
            require(row['error'] is None, 'Accepted constructor carries an error')
            observed = capture('accepted_' + str(i), row['capture'], full=True)
            claim(f'check {original} = .ok {observed}')
            claim(f'ActionTargetWire.materialize (2^64-1) {observed} = {encoder.target_result(row["capture"])}')
        else:
            require(row['capture'] is None, 'Refused constructor carries a capture')
            claim(f'check {original} = .error {tag(row["error"], ["UnownedObservation", "UnexpectedResumption", "ContradictorySuccessor"])}')
    for i, row in enumerate(records):
        fields(row, 'dispatched_action capture query_action query before target calls')
        observed = capture('record_' + str(i), row['capture'], full=True)
        claim(f'check {observed} = .ok {observed}')
        claim(f'ActionTargetWire.materialize (2^64-1) {observed} = {encoder.target_result(row["capture"])}')
        claim(f'{observed}.action = ({natural(row["dispatched_action"], 255)} : UInt8)')
        claim(f'({natural(row["query_action"], 255)} : UInt8) = {observed}.action')
        before, query = (option(row[k], encoder.sensory) for k in ('before', 'query'))
        # Only the provided sensory/query data are compared. No missing Event,
        # dispatch-cut identity, clock or slot transition is manufactured.
        claim(f'({query} : Option Sensory) = {before}')
        target, intake = (option(x, encoder.wire) for x in (row['capture']['target'], row['target']))
        claim(f'({intake} : Option (List UInt8)) = {target}')
        claim(f'({natural(row["calls"])} : Nat) = 1')
    source = ['import VerumExternalReplay.ActionTargetWire',
        'open AcquiredObserver ActionCapture', 'set_option warningAsError true',
        'set_option maxRecDepth 4096', 'namespace FrozenObserverExport',
        *encoder.definitions, *claims, 'end FrozenObserverExport']
    return '\n\n'.join(source)+'\n', dict(records=len(records), constructor_attempts=len(attempts),
        accepted=accepted, refused=len(attempts)-accepted, claims=len(claims))


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODELS, producers=[Path(__file__).resolve(), *lifecycle.PRODUCERS],
        scope='Exact finite typed constructor observations, retained sensory queries, dispatched actions and complete 64-bit target wires. No missing cut identities, slot/reset timing, successor-guard traces, native allocator failures, arbitrary Rust executions, predictive normalization or decision-policy correctness are inferred.',
        description=__doc__)
