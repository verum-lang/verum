"""Bounded subprocess execution and exact compiler/runtime identity for MIR replay."""
import hashlib
import os
import signal
import subprocess

RUST_VERSION = 'rustc 1.100.0-nightly (8925ea358 2026-08-20)'


def execute(command, cwd, log, env=None):
    with subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                          text=True, start_new_session=True) as process:
        try:
            stdout, stderr = process.communicate(timeout=30)
        except BaseException:
            os.killpg(process.pid, signal.SIGKILL)
            process.communicate()
            raise
    log.write_text(stdout + stderr)
    return process.returncode, stdout, stderr


def runtime_identity(executable):
    root = executable.resolve().parent.parent
    paths = [executable.resolve(), *sorted((root/'lib').rglob('*'))]
    result = {}
    for path in paths:
        if path.is_file():
            with path.open('rb') as stream:
                result[str(path.relative_to(root))] = hashlib.file_digest(stream, 'sha256').hexdigest()
    return result


