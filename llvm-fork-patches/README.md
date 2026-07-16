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
| 0002-aarch64-tti-detached-instr | libLLVMAArch64CodeGen.a | shouldConsiderAddressTypePromotion took the LLVMContext through `I.getParent()->getParent()` — null-deref when CodeGenPrepare::optimizeExt consults TTI for a transiently detached instruction. The value type's own context is identical and always valid (CODEGENPREP-TTI-DETACHED-1, task #21 layer 3). |
| 0003-aarch64-disable-globalisel-default | libLLVMAArch64CodeGen.a | `EnableGlobalISelAtO` cl::init(0)→(-1): at OptLevel::None upstream enables GlobalISel, whose RegBankSelect null-derefs on valid Verum IR. SelectionDAG is the mature backend Verum targets; None then uses SelDAG + RegAllocFast. Combined with the emitter being pinned to None (native_codegen), this sidesteps BOTH the GISel bug and the RegAllocGreedy/SplitEditor::finish crash at O1+ (task #21). |
