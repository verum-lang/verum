#!/usr/bin/env python3
"""Check frozen finite declaration cardinality and Uniform paid-look pricing.

This checks declaration validity before projecting arities for the census.
Primitive/literal equality uses the source-bound Bit/Sequence fixture codec.
Code-word decoding, materialization, native enumeration order, VM execution and
resource accounting are not certified here. Native source/owner fidelity and
the association between the declared family and acquired candidate are premises.
"""
import json
from pathlib import Path
import re
from observer_export_replay import (array, fields, natural, require, sequence,
    boolean, variant, load, module_closure, main as replay_main)

MODULES=module_closure(['CompositionalFamily'])

def exact(value):
    require(type(value) is str and re.fullmatch(r'0|[1-9][0-9]*',value),'Noncanonical exact integer')
    return value

def primitive(value):
    # The frozen producer serializes these three distinct enum constructors.
    # Unknown vocabularies need an explicit source-bound codec, not an arity guess.
    require(type(value) is str and value in ('Zero','Identity','Xor'),'Unknown fixture primitive')
    return json.dumps(value)

def literal(value):
    kind,payload=variant(value)
    if kind=='Bit':return f'(Sum.inl {boolean(payload)})'
    require(kind=='Sequence','Unknown fixture literal')
    return '(Sum.inr '+array(boolean(bit) for bit in sequence(payload))+')'

def program(value,depth=0):
    """Decode the complete captured Program type; do not assert it executes."""
    require(depth<=64,'Captured program exceeds extraction depth')
    fields(value,'steps output library')
    if value['output'] is not None:natural(value['output'])
    for step in sequence(value['steps']):
        fields(step,'operation arguments');kind,payload=variant(step['operation'])
        if kind=='Primitive':primitive(payload)
        elif kind=='Branch':
            fields(payload,'yes no');natural(payload['yes']);natural(payload['no'])
        else:
            # compositional_courts::export supplies this codec to Owners::snapshot,
            # including captured calls: Fold is scalar here, unlike generic serde.
            require(kind in ('Call','Fold'),'Unknown captured operation');natural(payload)
        for argument in sequence(step['arguments']):
            kind,payload=variant(argument)
            if kind=='Literal':literal(payload)
            else:
                require(kind in ('Input','Register'),'Unknown captured operand');natural(payload)
    for child in sequence(value['library']):program(child,depth+1)

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
        literals=[literal(v) for v in sequence(value['literals'])];operations=[];calls=[];controls=[]
        require(int(inputs)+max(0,int(steps)-1)+len(literals) < 2**64,'Operand pool exceeds native indices')
        for op in sequence(value['operations']):
            fields(op,'primitive arity')
            operations.append(f'({primitive(op["primitive"])},{natural(op["arity"])})')
        for call in sequence(value['calls']):
            fields(call,'arity program');program(call['program']);calls.append(natural(call['arity']))
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
        declarations.append(f'(checkedDeclaration {identity} {inputs} {steps} '
            f'({array(operations)} : List (String × Nat)) '
            f'({array(literals)} : List (Sum Bool (List Bool))) {array(calls)} {array(controls)})')
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
        f'def declarations : Option (List Declaration) := ({array(declarations)} : List (Option Declaration)).mapM id',
        f'def roots : List Nat := {array(roots)}',
        'def family : Option Nat := do\n  let declarations ← declarations\n  let signatures ← census [] declarations\n'
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
