# core/io Test Suite

Test coverage for Verum's I/O module.

## Test Organization

| File | Module | Coverage | Tests | Status |
|------|--------|----------|-------|--------|
| `protocols_test.vr` | `io/protocols` | StreamError, IoErrorKind, SeekFrom, IoResult | 31 | Passing |
| `buffer_test.vr` | `io/buffer` | BufReader, BufWriter, LineWriter, Cursor, IntoInnerError, copy, read_all | 62 | Passing |
| `path_test.vr` | `io/path` | Path, PathBuf, Components, Ancestors, PathIter | 100 | Passing |
| `fs_test.vr` | `io/fs` | File system operations, metadata, WalkDir, ReadDir, DirEntry, FileType, Permissions | 88 | Passing |
| `file_test.vr` | `io/file` | File read/write, seek, metadata, permissions | 89 | Passing |
| `stdio_test.vr` | `io/stdio` | stdin, stdout, stderr accessors | 21 | Passing |
| `io_protocols_extended_test.vr` | `io/protocols` | IoErrorKind Eq, SeekFrom Eq/Clone/Debug, StreamError Display/Eq, Chain/Take adapters, Empty/Sink/Cursor | 37 | Passing |
| `buffer_lines_test.vr` | `io/buffer`, `io/protocols` | BufRead read_line, lines, read_until, split; LinesIter, SplitIter, BytesIter; BufReader BufRead methods | 42 | Passing |
| `buffer_protocols_test.vr` | `io/buffer` | IntoInnerError Debug/Display, error(), into_inner() | 7 | Passing |
| `io_protocols_test.vr` | `io/protocols`, `io/path` | IoErrorKind Clone, Component Display | 17 | Passing |
| `io_seek_permissions_test.vr` | `io/protocols`, `io/path`, `io/file` | SeekFrom variants, Cursor Seek operations, Permissions from_mode/mode/readonly/set_readonly, PathBuf push/pop/extension, Path methods, OpenOptions, StreamError | 62 | Passing |

## Key Types Tested

### Path & PathBuf (100 tests)
- Construction: `Path.new()`, `PathBuf.new()`, `PathBuf.from_str()`, `Path.from_str()`
- Component access: `parent()`, `file_name()`, `file_stem()`, `extension()`
- Manipulation: `join()`, `join_str()`, `with_file_name()`, `with_extension()`
- Queries: `is_absolute()`, `is_relative()`, `has_root()`, `is_empty()`
- Conversion: `to_text()`, `to_path_buf()`, `as_str()`, `to_str()`, `as_path()`
- PathBuf mutation: `push()`, `pop()`, `set_file_name()`, `set_extension()`
- Iterators: `ancestors()`, `components()`, `iter()`
- Comparison: `starts_with()`, `ends_with()`, `strip_prefix()`
- Normalization: `normalize()`

### File (89 tests)
- File open/create/read/write operations
- Seek, rewind, stream_position
- Metadata access and sync operations

### File System (88 tests)
- Directory operations: create_dir, create_dir_all, remove_dir, remove_dir_all, read_dir, walk_dir
- File operations: read, write, remove_file, rename, copy, hard_link, read_link, canonicalize
- Metadata: exists, is_file, is_dir, is_symlink, len, timestamps, readonly, file_type
- Types: WalkDir, ReadDir, DirEntry, FileType, Permissions, FsMetadataRaw
- Convenience: fs_read, fs_read_to_string, fs_write, fs_write_str, set_current_dir

### BufReader/BufWriter/LineWriter/Cursor (62 tests)
- Buffered reading: read_line, read_until, buffer access, get_ref, get_mut, capacity
- Buffered writing: write_all, flush, buffer_len, into_inner, capacity
- LineWriter: get_ref, get_mut, write with/without newline, flush, with_capacity
- Cursor: get_ref, get_mut, position, seek, write-then-read, consume
- IntoInnerError type existence
- Utility functions: copy, read_all

### StreamError & IoErrorKind (31 tests)
- All 20 IoErrorKind variants tested
- StreamError construction: new(), with_message(), from_raw_os_error(), clone()
- SeekFrom variants: Start, End, Current
- IoResult type alias tests

### Stdio (21 tests)
- stdin, stdout, stderr type existence and accessors

## Test Count: 522 tests total (11 test files)

## Known Limitations

- Actual file I/O requires runtime (tests are typecheck-pass)
- Some protocol method calls (Read.read, Write.write) need runtime
- stderr capture and eprint/eprintln not tested
