# Endian carrier failure and component evidence

The ordinary SHA-256 failure occurs while encoding the bit length in
`Sha256.finalize`, after state access, compression and padding. The native
producer repair and the compiler changes in this branch remain unaccepted by
an ordinary rebuilt CLI, a whole service or AOT.

The source and wire consuming-receiver controls all pass. The early hypothesis
that consuming a mutated four-field receiver caused the bounds failure was
refuted. The pinned ordinary trace instead shows `UInt64.to_be_bytes` returning
a List object, followed by `TypedArrayLoad` with stride8 at index3. The archived
return descriptor contains PTR (TypeId14), not Byte (TypeId6). An explicit
`[Byte;8]` binding emits GetE against that same List pointer and passes; it does
not allocate or normalize a packed buffer. A typed byte-array literal uses
NewByteArray and passes. Narrow annotated UInt16/UInt32 controls also pass.

The ordinary inferred AOT control fails its assertion (exit255). Its exact
88,896-byte executable is retained compressed. The older original-summary file
predates that result and still says AOT pending; the AOT result receipt is the
later authority. These are historical failures, not current-release promises.

Component checkpoints separate the relevant causes:

- The first four fixed-carrier controls fail before the producer repair.
- Aliasing another source's `Word` as `Byte` is fixed by T1694. An unrelated
  declaration literally named `Byte` still corrupts the primitive signature;
  T1703 retains that separate failing source in `fixtures/source-owner-controls.rs.txt`.
  The first same-name failure stops that test before its second load order.
- The linked wrapper and register140/register270 controls fail before shared
  packed expansion and canonical register encoding, then pass in source and
  reloaded wire execution. Wire modules rebuild their band relocation state.
- Exact source UInt16/UInt32/negative Int16 methods pass. Float32 still selects
  an eight-byte producer and loses negative-zero roundtrip bits; T1701 owns the
  declared-method identity fix. The split gate is24 pass/2 fail.
- Native JIT first observes List-header bytes and incorrectly decodes packed
  input as zero. After paired producer/consumer repair, four source and decoded
  wire controls pass. Inferred indexing also executes correctly. The explicit
  annotation indexing case is refused before JIT because it still selects GetE;
  T1704 owns actual result-storage propagation with T1700's shared authority.

Unsuccessful harness attempts remain distinct: a private Rust import, a wrong
archive member name, a wrong AST accessor, missing wrapper encoder call sites,
LLVM preamble ordering, and omitted decoded bodies in the initial native wire
leg did not establish new language runtime failures. Native wire lowering
requires decoded instructions and translated jump offsets, as the real archive
loader supplies. The initially untracked native inline pointer was that skipped
wire body returning zero; the source leg passed. JIT uses a bounded host
allocation substrate and makes no allocator lifecycle or no-libc claim.

`manifest.json` records every retained file's original and saved hash. Raw logs
and emitted IR are losslessly gzip-compressed. JSON receipts and executed source
fixtures are unchanged, including their historical machine paths. Rust test
source identities are the recorded commits; the active fixtures live under the
VBC and codegen crate test directories. Scratch runner copies document the exact
bounded Cargo invocations and are provenance, not portable project tooling.
