# Held endian integration checkpoint

The combined source and serialized VBC gate passes all 27 controls at
`00146a43dff50f9cd3b5a0d114964f145dc3e5cc` (27.948 seconds). This includes
five receiver controls, 18 array and actual primitive-source controls, three
linked-wrapper/wide-register controls, and one foreign alias-mount control.
The actual Float32 byte and negative-zero roundtrips that failed in the earlier
split gate now pass with the exact declared numeric method-owner repair.

The native source and decoded-wire gate at
`a3b5cfa6a119d5680cdfb9b1d4d73485610124b3` finishes with five passing tests and
two failed test guards (39.703 seconds, exit 101). It is an incomplete gate.
Dynamic fixed-byte producers and consumers, inline widths, returned byte
payloads, and List input compatibility pass. The guards stop before executing
unsafe native code:

- An explicitly annotated `[Byte; 8]` binding of an endian call still emits
  `GetE` despite its packed producer. The inferred indexing leg executes in
  both source and decoded-wire form before the annotated leg is refused.
- A fixed-byte intrinsic argument with an actual `NewByteArray` producer still
  uses `GetE` inside the expanded `from_be_bytes_8` body. The separate List
  argument control passes, so globally replacing that load with a packed load
  would discard a valid input representation.

These are test-harness refusals, not production diagnostics or successful
runtime checks of the unsafe paths. The underlying call, explicit-binding,
argument, and Array-to-List contracts must agree before the producer repair can
land. A declared array type alone does not prove its physical storage.
T1700 owns callable List materialization, T1704 the binding/result boundary,
and T1703 retains the separate same-name foreign Byte declaration defect.
The two T1701 production commits are included here for combined testing;
their original source commits are `c38aa5ff8f1731b4e193509f929f150945f7c3c2`
and `06d0942cc69ec1c21881203e1d05323ff62cc8ee`. The later descriptor-name and
borrowed-carrier follow-up `e2f10e2caad99128979fdfc3a8cdb30b6aeedb9b` is
not included in these tested source identities.

All producer changes remain held. These gates do not establish a full VBC
library pass, repaired ordinary SHA-256/authentication, a fresh standard-library
bake, service publication, allocator behavior, or AOT acceptance. Native JIT
uses the bounded host allocation substrate documented in the test. The original
ordinary failure receipts and earlier harness failures remain unchanged.

The receipt directories `rust/declared-owner-integration` and
`rust/native-owner-integration` retain exact commands, source identities,
executable hashes, logs, and emitted IR. Their lossless copies are covered by
`manifest.json`; the earlier checkpoint identity is preserved separately.

The held chain starts from `529babe561b55f3491b2bac37c43f5b92572d779`:

```text
5692583b43a2a0682877d32c3fc6c071bcbb35b1 test(vbc): pin consuming receiver after mutable calls (T1698)
93908649827a57a7759c581db58db596fcb5bc4f test(vbc): expose fixed endian array carrier mismatch (T1698)
c0cd050885a98c84dbfe23a72f8dd80f63bde288 test(vbc): use public heap header export (T1698)
d814b20f96cb1400bf21b9cbb9900d02bbee3748 test(vbc): isolate source-owned byte return descriptors (T1698)
7ddbe971cce89c9813e57e107c41a15ba1b56177 fix(vbc): produce packed bytes from endian conversion (T1698)
f871e11bac89bfbd3291335c6af4db3894995843 test(vbc): cover endian array consumers and owner controls (T1698)
03fd4ad550e8b99f39ad04095ffedb74b37d2199 test(vbc): expose endian wrapper and wide-register carriers (T1698)
9c545554b20230545f5f475c5022259673fa04da fix(vbc): share packed endian expansion and register encoding (T1698)
8b8f2c6d11720984fe984f56c225007e7dbfc35a fix(vbc): finish shared wrapper register encoder migration (T1698)
59f0ed64f2665dd09d815780621de3331976f50d test(vbc): cover actual narrow endian bodies and wire relocation (T1698)
e82ea7f9040db92541a0a9a8f85fdb5078e968c5 test(vbc): select exact primitive implementation path (T1698)
37b7ba9cfea1dade0c27480a4a27253918fd0f27 test(codegen): pin native endian producer and packed input contracts (T1698)
d592c2801fbf1d455baa8c3c7c90ac57bf284153 test(codegen): keep target preamble before JIT declarations (T1698)
dd9bd40c7a3f5254fff172be6cb638ff73a52aa2 fix(codegen): align native endian producers and packed consumers (T1698)
949b9cc72d9608dfee5ccd59d7cb0170608e73cf test(codegen): decode loaded endian bodies and guard byte indexing (T1698)
2f20fa0e9164742bcabff298df63a99d41eed03e docs(vbc): retain causal endian carrier gates and separate owner defect (T1698, T1703)
c3767992730dec54e0c7b53b286176cb20613541 fix(vbc): retain declared numeric method owners (T1701)
00146a43dff50f9cd3b5a0d114964f145dc3e5cc fix(vbc): retain pointer-sized numeric declarations (T1701)
a3b5cfa6a119d5680cdfb9b1d4d73485610124b3 test(codegen): distinguish packed and list endian arguments (T1698, T1704)
```
