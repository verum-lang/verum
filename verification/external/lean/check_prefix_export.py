#!/usr/bin/env python3
"""Check frozen structural words, lexical catalogs and exact retained prices.

This consumer never executes recursive programs or authenticates physical
pointers. It reports which controls occur in the selected addresses; declaring
a catalog entry does not count as exercising it.
"""
import argparse
from collections import Counter
import json
from pathlib import Path
import re
import uuid
import check_joint_export

from observer_export_replay import array, atomic_report, fields, natural, require, sequence, verify
from check_joint_export import exact_integer


def nat(value):
    return natural(value)


def decimal(value):
    require(type(value) is str and re.fullmatch(r'0|[1-9][0-9]*', value) and len(value) <= 4096,
            'Expected exact canonical decimal')
    return value


def option(value):
    return 'none' if value is None else '(some '+nat(value)+')'


def atom(value):
    # Exact captured payload identity only, not an interpretation of primitives
    # or literal semantics. Duplicate JSON keys/non-JSON constants are refused
    # by the common frozen-export loader before this canonical serialization.
    return json.dumps(json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False), ensure_ascii=False)


def program(value):
    fields(value, 'steps output library')
    steps = []
    for step in sequence(value['steps']):
        fields(step, 'operation arguments')
        op = step['operation']
        require(type(op) is dict and len(op) == 1, 'Unknown program operation')
        kind, payload = next(iter(op.items()))
        if kind == 'Primitive':
            require(type(payload) is str, 'Expected primitive identity')
            encoded = '.primitive '+json.dumps(payload)
        elif kind == 'Call': encoded = '.call '+nat(payload)
        elif kind == 'Branch':
            fields(payload, 'yes no'); encoded = '.branch '+nat(payload['yes'])+' '+nat(payload['no'])
        elif kind == 'Fold':
            fields(payload, 'body'); encoded = '.fold '+nat(payload['body'])
        elif kind == 'Recur':
            fields(payload, 'ancestor member'); encoded = '.recur '+nat(payload['ancestor'])+' '+option(payload['member'])
        else: raise ValueError('Unsupported program operation: '+kind)
        args = []
        for arg in sequence(step['arguments']):
            require(type(arg) is dict and len(arg) == 1, 'Malformed operand')
            kind, payload = next(iter(arg.items()))
            require(kind in ('Input', 'Register', 'Literal'), 'Unsupported operand')
            args.append('.'+kind.lower()+' '+(atom(payload) if kind == 'Literal' else nat(payload)))
        steps.append('⟨('+encoded+'), '+array(args)+'⟩')
    return '(.mk '+array(steps)+' '+option(value['output'])+' '+array(program(p) for p in sequence(value['library']))+')'


def address(value):
    fields(value, 'owner instructions output'+(' children' if 'children' in value else ''))
    instructions = []
    for i in sequence(value['instructions']):
        fields(i, 'alternative operands')
        instructions.append('⟨'+nat(i['alternative'])+', '+array(nat(x) for x in sequence(i['operands']))+'⟩')
    children = []
    for child in sequence(value.get('children', [])):
        fields(child, 'instruction side address')
        children.append('('+nat(child['instruction'])+', '+nat(child['side'])+', '+address(child['address'])+')')
    return '(.mk '+nat(value['owner'])+' '+array(instructions)+' '+nat(value['output'])+' '+array(children)+')'


class Catalog:
    def __init__(self):
        self.owners, self.encoded, self.active = {}, [], set()

    def register(self, g):
        require(type(g) is dict, 'Missing grammar')
        fields(g, 'owner inputs extent operations literals calls'+(' version controls' if 'version' in g else ''))
        owner = int(nat(g['owner']))
        require(owner not in self.active, 'Cyclic owned lexical catalog')
        if owner in self.owners:
            require(self.owners[owner] == g, 'Same owner changed its complete catalog')
            return owner
        self.active.add(owner)
        extent = g['extent']
        if extent.get('kind') == 'Finite':
            fields(extent, 'kind steps'); limit = '(some '+nat(extent['steps'])+')'
        else:
            fields(extent, 'kind'); require(extent['kind'] == 'AllFinite', 'Unknown extent'); limit = 'none'
        version = g.get('version', 1)
        require(type(version) is int and version in (1, 2, 3), 'Unknown codec version')
        ops, calls, controls = [], [], []
        for op in sequence(g['operations']):
            fields(op, 'arity primitive')
            require(type(op['primitive']) is str, 'Missing primitive identity')
            ops.append('('+json.dumps(op['primitive'])+', '+nat(op['arity'])+')')
        for call in sequence(g['calls']):
            fields(call, 'arity program'); calls.append('('+nat(call['arity'])+', '+program(call['program'])+')')
        requires_v3 = False
        for control in sequence(g.get('controls', [])):
            kind = control.get('kind')
            if kind == 'Branch':
                fields(control, 'kind yes no')
                left, right = control['yes'], control['no']
                require(left['inputs'] == right['inputs'] and left['inputs']+1 < 2**64, 'Invalid branch arity')
                children = [left, right]
                controls.append('.branch '+str(self.register(left))+' '+str(self.register(right)))
            elif kind == 'Fold':
                fields(control, 'kind body'); child = control['body']
                require(child['inputs'] >= 2, 'Invalid fold arity')
                children = [child]; controls.append('.fold '+str(self.register(child)))
            elif kind == 'Recur':
                fields(control, 'kind ancestor member arity'); children = []; requires_v3 = True
                controls.append('.recur '+nat(control['ancestor'])+' '+option(control['member'])+' '+nat(control['arity']))
            else: raise ValueError('Unsupported lexical control')
            for child in children:
                require(child['extent']['kind'] == 'Finite', 'Unbounded lexical child')
                requires_v3 |= child.get('version', 1) >= 3
        require((version == 1 and not controls) or version == (3 if requires_v3 else 2), 'Codec version/catalog mismatch')
        self.active.remove(owner); self.owners[owner] = g
        self.encoded.append('⟨'+', '.join([str(owner),nat(g['inputs']),limit,str(version),array(ops),
            array(atom(v) for v in sequence(g['literals'])),array(calls),array(controls)])+'⟩')
        return owner



class Claims:
    def __init__(self, catalog):
        self.lines = ['import VerumExternalReplay.PrefixSyntax', 'open PrefixSyntax',
                      'set_option warningAsError true', 'namespace FrozenObserverExport',
                      'def catalog : List Grammar := '+array(catalog.encoded)]
        self.count = 0
        self.fuel = len(catalog.owners)+1

    def claim(self, equation, proof='decide_cbv'):
        self.lines.append(f'theorem p{self.count} : {equation} := by {proof}')
        self.count += 1

    def witness(self, i, w, word, body, input_, start, end):
        fields(w, 'language address decoded_address word consumed_bits materialized decoded_materialized audit')
        owner = nat(w['language']['owner'])
        self.lines += [f'def address{i} : Address := '+address(w['address']),
                       f'def word{i} : List Nat := '+array(nat(b) for b in word)]
        self.claim(f'decode {self.fuel} catalog {owner} word{i} = some (address{i}, [])','rfl')
        self.claim(f'encode {self.fuel} catalog {owner} address{i} = some word{i}','rfl')
        self.claim(f'address{i} = ({address(w["decoded_address"])} : Address)','rfl')
        for value in (w['materialized'],w['decoded_materialized'],body):
            self.claim(f'materialize {self.fuel} catalog {owner} address{i} = some {program(value)}','rfl')
        self.claim(f'({array(nat(b) for b in sequence(w["word"]))} : List Nat) = word{i}')
        self.claim(f'{nat(w["consumed_bits"])} = word{i}.length')
        self.claim(f'({nat(end)} : Nat) = {nat(start)} + word{i}.length')
        self.claim(f'decode {self.fuel} catalog {owner} ({input_}.drop {nat(start)}) = some (address{i}, {input_}.drop {nat(end)})','rfl')
        audit = w['audit']; fields(audit,'allowance decode_work materialize_work decoded_materialize_work')
        for key in ('decode_work','materialize_work','decoded_materialize_work'):
            self.claim(f'(0 : Nat) < {nat(audit[key])} ∧ {nat(audit[key])} ≤ {nat(audit["allowance"])}')
        # Concrete bridge to the universal Verum mandatory-header lemma.
        self.claim(f'((lookup catalog {owner}).map (fun g => decide (1 + width (alternatives g) ≤ word{i}.length))) = some true')

    def finish(self, **counts):
        return '\n\n'.join(self.lines+['end FrozenObserverExport'])+'\n', {'claims':self.count, **counts}


def bitword(value):
    require(value and all(type(b) is int and b in (0,1) for b in sequence(value)), 'Invalid code word')
    return value


def emit(data, versions, required_controls=(), *, embedded=False):
    fields(data, 'schema scope '+('' if embedded else 'library_origin ')+'source_order language_versions candidate definition_receipt '
                 'concatenated_input prefix_receipts suffix_start suffix name_bits name_denominator look '
                 'look_denominator selection_denominator')
    require(data['schema'] == 'synarc-generated-prefix-source-v1', 'Unsupported source export')
    require(all(type(v) is int for v in sequence(data['language_versions'])) and data['language_versions'] == list(versions), 'Unexpected source codec versions')
    c = data['candidate']
    fields(c, 'comparison_count declared_triples definition evidence_look family_count grammar index owner '
              'provenance structural_code_words structural_witnesses syntax_indices')
    require(c['declared_triples'] is None and c['family_count'] is None, 'Prefix source repriced or replaced by declaration')
    nat(c['definition'])
    grammar = c['grammar']; fields(grammar, 'admission_profile authority component_counts count owner source_order')
    require(grammar['count'] is None and grammar['component_counts'] == [None,None,None], 'Uniform count replaced prefix price')
    require(grammar['source_order'] == {1:'structural-odometer-v1',2:'compositional-odometer-v2',3:'recursive-odometer-v3'}[max(versions)],
            'Source order/version mismatch')
    witnesses = sequence(c['structural_witnesses']); prefixes = sequence(data['prefix_receipts'])
    words = sequence(c['structural_code_words'])
    require(len(witnesses) == len(prefixes) == len(words) == 3, 'Missing I/U/R component')
    for word in words:
        bitword(word)
    catalog = Catalog()
    for witness, version in zip(witnesses, versions):
        fields(witness, 'language address decoded_address word consumed_bits materialized decoded_materialized audit')
        require(witness['language'].get('version',1) == version, 'Unexpected component version')
        catalog.register(witness['language'])
    controls = Counter()
    for w in witnesses: controls.update(e['kind'] for e in selected_events(catalog,w['language'],w['address']))
    require(set(required_controls) <= controls.keys(), 'Required selected lexical/recursive controls absent')
    # These are numeric scope identities, never proofs of native Arc equality.
    for receipt in (c['provenance'],data['definition_receipt']):
        fields(receipt, 'authority comparison_count count index look name_prior owner')
        require(receipt['count'] is None, 'Retained prefix receipt became uniform')
        for key in ('authority','owner'): nat(receipt[key])
        for key in ('index','look','comparison_count'): exact_integer(receipt[key])
    require(c['provenance'] == data['definition_receipt'], 'Retained code/price authority changed')
    for key in ('owner','index','comparison_count'):
        require(c[key] == c['provenance'][key], 'Candidate/receipt identity mismatch')
    require(grammar['authority'] == c['provenance']['authority'] and grammar['owner'] == c['owner'], 'Source authority mismatch')
    require(c['evidence_look'] == c['provenance']['look'], 'Paid look changed')
    require(len(sequence(c['syntax_indices'])) == 3, 'Missing syntax coordinate')
    for x in c['syntax_indices']: exact_integer(x)
    look = decimal(data['look']); name_den = decimal(data['name_denominator'])
    look_den = decimal(data['look_denominator']); selected_den = decimal(data['selection_denominator'])
    require(int(look) == int(exact_integer(c['evidence_look'])), 'Look ordinal changed')
    require(c['provenance']['name_prior'] == f'KraftProgramV{max(versions)} {{ bits: {data["name_bits"]} }}', 'Naming scheme/length changed')
    require(type(data['name_bits']) is int and 0 < data['name_bits'] <= 65536, 'Unsupported word extent')
    proofs = Claims(catalog)
    claim = proofs.claim
    proofs.lines.append('def input : List Nat := '+array(nat(b) for b in sequence(data['concatenated_input'])))
    offset = 0
    for i,(w,prefix,word) in enumerate(zip(witnesses,prefixes,words)):
        fields(prefix, 'part start end word decoded_body')
        require(prefix['part'] == i, 'Prefix components reordered')
        proofs.witness(i,w,word,prefix['decoded_body'],'input',prefix['start'],prefix['end'])
        claim(f'({array(nat(b) for b in sequence(prefix["word"]))} : List Nat) = word{i}')
        claim(f'({nat(prefix["start"])} : Nat) = {offset}')
        offset += len(word)
    claim('input = word0 ++ word1 ++ word2 ++ '+array(nat(b) for b in sequence(data['suffix'])))
    claim(f'({nat(data["suffix_start"])} : Nat) = word0.length + word1.length + word2.length')
    claim(f'({data["name_bits"]} : Nat) = word0.length + word1.length + word2.length')
    claim(f'({name_den} : Nat) = 2^{data["name_bits"]}')
    claim(f'({look_den} : Nat) = ({look}+1)*({look}+2)')
    claim(f'({selected_den} : Nat) = {name_den}*{look_den}')
    claim(f'({exact_integer(c["comparison_count"])} : Nat) = {selected_den}')
    return proofs.finish(catalogs=len(catalog.owners),components=3,versions=list(versions),selected_controls=dict(controls))


def selected_events(catalog, g, a, depth=0):
    result = []
    for instruction in a['instructions']:
        offset = instruction['alternative']-len(g['operations'])-len(g['calls'])
        if offset >= 0:
            require(offset < len(g.get('controls',[])), 'Foreign selected control')
            control = g['controls'][offset]
            result.append({'depth':depth, 'kind':control['kind'], **{k:control[k] for k in ('ancestor','member','arity') if k in control}})
    for child in a.get('children',[]):
        owner = child['address']['owner']
        require(owner in catalog.owners, 'Foreign child owner')
        result += selected_events(catalog,catalog.owners[owner],child['address'],depth+1)
    return result


def emit_components(data, version, required_controls=()):
    fields(data, 'schema scope library_origin source_order source component declaration_envelope language_version '
                 'next_look_before next_look_after visited_prefix_count enumeration_complete stop_reason required_kinds samples audit')
    require(data['schema'] == 'synarc-generated-prefix-components-v1', 'Unsupported component export')
    require(type(data['language_version']) is int and data['language_version'] == version, 'Unexpected component version')
    require(data['source_order'] == 'native-structural-component-enumeration', 'Unexpected component source order')
    require(data['enumeration_complete'] is False, 'Sampled prefix is not exhaustive coverage')
    require(data['component'] in (0,1,2) and type(data['component']) is int, 'Unknown I/U/R component')
    source = data['source']; fields(source,'epoch owner raw_space shape')
    decimal(source['epoch']); decimal(source['raw_space'])
    require(source['owner'] is None, 'Component samples are not a joint retained candidate')
    envelope = data['declaration_envelope']; fields(envelope,'body_inputs nesting recursive steps')
    for key in ('body_inputs','nesting','steps'): nat(envelope[key])
    require(type(envelope['recursive']) is bool and envelope['recursive'] == (version == 3), 'Envelope/version mismatch')
    required = sequence(data['required_kinds']); samples = data['samples']
    require(type(samples) is dict and set(samples) == set(required) and len(required) == len(set(required)), 'Incomplete or repeated specimen roster')
    require(set(required) == ({'branch_distinct_children','fold'} | ({'root_recur','nested_parent_recur'} if version == 3 else set())), 'Unknown specimen scope')
    catalog = Catalog()
    for kind, sample in samples.items():
        fields(sample, 'kind component_name_denominator component_ordinal name_bits native_work_at_selection prefix_check_work '
                       'prefix_receipt structural_witness visited_prefix_at_selection')
        require(sample['kind'] == kind, 'Sample relabeled')
        g = sample['structural_witness']['language']
        require(g.get('version',1) == version, 'Unexpected root catalog version')
        catalog.register(g)
    require(all(not g['calls'] for g in catalog.owners.values()), 'Enumeration specimen contains a fixture/imported call')
    proofs = Claims(catalog); claim = proofs.claim
    look0,look1 = decimal(data['next_look_before']),decimal(data['next_look_after'])
    claim(f'({look0} : Nat) = {look1}')
    visited = decimal(data['visited_prefix_count'])
    audit = data['audit']
    fields(audit,'maximum_retained_specimens native_allowance native_work selected_prefix_check_work structural_snapshot_work '
                 'total_native_work traversal_work visited_body_materialization_work')
    for value in audit.values(): nat(value)
    claim(f'{len(samples)} ≤ ({audit["maximum_retained_specimens"]} : Nat)')
    claim(f'({audit["native_work"]} : Nat) = {audit["traversal_work"]} + {audit["visited_body_materialization_work"]} + {audit["selected_prefix_check_work"]}')
    claim(f'({audit["total_native_work"]} : Nat) = {audit["native_work"]} + {audit["structural_snapshot_work"]}')
    claim(f'({audit["total_native_work"]} : Nat) ≤ {audit["native_allowance"]}')
    events = {}
    for i,(kind,sample) in enumerate(samples.items()):
        w,prefix = sample['structural_witness'],sample['prefix_receipt']
        fields(prefix,'decoded_body end input start suffix word')
        word = bitword(w['word']); input_ = f'input{i}'
        proofs.lines.append(f'def {input_} : List Nat := '+array(nat(b) for b in sequence(prefix['input'])))
        proofs.witness(i,w,word,prefix['decoded_body'],input_,prefix['start'],prefix['end'])
        claim(f'({array(nat(b) for b in bitword(prefix["word"]))} : List Nat) = word{i}')
        claim(f'{input_}.drop {nat(prefix["start"])} = word{i} ++ '+array(nat(b) for b in sequence(prefix['suffix'])))
        claim(f'{nat(prefix["start"])} ≤ {input_}.length')
        claim(f'({nat(sample["name_bits"])} : Nat) = word{i}.length')
        claim(f'({decimal(sample["component_name_denominator"])} : Nat) = 2^word{i}.length')
        ordinal, at_selection = decimal(sample['component_ordinal']),decimal(sample['visited_prefix_at_selection'])
        claim(f'({ordinal} : Nat) < {at_selection} ∧ {at_selection} ≤ {visited}')
        claim(f'(0 : Nat) < {nat(sample["native_work_at_selection"])} ∧ {nat(sample["native_work_at_selection"])} ≤ {audit["native_work"]}')
        claim(f'(0 : Nat) < {nat(sample["prefix_check_work"])} ∧ {nat(sample["prefix_check_work"])} ≤ {audit["selected_prefix_check_work"]}')
        events[kind] = selected_events(catalog,w['language'],w['address'])
        if kind == 'branch_distinct_children':
            children = w['address'].get('children',[])
            require(any(e['depth']==0 and e['kind']=='Branch' for e in events[kind]) and len(children)>=2
                    and children[0]['address'] != children[1]['address'], 'Distinct branch children not selected')
        elif kind == 'fold':
            require(any(e['depth']==0 and e['kind']=='Fold' for e in events[kind]), 'Fold not selected')
        elif kind == 'root_recur':
            require(any(e['depth']==0 and e['kind']=='Recur' for e in events[kind]), 'Root Recur not selected')
        else:
            require(any(e['depth']>0 and e['kind']=='Recur' and e['ancestor']>0 for e in events[kind]), 'Nested parent Recur not selected')
    controls = Counter(e['kind'] for row in events.values() for e in row)
    require(set(required_controls) <= controls.keys(), 'Required selected lexical/recursive controls absent')
    return proofs.finish(catalogs=len(catalog.owners),components=len(samples),versions=[version],selected_controls=dict(controls),
                         selected_events=events,joint_candidate_receipts=0,enumeration_complete=False)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('export','provenance','output','report'): parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--sha256',required=True)
    mode=parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--versions',type=int,nargs=3,choices=(1,2,3))
    mode.add_argument('--component-version',type=int,choices=(2,3))
    parser.add_argument('--require-controls',choices=('Branch','Fold','Recur'),nargs='+',default=[])
    args=parser.parse_args()
    invocation={'run_id':uuid.uuid4().hex,'requested_export_sha256':args.sha256,'versions':args.versions,'component_version':args.component_version,'required_controls':args.require_controls}
    require(len({p.resolve() for p in (args.export,args.provenance,args.output,args.report)})==4,'Output collision')
    atomic_report(args.report,{**invocation,'status':'running'})
    try:
        emitter = (lambda data:emit_components(data,args.component_version,args.require_controls)) if args.component_version else (lambda data:emit(data,args.versions,args.require_controls))
        report=verify(args,emit=emitter,modules=['PrefixSyntax'],
            producers=[Path(__file__).resolve(),Path(check_joint_export.__file__).resolve()],scope='Finite frozen codec/materialization/price observations. Selected control coverage is explicit. No recursive VM execution, generic Rust refinement, physical ownership, opening-bound integration or admission certificate.')
    except Exception as error:
        atomic_report(args.report,{**invocation,'status':'failed','error':str(error)[:8192]});raise
    atomic_report(args.report,{**invocation,'status':'verified',**report})
    print(json.dumps(report['counts']))

if __name__=='__main__':main()
