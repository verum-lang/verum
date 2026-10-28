#!/usr/bin/env python3
"""Check frozen finite declaration cardinality and Uniform paid-look pricing.

This projects only the source declaration from the complete acquisition export.
Code-word decoding, materialization, native enumeration order, VM execution and
resource accounting are not certified here. Native source/owner fidelity and
the association between the declared family and acquired candidate are premises.
"""
import json
from pathlib import Path
import re
from observer_export_replay import (array, fields, natural, require, sequence,
    load, module_closure, main as replay_main)

MODULES=module_closure(['CompositionalFamily'])

def exact(value):
    require(type(value) is str and re.fullmatch(r'0|[1-9][0-9]*',value),'Noncanonical exact integer')
    return value

def emit(data):
    fields(data,'packets');packets=sequence(data['packets'])
    require(len(packets)==1,'Expected one acquired joint family')
    packet=packets[0]
    fields(packet,'format parts family_names charged_comparisons look index training validation installed_probe_lengths scope')
    require(packet['format']=='synarc-compositional-acquisition-v2','Unsupported acquisition schema')
    require(type(packet['scope']) is str and packet['scope'],'Missing scope')
    parts=sequence(packet['parts']);require(len(parts)==3,'Missing joint family component')
    declarations=[];owners={};active=set();control_count=0

    def language(value,depth=0):
        nonlocal control_count
        require(depth<=64,'Declaration tree exceeds extraction depth')
        modern='version' in value
        fields(value,'owner inputs extent operations literals calls'+(' version controls' if modern else ''))
        if modern:require(type(value['version']) is int and value['version']==2,'Unknown language version')
        identity=natural(value['owner']);payload=json.dumps(value,sort_keys=True,separators=(',',':'))
        require(identity not in active,'Cyclic declaration owner')
        if identity in owners:
            require(owners[identity]==payload,'Retained declaration owner changed')
            return identity
        active.add(identity)
        fields(value['extent'],'kind steps')
        require(value['extent']['kind']=='Finite','Uniform source requires a finite declaration')
        inputs=natural(value['inputs']);steps=natural(value['extent']['steps'])
        literals=len(sequence(value['literals']));arities=[];controls=[]
        require(int(inputs)+max(0,int(steps)-1)+literals < 2**64,'Operand pool exceeds native indices')
        for op in sequence(value['operations']):
            fields(op,'primitive arity');arities.append(natural(op['arity']))
        for call in sequence(value['calls']):
            fields(call,'arity program');arities.append(natural(call['arity']))
        for control in sequence(value.get('controls',[])):
            require(type(control) is dict and 'kind' in control,'Missing control kind')
            if control['kind']=='Branch':
                fields(control,'kind yes no')
                yes,no=language(control['yes'],depth+1),language(control['no'],depth+1)
                controls.append(f'(.branch {yes} {no})')
            else:
                fields(control,'kind body');require(control['kind']=='Fold','Unknown control')
                controls.append(f'(.fold {language(control["body"],depth+1)})')
            control_count+=1
        declarations.append(f'⟨{identity},{inputs},{literals},{steps},{array(arities)},{array(controls)}⟩')
        active.remove(identity);owners[identity]=payload
        return identity

    roots=[]
    for part in parts:
        fields(part,'language address word decoded_address consumed_bits materialized decoded_materialized audit')
        roots.append(language(part['language']))
    family=natural(packet['family_names']);charged=natural(packet['charged_comparisons'])
    look,index=exact(packet['look']),exact(packet['index'])
    lines=['import VerumExternalReplay.CompositionalFamily','open CompositionalFamily',
        'set_option warningAsError true','set_option maxRecDepth 4096','namespace FrozenObserverExport',
        f'def declarations : List Declaration := {array(declarations)}',
        f'def roots : List Nat := {array(roots)}',
        'def family : Option Nat := do\n  let signatures ← census [] declarations\n'
        '  let counts ← roots.mapM (fun owner => (signatures.find? (fun p => p.owner = owner)).map Signature.names)\n'
        '  pure counts.prod']
    claims=[f'family = some {family}',f'0 < ({family} : Nat)',f'({index} : Nat) < {family}',
        f'({charged} : Nat) = {family} * ({look}+1) * ({look}+2)']
    lines.extend(f'theorem c{i} : {p} := by decide_cbv' for i,p in enumerate(claims))
    lines.append('end FrozenObserverExport')
    return '\n\n'.join(lines)+'\n',dict(declarations=len(declarations),controls=control_count,
        components=len(roots),claims=len(claims))

if __name__=='__main__':
    replay_main(emit=emit,modules=MODULES,producers=[Path(__file__).resolve()],
        description=__doc__,scope=__doc__,export_loader=lambda path: {'packets':[load(path)]})
