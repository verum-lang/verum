"""Run-bound publication. A report path or successful process exit is not evidence."""
import hashlib
import json
import os
from pathlib import Path
import tempfile
import uuid


def encoded(value):
    return (json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False) + '\n').encode()


def identity(value):
    return hashlib.sha256(encoded(value)).hexdigest()


def atomic_write(path, value):
    path = Path(path)
    # A same-directory replacement never exposes a partial JSON success.
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix='.' + path.name, delete=False) as stream:
        temporary = Path(stream.name)
        try:
            stream.write(encoded(value))
            stream.flush()
            os.fsync(stream.fileno())
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise
    try:
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def require_completed(report, run_id, request, *, returncode):
    """The caller supplies the expected invocation AND exact input/tool manifest."""
    if (type(returncode) is not int or returncode != 0
            or type(report.get('schema_version')) is not int
            or report['schema_version'] != 3
            or report.get('run') != {'id': run_id, 'status': 'completed',
                                     'request_sha256': identity(request)}
            # Python container equality coerces bool/int/float recursively;
            # compare the same canonical JSON domain used by the run digest.
            or encoded(report.get('request')) != encoded(request)):
        raise ValueError('report does not complete this exact invocation and request')
    return report


class RunReport:
    def __init__(self, path, run_id=None):
        self.path = Path(path)
        self.run_id = str(uuid.UUID(run_id)) if run_id is not None else str(uuid.uuid4())
        self.request = None
        self.finished = False

    def __enter__(self):
        runs = self.path.with_name(self.path.name + '.runs')
        runs.mkdir(exist_ok=True)
        directory = runs / self.run_id
        # Never reuse an invocation, even after interruption. A retry is a new run.
        directory.mkdir(exist_ok=False)
        self.receipt = directory / 'receipt.json'
        self._publish('running', {})
        return self

    def _publish(self, status, payload):
        report = dict(payload, schema_version=3, request=self.request,
                      run={'id': self.run_id, 'status': status,
                           'request_sha256': identity(self.request)})
        atomic_write(self.receipt, report)
        atomic_write(self.path, report)
        return report

    def bind(self, request):
        if self.request is not None:
            raise ValueError('a run request cannot be rebound')
        # Freeze a value snapshot, not caller-owned mutable dictionaries.
        self.request = json.loads(encoded(request))
        self._publish('running', {})

    def complete(self, payload):
        if self.request is None or self.finished:
            raise ValueError('completion requires one bound, unfinished invocation')
        report = self._publish('completed', payload)
        require_completed(json.loads(self.path.read_bytes()), self.run_id, self.request, returncode=0)
        self.finished = True
        return report

    def __exit__(self, kind, error, traceback):
        if error is not None or not self.finished:
            self._publish('failed', {'failure': type(error).__name__ if error else 'IncompleteRun'})
        return False
