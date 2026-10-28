#!/usr/bin/env python3
"""Replay a frozen recipient transfer, its histories and exact categorical scores."""
from pathlib import Path
import check_lifecycle_export as lifecycle
from check_joint_export import decimal, exact_integer, receipt
from observer_export_replay import array, boolean, fields, natural, require, sequence, main as replay_main
from retained_transfer_codec import decode_letter


def ratio(value):
    fields(value, 'numerator denominator')
    n, d = (decimal(value[k]) for k in ('numerator', 'denominator'))
    require(int(d) > 0, 'Zero exact denominator')
    return f'⟨{n}, {d}⟩'


def score(value):
    fields(value, 'compression predictive samples')
    return f'⟨{ratio(value["compression"])}, {ratio(value["predictive"])}, {natural(value["samples"])}⟩'


def emit(data):
    fields(data, 'schema phase letter donor_definition recipient_definition initial_program candidate '
        'definition_receipt training validation joint_exact_score confirmed confirmation_rule installation_history installed_cut')
    require(data['schema'] == 'synarc-recipient-code-transfer-v2', 'Support-complete v2 export required')
    natural(data['phase'])
    require(data['confirmation_rule'] == 'joint_compression_strictly_above_training_then_joint_family_decide_v1',
        'Unknown confirmation rule')
    donor, recipient = data['donor_definition'], data['recipient_definition']
    base = dict(schema='synarc-observer-lifecycle-v1', definitions=[donor, recipient],
                ownership=[], naming=[], installation=[], refusals=[])
    require(lifecycle.validate_adapter(base) == 'AcquiredV2', 'Explicit AcquiredV2 required')
    source, _ = lifecycle.emit(base)
    lines = ['import VerumExternalReplay.CategoricalScore', source.removesuffix('end FrozenObserverExport\n')]
    names = []
    def claim(equation, proof='rfl'):
        name = 'transfer_' + str(len(names)); names.append(name)
        lines.append(f'theorem {name} : {equation} := by\n  {proof}')
    def same(a, b):
        claim(f'({exact_integer(a)} : Nat) = {exact_integer(b)}')
    def value_eq(a, b):
        claim(f'({lifecycle.value(a)} : Value) = {lifecycle.value(b)}')
    def program_eq(a, b):
        claim(f'({lifecycle.code(a)} : FiniteCode.Code Op Value) = {lifecycle.code(b)}')

    portable = decode_letter(data['letter'])
    for d in (donor, recipient):
        same(d['semantics']['max_trace'], portable['max_trace'])
        for phase in ('update', 'read'):
            program_eq(d[phase], portable[phase + '_program'])
            b = d[phase + '_binding']
            require(b['width'] == 1 and b['slots'] == portable[phase + '_slots'], 'Portable binding mismatch')
        value_eq(d['initial'], recipient['initial'])
    program_eq(data['initial_program'], portable['initial_program'])
    claim(f'({natural(donor["identity"])} : Nat) ≠ {natural(recipient["identity"])}', 'decide')
    owners = []
    for d in (donor, recipient):
        u, r = (d[p + '_binding']['interface'] for p in ('update', 'read'))
        require(u['shape'] == 'Event' and r['shape'] == 'Query', 'Wrong binding interface shape')
        require(u['owner'] is not None and r['owner'] == u['owner'], 'Missing or inconsistent lineage')
        require(u['epoch'] == r['epoch'] and u['raw_space'] == r['raw_space'], 'Inconsistent interface coordinates')
        owners.append(u['owner'])
    claim(f'({lifecycle.lineage(owners[0])} : Lineage) ≠ {lifecycle.lineage(owners[1])}', 'decide')
    prefix = 'd' + natural(recipient['identity']); cap = natural(recipient['semantics']['max_trace'])
    lines.append(f'def transfer_initial : FiniteCode.Code Op Value := {lifecycle.code(data["initial_program"])}')
    claim(f'FiniteCode.run (fun op args => (apply {cap} op args).toOption) transfer_initial [] = some {prefix}_initial',
          '(conv => lhs; cbv) <;> rfl')

    candidate = data['candidate']
    fields(candidate, 'owner definition grammar index syntax_indices family_count evidence_look comparison_count provenance '
                      'structural_code_words structural_witnesses declared_triples')
    grammar = fields(candidate['grammar'], 'owner authority count component_counts source_order admission_profile')
    require(grammar['source_order'] == 'declared-correlated-v1', 'Only declared correlated families supported')
    require(grammar['component_counts'] == [None]*3 and candidate['structural_code_words'] is None and
            candidate['structural_witnesses'] is None, 'Mixed family representations')
    triples = sequence(candidate['declared_triples'])
    require(triples, 'Empty correlated family')
    for triple in triples:
        require(len(sequence(triple)) == 3, 'Expected initializer/update/read triple')
        for program in triple: lifecycle.code(program)
    idx = int(exact_integer(candidate['index']))
    require(idx < len(triples), 'Correlated index out of range')
    for index in sequence(candidate['syntax_indices']): natural(index)
    require(candidate['syntax_indices'] == [idx]*3, 'Mixed correlated component indices')
    for actual, expected in zip(triples[idx], [data['initial_program'], recipient['update'], recipient['read']]):
        program_eq(actual, expected)
    same(candidate['family_count'], len(triples)); same(grammar['count'], len(triples))
    same(candidate['definition'], recipient['identity'])
    origin = receipt(candidate['provenance'], True); sealed = receipt(data['definition_receipt'], True)
    require(origin == sealed, 'Definition receipt differs from producer provenance')
    same(origin['authority'], grammar['authority']); same(origin['owner'], grammar['owner'])
    same(origin['count'], candidate['family_count']); same(origin['index'], candidate['index'])
    same(origin['look'], candidate['evidence_look']); same(origin['comparison_count'], candidate['comparison_count'])
    look = exact_integer(candidate['evidence_look'])
    count = len(triples)
    require(origin['name_prior'] == f'Uniform {{ original_cardinality: {count} }}', 'Correlated uniform name mismatch')
    claim(f'({exact_integer(candidate["comparison_count"])} : Nat) = {count}*({look}+1)*({look}+2)', 'decide')

    def history(item):
        fields(item, 'origin events'); fields(item['origin'], 'epoch lineage ordinal')
        o = item['origin']; lifecycle.wide(o['epoch']); lifecycle.lineage(o['lineage']); natural(o['ordinal'])
        require(o['lineage'] == owners[1] and o['epoch'] == recipient['update_binding']['interface']['epoch'],
                'Foreign history origin')
        result = []
        for row in sequence(item['events']):
            src, values = lifecycle.input_row(row)
            require(row['interface'] == recipient['update_binding']['interface'], 'Foreign history interface')
            require(len(row['values']) == 1 and row['values'][0]['kind'] == 'Event', 'Observed event required; gaps unsupported')
            v = row['values'][0]['value']
            require(all(v[k] == row['interface'][k] for k in ('owner','epoch','raw_space')), 'Event/source mismatch')
            result.append(f'({src}, {values})')
        return result
    training, fresh = data['training'], data['validation']
    for part in (training, fresh):
        fields(part, 'alphabets exact_score history named_mass_denominator naming profile purchased rows selection_mass_denominator')
    histories = [history(p['history']) for p in (training, fresh)]
    installed_history = history(data['installation_history'])
    all_histories = [training['history'], fresh['history'], data['installation_history']]
    for old, new in zip(all_histories, all_histories[1:]):
        require(old['origin'] == new['origin'] and new['events'][:len(old['events'])] == old['events'],
                'History origin or immutable prefix changed')
    lines.append(f'def transfer_fold (events : List (Source × List Value)) := PartialObservers.fold '
                 f'(fun state event => run {cap} {prefix}_update {prefix}_update_binding state event.1 event.2) events {prefix}_initial')

    def contexts(alphabets, rows):
        supports = {}; grouped = {}; reading_ids = {}
        for alphabet in sequence(alphabets):
            fields(alphabet, 'context symbols'); ctx = context(alphabet['context'])
            require(ctx not in supports, 'Duplicate alphabet context')
            symbols = [lifecycle.value(s) for s in sequence(alphabet['symbols'])]
            require(symbols and len(set(symbols)) == len(symbols), 'Empty or duplicate alphabet')
            supports[ctx] = symbols; grouped[ctx] = []
        for row in rows:
            ctx = context(row['context']); require(ctx in supports, 'Missing frozen support')
            symbol = lifecycle.value(row['outcome']); require(symbol in supports[ctx], 'Outcome outside frozen alphabet')
            reading = lifecycle.value(row['reading'])
            if reading not in reading_ids: reading_ids[reading] = len(reading_ids)
            grouped[ctx].append(f'⟨{reading_ids[reading]}, {supports[ctx].index(symbol)}⟩')
        return array(f'⟨{len(supports[ctx])}, {array(grouped[ctx])}⟩' for ctx in supports)
    def context(c):
        require(len(sequence(c)) == 3, 'Expected epoch/raw/action context')
        return tuple(decimal(n) for n in c)

    cuts = []
    for label, part, entries in zip(('training','fresh'), (training, fresh), histories):
        profile = fields(part['profile'], 'kind risk_denominator compression predictive')
        require(profile['kind'] == 'ExactPredictiveV1' and
                profile['compression'] == 'strict_child_kt_over_parent_kt_gt_selection_denominator' and
                profile['predictive'] == 'strict_child_kt_over_parent_ml_gt_selection_times_risk_denominator', 'Unsupported exact profile')
        risk = decimal(profile['risk_denominator']); require(int(risk)>0, 'Zero risk denominator')
        fields(part['naming'], 'kind original_cardinality')
        require(part['naming']['kind'] == 'Uniform', 'Correlated family requires uniform naming')
        for n in (part['named_mass_denominator'],part['naming']['original_cardinality']):
            claim(f'({decimal(n)} : Nat) = {count}')
        selection = decimal(part['selection_mass_denominator'])
        claim(f'({selection} : Nat) = {exact_integer(candidate["comparison_count"])}')
        require(grammar['admission_profile'] == f'ExactPredictiveV1 {{ risk: Mass {{ denominator: {risk} }} }}', 'Profile/debug receipt mismatch')
        part_cuts = []
        for row in sequence(part['rows']):
            fields(row, 'context cut outcome query reading'); n = int(natural(row['cut']))
            require(n <= len(entries), 'Cut beyond retained history')
            part_cuts.append(n)
            src, values = lifecycle.input_row(row['query'])
            require(row['query']['interface'] == recipient['read_binding']['interface'], 'Foreign query interface')
            for v in row['query']['values']:
                require(v['kind'] == 'Query', 'Query value required')
                require(all(v['value'][k] == row['query']['interface'][k] for k in ('owner','epoch','raw_space')), 'Query/source mismatch')
            claim(f'(transfer_fold {array(entries[:n])}).bind (fun state => '
                  f'run {cap} {prefix}_read {prefix}_read_binding state {src} {values}) = some {lifecycle.value(row["reading"])}',
                  '(conv => lhs; cbv) <;> rfl')
        require(part_cuts and all(a < b for a,b in zip(part_cuts,part_cuts[1:])), 'Cuts must be nonempty and strictly ordered')
        cuts.append(part_cuts)
        lines.append(f'def {label}_contexts : List CategoricalScore.Context := {contexts(part["alphabets"],part["rows"])}')
        lines.append(f'def {label}_score : CategoricalScore.Score := {score(part["exact_score"])}')
        claim(f'CategoricalScore.agrees {label}_contexts {label}_score = true', 'decide')
        claim(f'CategoricalScore.purchased {label}_score {selection} {risk} = {boolean(part["purchased"])}', 'decide')
    require(min(cuts[1]) > max(cuts[0]) and min(cuts[1]) > len(histories[0]), 'Fresh evidence overlaps training history')
    for k in ('alphabets','naming','profile','named_mass_denominator','selection_mass_denominator'):
        require(training[k] == fresh[k], 'Fresh family/support differs from training')
    lines.append('def joint_contexts : List CategoricalScore.Context := ' + contexts(training['alphabets'],training['rows']+fresh['rows']))
    lines.append('def joint_score : CategoricalScore.Score := ' + score(data['joint_exact_score']))
    claim('CategoricalScore.agrees joint_contexts joint_score = true', 'decide')
    claim(f'CategoricalScore.confirmed training_score joint_score {selection} {risk} = {boolean(data["confirmed"])}', 'decide')
    cut = data['installed_cut']; lifecycle.cut(cut)
    same(cut['definition'], recipient['identity'])
    # Instance ownership is an observed nominal token, not the family owner.
    natural(cut['owner']); natural(candidate['owner'])
    claim(f'transfer_fold {array(installed_history)} = some {lifecycle.value(cut["value"])}', '(conv => lhs; cbv) <;> rfl')
    lines.extend(['#print axioms ' + n for n in names] + ['end FrozenObserverExport'])
    return '\n\n'.join(lines)+'\n', {'claims':len(names), 'reading_replays':sum(map(len,cuts)),
        'history_events':len(installed_history), 'exact_scores':3, 'portable_programs':3}


def main():
    replay_main(emit=emit, modules=lifecycle.MODELS+['CategoricalScore'],
        producers=[Path(__file__).resolve(), Path(lifecycle.__file__).resolve(),
                   Path(__file__).with_name('check_joint_export.py'), Path(__file__).with_name('retained_transfer_codec.py')],
        description=__doc__, scope='Original v2 transfer: retained portable code and correlated family, nominal lineage separation, '
        'initialization, immutable history prefixes, fresh ordered cuts, observed reads and installed value, exact KT/ML scores '
        'with frozen support and strict paid-family decisions. Exported contexts/outcomes are inputs; their environment provenance, '
        'physical ownership allocation, audit/resource accounting, native terminal guards, ingress parser, universal native refinement '
        'and statistical optional-stopping validity are not certified.')

if __name__ == '__main__': main()
