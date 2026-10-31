#!/usr/bin/env python3
"""Fail-closed translation of checked, acyclic unsigned Rust MIR to Lean.

The supported subset has scalar locals, immutable record parameters, checked
addition/subtraction, remainder, comparisons, branches and panic. It has no
heap, aliasing, loops, arbitrary calls or assumed contracts. rustc and this
translator are explicit trusted frontend components; Lean checks the resulting
arithmetic theorem. Unsupported syntax never becomes a successful return.
"""
from dataclasses import dataclass
import re


@dataclass(frozen=True)
class Value:
    text: str
    kind: str


@dataclass(frozen=True)
class Overflow:
    operator: str
    left: Value
    right: Value


def require(condition, message):
    if not condition:
        raise ValueError(message)


def pair(text):
    """Split exactly two operands, respecting MIR projections and strings."""
    depth = 0
    for i, char in enumerate(text):
        depth += char == '('
        depth -= char == ')'
        if char == ',' and depth == 0:
            return text[:i].strip(), text[i+1:].strip()
    raise ValueError('Expected two operands')


class Translation:
    def __init__(self, mir, function, bits):
        require(isinstance(mir, str) and len(mir) <= 16*1024*1024, 'Invalid MIR input')
        require(type(bits) is int and 0 < bits <= 128, 'Invalid target width')
        require(re.fullmatch(r'[A-Za-z][A-Za-z0-9_]*', function), 'Invalid function name')
        headers = list(re.finditer(r'^fn (.+)\((.*?)\) -> usize \{\n', mir, re.M))
        selected = [m for m in headers if m[1] == function or m[1].endswith('::'+function)]
        require(len(selected) == 1, 'Function must have one exact MIR declaration')
        header = selected[0]
        # rustc renders each outer closing brace at column zero; block and scope
        # braces are indented. MIR format is compiler-version dependent.
        end = mir.find('\n}', header.end())
        require(end >= 0, 'Unterminated function')
        self.body = mir[header.end():end]
        self.bits = bits
        self.types = {}
        self.arguments = {}
        self.references = set()
        for parameter in header[2].split(', '):
            m = re.fullmatch(r'(_\d+): (usize|&[A-Za-z][A-Za-z0-9_]*)', parameter)
            require(m is not None, 'Unsupported parameter')
            name, kind = m.groups()
            self.types[name] = kind
            if kind == 'usize':
                self.arguments[name] = Value('arg'+name[1:], kind)
            else:
                self.references.add(name)
        sections = re.split(r'^    (bb\d+): \{\n', self.body, flags=re.M)
        require(len(sections) >= 3 and len(sections) % 2 == 1, 'Missing basic blocks')
        for line in sections[0].splitlines():
            line = line.strip()
            if not line or re.fullmatch(r'debug \w+ => .+;', line) or re.fullmatch(r'scope \d+ \{|}', line):
                continue
            m = re.fullmatch(r'let (?:mut )?(_\d+): (.+);', line)
            require(m is not None, 'Unsupported local declaration: '+line)
            name, kind = m.groups()
            require(kind in ('usize', 'bool', '(usize, bool)', '!', "std::fmt::Arguments<'_>"),
                    'Unsupported local type: '+kind)
            require(name not in self.types, 'Duplicate local')
            self.types[name] = kind
        self.blocks = {}
        for i in range(1, len(sections), 2):
            name, body = sections[i:i+2]
            lines = [line.strip() for line in body.strip().splitlines()]
            require(lines[-1] == '}', 'Malformed basic block')
            require(name not in self.blocks, 'Duplicate basic block')
            self.blocks[name] = lines[:-1]
        require('bb0' in self.blocks, 'Missing entry')
        require(self.types.get('_0') == 'usize', 'Return type mismatch')
        self.fields = set()
        self.visited = set()
        self.nodes = 0

    def operand(self, text, environment):
        m = re.fullmatch(r'(?:copy|move) (_\d+)', text)
        if m:
            value = environment.get(m[1])
            require(isinstance(value, Value), 'Uninitialized or unchecked local: '+text)
            return value
        m = re.fullmatch(r'(?:copy|move) \(\(\*(_\d+)\)\.(\d+): usize\)', text)
        if m:
            owner, index = m[1].strip(), int(m[2])
            require(owner in self.references and index < 256, 'Unsupported field projection')
            name = f'arg{owner[1:]}_{index}'
            self.fields.add(name)
            return Value(name, 'usize')
        m = re.fullmatch(r'(?:copy|move) \((_\d+)\.([01]): (usize|bool)\)', text)
        if m:
            value = environment.get((m[1], int(m[2])))
            require(isinstance(value, Value) and value.kind == m[3], 'Unchecked tuple projection')
            return value
        m = re.fullmatch(r'const (\d+)_usize', text)
        if m:
            require(int(m[1]) < 2**self.bits, 'Out-of-range literal')
            return Value(m[1], 'usize')
        raise ValueError('Unsupported operand: '+text)

    def expression(self, text, environment):
        m = re.fullmatch(r'(Lt|Le|Gt|Ge|Eq|Ne|Rem|AddWithOverflow|SubWithOverflow)\((.*)\)', text)
        if not m:
            return self.operand(text, environment)
        operator, operands = m.groups()
        left, right = (self.operand(x, environment) for x in pair(operands))
        require(left.kind == right.kind == 'usize', 'Expected unsigned operands')
        if operator.endswith('WithOverflow'):
            return Overflow(operator, left, right)
        if operator == 'Rem':
            return Value(f'({left.text} % {right.text})', 'usize')
        symbol = {'Lt':'<', 'Le':'≤', 'Gt':'>', 'Ge':'≥', 'Eq':'=', 'Ne':'≠'}[operator]
        return Value(f'({left.text} {symbol} {right.text})', 'bool')

    def block(self, name, environment, stack=()):
        require(name in self.blocks and name not in stack, 'Missing block or cyclic control flow')
        self.nodes += 1
        require(self.nodes <= 4096, 'Control-flow expansion limit')
        self.visited.add(name)
        environment = dict(environment)
        lines = self.blocks[name]
        prefix = ''
        pending = None
        for line in lines[:-1]:
            require(pending is None, 'Overflow tuple must be checked immediately')
            m = re.fullmatch(r'(_\d+) = (.*);', line)
            require(m is not None and m[1] in self.types, 'Unsupported statement: '+line)
            local, source = m.groups()
            value = self.expression(source, environment)
            if isinstance(value, Overflow):
                require(self.types[local] == '(usize, bool)', 'Overflow tuple type mismatch')
                pending = (local, value)
            else:
                require(self.types[local] == value.kind, 'Local type mismatch')
                if source.startswith('Rem('):
                    _, denominator = (self.operand(x, environment) for x in pair(source[4:-1]))
                    prefix += f'if {denominator.text} = 0 then none else\n'
                environment[local] = value
        require(lines, 'Empty basic block')
        last = lines[-1]
        advance = lambda target: self.block(target, environment, (*stack, name))
        if pending is not None:
            local, value = pending
            pattern = r'assert\(!move \(' + re.escape(local) + r'\.1: bool\), "(?:[^"\\]|\\.)*"(?:, .*)?\) -> \[success: (bb\d+), unwind continue\];'
            match = re.fullmatch(pattern, last)
            require(match is not None, 'Overflow check absent or changed')
            a, b = value.left.text, value.right.text
            if value.operator == 'SubWithOverflow':
                guard, result = f'{b} ≤ {a}', f'({a} - {b})'
            else:
                guard, result = f'{a} + {b} < {2**self.bits}', f'({a} + {b})'
            # On the checked-success edge the wrapped result is exactly the
            # mathematical result. No value escapes an overflow edge.
            environment[(local, 0)] = Value(result, 'usize')
            environment[(local, 1)] = Value('False', 'bool')
            return prefix + f'if {guard} then\n{advance(match[1])}\nelse none'
        if last == 'return;':
            value = environment.get('_0')
            require(isinstance(value, Value) and value.kind == 'usize', 'Invalid return')
            return prefix + f'some {value.text}'
        match = re.fullmatch(r'goto -> (bb\d+);', last)
        if match:
            return prefix + advance(match[1])
        match = re.fullmatch(r'switchInt\((.*)\) -> \[0: (bb\d+), otherwise: (bb\d+)\];', last)
        if match:
            value = self.operand(match[1], environment)
            require(value.kind == 'bool', 'Only Boolean switches supported')
            return prefix + f'if {value.text} then\n{advance(match[3])}\nelse\n{advance(match[2])}'
        match = re.fullmatch(r'assert\(!(.*?), "(?:[^"\\]|\\.)*"(?:, .*)?\) -> \[success: (bb\d+), unwind continue\];', last)
        if match:
            value = self.operand(match[1], environment)
            require(value.kind == 'bool', 'Non-Boolean assertion')
            return prefix + f'if {value.text} then none else\n{advance(match[2])}'
        # A string-only formatting operation followed immediately by the known
        # non-returning panic entry is an error outcome, never a successful one.
        match = re.fullmatch(r"(_\d+) = Arguments::<'_>::from_str\(const \"(?:[^\"\\]|\\.)*\"\) -> \[return: (bb\d+), unwind continue\];", last)
        if match:
            target = self.blocks.get(match[2], [])
            panic = re.fullmatch(
                r'(_\d+) = std::rt::panic_fmt\(move '+re.escape(match[1])+r'\) -> unwind continue;',
                target[0]) if len(target) == 1 else None
            require(panic is not None and self.types.get(panic[1]) == '!',
                    'Unsupported formatting continuation')
            require(self.types.get(match[1]) == "std::fmt::Arguments<'_>", 'Invalid format local')
            self.visited.add(match[2])
            return prefix + 'none'
        raise ValueError('Unsupported terminator: '+last)

    def lean(self):
        body = self.block('bb0', self.arguments)
        require(self.visited == self.blocks.keys(), 'Unvisited basic blocks')
        arguments = sorted(self.fields) + [v.text for v in self.arguments.values()]
        require(arguments, 'No scalar inputs')
        return 'def extracted ('+' '.join(arguments)+' : Nat) : Option Nat :=\n' + '\n'.join('  '+line for line in body.splitlines())+'\n'


def translate(mir, function, bits=64):
    return Translation(mir, function, bits).lean()
