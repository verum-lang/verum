"""Independent exact-term replay of categorical KT subprobability."""
import generate_categorical_bounds
import test_arithmetic_terms as replay

class CategoricalBounds(replay.ArithmeticTerms):
    CASES=staticmethod(generate_categorical_bounds.cases)
