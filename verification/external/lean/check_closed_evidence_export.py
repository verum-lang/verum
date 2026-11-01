#!/usr/bin/env python3
"""Replay finite native closed-support evidence without assuming general ML bounds."""
from pathlib import Path
import check_prefix_export as prefix
import check_joint_export
from observer_export_replay import array, fields, main, natural, require, sequence


def emit(data):
    fields(data,'schema scope boundary capacity_at_minimum comparison context_histograms look_denominator '
                'look_index maximum_alphabet minimum_eligible_rows ordered_rows profile queued_physical_ordinals '
                'retained_rows risk_denominator source threshold_denominator')
    require(data['schema']=='synarc-closed-evidence-feasibility-v1','Unknown closed-evidence schema')
    require(data['profile']=='ExactPredictiveV1','Unknown score profile')
    require(data['comparison']=='K^N <= threshold; candidate qualification requires strict >','Changed strict boundary')
    source,counts=prefix.emit(data['source'],data['source']['language_versions'],embedded=True)
    lines=source.removesuffix('end FrozenObserverExport\n').split('\n\n')
    lines.insert(0,'import VerumExternalReplay.ClosedEvidence')
    declarations=[];histograms=[]
    for context in sequence(data['context_histograms']):
        fields(context,'alphabet_size context histogram')
        outcomes=[];values=[]
        for cell in sequence(context['histogram']):
            fields(cell,'count outcome');outcomes.append(prefix.atom(cell['outcome']));values.append(natural(cell['count']))
        require(int(natural(context['alphabet_size']))==len(outcomes) and outcomes,'Invalid declared alphabet')
        declarations.append('⟨'+array(natural(x) for x in sequence(context['context']))+', '+array(outcomes)+'⟩')
        histograms.append(array(values))
    def observation(row):
        fields(row,'context cut ordinal outcome')
        return '⟨'+array(natural(x) for x in sequence(row['context']))+', '+prefix.atom(row['outcome'])+', '+natural(row['ordinal'])+', '+natural(row['cut'])+'⟩'
    rows=sequence(data['ordered_rows'])
    require(len(rows)<=4096,'Finite prefix replay exceeds diagnostic row limit')
    lines+=['def declarations : List ClosedEvidence.Declaration := '+array(declarations),
            'def observations : List ClosedEvidence.Observation := '+array(observation(row) for row in rows),
            'def capturedCounts : List (List Nat) := '+array(histograms)]
    count=counts['claims']
    def claim(prop, *, equality=True):
        nonlocal count
        # Equalities compute definitionally; inequalities use closed Nat
        # decision procedures. Both are reduced by the kernel without the
        # auxiliary rewrite lemmas used by decide_cbv for strings/lists.
        tactic='rfl' if equality else 'decide'
        lines.append(f'theorem p{count} : {prop} := by {tactic}');count+=1
    N=natural(data['retained_rows']);K=natural(data['maximum_alphabet']);minimum=natural(data['minimum_eligible_rows'])
    require(int(N)<=4096 and int(minimum)<=4096,'Minimum eligibility replay exceeds diagnostic prefix limit')
    D=prefix.decimal(data['threshold_denominator']);risk=prefix.decimal(data['risk_denominator'])
    look=prefix.decimal(data['look_index']);lookden=prefix.decimal(data['look_denominator'])
    require(int(look)==int(prefix.decimal(data['source']['look'])),'Paid look identity changed')
    claim('ClosedEvidence.valid declarations = true')
    claim('ClosedEvidence.histogram declarations observations = some capturedCounts')
    claim(f'({N} : Nat) = observations.length')
    claim(f'({K} : Nat) = (declarations.map (fun d => d.outcomes.length)).foldl max 0')
    claim(f'(1 : Nat) ≤ {K}',equality=False)
    claim(f'(List.range ({N}+1)).all (ClosedEvidence.prefixLowerBound declarations observations {K}) = true')
    claim('(observations.zip (observations.drop 1)).all (fun (a,b) => a.ordinal < b.ordinal && a.cut < b.cut) = true')
    claim(f'(0 : Nat) < {risk}',equality=False)
    claim(f'({lookden} : Nat) = ({look}+1)*({look}+2)')
    owners=[natural(w['language']['owner']) for w in data['source']['candidate']['structural_witnesses']]
    # Bind the threshold to full frozen catalogs, never to a guessed candidate
    # length or the already-reported threshold itself.
    claim(f'(ClosedEvidence.headerBits catalog {array(owners)}).map (fun bits => 2^bits*{lookden}*{risk}) = some {D}')
    candidate_den=prefix.decimal(data['source']['selection_denominator'])
    claim(f'({D} : Nat) ≤ {candidate_den}*{risk}',equality=False)
    claim(f'({K} : Nat)^{N} ≤ {D}',equality=False)
    claim(f'(List.range {minimum}).all (fun n => {K}^n ≤ {D}) = true')
    claim(f'({D} : Nat) < {K}^{minimum}',equality=False)
    claim(f'({prefix.decimal(data["capacity_at_minimum"])} : Nat) = {K}^{minimum}')
    boundary=data['boundary'];fields(boundary,'context ordinal outcome reason retained_rows same_native_owner')
    require(boundary['reason']=='MissingAlphabet','Unsupported closure reason')
    require(boundary['same_native_owner'] is True,'Native owner observation missing')
    claim(f'({natural(boundary["retained_rows"])} : Nat) = {N}')
    key=array(natural(x) for x in sequence(boundary['context']))
    claim(f'declarations.all (fun d => d.context != {key}) = true')
    claim(f'ClosedEvidence.ingestOutcome declarations capturedCounts {key} {prefix.atom(boundary["outcome"])} = none')
    claim(f'observations.all (fun row => row.ordinal < {natural(boundary["ordinal"])}) = true')
    queue=sequence(data['queued_physical_ordinals']);require(queue,'Missing retained physical tail')
    claim(f'({array(natural(x) for x in queue)} : List Nat).head? = some {natural(boundary["ordinal"])}')
    claim(f'(({array(natural(x) for x in queue)} : List Nat).zip {array(natural(x) for x in queue[1:])}).all (fun (a,b) => a < b) = true')
    return '\n\n'.join(lines+['end FrozenObserverExport'])+'\n',dict(claims=count,retained_rows=len(rows),
        prefixes=len(rows)+1,contexts=len(declarations),source_claims=counts['claims'],universal_ml_proof=False,
        closure_authority='observed native same-owner boundary; not a physical lifetime theorem')


if __name__=='__main__':
    main(emit=emit,modules=['ClosedEvidence'],producers=[Path(__file__).resolve(),Path(prefix.__file__).resolve(),Path(check_joint_export.__file__).resolve()],
        scope='Exact finite histogram/prefix/header/price diagnostics. Native boundary owner identity is an observation. No universal empirical-ML theorem, native closure/lifetime refinement or complete stopping-rule certificate.',description=__doc__)
