#!/usr/bin/env python3
"""Bind original invocation observations to binding, machine and output models.

Correspondence is not a certificate that the observed API contract is desirable.
Non-poisoned terminal retries are counted explicitly as a known native contract
defect, not silently reclassified or reexecuted.
"""
from pathlib import Path
from observer_export_replay import fields, require, sequence, main as replay_main
import retained_execution_snapshot as schema
import retained_execution_encoding as lean
import retained_execution_protocol as machine
import retained_invocation_protocol as wrapper
from check_retained_execution_export import MODELS as MACHINE_MODELS

MODELS = [*MACHINE_MODELS, 'ObserverAdmission']


def emit(data):
    fields(data, 'schema scope fixtures associations cases')
    require(data['schema'] == 'synarc-shared-vm-native-v1' and data['scope'] == 'invocation', 'Unsupported invocation schema/scope')
    claims = machine.Claims(); claims.lines.insert(1, 'import VerumExternalReplay.ObserverAdmission')
    fixtures, owners, cases = {}, {}, {}
    total_events, services, false_poison = 0, 0, 0

    def owner(key, kind, payload, fresh=False):
        schema.nat(key)
        require(not fresh or key not in owners, 'Fresh wrapper allocation reuses a retained owner')
        require(key not in owners or owners[key] == (kind, payload), 'Wrapper owner changed kind or immutable payload')
        owners[key] = (kind, payload)

    for f in sequence(data['fixtures']):
        fields(f, 'id initial update read updateCodeOwner readCodeOwner semanticsOwner updateBinding readBinding semantics')
        require(f['semantics'] == 'u64-checked-v1', 'Unknown wrapper semantics')
        require(f['id'] not in fixtures, 'Duplicate definition owner'); schema.nat(f['initial'])
        require(f['readCodeOwner'] != f['updateCodeOwner'], 'Definition constructor code allocations aliased')
        for phase in ('update', 'read'):
            source = schema.program(f[phase]); wrapper.binding(f[phase + 'Binding'])
            owner(f[phase + 'CodeOwner'], 'program', source)
        owner(f['id'], 'definition', f); owner(f['semanticsOwner'], 'semantics', f['semantics'])
        fixtures[f['id']] = f
    require(bool(fixtures), 'No wrapper definitions')
    for index, c in enumerate(sequence(data['cases'])):
        fields(c, 'id phase association initial services final')
        require(type(c['id']) is str and bool(c['id']) and c['id'] not in cases, 'Duplicate wrapper case')
        require(c['phase'] in ('read', 'update'), 'Invalid wrapper phase')
        initial = c['initial']; fields(initial, 'owner phase poisoned finished originalLimit inspected bound cut definition input invocation')
        cut = initial['cut']; fields(cut, 'definition instance state value')
        require(cut['definition'] in fixtures, 'Unbound native Cut definition')
        fixture = fixtures[cut['definition']]
        require(cut['value'] == fixture['initial'], 'Initial Cut differs from actual initializer')
        owner(cut['instance'], 'instance', cut['definition']); owner(cut['state'], 'state', (cut['definition'], cut['value']))
        external = initial['input']; fields(external, 'owner interface values'); schema.nat(external['interface']); schema.vector(external['values'])
        owner(external['owner'], 'owned_input', (external['interface'], external['values']))
        owner(initial['owner'], 'call', c['id'], fresh=True)
        expected_definition = {'updateCode': fixture['updateCodeOwner'], 'readCode': fixture['readCodeOwner'],
            'semantics': fixture['semanticsOwner'], 'updateBinding': fixture['updateBinding'], 'readBinding': fixture['readBinding']}
        require(initial == {'owner': initial['owner'], 'phase': c['phase'], 'poisoned': False, 'finished': False,
            'originalLimit': None, 'inspected': None, 'bound': [], 'cut': cut, 'definition': expected_definition,
            'input': external, 'invocation': None}, 'Invalid initial retained call')
        source = schema.program(fixture[c['phase']]); code_name = f'wrapper_program_{index}'
        claims.lines.append(f'def {code_name} : FiniteCode.Code Primitive Nat := {lean.code(source)}')
        runner = wrapper.Wrapper(c, fixture, source, code_name, claims, owner)
        rows = sequence(c['services']); require(bool(rows), 'No wrapper services')
        for n, s in enumerate(rows):
            require(s['id'] == str(n), 'Wrapper service order changed')
            owner(s['budgetOwner'], 'budget', (c['id'], n), fresh=True)
            result = runner.service_run(s)
            if result.get('refusal', {}).get('classification') == 'Poisoned':
                before = s['before']; inv = before['invocation']
                poisoned = before['poisoned'] or s['budgetBefore']['poisoned'] or (inv is not None and inv['poisoned'])
                if not poisoned: false_poison += 1
            total_events += len(s['trace']); services += 1
        require(runner.current == c['final'], 'Final retained wrapper state differs')
        require(c['association'] == {'cut': initial['cut'], 'input': initial['input'], 'definition': initial['definition'],
            'phase': c['phase'], 'originalLimit': c['final']['originalLimit']}, 'Wrapper association changed actual authority')
        cases[c['id']] = (c, runner.callbacks)
    for pair in sequence(data['associations']):
        fields(pair, 'whole sliced premises admittedService')
        require(pair['whole'] in cases and pair['sliced'] in cases and pair['whole'] != pair['sliced'], 'Unknown wrapper pair')
        a, ac = cases[pair['whole']]; b, bc = cases[pair['sliced']]
        require(a['association'] == b['association'] == pair['premises'], 'Wrapper pair authority/binding differs')
        require(pair['admittedService'] == 'sufficient_phased_unit_grants', 'Unknown wrapper service premise')
        require(ac == bc, 'Original wrapper callback streams differ')
        require(a['services'][-1]['returned'] == b['services'][-1]['returned'] and a['services'][-1]['returned']['kind'] == 'complete',
                'Wrapper pair lacks equal completed readings/preparations')
    return claims.text(), {'cases': len(cases), 'associations': len(data['associations']), 'services': services,
        'native_events': total_events, 'claims': claims.count, 'nonpoisoned_terminal_retries': false_poison}


def main():
    replay_main(emit=emit, modules=MODELS, producers=[Path(__file__).resolve(), Path(schema.__file__),
        Path(lean.__file__), Path(machine.__file__), Path(wrapper.__file__),
        Path(__file__).with_name('check_retained_execution_export.py')], description=__doc__,
        scope='Finite original wrapper observations: actual Cut/input/code/semantics owners, paid binding prefixes, '
        'nested machine replay, body retention, root selection, original allowance and per-service dispatch billing. '
        'Nonpoisoned terminal retries are separately reported native contract defects; correspondence does not certify '
        'their resumability, arbitrary callbacks, allocation, unobserved lifecycle operations or universal Rust correctness.')


if __name__ == '__main__': main()
