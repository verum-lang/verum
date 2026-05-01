# Weft Red-Team Audit — 2026-05-01

**Target:** `core/net/weft/**` (~9 200 LOC across 32 modules).
**Scope:** Production-readiness for high-RPS / fault-tolerant web servers.
**Methodology:** Systematic adversarial reading of accept-loop +
per-connection HTTP/1.1 pipeline + handler dispatch.  Each finding
includes an attack scenario, the source-level evidence, and a
fundamental (architecture-level) fix that makes the class of bug
**impossible by construction**, in keeping with Verum's core thesis.

---

## §1. Critical: Connection-count leak under handler panic

### Attack
Open many TCP connections in succession to a route whose handler
panics on any input.  Each panic kills the spawned task without
running the `in_flight.fetch_sub` decrement at the end of the
spawn closure.  After ~10 000 panic-prone requests, `in_flight`
saturates at `max_concurrent_connections`; the listener refuses
every subsequent connection with 503.  The server is dead while
the verum process is still healthy — **silent DoS via
buggy-handler attack**.

### Evidence
`core/net/weft/listener.vr:213-216`:

```verum
let _ = task.spawn(async move {
    runner_c.run(stream, peer).await;
    counter.fetch_sub(1, MemoryOrdering.AcqRel);
});
```

If `runner_c.run` returns or panics, `fetch_sub` only runs in the
return case.  No try/finally; no `defer`; no RAII guard.

### Fundamental fix
Replace the manual counter with an RAII `ConnectionTicket`
acquired from a `ConnectionLimiter`.  The ticket's `Drop` impl
calls `fetch_sub` unconditionally (whether the holding task
returned normally or unwound).  Verum has CBGR-managed Drop, so
this is a sound enforcement vector — the type system makes the
decrement leakage impossible at compile time.

```verum
public type ConnectionTicket is { limiter: Shared<ConnectionLimiter> };
implement Drop for ConnectionTicket {
    fn drop(&mut self) { self.limiter.release(); }
}
```

The accept loop then becomes:

```verum
let ticket = match limiter.acquire() {
    Some(t) => t,
    None    => { runner.refuse_overloaded(stream).await; continue; }
};
task.spawn(async move {
    let _guard = ticket;          // released on every exit path
    runner_c.run(stream, peer).await;
});
```

---

## §2. Critical: Slowloris DoS — no read-timeout discipline

### Attack
Open TCP connection.  Send `G` (one byte).  Wait 60 seconds.
Send `E`.  Wait 60 seconds.  …  Loop forever, holding the
connection.  Repeat from `max_concurrent_connections + 1`
distinct source IPs.  Server saturates; legitimate clients see
503.  Classic 1990s DoS, still effective.

### Evidence
`core/net/weft/connection.vr:163-197` —
`serve_one_message`'s read loop calls
`stream.read_cancellable(&mut chunk, token).await` with **no
deadline**.  The `token` cancels only on listener-wide shutdown,
not per-request.  No `Instant::now() + Duration::from_secs(30)`
deadline check anywhere in the loop.

`ConnectionConfig` (line 28):
```verum
public type ConnectionConfig is {
    max_request_size: Int,
    read_buffer_capacity: Int,
    keep_alive: Bool,
};
```
Three fields.  **No `read_header_timeout`, no `read_body_timeout`,
no `idle_timeout`.**  The spec promises p99.9 < 200 µs but the
implementation has no time discipline at all.

### Fundamental fix
1. Add three `Duration` fields to `ConnectionConfig` —
   `header_deadline`, `body_deadline`, `idle_deadline` — with
   reasonable defaults (5 s / 30 s / 60 s).
2. Wrap `stream.read_cancellable` in a `with_timeout(deadline,
   future)` combinator that races the read against a timer.  On
   timeout: 408 Request Timeout, close.
3. Tie the deadlines to the listener token via a child
   cancellation source so a graceful shutdown cancels in-flight
   reads.

This is unification with `core/async/timer.vr`'s existing
`with_timeout` — already imported in the listener (`mount
core.async.timer`) but not used in the connection's read path.
The fix is "wire what's already there."

---

## §3. High: O(n²) attacker-controlled byte buffer conversion

### Attack
Send a 1 MiB request whose headers exhaust `max_request_size`
(default 1 MiB).  The body itself is small, but the headers fill
the budget exactly.  Each TCP read brings ~4 KiB; the parser
needs all of it before deciding `Done`.  Server does ~256
iterations × O(n) byte conversion = **O(n²) work for O(n) data**.

### Evidence
`core/net/weft/connection.vr:184` (inside the read loop):
```verum
let bytes = int_list_to_bytes(&accum);   // converts ENTIRE accumulator
match parser.feed(&bytes) { ... }
```

`int_list_to_bytes` walks `accum` in full each iteration.  For a
1 MiB header line received in 4 KiB chunks, total copy work is
roughly `1 MiB × 256 / 2 ≈ 128 MiB` of byte moves for one
request.  At 10 k concurrent slow uploads this is 1.2 TiB of
busywork inside `int_list_to_bytes` alone.

### Fundamental fix
Replace the `List<Int>` accumulator with `BufPool`-backed
contiguous `&mut [u8]` (already present at
`core/net/weft/bufpool.vr`).  Parser feeds incremental views
without re-copying.  HTTP parsing should be O(n) on bytes
received, not O(n²).

Secondary: change the type from `List<Int>` to `List<Byte>` —
pre-fix uses 8x memory (Int = i64 NaN-boxed Value, Byte = 1
byte).  16 KiB read buffer occupies 128 KiB.

---

## §4. High: 8× memory amplification in read buffers — **PARTIALLY FIXED**

### Attack
Server sized for 10 000 connections × 16 KiB read buffer = 160
MiB by spec.  Actual VBC representation: 10 000 × 128 KiB = 1.28
GiB just for read buffers — **8× memory inflation versus the
spec's footprint target** (< 8 KiB per idle connection).

### Evidence
Same site as §3.  `let mut accum: List<Int> = List.with_capacity(...)`.
NaN-boxed Value = 8 bytes per element vs 1 byte for `List<Byte>`
*if* the runtime uses a packed-byte backing.

### Status
Type-system migration **complete**: `core/io/async_protocols.vr`'s
`AsyncRead` / `AsyncWrite` / `AsyncBufRead` protocols speak
`List<Byte>` end-to-end; every implementer (TcpStream, UnixStream,
TlsStream) drops the int↔byte marshalling code (~30 LOC saved per
impl); the `WeftTransport` adapter is now a direct passthrough.
The semantic gain (bytes are bytes) is realised; the per-byte
cast on read+write is eliminated.

The **runtime memory reduction remains pending** a separate
VBC-level optimisation: today both `List<Byte>` and `List<Int>`
use the canonical 3-slot list layout with one NaN-boxed Value per
element (`alloc_byte_list` in
`crates/verum_vbc/src/interpreter/dispatch_table/handlers/heap_helpers.rs:124`),
so byte-typing alone does not change the heap footprint.  The
heap helpers already support a packed-byte FatRef path
(`extract_byte_slice` arm `1 => unsafe { slice::from_raw_parts(...) }`
at line 173) — the optimisation is to make codegen lower
`List<Byte>` to that packed path for read-buffer-shaped allocations.
Tracked separately so the type-system migration can land cleanly.

---

## §5. Medium: max_request_size enforced after read, not during

### Attack
`config.max_request_size` is checked *after* `chunk.push` in the
loop.  A single 100 MiB read (from a malicious peer that
TCP-coalesces a huge buffer into one read) would push past the
limit before the check fires, allocating ~100 MiB before
rejection.  Less critical than slowloris but still a memory
amplifier.

### Evidence
`core/net/weft/connection.vr:164-166`.  The `chunk` allocation
is bounded by `List.with_capacity(4096)` so practically the
overshoot is one chunk = 4 KiB, not catastrophic.  The
chunked-decoder path at line 426 does the right thing already.

### Fundamental fix
Tighten the check to fire on `accum.len() + n > max_request_size`
*before* the per-read append loop runs.  One-line change.

---

## §6. Medium: refuse_overloaded write is unbounded — **FIXED**

**Status:** Closed.  `core/net/weft/listener.vr::refuse_overloaded`
now uses fire-and-forget `task.spawn` + `future_timeout_ms(1_000, ...)`
to bound the write at one second.  The accept loop returns immediately
after the spawn — slow refused clients can no longer gate accept.
See lines 181-198 of listener.vr (pre-fix vs post-fix annotated in
source).

### Attack
Send TCP SYN, complete handshake, never read.  Server hits
`max_concurrent_connections`, calls `refuse_overloaded` on the
NEXT connection's stream.  `refuse_overloaded` calls
`stream.write_async(&buf)` — also without a deadline.  Slow
attackers force the server to hang on writes to slow refused
clients, blocking the accept loop.

### Evidence
`core/net/weft/listener.vr:166`:
```verum
let _ = stream.write_async(&buf).await;
```
No `with_timeout`.  This is the slow-client variant of slowloris
on the *refusal* path.

### Fundamental fix
Same `with_timeout` wrapper as §2.  Also: the
`refuse_overloaded` write should fire-and-forget — spawn the
write into a side-task so it doesn't gate the accept loop.

---

## §7. Architectural: backpressure decision is not type-enforced — **FIXED**

### Attack
The spec promises (§1.3) "Each `Service` обязан декларировать
`poll_ready` — отсутствует → compile error".  The implementation
has a `Service` protocol but the typechecker doesn't enforce
that **every** `Layer`-wrapped service uses the inner's
`poll_ready` properly.  A middleware author can swallow the
`Pending` signal by hard-coding `Poll::Ready(Ok(()))` in their
own `poll_ready`.  Hidden queues then grow unbounded; the system
collapses under load with no diagnostic trail.

### Fundamental fix — affine Permit pattern (LANDED)
The fix uses Verum's affine semantics rather than a new
refinement DSL.  `core/net/weft/service.vr` ships a move-only
`Permit<S, Req, Resp>` token:
```verum
public type Permit<S: Service<Req, Resp>, Req, Resp> is {
    service: &mut S,
}
implement<S: Service<Req, Resp>, Req, Resp> Permit<S, Req, Resp> {
    public async fn dispatch(self, req: Req) -> Result<Resp, S.Error> {
        self.service.call(req).await
    }
}
public fn ready<S: Service<Req, Resp>, Req, Resp>(svc: &mut S)
    -> PollReadyFuture<S, Req, Resp>;
```
Move-only / no Clone makes `Permit` affine — consumed by exactly
one `dispatch`.  The `&mut self` borrow held by the Permit
excludes concurrent mutation of the inner service.  `ready()`
is the only constructor; it internally awaits `poll_ready` via
`PollReadyFuture`.  **The compiler enforces ready-before-call as
a borrow-checker invariant; a middleware that lies cannot ladder
the lie because forwarding to its inner Service requires acquiring
its inner Permit, which requires polling its inner `poll_ready`.**

### Verification
`verum check core/net/weft/service.vr` — 0 errors.  Forward
type-parameter references inside generic bounds (`Permit<S:
Service<Req, Resp>, Req, Resp>` where the bound on `S` references
`Req`/`Resp` declared later in the same parameter list) parse and
resolve cleanly; the earlier "blocked on parser" status was a
mis-diagnosis caused by transient stdlib parse errors in
`core/async/executor.vr` (parallel-agent WIP) that masqueraded as
a forward-ref failure.  The language already supports the pattern.

---

## §8. Architectural: handler signature doesn't track failure modes — **FIXED**

### Attack
A handler returns `Result<Response, WeftError>`.  But the
*subset* of `WeftError` it can produce is not tracked at the
type level.  A middleware that maps errors to status codes can't
know whether a 500 is "the handler claimed Internal" or "Verum's
runtime panicked through the future".  Observability suffers;
incident triage takes longer.

### Fundamental fix — associated `Error` type on `Handler` (LANDED)
`core/net/weft/handler.vr` ships:
```verum
public type Handler is protocol {
    type Error: IntoResponse = WeftError;
    async fn handle(&self, req: WeftRequest) -> Result<Response, Self.Error>;
};
```
The associated `Error` type carries an `IntoResponse` bound — the
framework can always render a reply, so any user-defined override
is sound by construction.  The default `= WeftError` means every
existing `implement Handler for X` site (16 across router, rpc,
adaptive, timeout, spiffe, health, wasm_filter, zero_rtt_gate,
metrics, tracing, backpressure × 5, handler.vr) inherits the
framework's closed sum unchanged — zero migration cost.

Domain handlers can now opt into richer per-handler typing
(`type Error = MyDomainError;` where `MyDomainError: IntoResponse`),
unlocking middleware that type-routes: e.g.
`RetryLayer<H> where H::Error: RetryClassify` statically reaches
only handlers whose error converts to a retry classification.
Verum's existing computational properties (`Async | IO |
Fallible<E> | Mutates`) are inferred from the body and become
observable through this signature surface — a handler that
declares `Fallible<DatabaseError>` is statically routable to a
DB-recovery middleware.

### Verification
`verum check core/net/weft/{handler,router,backpressure,health,
rpc,adaptive,timeout,spiffe,wasm_filter,zero_rtt_gate,metrics,
tracing}.vr` — 0 errors caused by this change across all sites.
The single pre-existing E101 (`HttpRequest` mount-rename
resolution gap) in `handler.vr:90` is unrelated and predates this
work — confirmed by stash-and-recheck.  Default associated types
on protocols verified working via probe at
`/tmp/protocol_default_probe.vr`.

---

## §9. Critical: panic in handler kills connection silently

### Attack
Handler panics inside `app.handle(req).await`.  The spawned
connection task unwinds without sending a response.  Client
sees connection RST.  No 500 logged; no metric incremented; the
listener's per-connection counter (after §1 fix) decrements but
the client's request was lost without a trace.

### Evidence
`core/net/weft/connection.vr:265-268`:
```verum
let response = match app.handle(req).await {
    Ok(r)  => r,
    Err(e) => e.into_response(),
};
```
This handles the *typed* error path.  An unwinding panic from
inside `handle` (e.g., array out-of-bounds, divide-by-zero,
unwrap on None) isn't caught.

### Fundamental fix
Wrap the call in Verum's `catch_unwind` (or the equivalent
panic-fence primitive).  On caught panic: emit a 500 with the
panic message digested into a stable error code, increment a
`weft.handler.panic` metric, log the panic with request path /
method.  The connection survives for keep-alive's next
request.

The framework should make handler panics observable and
non-fatal **by default** — the developer should opt INTO
"abort the connection on panic" if they want fail-fast
semantics.

---

## §10. Architectural: no per-IP rate limit / connection throttle

### Attack
Single attacker IP opens 10 000 connections from one box.
Server saturates `max_concurrent_connections`.  No legit client
can connect.  The DoS surface includes everyone on the same
NAT.

### Fundamental fix
Add `PerIpLimiter` to `ListenerConfig`: max N concurrent
connections per peer-IP.  Implementation: `HashMap<IpAddr,
AtomicInt>` with TTL eviction.  The check happens *before*
`task.spawn` so attackers can't even sustain the per-conn cost
of an accepted connection.

---

# Implementation priority

| §  | Severity | Class                                | Impl effort  |
|----|----------|--------------------------------------|--------------|
| §1 | Critical | DoS via panic-prone handler          | ~50 LOC RAII |
| §2 | Critical | Slowloris DoS                        | ~80 LOC      |
| §9 | Critical | Handler panic silently drops request | ~60 LOC      |
| §3 | High     | O(n²) work amplifier                 | ~150 LOC     |
| §4 | High     | 8× memory inflation                  | ~200 LOC sweep |
| §10| High     | Per-IP DoS surface                   | ~100 LOC     |
| §5 | Medium   | One-chunk overshoot                  | ~10 LOC      |
| §6 | Medium   | refuse_overloaded slow-write         | ~20 LOC      |
| §7 | Architectural | Backpressure type-enforcement   | refinement-type plumbing |
| §8 | Architectural | Handler effect tracking         | effect-type plumbing |

The first three are **production-blockers** — without §1 the
server is one bad request away from a silent hang; without §2 a
single attacker can DoS it; without §9 a single panic loses a
client request without a trace.  These fixes turn weft from
"axum-shaped scaffold" into "actually production-deployable."

§7–§8 are the deeper Verum-thesis-aligned fixes — they make
**incorrect server code uncompilable**, the goal of "не позволять
создавать некорректные или уязвимые системы" (Verum's core
architectural idea per the project README and CLAUDE.md).
