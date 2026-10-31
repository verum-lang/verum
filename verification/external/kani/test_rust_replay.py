"""Admission controls; executable Rust checks live in the integration fixture."""
from copy import deepcopy
import json
from pathlib import Path
import tempfile
import unittest
from rust_replay import FORMAT, load, prepare, sha, validate_result


def result():
    return {
        'metadata': {'version': '1.0', 'kani_version': '0.68.0'},
        'tools': {'kani': '0.68.0'},
        'verification_results': {
            'summary': {'total_harnesses': 1, 'executed': 1, 'status': 'completed', 'successful': 1, 'failed': 0},
            'results': [{'harness_id': 'contract', 'status': 'Success', 'checks': [
                {'id': 1, 'status': 'Success', 'category': 'assertion', 'description': 'exact',
                 'location': {'file': 'main.rs'}},
                {'id': 2, 'status': 'Satisfied', 'category': 'cover', 'description': 'boundary'},
            ]}],
        },
        'harness_metadata': [{'pretty_name': 'contract', 'source': {'file': 'main.rs'},
                              'attributes': {'kind': 'Proof', 'should_panic': False},
                              'contract': {'contracted_function_name': None, 'recursion_tracker': None},
                              'has_loop_contracts': False, 'is_automatically_generated': False,
                              'is_bounded': False, 'is_ctor_based': False}],
        'error_details': [{'harness_id': 'contract', 'has_errors': False}],
        'property_details': [{'harness_id': 'contract', 'property_details': {
            'total_properties': 2, 'passed': 1, 'failed': 0, 'unreachable': 0, 'undetermined': 0,
            'solver_error': 0, 'satisfied': 1, 'unsatisfiable': 0, 'covered': 0, 'uncovered': 0}}],
        'cbmc': [{'harness_id': 'contract'}],
    }


EXPECTED = {'contract': {'source': 'main.rs', 'covers': ['boundary']}}


class AdmissionTests(unittest.TestCase):
    def test_complete_obligations(self):
        self.assertEqual(validate_result(result(), EXPECTED, 0),
                         {'harnesses': 1, 'properties': 2, 'statuses': {'Success': 1, 'Satisfied': 1}})

    def test_status_never_substitutes_for_properties(self):
        for status in ('Failure', 'Undetermined', 'SolverError', 'Unreachable', 'unknown'):
            with self.subTest(status=status):
                doc = result()
                doc['verification_results']['results'][0]['checks'][0]['status'] = status
                with self.assertRaises(ValueError): validate_result(doc, EXPECTED, 0)

    def test_vacuous_or_missing_cover_refused(self):
        for checks in ([], [result()['verification_results']['results'][0]['checks'][0]],
                       [{'id': 2, 'status': 'Unsatisfiable', 'category': 'cover', 'description': 'boundary'}]):
            doc = result()
            doc['verification_results']['results'][0]['checks'] = checks
            with self.assertRaises(ValueError): validate_result(doc, EXPECTED, 0)

    def test_each_roster_is_complete_and_unique(self):
        for key in ('harness_metadata', 'error_details', 'property_details', 'cbmc'):
            for rows in ([], [result()[key][0], result()[key][0]]):
                with self.subTest(key=key, rows=len(rows)):
                    doc = result(); doc[key] = rows
                    with self.assertRaises(ValueError): validate_result(doc, EXPECTED, 0)
        doc = result()
        doc['verification_results']['results'].append(deepcopy(doc['verification_results']['results'][0]))
        with self.assertRaises(ValueError): validate_result(doc, EXPECTED, 0)

    def test_substituted_harness_or_panic_expectation_refused(self):
        for key, value in (('source', {'file': 'other.rs'}),
                           ('attributes', {'kind': 'Proof', 'should_panic': True}),
                           ('attributes', {'kind': 'Proof', 'should_panic': False, 'stubs': ['replacement']}),
                           ('has_loop_contracts', True), ('is_bounded', True)):
            doc = result(); doc['harness_metadata'][0][key] = value
            with self.assertRaises(ValueError): validate_result(doc, EXPECTED, 0)

    def test_all_summary_counts_checked(self):
        for key in result()['property_details'][0]['property_details']:
            doc = result(); doc['property_details'][0]['property_details'][key] += 1
            with self.assertRaises(ValueError): validate_result(doc, EXPECTED, 0)
        for key in ('executed', 'successful', 'total_harnesses', 'failed'):
            doc = result(); doc['verification_results']['summary'][key] += 1
            with self.assertRaises(ValueError): validate_result(doc, EXPECTED, 0)

    def test_process_failure_cannot_reuse_success(self):
        for code in (-9, 1, 124):
            with self.assertRaises(ValueError): validate_result(result(), EXPECTED, code)

    def test_duplicate_json_is_not_silently_overwritten(self):
        with self.assertRaises(ValueError): load(b'{"status": "failed", "status": "success"}')
        with self.assertRaises(ValueError): load(b'{"status": NaN}')


class SnapshotTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.source = b'// production\nfn value() -> u64 { 7 }\n'
        (self.root / 'source.rs').write_bytes(self.source)
        self.manifest = {'format': FORMAT,
            'sources': {'source.rs': {'path': 'source.rs', 'sha256': sha(self.source)}},
            'files': {'main.rs': [{'source': 'source.rs', 'start': 0, 'end': len(self.source)},
                                  {'text': '\n// explicit harness\n'}]},
            'entry': 'main.rs', 'harnesses': EXPECTED, 'premises': ['Selected Rust body under std semantics']}

    def test_exact_snapshot_retains_original_and_selections(self):
        workspace, snapshot = prepare(self.manifest, self.root, self.root/'run')
        self.assertEqual((workspace/'main.rs').read_bytes(), self.source+b'\n// explicit harness\n')
        self.assertEqual((workspace.parent/'original/source.rs').read_bytes(), self.source)
        self.assertEqual(snapshot['selections'][0]['sha256'], sha(self.source))
        (self.root/'source.rs').write_bytes(b'changed')
        self.assertEqual((workspace.parent/'original/source.rs').read_bytes(), self.source)

    def test_stale_source_hash_refused(self):
        (self.root/'source.rs').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'Source hash mismatch'):
            prepare(self.manifest, self.root, self.root/'run')

    def test_output_collision_never_overwrites_history(self):
        prepare(self.manifest, self.root, self.root/'run')
        with self.assertRaises(FileExistsError): prepare(self.manifest, self.root, self.root/'run')

    def test_path_escape_and_noncanonical_selection_refused(self):
        for name in ('../outside.rs', '/outside.rs', './inside.rs', 'a/../inside.rs', 'a\\inside.rs'):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                manifest = deepcopy(self.manifest)
                manifest['files'] = {name: manifest['files']['main.rs']}
                with self.assertRaises(ValueError): prepare(manifest, self.root, Path(directory)/'run')
        for start, end in ((-1, 1), (0, len(self.source)+1), (1, 1), (True, 2)):
            with tempfile.TemporaryDirectory() as directory:
                manifest = deepcopy(self.manifest)
                manifest['files']['main.rs'][0].update(start=start, end=end)
                with self.assertRaises(ValueError): prepare(manifest, self.root, Path(directory)/'run')


if __name__ == '__main__':
    unittest.main()
