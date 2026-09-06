# 19 — File I/O

The `core.io` module provides three layers depending on file size and
access pattern:

| Layer | API | When |
|---|---|---|
| One-shot | `read_to_string` / `write` | File fits in memory |
| Buffered | `BufReader` / `BufWriter` | Streaming over a large file |
| Raw | `File.open` + `read` / `write` | Custom buffering / async |

## One-shot helpers

`read_to_string(path) -> Result<Text, IoError>` reads the whole file,
validates UTF-8, and returns a `Text`. `write(path, &[Byte]) ->
Result<(), IoError>` overwrites the file. These are the right choice
for config files, small data files, anything ≤ a few MiB.

## BufReader

For larger files, wrap a `File` in `BufReader` and iterate `.lines()`.
The reader keeps an ~8 KiB buffer — one read syscall per buffer fill,
not per line. Line splitting happens in-memory.

```verum
let reader = BufReader.new(File.open(&path)?);
for line in reader.lines() {
    let line = line?;          // each yields Result<Text, IoError>
    process(&line);
}
```

## Async file I/O

`core.io` exports synchronous I/O. For async, the type is `AsyncFile`
and it lives in the SAME module — `core/io/file.vr:509` — not in a
separate `core.async.fs`, which does not exist. Its surface is
`open` / `open_with_options` / `create` / `read` / `read_to_end` /
`read_to_string` / `write` / `write_all` / `seek` / `flush` /
`sync_all`, each `async`, plus `get_ref` / `get_mut` / `into_inner` /
`size`.

Two shapes differ from the sync API rather than mirroring it:
`AsyncFile.open` takes the path as `&Text`, not `&Path`; and
`BufReader`'s async line reader is `next_line_async()`, which answers
`IoResult<Maybe<Text>>` — `Maybe.None` is EOF, not a zero byte count.

## Errors

`IoError` is the common error type — `core/io/mod.vr:56` aliases it to
`StreamError`. **It is a RECORD, not a sum:**

```verum
public type StreamError is { kind: IoErrorKind, message: Maybe<Text> };
```

so you match on `err.kind`, not on the error itself, and the message is
a separate `Maybe<Text>` field rather than a payload on a variant.
`IoErrorKind` is the sum — `NotFound`, `PermissionDenied`,
`ConnectionRefused`, `ConnectionReset`, `ConnectionAborted`,
`NotConnected`, `AddrInUse`, `AddrNotAvailable`, `BrokenPipe`,
`AlreadyExists`, `WouldBlock`, `InvalidInput`, `InvalidData`,
`TimedOut`, `WriteZero`, `Interrupted`, `UnexpectedEof`, `OutOfMemory`,
`Unsupported`, `Other` — and `Other` carries NO payload, so
`Other(message)` does not typecheck. Put `Other` last in a match for
readability, not because it binds anything.
