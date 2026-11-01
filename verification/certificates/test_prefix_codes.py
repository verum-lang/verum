"""Independent Lean replay of exact native prefix decoding and Kraft proofs."""
import generate_prefix_codes
import test_arithmetic_terms as replay


class PrefixCodeTerms(replay.ArithmeticTerms):
    CASES = staticmethod(generate_prefix_codes.cases)
