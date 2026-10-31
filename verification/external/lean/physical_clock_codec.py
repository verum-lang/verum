"""One strict physical-clock representation shared by native replay adapters.

Identity allocation and receipt consumption belong to the enclosing protocol;
callbacks register names without replacing the complete owned coordinates.
"""
from observer_export_replay import boolean, fields, natural, require, variant

PHASES = {'ready', 'inFlight', 'terminalPending'}
KINDS = {'Death': 'death', 'Time': 'time', 'Completed': 'completed', 'Ended': 'ended'}
REFUSALS = {name[0].upper() + name[1:]: name for name in
            ['missingEpisode', 'ageOverflow', 'pendingOutcome', 'pendingTerminal',
             'undeclaredTime', 'foreignPoint']}
LIFECYCLE = {name[0].upper() + name[1:]: name for name in
             ['pendingBoundary', 'unknownDelivery', 'pendingPhysicalTime', 'pendingTerminalClosure']}


def option(value, encode):
    return 'none' if value is None else '(some ' + encode(value) + ')'


def status(value, reasons=REFUSALS):
    require(type(value) is str, 'Expected typed native status')
    if value == 'Ok(())':
        return '.ok ()'
    require(value.startswith('Err(') and value.endswith(')') and value[4:-1] in reasons,
            'Unknown native refusal')
    return '.error .' + reasons[value[4:-1]]


def time(value):
    fields(value, 'episode age')
    return '⟨' + natural(value['episode']) + ',' + natural(value['age']) + '⟩'


def point(value, origin=natural):
    fields(value, 'origin time')
    return '⟨' + origin(value['origin']) + ',' + time(value['time']) + '⟩'


def clock(value, origin=natural):
    fields(value, 'origin episode age phase')
    require(type(value['phase']) is str and value['phase'] in PHASES, 'Unknown clock phase')
    return '(⟨' + origin(value['origin']) + ',' + option(value['episode'], natural) + ',' + \
        natural(value['age']) + ',.' + value['phase'] + '⟩ : PhysicalClock.Clock Nat)'


def receipt(value, origin=natural, sample=natural):
    fields(value, 'point sample transition successor')
    kind, argument = variant(value['transition'])
    require((kind == 'tick' and argument is None) or
            (kind == 'terminal' and type(argument) is str and argument in KINDS),
            'Unknown physical transition')
    transition = '.tick' if kind == 'tick' else '(.terminal .' + KINDS[argument] + ')'
    return '(⟨' + point(value['point'], origin) + ',' + option(value['sample'], sample) + ',' + \
        transition + ',' + option(value['successor'], time) + '⟩ : PhysicalClock.Receipt Nat)'


def journal(value):
    if value is None:
        return 'none'
    fields(value, 'pending_boundary history_known')
    return '(some ⟨' + boolean(value['pending_boundary']) + ',' + boolean(value['history_known']) + '⟩)'
