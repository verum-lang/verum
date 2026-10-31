#!/usr/bin/env python3
"""Replay actual owned VM/clock boundaries with an explicitly omitted prefix.

This checks complete finite carrier observations and each recorded handoff,
and independently proves eager external-input exclusion for supported programs.
Native primitive semantics, exporter truthfulness, scheduler completeness and
the omitted prefix remain separate obligations. No world period is inferred.
"""
from pathlib import Path
import copy

from observer_export_replay import (MAX64, array, boolean, fields, natural, module_closure,
    require, sequence, variant, main as replay_main)
from acquired_export_codec import SENSORY as decoder, lineage, wide
import acquired_export_codec
import physical_clock_codec as clock
from check_lifecycle_export import cut_registry, validate_adapter
import check_lifecycle_export

ROOT_MODELS = ['AcquiredObserver', 'ObserverOwnership', 'PhysicalClock', 'BoundedLasso', 'ExactNames', 'ReadReuse']
MODELS = module_closure(ROOT_MODELS)
PRODUCERS = [Path(m.__file__).resolve() for m in
             (acquired_export_codec, clock, check_lifecycle_export)] + [Path(__file__).resolve()]
SCOPE = (__doc__ + ' VM values and complete reports are replayed with explicit verification '
         'budgets; native resource debits are not reconstructed. Work receipts are checked '
         'for conservation only. Identity names require retained native allocation evidence.')


def same(left, right, message):
    require(left == right, message)


def result(value):
    kind, body = variant(value)
    require(kind in ('Ok', 'Err'), 'Unknown result variant')
    return kind, body


def execution_limit(value):
    kind, body = variant(value)
    if kind == 'Unlimited':
        require(body is None, 'Unexpected unlimited payload')
        return 'none'
    require(kind == 'Dispatches', 'Unknown execution limit')
    return '(some ' + natural(body) + ')'


class Replay:
    def __init__(self, data):
        fields(data, 'protocol case maximum initialization identity_scope definitions histories certificates samples steps')
        same(data['protocol'], 'synarc-source-cycle-v1', 'Unknown source-cycle protocol')
        require(type(data['maximum']) is int and data['maximum'] == MAX64, 'Expected exact u64 maximum')
        for key in ('case', 'initialization', 'identity_scope'):
            require(type(data[key]) is str and data[key], 'Missing explicit ' + key)
        validate_adapter(data, decoder=decoder)
        self.data = data
        self.rows = sequence(data['steps'])
        require(self.rows, 'Empty native execution')
        self.lines = ['import VerumExternalReplay.' + m for m in ROOT_MODELS] + [
            'open AcquiredObserver', 'set_option warningAsError true', 'namespace FrozenObserverExport']
        self.claims = set()
        self.definitions = {}
        self.register, self.owners, self.states = cut_registry(self.definitions, self.claim, decoder=decoder)
        self.certificates = {}
        self.histories = {}
        self.samples = {}
        self.prepared, self.dispatches, self.committed, self.applied = {}, {}, {}, {}
        self.consumed_preparations, self.consumed_receipts = set(), set()
        self.paired_native, self.paired_physical = set(), set()
        self.origins = set()
        self.elapsed = None
        for d in sequence(data['definitions']):
            fields(d, 'identity initial update read update_binding read_binding semantics')
            identity = natural(d['identity'])
            require(d['identity'] not in self.definitions, 'Duplicate definition')
            self.definitions[d['identity']] = d
            decoder.value(d['initial'])
            for phase in ('update', 'read'):
                self.lines.append(f'def d{identity}_{phase} : FiniteCode.Code Op Value := {decoder.code(d[phase])}')
                self.lines.append(f'def d{identity}_{phase}_binding : ObserverAdmission.Binding Source := {decoder.binding(d[phase+"_binding"])}')
        for h in sequence(data['histories']):
            fields(h, 'identity origin'); natural(h['identity'])
            fields(h['origin'], 'lineage epoch ordinal')
            lineage(h['origin']['lineage']); wide(h['origin']['epoch']); natural(h['origin']['ordinal'])
            require(h['identity'] not in self.histories, 'Duplicate history')
            self.histories[h['identity']] = h
        for s in sequence(data['samples']):
            fields(s, 'identity lineage ordinal'); natural(s['identity']); lineage(s['lineage']); natural(s['ordinal'])
            require(s['identity'] not in self.samples and
                    all((s['lineage'], s['ordinal']) != (v['lineage'], v['ordinal']) for v in self.samples.values()),
                    'Duplicate sample identity')
            self.samples[s['identity']] = s
        for c in sequence(data['certificates']):
            self.certificate(c)

    def claim(self, equation, proof='rfl'):
        # Repeated observations have the same obligation, not independent proof credit.
        if (equation, proof) not in self.claims:
            self.lines.append(f'theorem s{len(self.claims)} : {equation} := by\n  {proof}')
            self.claims.add((equation, proof))

    def cut(self, value):
        self.register(value)
        return '(' + decoder.cut(value) + ' : ObserverOwnership.Cut Nat Nat Nat)'

    def vm(self, cut, phase, supplied, expected, report, limit):
        self.cut(cut)
        definition = self.definitions[cut['definition']]
        cap = natural(definition['semantics']['max_trace'])
        interface, values = decoder.input_row(supplied)
        prefix = f'd{cut["definition"]}_{phase}'
        state, expected = decoder.value(cut['value']), decoder.value(expected)
        # An explicit ample verification work grant reproduces the successful
        # report. It is not represented as the actual native work debit.
        invocation = (f'ObserverAdmission.evaluate (apply {cap}) .{phase} {prefix} {prefix}_binding '
                      f'{state} {interface} {values} ⟨{execution_limit(limit)}, {MAX64}, 0, 0, false⟩')
        self.claim(f'observe ({invocation}) = .ok ({expected}, {decoder.report(report)})',
                   f'unfold {prefix} {prefix}_binding\n  reduce_admission')
        self.claim(f'run {cap} {prefix} {prefix}_binding {state} {interface} {values} = some {expected}',
                   f'unfold {prefix} {prefix}_binding AcquiredObserver.run\n'
                   '  simp [FiniteCode.bind, List.mapM, List.mapM.loop]\n  reduce_finite_code')

    def certificate(self, c):
        fields(c, 'identity definition autonomy limits inputs carrier readings transitions closing_target period observation_labels phase_codes scope')
        natural(c['identity'])
        natural(c['definition'])
        require(c['identity'] not in self.certificates and c['definition'] in self.definitions, 'Unknown or duplicate certificate')
        fields(c['autonomy'], 'definition applies')
        natural(c['autonomy']['definition']); boolean(c['autonomy']['applies'])
        same(c['autonomy'], {'definition':c['definition'], 'applies':True}, 'Autonomy owner mismatch')
        definition = self.definitions[c['definition']]
        cap = natural(definition['semantics']['max_trace'])
        for phase in ('update', 'read'):
            prefix = f'd{c["definition"]}_{phase}'
            excluded = '(fun source => match source with | .state => false | .external _ => true)'
            # Re-establish eager independence from the exported executable code,
            # rather than elevating the native `applies` flag to a proof.
            self.claim(f'∀ (state : Value) (le re left right : List Value) '
                f'(limits : FiniteExecution.Limits) (used : Nat), '
                f'ObserverAdmission.bindValues {prefix}_binding.width state le {prefix}_binding.slots 0 = .ok left → '
                f'ObserverAdmission.bindValues {prefix}_binding.width state re {prefix}_binding.slots 0 = .ok right → '
                f'FiniteExecution.execute (apply {cap}) {prefix} limits left used = '
                f'FiniteExecution.execute (apply {cap}) {prefix} limits right used',
                f'intro state le re left right limits used hl hr\n'
                f'  apply ReadEvidence.execute_agreement\n'
                f'  apply ReadReuse.binding_key_agreement {prefix} {prefix}_binding state state le re left right '
                f'{excluded} (by decide_cbv) ?_ hl hr\n'
                '  intro source _ allowed\n  cases source with\n'
                '  | state => rfl\n  | external n => cases allowed')
        fields(c['limits'], 'states workspace execution')
        natural(c['limits']['states']); natural(c['limits']['workspace']); execution_limit(c['limits']['execution'])
        carrier, readings, transitions = map(sequence, (c['carrier'], c['readings'], c['transitions']))
        n = len(carrier); start, period = c['closing_target'], c['period']
        natural(start); natural(period)
        require(0 < n <= c['limits']['states'] and start < n and 0 < period <= n,
                'Invalid finite carrier bounds')
        require(len(readings) == len(transitions) == len(sequence(c['phase_codes'])) == len(sequence(c['observation_labels'])) == n,
                'Incomplete finite carrier')
        self.claim(f'({decoder.value(carrier[0]["value"])} : Value) = {decoder.value(definition["initial"])}')
        self.claim(f'({array(decoder.value(v["value"]) for v in carrier)} : List Value).Nodup', 'decide_cbv')
        inputs = sequence(c['inputs']); require(len(inputs) == 2, 'Expected update and read inputs')
        for item in inputs: decoder.input_row(item)
        for i, current in enumerate(carrier):
            same(current['definition'], c['definition'], 'Carrier definition changed')
            self.cut(current)
            read, transition = readings[i], transitions[i]
            fields(read, 'cut value execution'); fields(transition, 'receipt execution')
            fields(transition['receipt'], 'before after')
            same(read['cut'], current, 'Reading detached from carrier')
            before, after = transition['receipt']['before'], transition['receipt']['after']
            same(before, current, 'Transition detached from carrier')
            # The closing value may repeat; its new state allocation may not.
            require(after['state'] not in self.states, 'Carrier commit reused a state allocation')
            self.cut(after)
            self.claim(f'ObserverOwnership.commit {self.cut(current)} ⟨{self.cut(current)}, {natural(after["state"])}⟩ = '
                       f'⟨{self.cut(after)}, .ok ⟨{self.cut(current)}, {self.cut(after)}⟩⟩')
            successor = i+1 if i+1 < n else start
            self.claim(f'({decoder.value(after["value"])} : Value) = {decoder.value(carrier[successor]["value"])}')
            if successor != start or i+1 < n:
                same(after, carrier[successor], 'Nonclosing carrier revision disconnected')
            self.vm(current, 'read', inputs[1], read['value'], read['execution'], c['limits']['execution'])
            self.vm(current, 'update', inputs[0], after['value'], transition['execution'], c['limits']['execution'])
            phase = int(natural(c['phase_codes'][i]))
            require(phase < period, 'Phase outside period')
            self.claim(f'({natural(c["phase_codes"][successor])} : Nat) = ({phase}+1)%{period}')
            natural(c['observation_labels'][i])
        # Verify equality classes using complete values, not exported labels alone.
        for i in range(n):
            for j in range(i):
                eq = c['observation_labels'][i] == c['observation_labels'][j]
                self.claim(f'decide (({decoder.value(readings[i]["value"])} : Value) = '
                           f'{decoder.value(readings[j]["value"])}) = {str(eq).lower()}', 'decide_cbv')
                if c['phase_codes'][i] == c['phase_codes'][j]:
                    require(eq, 'Same phase has different reads')
        for shorter in range(1, period):
            require(any(c['observation_labels'][start+j] !=
                        c['observation_labels'][start+(j+shorter)%(n-start)] for j in range(n-start)),
                    'Exported observable period is not minimal')
        require(type(c['scope']) is str and c['scope'], 'Missing certificate scope')
        self.certificates[c['identity']] = c

    def snapshot(self, s):
        fields(s, 'floor clock journal installed namespace certificate binding following_binding inspection')
        fields(s['floor'], 'lineage input_epoch output_epoch')
        lineage(s['floor']['lineage']); wide(s['floor']['input_epoch']); wide(s['floor']['output_epoch'])
        clock.clock(s['clock']); self.origins.add(s['clock']['origin'])
        fields(s['journal'], 'pending_boundary history_known ordinal')
        boolean(s['journal']['pending_boundary']); boolean(s['journal']['history_known']); natural(s['journal']['ordinal'])
        installed = s['installed']; fields(installed, 'cut history origin_cut')
        self.cut(installed['cut']); natural(installed['history']); natural(installed['origin_cut'])
        require(installed['history'] in self.histories, 'Unknown installed history')
        keys = decoder.namespace(s['namespace'])
        self.claim(f'({keys} : List (Nat × Value)).Nodup', 'decide_cbv')
        if s['certificate'] is not None:
            natural(s['certificate'])
            require(s['certificate'] in self.certificates, 'Unknown analysis certificate')
        binding = s['binding']
        if binding is not None:
            fields(binding, 'certificate anchor_event anchor_point anchor_phase point bound_cut state_index pending_sample')
            natural(binding['certificate'])
            require(binding['certificate'] in self.certificates, 'Unknown binding certificate')
            c = self.certificates[binding['certificate']]
            same(binding['certificate'], s['certificate'], 'Binding/analysis certificate mismatch')
            for p in ('anchor_point','point'): clock.point(binding[p]); self.origins.add(binding[p]['origin'])
            natural(binding['anchor_event']); natural(binding['anchor_phase']); natural(binding['state_index'])
            require(binding['state_index'] < len(c['carrier']) and binding['anchor_phase'] < c['period'], 'Invalid binding phase')
            self.cut(binding['bound_cut'])
            same(binding['bound_cut']['definition'], c['definition'], 'Foreign bound definition')
            self.claim(f'({decoder.value(binding["bound_cut"]["value"])} : Value) = '
                       f'{decoder.value(c["carrier"][binding["state_index"]]["value"])}')
            if binding['pending_sample'] is not None:
                fields(binding['pending_sample'], 'lineage ordinal')
                lineage(binding['pending_sample']['lineage']); natural(binding['pending_sample']['ordinal'])
        following = s['following_binding']
        if following is not None:
            fields(following, 'certificate original_cut latest_cut history origin_cut comparison_cursor comparison_pending found_index shift broken taken poisoned')
            natural(following['certificate'])
            require(following['certificate'] in self.certificates, 'Unknown following certificate')
            for k in ('original_cut','latest_cut'): self.cut(following[k])
            for k in ('history','origin_cut','comparison_cursor'): natural(following[k])
            for k in ('comparison_pending','broken','taken','poisoned'): boolean(following[k])
            if following['found_index'] is not None: natural(following['found_index'])
            fields(following['shift'], 'states start capped residue')
            for v in following['shift'].values(): natural(v)
            c = self.certificates[following['certificate']]
            sh = following['shift']
            same((sh['states'], sh['start']), (len(c['carrier']), c['closing_target']), 'Foreign lasso')
            require(sh['capped'] <= sh['states'] and sh['residue'] < sh['states']-sh['start'], 'Unbounded shift')
            if following['found_index'] is not None:
                require(following['found_index'] < len(c['carrier']), 'Member outside carrier')
                self.claim(f'({decoder.value(following["original_cut"]["value"])} : Value) = '
                           f'{decoder.value(c["carrier"][following["found_index"]]["value"])}')
        # Inspection is a separate native call, compared with the private state.
        visible = binding is not None and binding['pending_sample'] is None and \
            binding['bound_cut'] == installed['cut'] and s['clock']['phase'] == 'ready' and \
            s['clock']['episode'] is not None and s['clock']['age'] < MAX64 and \
            binding['point'] == {'origin':s['clock']['origin'], 'time':{'episode':s['clock']['episode'],'age':s['clock']['age']}}
        expected = None
        if visible:
            c = self.certificates[binding['certificate']]
            anchor, current = binding['anchor_point'], binding['point']
            same(current['origin'],anchor['origin'],'Completed binding changed physical owner')
            same(current['time']['episode'],anchor['time']['episode'],'Completed binding crossed episode')
            require(current['time']['age'] >= anchor['time']['age'],'Binding age moved before anchor')
            ticks = current['time']['age']-anchor['time']['age']
            self.claim(f'({natural(c["phase_codes"][binding["state_index"]])} : Nat) = '
                       f'({natural(binding["anchor_phase"])}+{ticks})%{natural(c["period"])}')
            expected = {'floor':s['floor'], 'primitive_observable_period':c['period'],
                        'phase':c['phase_codes'][binding['state_index']], 'anchor_phase':binding['anchor_phase'],
                        'anchor_episode':binding['anchor_point']['time']['episode'], 'anchor_age':binding['anchor_point']['time']['age'],
                        'episode':binding['point']['time']['episode'], 'life_age':binding['point']['time']['age']}
        same(s['inspection'], {'Ok':expected}, 'Public inspection disagrees with owned binding')

    def stable(self, b, a, *changing):
        for key in b:
            if key not in (*changing, 'inspection'):
                same(b[key], a[key], 'Unexpected mutation: '+key)

    def row(self, row):
        fields(row, 'seq operation input result before after')
        i, op, arg, actual, b, a = (row[k] for k in ('seq','operation','input','result','before','after'))
        previously_named = set(self.origins)
        self.snapshot(b); self.snapshot(a)
        before, after = b['installed']['cut'], a['installed']['cut']
        if op == 'anchor':
            self.anchor(row)
        elif op == 'prepare':
            fields(arg, 'input work_grant execution_limit'); natural(arg['work_grant']); execution_limit(arg['execution_limit'])
            fields(actual, 'read prepared execution remaining_work'); natural(actual['remaining_work'])
            require(actual['remaining_work'] <= arg['work_grant'], 'Work grew during preparation')
            read, prep = actual['read'], actual['prepared']
            fields(read, 'input cut value execution name'); decoder.prepared(prep)
            same(read['cut'], before, 'Read of foreign cut'); same(prep['before'], before, 'Prepared foreign cut')
            require(prep['after']['state'] not in self.states, 'Preparation reused a state allocation')
            self.cut({**before, **prep['after']})
            self.vm(before, 'read', read['input'], read['value'], read['execution'], arg['execution_limit'])
            self.vm(before, 'update', arg['input'], prep['after']['value'], actual['execution'], arg['execution_limit'])
            self.stable(b, a, 'namespace')
            for key in ('owner','epoch','capacity'): same(b['namespace'][key], a['namespace'][key], 'Namespace owner changed')
            inputs = sequence(read['input']['values']); require(len(inputs) == 1 and inputs[0]['kind'] == 'Query', 'Missing actual query')
            key = f'({natural(inputs[0]["value"]["input"])}, {decoder.value(read["value"])})'
            self.claim(f'ExactNames.intern (min {natural(b["namespace"]["capacity"])} (2^56)) '
                       f'({decoder.namespace(b["namespace"])} : List (Nat × Value)) {key} = '
                       f'({decoder.namespace(a["namespace"])}, some {natural(read["name"])})', 'decide_cbv')
            self.prepared[i] = row
        elif op == 'dispatch':
            fields(arg, 'preparation point action'); natural(arg['action'],255)
            require(arg['preparation'] in self.prepared, 'Unknown preparation')
            fields(actual, 'cut receipt capture physical_point world_calls')
            cut = actual['cut']; fields(cut, 'lineage ordinal time raw state action epoch raw_space pose sensory history_end inputs')
            lineage(cut['lineage']); natural(cut['ordinal']); fields(cut['time'], 'episode life_age')
            for k in ('raw','state','action','history_end'): natural(cut[k])
            wide(cut['epoch']); wide(cut['raw_space'])
            if cut['pose'] is not None: natural(cut['pose'],255)
            if cut['sensory'] is not None: decoder.sensory_value(cut['sensory'])
            entries = sequence(cut['inputs']); epochs = set()
            for entry in entries:
                require(len(sequence(entry)) == 2, 'Invalid cut input'); wide(entry[0]); natural(entry[1])
                require(entry[0] not in epochs, 'Duplicate cut input epoch'); epochs.add(entry[0])
            edge = actual['receipt']; clock.receipt(edge)
            require(edge['sample'] in self.samples, 'Unknown physical sample')
            same(self.samples[edge['sample']], {'identity':edge['sample'],'lineage':cut['lineage'],'ordinal':cut['ordinal']}, 'Sample detached from cut')
            same(actual['physical_point'], arg['point'], 'Cut point detached from dispatch')
            same(edge['point'], arg['point'], 'Receipt point detached from dispatch')
            same(cut['time'], {'episode':arg['point']['time']['episode'],'life_age':arg['point']['time']['age']}, 'Cut time differs from owned point')
            fields(actual['capture'], 'action outcome'); same(actual['capture']['action'], arg['action'], 'Captured action mismatch')
            same(cut['action'], arg['action'], 'Cut action mismatch')
            preparation = self.prepared[arg['preparation']]
            query = preparation['result']['read']['input']['values'][0]['value']
            same(cut['raw'], query['raw'], 'Dispatch raw state differs from prepared view')
            same(dict(cut['inputs']).get(b['floor']['output_epoch']), preparation['result']['read']['name'],
                 'Dispatch omitted the actual native reading name')
            kind, body = variant(actual['capture']['outcome'])
            if kind in ('step','refused'): require(body is None, 'Unexpected outcome payload'); transition = '.tick'
            else:
                require(kind == 'terminal', 'Unknown outcome'); fields(body, 'kind resumption')
                require(body['kind'] in clock.KINDS and body['resumption'] in ('Stopped','Resumed'), 'Unknown terminal outcome')
                transition = '(.terminal .'+clock.KINDS[body['kind']]+')'
            self.claim(f'PhysicalClock.start {MAX64} {clock.clock(b["clock"])} {clock.point(arg["point"])} = ({clock.clock(a["clock"])}, .ok ())')
            self.claim(f'PhysicalClock.observe {MAX64} {clock.point(arg["point"])} (some {natural(edge["sample"])}) {transition} = some {clock.receipt(edge)}')
            self.stable(b,a,'clock','journal')
            for key in ('pending_boundary','history_known'): same(b['journal'][key], a['journal'][key], 'Journal gate changed')
            same(cut['ordinal'], b['journal']['ordinal'], 'Cut skipped journal position')
            same(a['journal']['ordinal'], cut['ordinal']+1, 'Journal did not advance once')
            natural(actual['world_calls']); self.dispatches[i] = row
        elif op == 'native_commit':
            fields(arg, 'dispatch preparation'); fields(actual, 'input receipt execution')
            require(arg['dispatch'] in self.dispatches and arg['preparation'] in self.prepared, 'Unknown native handoff')
            same(self.dispatches[arg['dispatch']]['input']['preparation'], arg['preparation'], 'Commit joined another preparation')
            require(arg['preparation'] not in self.consumed_preparations, 'Preparation consumed twice')
            prep = self.prepared[arg['preparation']]
            same(actual['input'], prep['input']['input'], 'Committed input replaced')
            same(actual['execution'], prep['result']['execution'], 'Committed execution replaced')
            fields(actual['receipt'], 'before after')
            same(actual['receipt'], {'before':before,'after':after}, 'Commit receipt detached from installed state')
            self.claim(f'ObserverOwnership.commit {self.cut(before)} {decoder.prepared(prep["result"]["prepared"])} = '
                       f'⟨{self.cut(after)}, .ok ⟨{self.cut(before)}, {self.cut(after)}⟩⟩')
            self.stable(b,a,'installed','following_binding')
            for key in ('history','origin_cut'): same(b['installed'][key],a['installed'][key], 'Installation authority changed')
            self.follow_commit(b,a,actual['receipt'])
            self.consumed_preparations.add(arg['preparation']); self.committed[i] = row
        elif op == 'pair_native':
            self.pair_native(row)
        elif op == 'physical_commit':
            fields(arg, 'dispatch'); require(arg['dispatch'] in self.dispatches, 'Unknown physical handoff')
            require(arg['dispatch'] not in self.consumed_receipts, 'Receipt consumed twice')
            edge = self.dispatches[arg['dispatch']]['result']['receipt']
            same(actual, {'Ok':edge}, 'Applied edge differs from consumed receipt')
            self.claim(f'PhysicalClock.commit {clock.clock(b["clock"])} {clock.receipt(edge)} = some ({clock.clock(a["clock"])}, .ok ())')
            self.stable(b,a,'clock'); self.consumed_receipts.add(arg['dispatch']); self.applied[i] = row
        elif op == 'pair_physical':
            self.pair_physical(row)
        elif op == 'close_terminal':
            require(arg is None and actual == {'Ok':None}, 'Unexpected terminal closure')
            self.claim(f'PhysicalClock.closeTerminal {MAX64} {clock.clock(b["clock"])} = ({clock.clock(a["clock"])}, .ok ())')
            self.stable(b,a,'clock')
        elif op == 'begin_run':
            require(arg is None, 'Unexpected restart payload')
            kind, reason = result(actual)
            if kind == 'Ok':
                require(reason is None and a['clock']['origin'] not in previously_named | {b['clock']['origin']}, 'Restart reused origin')
                require(a['binding'] is None, 'Restart retained source binding')
                expected = '.ok ()'
            else:
                require(reason in clock.LIFECYCLE, 'Unknown restart refusal')
                same(a,b,'Refused restart mutated state'); expected = '.error .'+clock.LIFECYCLE[reason]
            gate = clock.journal({k:b['journal'][k] for k in ('pending_boundary','history_known')})
            self.claim(f'PhysicalClock.beginRun {MAX64} {natural(a["clock"]["origin"])} {gate} '
                       f'{clock.clock(b["clock"])} {self.cut(before)} id = (({clock.clock(a["clock"])}, {self.cut(after)}), {expected})')
            self.stable(b,a,'clock','binding')
        else:
            raise ValueError('Unsupported source-cycle operation: '+str(op))

    def anchor(self, row):
        i,arg,actual,b,a = (row[k] for k in ('seq','input','result','before','after'))
        fields(arg, 'limits quoted_work'); fields(arg['limits'], 'states workspace execution')
        for k in ('states','workspace'): natural(arg['limits'][k])
        execution_limit(arg['limits']['execution']); natural(arg['quoted_work'])
        kind, body = result(actual)
        if kind == 'Err':
            require(type(body) is str and body in ('NoWork','SourceGap','SourceBoundary','DifferentLimits'), 'Unsupported service refusal')
            p,f = b['clock'],b['following_binding']
            current = f is not None and not f['broken'] and not f['taken'] and \
                f['latest_cut'] == b['installed']['cut'] and f['history'] == b['installed']['history'] and \
                f['origin_cut'] == b['installed']['origin_cut']
            supported = {
                'SourceBoundary':p['phase'] != 'ready' or p['episode'] is None or p['age'] == MAX64,
                'SourceGap':f is not None and not current,
                'DifferentLimits':b['certificate'] is not None and
                    arg['limits'] != self.certificates[b['certificate']]['limits'],
                'NoWork':b['inspection']['Ok'] is not None,
            }
            require(supported[body], 'Refusal lacks exported premises')
            same(b,a,'Refused service mutated state'); return
        fields(body, 'quoted allowed spent returned strict_analysis source_cycle')
        for k in ('quoted','allowed','spent','returned'): natural(body[k])
        same(body['quoted'], arg['quoted_work'], 'Work quote changed')
        require(body['allowed'] <= body['quoted'] and body['allowed'] == body['spent']+body['returned'], 'Work conservation failed')
        same(body['source_cycle'], a['inspection'], 'Service and inspection disagree')
        require(type(body['strict_analysis']) is str, 'Invalid strict analysis status')
        self.stable(b,a,'certificate','binding','following_binding')
        if body['spent'] == 0: same(b,a,'Zero work mutated source ownership')
        if b['certificate'] is not None: same(b['certificate'],a['certificate'],'Certificate replaced')
        if a['certificate'] is not None: same(arg['limits'],self.certificates[a['certificate']]['limits'],'Original limits changed')
        old,new = b['following_binding'],a['following_binding']
        if old is None and new is not None:
            require(not new['broken'] and not new['taken'] and not new['poisoned'], 'Invalid initial following owner')
            same(new['original_cut'],b['installed']['cut'],'Following comparison did not capture current cut')
            same(new['latest_cut'],new['original_cut'],'Invented initial commit')
            same((new['history'],new['origin_cut']),(b['installed']['history'],b['installed']['origin_cut']),'Following authority mismatch')
            same((new['shift']['capped'],new['shift']['residue']),(0,0),'Nonzero initial shift')
            self.elapsed = 0
        elif old is not None and new is not None:
            for k in ('certificate','original_cut','latest_cut','history','origin_cut','shift','broken','taken'):
                same(old[k],new[k],'Service changed commit chain: '+k)
            require(new['comparison_cursor'] >= old['comparison_cursor'], 'Comparison cursor rewound')
        if a['binding'] is not None and a['binding'] != b['binding']:
            binding = a['binding']; c = self.certificates[binding['certificate']]
            require(new is None and binding['pending_sample'] is None, 'Incomplete source anchor')
            same(binding['anchor_event'],i,'Anchor names another operation')
            same(binding['bound_cut'],a['installed']['cut'],'Anchor detached from installed cut')
            same(binding['anchor_point'],binding['point'],'New anchor point changed')
            self.claim(f'PhysicalClock.point {MAX64} {clock.clock(a["clock"])} = .ok {clock.point(binding["point"])}')
            same(binding['anchor_phase'],c['phase_codes'][binding['state_index']],'Invented anchor phase')
            # A comparison may start and finish inside one coarse service call.
            if old is not None:
                require(not old['broken'] and not old['taken'] and not old['poisoned'], 'Broken following owner sealed')
                same(old['latest_cut'],binding['bound_cut'],'Sealed a stale revision')
                matches = [j for j,v in enumerate(c['carrier']) if v['value'] == old['original_cut']['value']]
                require(len(matches) == 1, 'Original membership not exact')
                if old['found_index'] is not None: same(old['found_index'],matches[0],'Wrong captured member')
                self.transport(old['shift'],matches[0],binding['state_index'])
            self.elapsed = None
        elif old is not None and new is None:
            raise ValueError('Following comparison vanished without a source anchor')

    def transport(self, shift, index, expected):
        n,c,k,r = (shift[x] for x in ('states','start','capped','residue'))
        self.claim(f'BoundedLasso.transport {n} {c} ⟨{k},{r}⟩ {index} = {natural(expected)}')

    def follow_commit(self, b, a, receipt):
        old,new = b['following_binding'],a['following_binding']
        if old is None: require(new is None,'Commit invented following binding'); return
        require(new is not None,'Commit erased following binding')
        valid = not old['broken'] and not old['taken'] and old['latest_cut'] == receipt['before'] and \
            receipt['after'] == a['installed']['cut'] and old['history'] == a['installed']['history'] and \
            old['origin_cut'] == a['installed']['origin_cut'] and \
            self.certificates[old['certificate']]['definition'] == receipt['after']['definition']
        expected = copy.deepcopy(old)
        if valid:
            expected['latest_cut'] = receipt['after']
            sh = old['shift']; n,c,k,r = (sh[x] for x in ('states','start','capped','residue'))
            expected['shift'] = {**sh,'capped':min(k+1,n),'residue':(r+1)%(n-c)}
            require(self.elapsed is not None,'Following chain has no recorded origin')
            self.elapsed += 1
            same(expected['shift']['capped'],min(self.elapsed,n),'Commit count lost')
            same(expected['shift']['residue'],self.elapsed%(n-c),'Commit residue lost')
            self.claim(f'BoundedLasso.push {n} {c} ⟨{k},{r}⟩ = ⟨{expected["shift"]["capped"]},{expected["shift"]["residue"]}⟩')
        else: expected['broken'] = True
        same(new,expected,'Following owner did not consume exactly the actual commit')

    def pair_native(self,row):
        arg,b,a = row['input'],row['before'],row['after']
        fields(arg,'dispatch native_commit'); same(row['result'],{'returned':'Unit'},'Unexpected pairing result')
        require(arg['native_commit'] in self.committed and arg['dispatch'] in self.dispatches,'Unknown native pair')
        commit = self.committed[arg['native_commit']]
        same(commit['input']['dispatch'],arg['dispatch'],'Pair joined another dispatch')
        old = b['binding']; expected = None
        if old is not None:
            dispatch = self.dispatches[arg['dispatch']]['result']; cut = dispatch['cut']
            source = self.definitions[commit['result']['receipt']['before']['definition']]['update_binding']['interface']
            supplied = commit['result']['input']; values = supplied['values']
            event = values[0]['value'] if len(values)==1 and values[0]['kind']=='Event' else None
            valid = old['pending_sample'] is None and old['point'] == dispatch['physical_point'] and event is not None and \
                source == supplied['interface'] and source['shape'] == 'Event' and source['raw_space'] == cut['raw_space'] and \
                (source['owner'] is None or source['owner'] == cut['lineage'])
            if valid:
                valid = all(event[k] == source[k] for k in ('epoch','raw_space','owner')) and \
                    all(event[k] == cut[v] for k,v in (('raw','raw'),('action','action'),('pose','pose'))) and \
                    event.get('sensory') == cut['sensory'] and \
                    dict(cut['inputs']).get(source['epoch']) == event['input'] and \
                    (event['episode'],event['life_age']) == (cut['time']['episode'],cut['time']['life_age'])
            receipt = commit['result']['receipt']
            valid = valid and old['bound_cut'] == receipt['before'] and receipt['after'] == a['installed']['cut']
            if valid:
                require(arg['native_commit'] not in self.paired_native,'Native commit paired twice')
                c = self.certificates[old['certificate']]
                index = old['state_index']+1
                if index == len(c['carrier']): index = c['closing_target']
                expected = {**old,'bound_cut':receipt['after'],'state_index':index,
                            'pending_sample':{'lineage':cut['lineage'],'ordinal':cut['ordinal']}}
        same(a['binding'],expected,'Native pairing disagrees with actual ownership/input')
        self.stable(b,a,'binding'); self.paired_native.add(arg['native_commit'])

    def pair_physical(self,row):
        arg,b,a = row['input'],row['before'],row['after']
        fields(arg,'physical_commit'); same(row['result'],{'returned':'Unit'},'Unexpected physical pairing result')
        require(arg['physical_commit'] in self.applied,'Unknown Applied owner')
        applied = self.applied[arg['physical_commit']]
        edge = applied['result']['Ok']; sample = self.samples[edge['sample']]
        old = b['binding']; expected = None
        if old is not None and edge['transition'] == {'tick':None} and \
                old['pending_sample'] == {'lineage':sample['lineage'],'ordinal':sample['ordinal']} and \
                old['point'] == edge['point'] and old['bound_cut'] == a['installed']['cut']:
            require(arg['physical_commit'] not in self.paired_physical,'Applied paired twice')
            expected = {**old,'point':{'origin':edge['point']['origin'],'time':edge['successor']},'pending_sample':None}
        same(a['binding'],expected,'Physical pairing disagrees with actual Applied')
        self.stable(b,a,'binding'); self.paired_physical.add(arg['physical_commit'])

    def emit(self):
        initial = self.rows[0]['before']
        require(initial['binding'] is None and initial['following_binding'] is None,
                'Initial source ownership requires an exported construction')
        previous = None
        for i,row in enumerate(self.rows):
            require(type(row) is dict and row.get('seq') == i and type(row['seq']) is int,'Noncontiguous native sequence')
            if previous is not None: same(row['before'],previous,'Disconnected native before-state')
            self.row(row); previous = row['after']
        require(set(self.prepared) == self.consumed_preparations,'Unconsumed native preparation')
        require(set(self.dispatches) == self.consumed_receipts,'Unconsumed physical receipt')
        require(set(self.committed) == self.paired_native,'Unpaired native commit')
        require(set(self.applied) == self.paired_physical,'Unpaired physical application')
        self.lines.append('end FrozenObserverExport')
        return '\n\n'.join(self.lines)+'\n', {'claims':len(self.claims),'native_steps':len(self.rows),
            'certificates':len(self.certificates),'native_commits':len(self.committed)}


def emit(data):
    return Replay(data).emit()


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODELS, producers=PRODUCERS, scope=SCOPE, description=__doc__)
