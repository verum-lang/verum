"""Exact rational cross cancellation: values, canonicality and sealed accumulation."""
from fractions import Fraction
from math import gcd
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.CanonicalProduct
open ExactMass CanonicalProduct
set_option warningAsError true
def raw (n d : Nat) : ExactMass.Fraction := ⟨n,d+1,Nat.zero_lt_succ d⟩
def half := raw 1 1
def third := raw 1 2
def twoThirds := raw 2 2
def threeFifths := raw 3 4
def rawHalf := raw 2 3
def rawThird := raw 3 8
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


def fraction(value):
    return f'(raw {value.numerator} {value.denominator - 1})'


class CanonicalProductTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.CanonicalProduct'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-canonical-product-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            return check(path, expected)

    def test_independent_rational_values(self):
        pairs = [(Fraction(a,b),Fraction(c,d)) for a,b,c,d in (
            (0,1,0,1), (0,1,7,11), (7,11,0,1), (1,1,1,1),
            (1,2,1,3), (2,3,3,5), (13,17,17,13), (37,101,202,111),
            (2**128+1,2**127,2**64,2**128+1),
            (2**192+7,2**128+3,2**128+3,2**192+7),
            (2**128+1,3,5,2**128+3), (3,2**128+1,2**128+3,5))]
        statements = []
        for a,b in pairs:
            statements.append(f'example : crossMul {fraction(a)} {fraction(b)} = '
                              f'{fraction(a*b)} := by decide_cbv')
            if b:
                statements.append(f'example : crossDiv {fraction(a)} {fraction(b)} (by decide_cbv) = '
                                  f'{fraction(a/b)} := by decide_cbv')
        self.prove('\n'.join(statements))

    def test_euclidean_gcd_endpoints_and_large_inputs(self):
        pairs = [(0,0),(0,7),(7,0),(0,1),(1,0),(1,1),(48,18),(18,48),
                 (2**256,3),(2**256+1,2**128+1),(2**512-1,2**256-1)]
        self.prove('\n'.join(
            f'example : nativeGcd {a} {b} = {gcd(a,b)} := by decide_cbv' for a,b in pairs))

    def test_accumulation_and_raw_boundary(self):
        self.prove('''
example : Equal (crossMul rawHalf rawThird) (mul rawHalf rawThird) := cross_mul_exact _ _
example : (crossMul rawHalf rawThird).numerator = 6 ∧
    (crossMul rawHalf rawThird).denominator = 36 := by decide_cbv
example : ¬ Canonical (crossMul rawHalf rawThird) := by decide_cbv
example : (Reduced.ofFraction rawHalf).value = half := by decide_cbv
example : crossMul rawHalf (raw 2 1) = half := by decide_cbv
example : (product Reduced.one [Reduced.ofFraction rawHalf,
    Reduced.ofFraction rawThird, Reduced.ofFraction (raw 6 0)]).value = one := by decide_cbv
example : ¬ Less (crossMul twoThirds threeFifths) (raw 2 4) := by decide_cbv
example : crossMul zero threeFifths = zero := by decide_cbv
''')

    def test_false_canonicality_and_values_rejected(self):
        for claim in (
            'nativeGcd 0 0 = 1',
            'nativeGcd 0 7 = 0',
            'nativeGcd 48 18 = 1',
            'euclid 48 18 = 18',
            'Canonical rawHalf',
            'Canonical (crossMul rawHalf rawThird)',
            'crossMul rawHalf rawThird = normalize (mul rawHalf rawThird)',
            'Canonical (mul twoThirds threeFifths)',
            'crossMul twoThirds threeFifths = raw 6 14',
            'crossMul twoThirds threeFifths = raw 1 4',
            'crossDiv half third (by decide_cbv) = raw 1 5',
            'Less (crossMul twoThirds threeFifths) (raw 2 4)',
            'crossMul zero half = one',
            '(crossMul rawHalf rawThird).denominator = 6',
            '(product Reduced.one [Reduced.ofFraction half, Reduced.ofFraction third]).value = half',
        ):
            with self.subTest(claim=claim):
                result = self.prove('example : ' + claim + ' := by decide_cbv', FALSE)
                self.assertEqual(sum(d['severity'] == 'error' for d in result['diagnostics']), 1)
        result = self.prove('example : ExactMass.Fraction := crossDiv half zero (by decide_cbv)', FALSE)
        self.assertEqual(sum(d['severity'] == 'error' for d in result['diagnostics']), 1)

    def test_general_laws_have_checked_axioms(self):
        source = (ROOT / 'VerumExternalReplay/CanonicalProduct.lean').read_text()
        names = re.findall(r'^theorem (\w+)', source, re.M)
        self.assertEqual(len(names), 21)
        result = self.prove('\n'.join('#print axioms CanonicalProduct.' + name for name in names))
        rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rosters), len(names))
        for row in rosters:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)


if __name__ == '__main__':
    unittest.main()
