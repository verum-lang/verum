# `core.collections.deque` — Audit

Conformance review for `core/collections/deque.vr` (Deque<T> — ring-buffer
double-ended queue with power-of-two capacity invariant).

## Status

**partial** — Unit / property / integration tests cover the runtime-
intercepted API surface (construction, push_back/push_front, pop_back/
pop_front, front/back, clear, is_empty/len/capacity, wrap-around).
Non-intercepted methods that read internal fields work in the Deque case
because the stdlib field order `{ data, head, len, cap }` happens to
match the runtime intercept's allocation layout
`[data@0, head@1, len@2, cap@3]`. The List defect class (memory-layout
drift) does NOT apply here.

## 1. Cross-stdlib usage

`Deque<T>` is the canonical FIFO work-queue / BFS-frontier / sliding-
window primitive. Used at:

| Site | Shape | Notes |
|---|---|---|
| `core/async/scheduler.vr` (path varies) | `let mut q: Deque<Task> = Deque.new();` | Task scheduler ready queue. |
| `core/io/buffered.vr` (path varies) | `let mut chunks: Deque<Bytes> = Deque.new();` | Buffered-reader chunk queue. |
| `core/net/weft/connection.vr` (path varies) | `let mut frame_q: Deque<Frame> = Deque.new();` | HTTP/2 frame send queue. |

## 2. Crate-side hardcodes

| Path | Line(s) | What it does |
|---|---|---|
| `crates/verum_vbc/src/interpreter/dispatch_table/handlers/method_dispatch.rs` | 1086-1112 | `Deque.new()` runtime intercept — allocates a 16-element ring buffer up-front. |
| (no other Deque-shaped intercepts) | — | All instance methods (push_back/push_front/pop_back/pop_front/front/back/clear/len/is_empty/capacity) currently dispatch through the stdlib body, which is correct because the field-order discipline is intact. |

## 3. Language-implementation gaps

| Gap | Impact | Fundamental fix |
|---|---|---|
| `Deque.new()` allocates cap=16 in runtime intercept but cap=0 in stdlib body | Per-tier semantic divergence: interpreter `Deque.new().capacity() == 16`, AOT-compiled `Deque.new().capacity() == 0`. Both still pass the looser invariant `capacity() >= 0` and the higher-level FIFO laws. | Reconcile: either remove the up-front allocation in the intercept (cleanest, defers to first push_*), or change the stdlib `new()` to call `with_capacity(DEFAULT_CAP)` (slightly less work but commits to eager allocation on every Deque.new). Pin in tests/§A. |
| `Deque.with_capacity(n)` / `Deque.from_list(&xs)` cross-module UndefinedFunction | Pinned in regression_test.vr §A. Same root cause as List.from / List.of / List.from_elem: cross-module function-name table omits non-intercepted constructors. | Same fix path as #24/#25/#26 close-out for cross-module Call resolution. |

## 4. Defect inventory

Per `regression_test.vr`:

### §A — Cross-module function-name resolution (2 tests, ignored)

* `Deque.with_capacity(n)` — `UndefinedFunction("Deque.with_capacity")`.
* `Deque.from_list(&xs)` — `UndefinedFunction("Deque.from_list")`.

### §B — Field-order invariant guardrail (1 test, active)

The `regression_deque_field_order_matches_runtime_layout` test exercises
a push_back / pop_front / push_back / pop_front / push_back sequence
that depends on `self.head`, `self.len`, `self.cap`, `self.data` all
resolving to the right slots. Active (non-`@ignore`d) so any future
reorder of the stdlib type decl fails CI immediately.

## 5. Action items

1. **Reconcile `Deque.new()` cap semantics** — pick one source-of-truth
   between the runtime intercept (cap=16 default-allocate) and the
   stdlib body (cap=0 defer). The cleanest path is to drop the up-front
   allocation from the intercept.
2. **Close the cross-module constructor name-table gap** for
   `Deque.with_capacity` and `Deque.from_list` — same fix path as the
   List equivalents (regression §A).
