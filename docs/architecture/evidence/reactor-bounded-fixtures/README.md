# Bounded reactor fixture evidence

The two socket-positive reactor tests use bounded connection setup and retain
owned peers through their unchanged `Ready` assertions. The writable fixture
confirms both socket endpoints before waiting. The arrival fixture preserves
its existing scheduling, captures the readiness outcome, then finishes its
bounded connector and checks setup before classifying that outcome. This does
not prove that reactor registration preceded the connection.

The single focused serial run at `e78aa2a27` selected exactly two tests and passed
both: 0.06 seconds in tests, 29.223 seconds including compilation. No retry,
fixture deadline increase, serialization change, test ignore or production
networking change was made. Only a finished connector is joined; cleanup has
an explicit deadline and reports a worker which exceeds it.

[The integration receipt](integration.json) links the exact Cargo command,
complete frozen VBC source hashes, retained executable, verbatim compressed log,
unchanged inherited artifact identities and explicit target release. The
precompiled artifacts were reused; no fresh standard-library producer ran.

T1650 remains open. This is a serial fixture result, without parallel, complete
library, ordinary CLI, native/AOT or registry-service acceptance. The previous
full-run arrival failure had unconfirmed setup and is not a causal reactor
regression oracle. TCP connection failures remain distinct from readiness
outcomes. The UDP receive failure returned `None`, which collapses registration,
clone and receive errors; the retained evidence does not prove a UDP timeout.
