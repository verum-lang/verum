"""Only completed semantic verdicts can satisfy portable proof controls."""
import copy
import unittest
from certificate_runner import validate_cases, validate_verdicts

PROFILE = 'verum-native/1'


def verdicts(accepted):
    native = {'profile': PROFILE, 'accepted': accepted,
              'outcome': 'accepted' if accepted else 'rejected',
              'diagnostic': None if accepted else {'stage':'substitution','code':'type_mismatch'}}
    lean = {'id':'test', 'ok': accepted}
    if not accepted:
        lean['error'] = 'TypeMismatch'
    return native, lean


class VerdictTests(unittest.TestCase):
    def test_empty_ambiguous_or_malformed_battery_is_not_verification(self):
        for cases in ([], [('x', {}, False), ('x', {}, True)],
                      [('', {}, True)], [('x', {}, 0)], [(None, {}, True)]):
            with self.subTest(cases=cases), self.assertRaises(ValueError):
                validate_cases(cases)
        validate_cases([('x', {}, True), ('y', {}, False)])

    def check(self, native, lean, accepted=False, code=None):
        validate_verdicts('test', native, lean, accepted, PROFILE,
                          (0 if accepted else 1) if code is None else code)

    def test_acceptance_and_semantic_rejection(self):
        for expected in (True, False):
            with self.subTest(expected=expected):
                self.check(*verdicts(expected), accepted=expected)
        native, lean = verdicts(False)
        native['diagnostic']['stage'] = 'normalization'
        self.check(native, lean)

    def test_native_operational_outcomes_never_satisfy_negative_controls(self):
        for outcome in ('unsupported', 'resource_exhausted', 'inconclusive', 'accepted', None):
            native, lean = verdicts(False)
            native['outcome'] = outcome
            with self.subTest(outcome=outcome), self.assertRaises(RuntimeError):
                self.check(native, lean)

    def test_native_unsupported_error_cannot_be_relabelled_as_rejected(self):
        for error in ('normalization_limit', 'universe_limit', 'foundation_required', 'unsupported_kernel_extension',
                      'structural_limit', 'invalid_limits', 'future_unknown_failure'):
            native, lean = verdicts(False)
            native['diagnostic']['code'] = error
            with self.subTest(error=error), self.assertRaises(RuntimeError):
                self.check(native, lean)

    def test_independent_operational_failure_is_not_semantic_rejection(self):
        for error in ('FuelExhausted', 'UniverseOverflow', 'FoundationRequired',
                      'UnsupportedProfile', 'UnknownFailure', None):
            native, lean = verdicts(False)
            lean['error'] = error
            with self.subTest(error=error), self.assertRaises(RuntimeError):
                self.check(native, lean)

    def test_verdict_values_must_be_boolean(self):
        for key, side in [('accepted',0), ('ok',1)]:
            for value in (0, '', None, [], {}):
                reports = list(verdicts(False))
                reports[side][key] = value
                with self.subTest(key=key,value=value), self.assertRaises(RuntimeError):
                    self.check(*reports)

    def test_checker_identity_exit_and_stage_are_not_optional(self):
        native, lean = verdicts(False)
        for field, value in [('profile','verum-native-funext/1'), ('diagnostic',None),
                             ('diagnostic',{'stage':'admission','code':'type_mismatch'})]:
            changed = copy.deepcopy(native)
            changed[field] = value
            with self.subTest(field=field,value=value), self.assertRaises(RuntimeError):
                self.check(changed,lean)
        with self.assertRaises(RuntimeError):
            self.check(native,lean,code=0)
        with self.assertRaises(RuntimeError):
            self.check(native,lean | {'id':'another'})

    def test_accepted_outcome_cannot_carry_a_failure(self):
        native, lean = verdicts(True)
        with self.assertRaises(RuntimeError):
            self.check(native | {'diagnostic':{}}, lean, accepted=True)
        with self.assertRaises(RuntimeError):
            self.check(native, lean | {'error':'FuelExhausted'}, accepted=True)
        with self.assertRaises(RuntimeError):
            self.check(native | {'outcome':'inconclusive'}, lean, accepted=True)


if __name__ == '__main__':
    unittest.main()
