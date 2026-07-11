# LLVM fork patches

Verum-local patches to the in-tree LLVM checkout (`llvm/llvm-project`,
NOT tracked by this repo — see `.gitignore`). The checkout carries these
commits on top of upstream; this directory is the DURABLE record so a
fresh checkout can be reconstructed:

```
cd llvm/llvm-project
git am ../../llvm-fork-patches/*.patch
cd ../build && ninja <affected libs>   # e.g. libLLVMTransformUtils.a
cp lib/<affected>.a ../install/lib/
```

| Patch | Affected lib | Why |
|---|---|---|
| 0001-simplifycfg-lockstep-guard | libLLVMTransformUtils.a | sinkCommonCodeFromPredecessors null-deref: profitability scans stepped LockstepReverseIterator past exhaustion (debug-instr skip asymmetry) and dereferenced `(*LRI)[0]` blind — crashed every affected `write_to_file` (SIMPLIFYCFG-SINK-NULL-1, task #21). Guards keep the optimization on; sinking fewer instructions is always sound. |
