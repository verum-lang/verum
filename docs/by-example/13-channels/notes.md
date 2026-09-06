# 13 — Channels

Channels are how async tasks communicate. Verum ships several flavors
in `core.async.channel`:

| Function | Capacity | Use case |
|---|---|---|
| `bounded<T>(n)` | Fixed-size ring buffer | Backpressure on producer |
| `bounded_channel<T>(n)` | same, longer name | both are exported |
| `unbounded_channel<T>()` | Grows dynamically | Producer never waits |
| `channel<T>()` | Grows dynamically | the short name for the same thing |
| `oneshot<T>()` | Exactly one send | Reply channels, RPC |

There is no turbofish in Verum: the type argument goes in angle
brackets directly after the name, `bounded<T>(n)`, never
`bounded::<T>(n)`. And there is no `unbounded` — the exported names are
`unbounded_channel` and `channel`.

`oneshot<T>()` is real and answers a DIFFERENT pair —
`(OneshotSender<T>, OneshotReceiver<T>)`, not `(Sender<T>, Receiver<T>)`
— so it is not a capacity-1 `bounded` and the two are not
interchangeable.

## Backpressure for free

Bounded channels make `send` block when the buffer is full —
producers automatically slow down to match the consumer's rate. No
extra "rate limiter" component needed; the channel's capacity *is*
the rate limiter.

## Multi-producer pattern

`Sender<T>` is `Clone` — clone it once per producer and each producer
gets its own handle. The channel closes (and `recv` returns `None`)
when the **last** sender is dropped; no sentinel value or "I'm done"
flag needed.

## Single-receiver pattern (this example)

This example uses a single owned `Receiver<T>` for guaranteed
delivery ordering: the consumer sees messages in send-order from
each producer. For multi-consumer fan-out, use `Receiver::shared()`
to convert to a `SharedReceiver<T>` that can be cloned.

## Why not Mutex<Vec<T>>

A naive `Arc<Mutex<Vec<T>>>` queue forces every consumer to acquire
the lock just to check if the queue is empty, and producers must
notify consumers separately (condvar). Channels bake the
synchronization in: `send` and `recv` are the only operations, and
both compose with `.await` so they yield instead of blocking the OS
thread.
