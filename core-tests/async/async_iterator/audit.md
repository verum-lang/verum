# `core.async.async_iterator` — audit

The `AsyncIterator` protocol backs `for await ... in ...` syntactic
sugar.  The compiler's for-await-loop lowering rewrites the loop body
into an explicit `loop { match it.next().await { Some(_) => …; None
=> break; } }`.  `IntoAsyncIterator` mirrors `IntoIterator`'s role
for sync iterators: a way to construct an iterator from a
collection-shaped value.

## 1. Cross-stdlib usage

| owning module | concrete impl |
|---|---|
| `core.async.channel.Receiver<T>` | direct `implement<T> AsyncIterator for Receiver<T>` block |
| `core.async.broadcast.BroadcastReceiver<T>` | same |
| `core.net.tcp.AsyncIncoming<'a>` | same; yields `Result<TcpStream, IoError>` |
| `core.net.unix.AsyncIncoming<'a>` | same; yields `Result<UnixStream, UnixError>` |
| Stream→AsyncIterator | **NOT WIRED** — see §3 §A |

## 2. Crate-side hardcodes / drift surfaces

| site | drift surface | risk |
|---|---|---|
| `crates/verum_types/src/protocol.rs::resolve_async_iterator_protocol` | for-await-loop lowering reads the `AsyncIterator` protocol's `Item` associated type and `next` signature | MEDIUM — drift between this protocol declaration and the compiler's lowering would break every `for await` site silently |
| Grammar `for_await_loop` production (v2.10) | sugar surface | LOW — grammar is authoritative |

## 3. Language-implementation gaps surfaced by this suite

### §A — Stream→AsyncIterator blanket impl projection-reduction gap

The async_iterator.vr module docstring says (verbatim):

> A blanket `implement<S: Stream> AsyncIterator for S` is semantically
> the right answer and is what this module *wants* to express. The
> current compiler does not reduce associated-type projections
> (`S.Item`) through blanket impls — `for await v in receiver { ... }`
> ends up with `v` typed as an unresolved `Item<Receiver<T>>` instead
> of the concrete `T`.

**Workaround in stdlib**: each Stream-shaped type carries its own
direct `implement<T> AsyncIterator for Self` block alongside its
Stream impl.  This is correct but redundant — when the protocol
resolver gains projection-reduction, all four impls collapse into the
blanket and the owning modules get smaller.

**Tracking task**: protocol-resolver projection-reduction (upstream;
not specific to async_iterator).

### §B — `IntoAsyncIterator` self-conversion blanket

The blanket `implement<A: AsyncIterator> IntoAsyncIterator for A` is
the @inline identity — `a.into_async_iter()` returns `a` for any
AsyncIterator.  This compiles cleanly under the Tier-0 interpreter
(pinned by `property_test.vr §A`).  The associated-type projection
`B.IntoAsyncIter` at function boundary also compiles (pinned by
`integration_test.vr §B`).

The combination — a function bound by `IntoAsyncIterator` invoking
`.into_async_iter()` and then `.next().await` on the result — has not
been pinned at this layer because it requires a running async
context.  Once the executor lands (task #10 close), every concrete
AsyncIterator impl's `next().await` round-trip exercises this path.

## Action items landed in this branch

- `unit_test.vr` expanded from 2 → 4 tests:
  - Both protocols mount cleanly
  - Protocol-bound generic functions compile (catches archive-load
    regressions of the protocol-typed surface)
- New `property_test.vr` with 3 laws:
  - IntoAsyncIterator self-conversion blanket compiles for any A
  - Multi-bound (`A: AsyncIterator + Clone`) compiles
  - Associated-type projection `B.IntoAsyncIter` resolves at boundary
- New `integration_test.vr` with 2 cross-module scenarios:
  - `List<A>` round-trip with `A: AsyncIterator` bound
  - Associated-type at function boundary
- `regression_test.vr` retained as-is (one mount-no-panic pin).

## Action items deferred

| § | scope | tracking | est. |
|---|---|---|---|
| §A | protocol-resolver projection-reduction through blanket impls | upstream | multi-week |
| §B | live executor + `for await` end-to-end test | task #10 | gated on AOT |
