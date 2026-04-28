# 13 — Channels

Channels are how async tasks communicate. Verum ships several flavors
in `core.async.channel`:

| Function | Capacity | Use case |
|---|---|---|
| `bounded::<T>(n)` | Fixed-size ring buffer | Backpressure on producer |
| `unbounded::<T>()` | Grows dynamically | Producer never waits |
| `oneshot::<T>()` | Exactly one send | Reply channels, RPC |

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
