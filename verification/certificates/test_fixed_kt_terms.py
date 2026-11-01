"""Lean kernel replay of the exact Verum KT terms, with no admitted axioms."""
import generate_fixed_kt
import test_arithmetic_terms as replay


class FixedKTTerms(replay.ArithmeticTerms):
    CASES = staticmethod(generate_fixed_kt.cases)
