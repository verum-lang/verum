"""Independent frozen-wire controls; synthetic mutations confer no admission."""
import unittest

from retained_transfer_codec import (
    CodecError, MAX_BYTES, MAX_CELLS, MAX_DEPTH, USIZE_MAX, VERSION, decode_letter,
)


# Original v40o recipient-0/recipient-3 card; independent of the builders below.
FROZEN_LETTER = (
    "observer-v1\t"
    '53594e4152432d4f425345525645520100001000000000000200000000000000'
    '0001020000000000000000010100000000000000010000000000000000000000'
    '0000000000000000000000000000000100000000000000010000000000000000'
    '0003010000000000000000010000000000000000000000000000000100000000'
    '0000000100000000000000000008010000000000000000000000000000000000'
    '00000000000000'
    "\n"
)


def u64(n):
    return n.to_bytes(8, "little")


def primitive(opcode, arguments=()):
    return b"\x00" + bytes([opcode]) + u64(len(arguments)) + b"".join(arguments)


def call(index, arguments=()):
    return b"\x01" + u64(index) + u64(len(arguments)) + b"".join(arguments)


def input_at(index):
    return b"\x00" + u64(index)


def register_at(index):
    return b"\x01" + u64(index)


def literal(value):
    return b"\x02" + value


def program(steps, output=0, library=()):
    out = b"\x00" if output is None else b"\x01" + u64(output)
    return u64(len(steps)) + out + b"".join(steps) + u64(len(library)) + b"".join(library)


UNIT = program([primitive(0)])


def payload(initial=UNIT, update=UNIT, read=UNIT, update_slots=b"", read_slots=b"", max_trace=17):
    return (VERSION + u64(max_trace) + u64(len(update_slots)) + update_slots
            + u64(len(read_slots)) + read_slots + initial + update + read)


def card(data):
    return "observer-v1\t" + data.hex() + "\n"


class TransferCodecTests(unittest.TestCase):
    def reject(self, data, message):
        with self.assertRaisesRegex(CodecError, message):
            decode_letter(card(data))

    def test_original_native_card(self):
        result = decode_letter(FROZEN_LETTER)
        self.assertEqual(result, {
            "max_trace": 1048576,
            "update_slots": ["State", {"External": 0}],
            "read_slots": ["State", {"External": 0}],
            "initial_program": {"steps": [{"operation": {"Primitive": "Unit"}, "arguments": []}],
                                "output": 0, "library": []},
            "update_program": {"steps": [{"operation": {"Primitive": "Identity"}, "arguments": [{"Input": 1}]}],
                               "output": 0, "library": []},
            "read_program": {"steps": [{"operation": {"Primitive": "EventInput"}, "arguments": [{"Input": 0}]}],
                             "output": 0, "library": []},
        })

    def test_exactly_one_canonical_card(self):
        cases = [FROZEN_LETTER[:-1], FROZEN_LETTER + "\n", FROZEN_LETTER * 2,
                 " " + FROZEN_LETTER, FROZEN_LETTER.replace("\n", "\r\n"),
                 FROZEN_LETTER.replace("53594e", "53594E"), "observer-v1\t0\n",
                 "observer-v1\t\n", "observer-v1\t00 00\n", b"observer-v1\t00\n"]
        for value in cases:
            with self.subTest(value=str(value)[:40]), self.assertRaises(CodecError):
                decode_letter(value)

    def test_truncation_and_trailing(self):
        raw = bytes.fromhex(FROZEN_LETTER.split("\t")[1].strip())
        for end in range(len(raw)):
            with self.subTest(end=end), self.assertRaises(CodecError):
                decode_letter(card(raw[:end]))
        self.reject(raw + b"\x00", "trailing")
        self.reject(bytes([raw[0] ^ 1]) + raw[1:], "version")

    def test_frozen_opcode_table_and_arities(self):
        operations = [
            ("Unit", 0), ("Zero", 0), ("EmptyTrace", 0), ("Identity", 1),
            ("Append", 2), ("At", 2), ("Last", 1), ("Length", 1),
            ("EventInput", 1), ("EventRaw", 1), ("EventAction", 1), ("EventAge", 1),
            ("EventPose", 1), ("QueryInput", 1), ("QueryRaw", 1), ("QueryAge", 1),
            ("QueryAction", 1), ("QueryPose", 1), ("Pair", 2), ("First", 1),
            ("Second", 1), ("Equal", 2), ("Select", 3), ("Add", 2), ("Rem", 2),
            ("Next", 1), ("Refused", 1),
        ]
        for opcode, (name, arity) in enumerate(operations):
            with self.subTest(name=name):
                args = [literal(b"\x00")] * arity
                decoded = decode_letter(card(payload(initial=program([primitive(opcode, args)]))))
                self.assertEqual(decoded["initial_program"]["steps"][0]["operation"], {"Primitive": name})
                self.reject(payload(initial=program([primitive(opcode, args + [literal(b"\x00")])])), "arity")
        self.reject(payload(initial=program([primitive(27)])), "opcode")
        self.reject(payload(initial=program([b"\x02"])), "operation tag")

    def test_literal_domain_is_closed(self):
        value = b"\x03\x01" + u64(USIZE_MAX) + b"\x03\x02\x01\x04"
        result = decode_letter(card(payload(initial=program([primitive(3, [literal(value)])]), max_trace=USIZE_MAX)))
        self.assertEqual(result["max_trace"], USIZE_MAX)
        self.assertEqual(result["initial_program"]["steps"][0]["arguments"], [{"Literal": {
            "kind": "Pair", "first": {"kind": "Nat", "value": USIZE_MAX},
            "second": {"kind": "Pair", "first": {"kind": "Bool", "value": True},
                       "second": {"kind": "Trace", "events": []}},
        }}])
        pair = result["initial_program"]["steps"][0]["arguments"][0]["Literal"]
        self.assertIs(type(pair["first"]["value"]), int)
        self.assertIs(pair["second"]["first"]["value"], True)
        self.assertIs(type(result["max_trace"]), int)
        self.reject(payload(initial=program([primitive(3, [literal(b"\x02\x02")])])), "Boolean")
        self.reject(payload(initial=program([primitive(3, [literal(b"\x05")])])), "literal tag")
        self.reject(payload(initial=program([primitive(3, [b"\x03"])])), "operand tag")

    def test_output_selection_and_prior_registers(self):
        body = program([primitive(0), primitive(3, [register_at(0)])], output=None)
        decoded = decode_letter(card(payload(initial=body)))["initial_program"]
        self.assertIsNone(decoded["output"])
        self.assertEqual(decoded["steps"][1]["arguments"], [{"Register": 0}])
        self.reject(payload(initial=program([], output=None)), "empty program")
        self.reject(payload(initial=program([primitive(0)], output=1)), "output register")
        self.reject(payload(initial=program([primitive(3, [register_at(0)])])), "prior result")
        self.reject(payload(initial=program([primitive(0), primitive(3, [register_at(1)])])), "prior result")
        self.reject(payload(initial=u64(1) + b"\x02" + primitive(0) + u64(0)), "output tag")

    def test_input_bindings_and_usize_overflow(self):
        self.reject(payload(initial=program([primitive(3, [input_at(0)])])), "input index")
        self.reject(payload(update=program([primitive(3, [input_at(1)])]), update_slots=b"\x00"), "input index")
        self.reject(payload(update_slots=b"\x02"), "slot tag")
        bad = program([primitive(3, [input_at(USIZE_MAX)])])
        self.reject(payload(initial=program([primitive(0)], library=[bad])), "overflows usize")
        # An unused lexical function may require a large width without allocating it.
        large = program([primitive(3, [input_at(USIZE_MAX-1)])])
        decoded = decode_letter(card(payload(initial=program([primitive(0)], library=[large]))))
        self.assertEqual(decoded["initial_program"]["library"][0]["steps"][0]["arguments"], [{"Input": USIZE_MAX-1}])

    def test_calls_are_lexical_and_preserve_surplus_arguments(self):
        identity = program([primitive(3, [input_at(0)])])
        nested = program([call(0, [input_at(0)])], library=[identity])
        root = program([call(1, [literal(b"\x01" + u64(7)), literal(b"\x00")])], library=[UNIT, nested])
        decoded = decode_letter(card(payload(initial=root)))["initial_program"]
        self.assertEqual(decoded["steps"][0]["operation"], {"Call": 1})
        self.assertEqual(len(decoded["steps"][0]["arguments"]), 2)
        self.assertEqual(decoded["library"][1]["library"][0]["steps"][0]["operation"], {"Primitive": "Identity"})
        self.reject(payload(initial=program([call(0)])), "lexical library")
        self.reject(payload(initial=program([call(0)], library=[identity])), "too few inputs")
        escaped = program([call(0)])
        self.reject(payload(initial=program([primitive(0)], library=[escaped])), "lexical library")
        # Even an unused surplus argument must be valid in its caller's frame.
        self.reject(payload(initial=program([call(0, [input_at(0)])], library=[UNIT])), "input index")

    def test_exact_program_depth_boundary(self):
        nested = UNIT
        for _ in range(MAX_DEPTH):
            nested = program([primitive(0)], library=[nested])
        decode_letter(card(payload(initial=nested)))
        self.reject(payload(initial=program([primitive(0)], library=[nested])), "depth")

    def test_literal_depth_shares_program_depth(self):
        nested = b"\x00"
        for _ in range(MAX_DEPTH-1):
            nested = b"\x03" + nested + b"\x00"
        decode_letter(card(payload(initial=program([primitive(3, [literal(nested)])]))))
        self.reject(payload(initial=program([primitive(3, [literal(b"\x03" + nested + b"\x00")])])), "depth")

    def test_exact_shared_cell_boundary(self):
        # Three program roots plus the two other Unit steps consume five cells.
        exact = program([primitive(0)] * (MAX_CELLS-5))
        decode_letter(card(payload(initial=exact)))
        over = program([primitive(0)] * (MAX_CELLS-4))
        self.reject(payload(initial=over), "capacity")
        # Binding slots consume the same budget; vector roots do not.
        self.reject(payload(initial=exact, update_slots=b"\x00"), "capacity")
        self.reject(VERSION + u64(0) + u64(MAX_CELLS), "capacity")

    def test_exact_byte_boundary(self):
        # A call costs 17 wire bytes, a Unit 10; the fixed framing is 170 bytes.
        # 60,008*17 + 2,827*10 + 170 = 1,048,576, below the shared cell limit.
        initial = program([call(0)] * 60008 + [primitive(0)] * 2827, library=[UNIT])
        raw = payload(initial=initial)
        self.assertEqual(len(raw), MAX_BYTES)
        decode_letter(card(raw))
        self.reject(raw + b"\x00", "byte limit")


if __name__ == "__main__":
    unittest.main()
