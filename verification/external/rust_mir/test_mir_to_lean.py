"""The unsigned MIR frontend rejects unsupported or incomplete semantics."""
import unittest
from mir_to_lean import translate

MIR = '''fn add(_1: usize, _2: usize) -> usize {
    let mut _0: usize;
    let mut _3: (usize, bool);
    bb0: {
        _3 = AddWithOverflow(copy _1, copy _2);
        assert(!move (_3.1: bool), "overflow", copy _1, copy _2) -> [success: bb1, unwind continue];
    }
    bb1: {
        _0 = move (_3.0: usize);
        return;
    }
}
'''


class TranslationTests(unittest.TestCase):
    def test_checked_unsigned_addition_retains_overflow(self):
        self.assertEqual(translate(MIR, 'add', 64),
            'def extracted (arg1 arg2 : Nat) : Option Nat :=\n'
            '  if arg1 + arg2 < 18446744073709551616 then\n'
            '  some (arg1 + arg2)\n  else none\n')
        self.assertIn('< 256', translate(MIR, 'add', 8))

    def test_subtraction_retains_underflow(self):
        self.assertIn('if arg2 ≤ arg1 then\n  some (arg1 - arg2)\n  else none',
                      translate(MIR.replace('AddWithOverflow', 'SubWithOverflow'), 'add'))

    def test_missing_or_inverted_overflow_check_is_refused(self):
        for before, after in [('assert(!move', 'assert(move'),
                              ('_3.1: bool', '_3.0: bool'),
                              ('_3.0: usize', '_3.1: usize')]:
            with self.subTest(mutation=after), self.assertRaises(ValueError):
                translate(MIR.replace(before, after), 'add')

    def test_unsupported_operation_call_or_cycle_is_refused(self):
        for altered in [
            MIR.replace('AddWithOverflow', 'MulWithOverflow'),
            MIR.replace('AddWithOverflow', 'Add'),
            MIR.replace('_0 = move (_3.0: usize);', '_0 = external(copy _1);'),
            MIR.replace('return;', 'goto -> bb0;'),
            MIR.replace('success: bb1', 'success: bb7'),
            MIR.replace('        _3 =', '        unknown_effect;\n        _3 ='),
            MIR.replace('    bb1: {', '    bb1 (cleanup): {'),
        ]:
            with self.subTest(mir=altered), self.assertRaises(ValueError):
                translate(altered, 'add')

    def test_unvisited_blocks_and_ambiguous_function_are_refused(self):
        with self.assertRaisesRegex(ValueError, 'Unvisited'):
            translate(MIR[:-2]+'    bb9: {\n        return;\n    }\n}\n', 'add')
        with self.assertRaisesRegex(ValueError, 'one exact'):
            translate(MIR+MIR, 'add')
        with self.assertRaises(ValueError):
            translate(MIR, 'other')

    def test_remainder_adds_zero_divisor_guard(self):
        mir = '''fn remainder(_1: usize, _2: usize) -> usize {
    let mut _0: usize;
    bb0: {
        _0 = Rem(copy _1, copy _2);
        return;
    }
}
'''
        self.assertIn('if arg2 = 0 then none else\n  some (arg1 % arg2)', translate(mir, 'remainder'))
        with self.assertRaises(ValueError):
            translate(mir.replace('usize', 'u64'), 'remainder')

    def test_mutable_reference_and_local_type_confusion_are_refused(self):
        for altered in [MIR.replace('_1: usize', '_1: &mut State'),
                        MIR.replace('_0: usize', '_0: bool'),
                        MIR.replace('copy _1, copy _2', 'copy _8, copy _2')]:
            with self.subTest(mir=altered), self.assertRaises(ValueError):
                translate(altered, 'add')

    def test_no_code_injection_from_function_or_constant(self):
        with self.assertRaises(ValueError):
            translate(MIR, 'add\naxiom bad : False')
        for bits in [0, True, 129]:
            with self.assertRaises(ValueError):
                translate(MIR, 'add', bits)


if __name__ == '__main__':
    unittest.main()
