"""Share complete trace literals and prove their lengths/observed appends in Lean."""
import check_lifecycle_export as lifecycle
from observer_export_replay import array


def share_traces(data, source):
    traces = {}
    pending = [data]
    while pending:
        node = pending.pop()
        if type(node) is list:
            pending.extend(node)
        elif type(node) is dict:
            if node.get('kind') == 'Trace':
                # The enclosing strict decoder has already checked each value.
                events = tuple(lifecycle.event(e) for e in node['events'])
                traces.setdefault(events, 'retained_trace_' + str(len(traces)))
            else:
                pending.extend(node.values())
    declarations, proofs = [], []
    for events, name in traces.items():
        source = source.replace('(.trace ' + array(events) + ')', '(.trace ' + name + ')')
        declarations.append(f'def {name} : List AcquiredObserver.Event := {array(events)}')
        proof = name + '_length'; proofs.append(proof)
        declarations.append(f'@[cbv_eval]\ntheorem {proof} : List.length {name} = {len(events)} := by\n'
                            f'  reduce_trace_length {len(events)}')
        if events and events[:-1] in traces:
            before = traces[events[:-1]]
            proof = name + '_append'; proofs.append(proof)
            # Put appends after all value declarations; a prefix can have been
            # encountered later in the foreign object's traversal order.
    for events, name in traces.items():
        if events and events[:-1] in traces:
            before = traces[events[:-1]]
            declarations.append(f'@[cbv_eval]\ntheorem {name}_append : '
                f'List.append {before} [{events[-1]}] = {name} := by\n'
                f'  reduce_trace_append {len(events)-1}')
    if not traces:
        return source, 0
    declarations.append('attribute [cbv_opaque] ' + ' '.join(traces.values()))
    source = 'import VerumExternalReplay.TraceReduction\n\n' + source
    source = source.replace('namespace FrozenObserverExport',
        'namespace FrozenObserverExport\n\n' + '\n\n'.join(declarations), 1)
    source = source.replace('end FrozenObserverExport',
        '\n'.join('#print axioms ' + p for p in proofs) + '\n\nend FrozenObserverExport')
    return source, len(proofs)
