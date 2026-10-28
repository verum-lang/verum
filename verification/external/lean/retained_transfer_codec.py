"""Independent decoder for the frozen SYNARC observer-v1 transfer adapter.

This adapter accepts exactly one canonical writer card, including its final
newline. It does not model the native letter reader's tolerant line filtering.
The payload contract is portable.rs from the frozen v40o source, on a 64-bit
host. Decoding establishes finite syntax only, never ownership or admission.

Returned programs use the lifecycle JSON AST. ``output=None`` retains the
native meaning: select the final register after executing the nonempty body.
Library references are lexical; surplus call arguments remain eager operands.
"""
import re


VERSION = b"SYNARC-OBSERVER\x01"
MAX_BYTES = 1024 * 1024
MAX_DEPTH = 64
MAX_CELLS = 65536
USIZE_MAX = (1 << 64) - 1
_PREFIX = "observer-v1\t"
_HEX = re.compile(r"[0-9a-f]+")
_OPERATIONS = (
    ("Unit", 0), ("Zero", 0), ("EmptyTrace", 0), ("Identity", 1),
    ("Append", 2), ("At", 2), ("Last", 1), ("Length", 1),
    ("EventInput", 1), ("EventRaw", 1), ("EventAction", 1),
    ("EventAge", 1), ("EventPose", 1), ("QueryInput", 1),
    ("QueryRaw", 1), ("QueryAge", 1), ("QueryAction", 1),
    ("QueryPose", 1), ("Pair", 2), ("First", 1), ("Second", 1),
    ("Equal", 2), ("Select", 3), ("Add", 2), ("Rem", 2),
    ("Next", 1), ("Refused", 1),
)


class CodecError(ValueError):
    """The card is outside this adapter's frozen syntax contract."""


class _Reader:
    def __init__(self, payload: bytes):
        self.payload = payload
        self.position = len(VERSION)
        self.cells = 0

    def cell(self, depth: int) -> None:
        if depth > MAX_DEPTH or self.cells >= MAX_CELLS:
            raise CodecError("capacity: syntax depth or cell limit")
        self.cells += 1

    def byte(self) -> int:
        if self.position == len(self.payload):
            raise CodecError("truncated: expected byte")
        value = self.payload[self.position]
        self.position += 1
        return value

    def number(self) -> int:
        end = self.position + 8
        if end > len(self.payload):
            raise CodecError("truncated: expected little-endian u64")
        value = int.from_bytes(self.payload[self.position:end], "little")
        self.position = end
        return value

    def count(self) -> int:
        value = self.number()
        if value > MAX_CELLS - self.cells or value > len(self.payload) - self.position:
            raise CodecError("capacity: declared count exceeds retained cells or remaining bytes")
        return value

    def slots(self) -> list:
        slots = []
        for _ in range(self.count()):
            self.cell(0)
            tag = self.byte()
            if tag == 0:
                slots.append("State")
            elif tag == 1:
                slots.append({"External": 0})
            else:
                raise CodecError("binding: invalid input slot tag")
        return slots

    def value(self, depth: int) -> dict:
        self.cell(depth)
        tag = self.byte()
        if tag == 0:
            return {"kind": "Unit"}
        if tag == 1:
            return {"kind": "Nat", "value": self.number()}
        if tag == 2:
            boolean = self.byte()
            if boolean > 1:
                raise CodecError("shape: invalid Boolean literal")
            return {"kind": "Bool", "value": bool(boolean)}
        if tag == 3:
            first = self.value(depth + 1)
            second = self.value(depth + 1)
            return {"kind": "Pair", "first": first, "second": second}
        if tag == 4:
            return {"kind": "Trace", "events": []}
        raise CodecError("shape: unsupported portable literal tag")

    def program(self, inputs: int | None, depth: int) -> tuple[dict, int]:
        self.cell(depth)
        count = self.count()
        if count == 0:
            raise CodecError("shape: empty program body")
        output_tag = self.byte()
        if output_tag == 0:
            output = None
        elif output_tag == 1:
            output = self.number()
            if output >= count:
                raise CodecError("shape: output register outside program body")
        else:
            raise CodecError("shape: invalid output tag")

        required_inputs = 0
        steps = []
        for pc in range(count):
            self.cell(depth)
            tag = self.byte()
            primitive_arity = None
            if tag == 0:
                opcode = self.byte()
                if opcode >= len(_OPERATIONS):
                    raise CodecError("shape: unknown primitive opcode")
                name, primitive_arity = _OPERATIONS[opcode]
                operation = {"Primitive": name}
            elif tag == 1:
                operation = {"Call": self.number()}
            else:
                raise CodecError("shape: invalid operation tag")
            arity = self.count()
            if primitive_arity is not None and arity != primitive_arity:
                raise CodecError("shape: primitive arity mismatch")
            arguments = []
            for _ in range(arity):
                self.cell(depth)
                operand = self.byte()
                if operand == 0:
                    index = self.number()
                    if inputs is not None and index >= inputs:
                        raise CodecError("binding: input index outside declared slots")
                    if index == USIZE_MAX:
                        raise CodecError("capacity: required input width overflows usize")
                    required_inputs = max(required_inputs, index + 1)
                    arguments.append({"Input": index})
                elif operand == 1:
                    index = self.number()
                    if index >= pc:
                        raise CodecError("shape: register operand is not a prior result")
                    arguments.append({"Register": index})
                elif operand == 2:
                    arguments.append({"Literal": self.value(depth + 1)})
                else:
                    raise CodecError("shape: invalid operand tag")
            steps.append({"operation": operation, "arguments": arguments})

        library = []
        widths = []
        for _ in range(self.count()):
            callee, width = self.program(None, depth + 1)
            library.append(callee)
            widths.append(width)
        for step in steps:
            if "Call" in step["operation"]:
                index = step["operation"]["Call"]
                if index >= len(library):
                    raise CodecError("shape: call index outside lexical library")
                if widths[index] > len(step["arguments"]):
                    raise CodecError("binding: call supplies too few inputs")
        return {"steps": steps, "output": output, "library": library}, required_inputs


def decode_letter(letter: str) -> dict:
    """Decode one canonical frozen observer card, or raise ``CodecError``.

    Size is checked before allocating decoded bytes. The native decoder counts
    binding slots, but not binding-vector roots, against its shared cell limit.
    No donor identity, certificate, live state, alphabet or risk is inferred.
    """
    if not isinstance(letter, str):
        raise CodecError("card: expected text")
    if len(letter) > len(_PREFIX) + 2 * MAX_BYTES + 1:
        raise CodecError("capacity: encoded payload exceeds byte limit")
    if not letter.startswith(_PREFIX) or not letter.endswith("\n"):
        raise CodecError("card: expected one canonical observer-v1 card with final newline")
    encoded = letter[len(_PREFIX):-1]
    if len(encoded) % 2 or _HEX.fullmatch(encoded) is None:
        raise CodecError("card: expected one lowercase hexadecimal payload")
    payload = bytes.fromhex(encoded)
    if len(payload) > MAX_BYTES:
        raise CodecError("capacity: payload exceeds byte limit")
    if not payload.startswith(VERSION):
        raise CodecError("version: unsupported observer payload")
    reader = _Reader(payload)
    max_trace = reader.number()
    update_slots = reader.slots()
    read_slots = reader.slots()
    initial, _ = reader.program(0, 0)
    update, _ = reader.program(len(update_slots), 0)
    read, _ = reader.program(len(read_slots), 0)
    if reader.position != len(payload):
        raise CodecError("trailing: bytes after the three program roots")
    return {
        "max_trace": max_trace,
        "update_slots": update_slots,
        "read_slots": read_slots,
        "initial_program": initial,
        "update_program": update,
        "read_program": read,
    }
