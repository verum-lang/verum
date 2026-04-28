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

`core.io` exports synchronous I/O. For async (running on the runtime,
yielding to the executor at I/O boundaries), use
`core.async.fs::File`. The async file API mirrors the sync one with
`.await` at every read/write call.

## Errors

`IoError` is the common error type — it's a `StreamError` aliased
under that name to match the codebase's convention. The variants
include `NotFound`, `PermissionDenied`, `WouldBlock`, `BrokenPipe`,
plus `Other(message)` for anything the kernel didn't categorise.
Match on the variant for recovery; pattern-match `Other(_)` last.
