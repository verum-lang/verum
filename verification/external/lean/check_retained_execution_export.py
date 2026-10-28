#!/usr/bin/env python3
"""Replay original native shared-machine transitions against the retained Lean model.

This accepts only the frozen named u64 adapters. Invocation wrapper evidence is a
separate scope and is deliberately not silently treated as a machine artifact.
"""
from pathlib import Path
from observer_export_replay import fields, require, sequence, main as replay_main
import retained_execution_snapshot as schema
import retained_execution_encoding as lean
import retained_execution_protocol as protocol

MODELS = ['TypedPrograms', 'FiniteCode', 'FiniteExecution', 'RetainedExecution',
          'RetainedExecutionTrace', 'RetainedNative']
SEMANTICS = {'adapter': 'u64-checked-v1', 'add': 'checked_u64_add', 'identity': 'one_argument',
             'reject': 'undefined', 'resource': 'resource', 'panic': 'unwind', 'otherArity': 'undefined'}


def emit(data):
    fields(data, 'schema scope semantics fixtures associations cases')
    require(data['schema'] == 'synarc-shared-vm-native-v1' and data['scope'] == 'machine', 'Unsupported native VM scope/schema')
    require(data['semantics'] == SEMANTICS, 'Unrecognized primitive semantics')
    claims = protocol.Claims(); fixtures = {}; owners = {}; case_results = {}; total_events = 0

    def owner(key, kind, payload):
        schema.nat(key)
        require(key not in owners or owners[key] == (kind, payload), 'Retained owner changed kind or immutable payload')
        owners[key] = (kind, payload)

    for fixture in sequence(data['fixtures']):
        fields(fixture, 'id program inputs'); key = fixture['id']; schema.nat(key)
        require(key not in fixtures, 'Duplicate native fixture owner')
        source = schema.program(fixture['program']); schema.vector(fixture['inputs'])
        owner(key, 'program', source); fixtures[key] = (source, fixture['inputs'])
    require(bool(fixtures), 'No source fixtures')
    for index, case in enumerate(sequence(data['cases'])):
        adapter = case.get('adapter'); custom = adapter == 'custom-code-missing-operands-v1'
        require(custom or adapter in ('owned-program-u64-v1', 'borrowed-program-u64-v1'), 'Unknown Code adapter')
        keys = 'id adapter authority policy originalLimit seed initial services terminalReport'
        keys += ' code' if custom else ' fixture association'
        if adapter == 'owned-program-u64-v1': keys += ' finalPause'
        fields(case, keys); identity = case['id']
        require(type(identity) is str and bool(identity) and identity not in case_results, 'Duplicate/missing case identity')
        authority = case['authority']; fields(authority, 'program semantics input machine')
        if custom:
            source, inputs = schema.custom_program(case['code']), []
            owner(authority['program'], 'program', source)
        else:
            require(case['fixture'] in fixtures and authority['program'] == case['fixture'], 'Case source ownership mismatch')
            source, inputs = fixtures[case['fixture']]
        owner(authority['semantics'], 'semantics', SEMANTICS)
        owner(authority['input'], 'input', inputs)
        require(authority['machine'] not in owners, 'Native machine owner was reused')
        owner(authority['machine'], 'machine', identity)
        bound = schema.limit(case['originalLimit']); policy = case['policy']
        require(policy in ('phased', 'atomic', 'unmetered'), 'Unknown native policy')
        seed = case['seed']; injected = seed.get('kind') == 'explicit_counter_injection'
        fields(seed, 'kind frames pending dispatch expanded calls' + (' reachableFromNormalProgram' if injected else ''))
        require(seed['kind'] in ('normal', 'explicit_counter_injection'), 'Unsupported injected state')
        require(seed['frames'] == [] and seed['pending'] is None and seed['calls'] == '0', 'Invalid initial frame seed')
        expected_dispatch = str(schema.MAX64) if injected else '0'
        require(seed['dispatch'] == seed['expanded'] == expected_dispatch, 'Unexpected injected counters')
        if injected: require(seed['reachableFromNormalProgram'] is False, 'Injected state pretends normal reachability')
        initial = case['initial']
        expected = {'policy': policy, 'originalLimit': case['originalLimit'], 'rootInputs': inputs, 'poisoned': False,
                    'counters': {k: seed[k] for k in ('dispatch', 'expanded', 'calls')},
                    'terminal': None, 'frames': [], 'pending': None}
        require(initial == expected, 'Native initial state differs from source/input/seed')
        schema.snapshot(initial, source, policy=policy, original_limit=bound, context='idle')
        if not custom:
            require(case['association'] == {'program': authority['program'], 'semantics': authority['semantics'],
                'initialInputOwner': authority['input'], 'policy': policy, 'originalLimit': case['originalLimit'],
                'normalInitialState': not injected}, 'Association does not name actual retained authority')
        code_name = f'program_{index}'
        claims.lines.append(f'def {code_name} : FiniteCode.Code Primitive Nat := {lean.code(source)}')
        current, work, callbacks = initial, 0, []
        services = sequence(case['services']); require(bool(services), 'No observed native services')
        for service_index, service in enumerate(services):
            require(service['id'] == str(service_index), 'Service order changed')
            require(service['budgetOwner'] not in owners, 'Fresh service reused a budget identity')
            owner(service['budgetOwner'], 'budget', (identity, service_index))
            require(service['before'] == current, 'Retained state replaced between services')
            current, work, observed = protocol.Service(service, source, code_name, work, claims).execute()
            callbacks.extend(observed); total_events += len(service['trace'])
        result = case['terminalReport']; last = services[-1]['returned']
        if result is not None:
            require(last['kind'] != 'unwind', 'Fabricated report after unwind')
            protocol.check_report(result)
            reason = last['reason'] if last['kind'] == 'suspended' else None
            pause = 'none' if reason is None else '(some .' + lean.pause(reason) + ')'
            claims.claim(f'RetainedNative.finish {lean.state(current, work)} {pause} = {lean.report(result)}')
        else:
            require(last['kind'] in ('suspended', 'unwind'), 'Missing report for native completion')
        if 'finalPause' in case:
            expected_pause = last['reason'] if result is not None and last['kind'] == 'suspended' else None
            require(case['finalPause'] == expected_pause, 'Final pause differs from consuming report')
        case_results[identity] = (case, current, callbacks)
    require(bool(case_results), 'No native cases')
    for pair in sequence(data['associations']):
        fields(pair, 'whole sliced premises admittedService')
        require(pair['whole'] in case_results and pair['sliced'] in case_results and pair['whole'] != pair['sliced'], 'Unknown execution pair')
        whole, whole_state, whole_callbacks = case_results[pair['whole']]
        sliced, sliced_state, sliced_callbacks = case_results[pair['sliced']]
        require(whole.get('association') == sliced.get('association') == pair['premises'], 'Whole/sliced authority premises differ')
        require(whole['initial'] == sliced['initial'], 'Whole/sliced initial states differ')
        require(pair['admittedService'] == ('sufficient_atomic_grants' if whole['policy'] == 'atomic' else 'sufficient'), 'Unsupported service sufficiency claim')
        require(whole_state == sliced_state and whole_callbacks == sliced_callbacks, 'Whole/sliced retained state or ordered callbacks differ')
        require(whole_state['terminal'] is not None and whole_state['terminal']['stop'] == 'complete', 'Incomplete pair claimed completed equality')
        require(whole['terminalReport'] == sliced['terminalReport'], 'Associated terminal reports differ')
    return claims.text(), {'cases': len(case_results), 'associations': len(data['associations']),
        'native_events': total_events, 'claims': claims.count}


def main():
    replay_main(emit=emit, modules=MODELS,
        producers=[Path(__file__).resolve(), Path(schema.__file__), Path(lean.__file__), Path(protocol.__file__)],
        description=__doc__, scope='Finite original shared-machine traces for named u64 adapters, exact gates, '
        'active guards, callback results, retained states, interrupted panic charges and owner associations. '
        'Not Invocation binding/root-output, authenticated pointers, arbitrary adapters/allocators or universal Rust refinement.')


if __name__ == '__main__': main()
