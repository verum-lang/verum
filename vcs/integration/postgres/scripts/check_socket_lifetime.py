#!/usr/bin/env python3
"""Verify PostgreSQL socket release before interpreter process shutdown.

Requires lsof and a plaintext test PostgreSQL configured by VERUM_PG_TEST_*
(the integration fixture defaults are accepted). Uses only SELECT and connection
attempts; it neither creates nor stops the database.
"""
import argparse
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time

MARKER = "postgres lifetime: connections closed; inspecting process descriptors"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("verum", type=Path)
    parser.add_argument("--timeout", type=float, default=300)
    options = parser.parse_args()
    executable = str(options.verum.resolve(strict=True))
    if not 0 < options.timeout <= 1800:
        parser.error("timeout must be between 0 and 1800 seconds")
    lsof = shutil.which("lsof")
    if lsof is None:
        parser.error("lsof is required to inspect sockets before process exit")
    port = int(os.environ.get("VERUM_PG_TEST_PORT", "55432"))
    source = Path(__file__).resolve().parents[1] / "tests/t34_socket_lifetime.vr"
    with tempfile.TemporaryDirectory(prefix="verum-postgres-lifetime-") as temporary:
        project = Path(temporary)
        (project / "src").mkdir()
        (project / "verum.toml").write_text(
            '[cog]\nname="postgres-socket-lifetime"\nversion="0.1.0"\nedition="2026"\n')
        shutil.copyfile(source, project / "src/main.vr")
        output, errors = project / "stdout.log", project / "stderr.log"
        with output.open("wb") as stdout, errors.open("wb") as stderr:
            child = subprocess.Popen([executable, "run", "--tier", "interpret"],
                cwd=project, env={**os.environ, "NO_COLOR": "1", "VERUM_NO_OBJECT_CACHE": "1"},
                stdout=stdout, stderr=stderr, start_new_session=True)
            try:
                deadline = time.monotonic() + options.timeout
                while MARKER not in output.read_text(errors="replace"):
                    if child.poll() is not None:
                        raise RuntimeError(f"socket probe exited early ({child.returncode})")
                    if time.monotonic() >= deadline:
                        raise TimeoutError("socket probe did not reach descriptor inspection")
                    time.sleep(0.1)
                descriptors = subprocess.run([lsof, "-nP", "-a", "-p", str(child.pid), "-iTCP"],
                    capture_output=True, text=True, timeout=10)
                if descriptors.returncode not in (0, 1) or descriptors.stderr.strip():
                    raise RuntimeError("descriptor inspection failed: " + descriptors.stderr)
                if child.poll() is not None:
                    raise RuntimeError("probe exited before descriptor inspection completed")
                leaked = [line for line in descriptors.stdout.splitlines()
                    if line.partition("->")[2].split(" ", 1)[0].endswith(":" + str(port))]
                if leaked:
                    raise AssertionError("database sockets remain after close/rejection:\n" + "\n".join(leaked))
                code = child.wait(timeout=30)
                if code:
                    raise RuntimeError(f"socket probe failed after inspection ({code})")
                print("postgres lifetime: close, authentication rejection and TLS refusal leave no sockets")
            except Exception as error:
                raise RuntimeError(str(error) + "\n" + output.read_text(errors="replace")[-4000:]
                    + "\n" + errors.read_text(errors="replace")[-8000:]) from error
            finally:
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait()


if __name__ == "__main__":
    main()
