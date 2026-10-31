import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import uuid

from run_report import RunReport, require_completed
import certificate_runner as runner


class RunReportTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / 'report.json'
        self.request = {'input': 'exact', 'tool': 'frozen'}

    def succeed(self):
        with RunReport(self.path) as run:
            run.bind(self.request)
            report = run.complete({'cases': ['checked']})
        return run.run_id, report

    def test_exact_run_request_and_exit_required(self):
        run_id, report = self.succeed()
        require_completed(report, run_id, self.request, returncode=0)
        for identity, request, code in [(str(uuid.uuid4()), self.request, 0),
                (run_id, {'input':'different'}, 0), (run_id, self.request, 1),
                (run_id, self.request, False), (run_id, self.request, -9)]:
            with self.assertRaises(ValueError):
                require_completed(report, identity, request, returncode=code)

    def test_failed_or_interrupted_rerun_invalidates_old_success(self):
        for error in [RuntimeError('failure'), KeyboardInterrupt()]:
            self.succeed()
            with self.assertRaises(type(error)):
                with RunReport(self.path) as run:
                    run.bind(self.request)
                    raise error
            report = json.loads(self.path.read_bytes())
            self.assertEqual(report['run']['status'], 'failed')
            with self.assertRaises(ValueError):
                require_completed(report, run.run_id, self.request, returncode=0)

    def test_hard_interruption_leaves_running_not_success(self):
        old_id, old = self.succeed()
        run = RunReport(self.path).__enter__()  # Simulate process killed before __exit__.
        report = json.loads(self.path.read_bytes())
        self.assertEqual(report['run']['status'], 'running')
        for snapshot in [old, report]:
            with self.assertRaises(ValueError):
                require_completed(snapshot, run.run_id, self.request, returncode=0)

    def test_reused_identifier_cannot_execute_again(self):
        run_id, report = self.succeed()
        with self.assertRaises(FileExistsError):
            with RunReport(self.path, run_id):
                self.fail('a reused invocation ran')
        with self.assertRaises(ValueError):
            require_completed(report, run_id, self.request, returncode=1)

    def test_failed_publication_cannot_promote_an_old_run(self):
        _, report = self.succeed()
        run = RunReport(self.path)
        with patch('run_report.os.replace', side_effect=OSError('write refused')):
            with self.assertRaises(OSError):
                run.__enter__()
        with self.assertRaises(ValueError):
            require_completed(report, run.run_id, self.request, returncode=0)
        self.assertFalse(list(self.path.parent.glob('.report.json*')))

    def test_request_is_frozen_and_cannot_be_rebound(self):
        with RunReport(self.path) as run:
            request = dict(self.request)
            run.bind(request)
            request['input'] = 'changed'
            self.assertEqual(run.request, self.request)
            with self.assertRaises(ValueError):
                run.bind(request)
            run.complete({})

    def test_concurrent_runs_cannot_substitute_for_each_other(self):
        first = RunReport(self.path).__enter__()
        second = RunReport(self.path).__enter__()
        first.bind(self.request)
        second.bind(self.request)
        first.complete({})
        second.complete({})
        with self.assertRaises(ValueError):
            require_completed(json.loads(self.path.read_bytes()), first.run_id, self.request, returncode=0)
        require_completed(json.loads(first.receipt.read_bytes()), first.run_id, self.request, returncode=0)

    def test_empty_and_malformed_batteries_invalidate_previous_report(self):
        for cases in [[], [('missing fields',)], [('x', {}, True), ('x', {}, True)]]:
            self.succeed()
            with patch('sys.argv', ['check', '--native','missing','--lean','missing','--report',str(self.path)]):
                with self.assertRaises(ValueError):
                    runner.run(cases, 'control', [])
            self.assertEqual(json.loads(self.path.read_bytes())['run']['status'], 'failed')

    def test_completed_lifecycle_cannot_replace_semantic_replay_controls(self):
        from test_certificate_runner import verdicts, PROFILE
        import copy
        request = {'controls': [['test', True, PROFILE]], 'tools': {},
                   'producer_sources': {}, 'battery_sha256': 'battery'}
        native, lean = verdicts(True)
        payload = {'tools': {}, 'producer_sources': {},
            'artifacts': {'battery.json': 'battery', 'proof.json': 'proof'},
            'cases': [{'id':'test', 'expected':True, 'artifact':'proof.json',
                       'judgment_sha256':'proof', 'native':native,
                       'native_returncode':0, 'independent_lean':lean}]}
        with RunReport(self.path) as run:
            run.bind(request)
            report = run.complete(payload)
        runner.require_replay_report(report, run.run_id, request, returncode=0)
        mutations = []
        empty = copy.deepcopy(report); empty['cases'] = []; mutations.append(empty)
        exhausted = copy.deepcopy(report); exhausted['cases'][0]['native']['outcome'] = 'resource_exhausted'; mutations.append(exhausted)
        wrong = copy.deepcopy(report); wrong['cases'][0]['id'] = 'unrelated'; mutations.append(wrong)
        artifact = copy.deepcopy(report); artifact['artifacts']['proof.json'] = 'changed'; mutations.append(artifact)
        for changed in mutations:
            with self.assertRaises((ValueError, RuntimeError)):
                runner.require_replay_report(changed, run.run_id, request, returncode=0)

    def test_incomplete_context_is_not_success(self):
        with RunReport(self.path):
            pass
        self.assertEqual(json.loads(self.path.read_bytes())['run']['status'], 'failed')


if __name__ == '__main__':
    unittest.main()
