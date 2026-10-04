#!/usr/bin/env python3
"""T1507/T1516: explicit localhost integration gate, separate from source gates.

One Verum invocation runs repository language contracts and four HTTP clients.
Both tiers use the same source; AOT therefore compiles once. Generated sources
and logs remain in the printed temporary directory, including after failure.
"""
import argparse
import json
import os
from pathlib import Path
import re
import select
import signal
import socket
import subprocess
import tempfile
import time


CASES = ("http200", "header_timeout", "cancelled", "slow_drip")
REQUEST = b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
BUDGET_MS, DRIP_INTERVAL = 500, 0.08
FIXTURES = (
    ("vbc/shared_method_receiver.vr", "shared_contract"),
    ("async/explicit_future_protocol.vr", "future_contract"),
    ("stdlib-runtime/cancellation_and_connection_capacity.vr", "cancellation_contract"),
    ("async/weft_read_deadlines.vr", "weft_contract"),
    ("vbc/generic_replace_iteration.vr", "generic_replace_contract"),
)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def emit(**fields):
    print(json.dumps(fields, ensure_ascii=False), flush=True)


def prepare(repo, port):
    parts, expected, calls = [], [], []
    for relative, name in FIXTURES:
        path = repo / "vcs/specs/L0-critical" / relative
        source = path.read_text(encoding="utf-8")
        block = re.search(r"// @expected-stdout-begin\n(.*?)// @expected-stdout-end", source, re.S)
        require(block is not None, f"missing expected stdout in {path}")
        expected.extend(re.sub(r"^// ?", "", line) for line in block[1].splitlines())
        source, count = re.subn(r"(?m)^(?:async\s+)?fn\s+main\s*\(\s*\)",
                                f"async fn {name}()", source)
        require(count == 1, f"expected exactly one main in {path}, found {count}")
        parts.append(source)
        calls.append(f"    {name}().await;")
    template = (Path(__file__).resolve().parent / "fixtures/weft_http_contract.vr.in").read_text()
    template = (template.replace("@PORT@", str(port)).replace("@BUDGET_MS@", str(BUDGET_MS))
                .replace("@CONTRACT_CALLS@", "\n".join(calls)))
    require(not re.search(r"@[A-Z_]+@", template), "unexpanded server template placeholder")
    return "\n\n".join(parts + [template]), expected


def tail(log, size=16384):
    with log.open("rb") as data:
        data.seek(max(0, log.stat().st_size - size))
        return data.read().decode(errors="replace")


def stop_owned(process):
    if process.poll() is not None:
        return
    def stop(force):
        try:
            if os.name == "posix":
                os.killpg(process.pid, signal.SIGKILL if force else signal.SIGTERM)
            elif force:
                process.kill()
            else:
                process.terminate()
        except ProcessLookupError:
            pass
    stop(False)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        stop(True)
        process.wait(timeout=5)


def connect(process, port, log, timeout):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        require(process.poll() is None, f"child exited before listening: {tail(log)}")
        try:
            return socket.create_connection(("127.0.0.1", port), timeout=0.2)
        except (ConnectionRefusedError, TimeoutError):
            time.sleep(0.05)
    raise TimeoutError(f"listener was not ready within {timeout}s")


def wait_marker(process, log, marker, timeout):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if marker in tail(log).splitlines():
            return time.monotonic()
        require(process.poll() is None, f"missing {marker!r}: {tail(log)}")
        time.sleep(0.01)
    raise TimeoutError(f"missing {marker!r} after {timeout}s: {tail(log)}")


def read_response(client, case, start, timeout):
    response, sent, first = bytearray(), 0, None
    next_send, end = start, start + timeout
    if case == "http200":
        client.sendall(REQUEST)
        sent = len(REQUEST)
    while time.monotonic() < end:
        now = time.monotonic()
        if case == "slow_drip" and sent < len(REQUEST) and now >= next_send:
            try:
                client.sendall(REQUEST[sent:sent + 1])
                sent += 1
            except (BrokenPipeError, ConnectionResetError):
                pass  # The peer may have queued its 408 before closing.
            next_send = now + DRIP_INTERVAL
        interval = min(0.05, max(0, end - now))
        if case == "slow_drip" and sent < len(REQUEST):
            interval = min(interval, max(0, next_send - now))
        if not select.select([client], [], [], interval)[0]:
            continue
        try:
            chunk = client.recv(4096)
        except ConnectionResetError:
            break
        if not chunk:
            break
        if first is None:
            first = time.monotonic()
        response.extend(chunk)
        require(len(response) <= 65536, "response exceeded 64 KiB")
    else:
        raise TimeoutError(f"response/close exceeded {timeout}s: {bytes(response[:200])!r}")
    return bytes(response), first, sent


def run(args, program, log, port, expected):
    with log.open("wb") as output:
        process = subprocess.Popen([str(args.cli), "run", "--tier", args.tier, str(program)],
                                   cwd=args.repo, stdout=output, stderr=output, start_new_session=True)
        try:
            for case in CASES:
                with connect(process, port, log, args.startup_timeout) as client:
                    started = wait_marker(process, log, f"armed={case}", args.armed_timeout)
                    response, first, sent = read_response(client, case, started, args.io_timeout)
                outcome = "ok" if case == "http200" else "cancelled" if case == "cancelled" else "timeout"
                wait_marker(process, log, f"case={case} outcome={outcome}", args.armed_timeout)
                elapsed = time.monotonic() - started
                if case == "cancelled":
                    require(not response, f"cancelled connection wrote {response[:200]!r}")
                    require(elapsed <= args.cancel_limit, f"cancel took {elapsed:.3f}s")
                else:
                    status = b"200" if case == "http200" else b"408"
                    require(response.startswith(b"HTTP/1.1 " + status + b" "), repr(response[:200]))
                    headers, separator, body = response.partition(b"\r\n\r\n")
                    require(bool(separator), "response has no header terminator")
                    require(all(byte in (10, 13) or 32 <= byte < 127 for byte in headers),
                            f"response headers contain non-ASCII bytes: {headers[:200]!r}")
                    expected_body = bytes((0x56, 0x00, 0xFF, 0x2A)) if case == "http200" else b"request timeout"
                    require(body == expected_body, f"wire body mismatch: {body[:200]!r}")
                    if case != "http200":
                        require(first is not None, "no timeout response")
                        delay = first - started
                        require(BUDGET_MS / 2000 <= delay <= BUDGET_MS / 1000 + args.deadline_slack,
                                f"timeout response delay {delay:.3f}s is outside the deadline tolerance")
                    if case == "slow_drip":
                        require(2 <= sent < len(REQUEST), f"drip sent {sent} bytes before timeout")
                emit(case=case, tier=args.tier, verdict="PASS", elapsed_s=round(elapsed, 3),
                     sent_bytes=sent, response=response[:200].decode(errors="replace"))
            process.wait(timeout=args.exit_timeout)
            require(process.returncode == 0, f"child exit={process.returncode}: {tail(log)}")
            lines = log.read_text(errors="replace").splitlines()
            for line in expected:
                require(line in lines, f"missing contract output: {line}")
            emit(tier=args.tier, verdict="PASS", contracts=len(FIXTURES), http_cases=len(CASES))
        finally:
            stop_owned(process)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path, help="explicit Verum CLI executable")
    parser.add_argument("--tier", choices=("interpret", "aot"), default="interpret")
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--output-dir", type=Path, help="parent for a unique retained run directory")
    parser.add_argument("--startup-timeout", type=float, help="seconds; default 300 interpret / 1200 AOT")
    parser.add_argument("--armed-timeout", type=float, default=20)
    parser.add_argument("--io-timeout", type=float, default=10)
    parser.add_argument("--exit-timeout", type=float, default=10)
    parser.add_argument("--cancel-limit", type=float, default=2)
    parser.add_argument("--deadline-slack", type=float, default=2)
    parser.add_argument("--prepare-only", action="store_true", help="generate source without CLI or sockets")
    args = parser.parse_args()
    if args.startup_timeout is None:
        args.startup_timeout = 1200 if args.tier == "aot" else 300
    for name in ("startup_timeout", "armed_timeout", "io_timeout", "exit_timeout", "cancel_limit", "deadline_slack"):
        if not 0 < getattr(args, name) < float("inf"):
            parser.error(f"--{name.replace('_', '-')} must be finite and positive")
    args.repo, args.cli = args.repo.resolve(), args.cli.resolve()
    if args.output_dir:
        args.output_dir.mkdir(parents=True, exist_ok=True)
    run_dir = Path(tempfile.mkdtemp(prefix="weft-http-", dir=args.output_dir)).resolve()
    program, log = run_dir / "server.vr", run_dir / "server.log"
    log.touch()
    emit(run_dir=str(run_dir), source=str(program), log=str(log), cli=str(args.cli), tier=args.tier)
    try:
        port = 0
        if not args.prepare_only:
            # Release the ephemeral reservation before our child binds it.
            # Child-owned armed markers reject an unrelated listener in a bind race.
            with socket.socket() as reservation:
                reservation.bind(("127.0.0.1", 0))
                port = reservation.getsockname()[1]
        source, expected = prepare(args.repo, port)
        program.write_text(source, encoding="utf-8")
        if args.prepare_only:
            emit(verdict="PREPARED", contracts=len(FIXTURES), expected_lines=len(expected), http_cases=len(CASES))
        else:
            run(args, program, log, port, expected)
        return 0
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        emit(tier=args.tier, verdict="FAIL", error=str(error), log=str(log))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
