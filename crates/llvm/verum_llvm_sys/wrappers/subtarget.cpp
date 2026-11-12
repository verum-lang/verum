// Compiler-host binding to LLVM metadata, never linked into a Verum program.
// No target tables or runtime operations live here: LLVM owns these facts.
#include "llvm-c/TargetMachine.h"
#include "llvm/CodeGen/TargetLowering.h"
#include "llvm/CodeGen/TargetSubtargetInfo.h"
#include "llvm/IR/Function.h"
#include "llvm/Target/TargetMachine.h"

extern "C" bool verum_llvm_subtarget_info(
    LLVMTargetMachineRef Machine, LLVMValueRef Function, unsigned *MaxAtomicBits,
    void (*Feature)(const char *, size_t, bool, void *), void *Context) {
  if (!Machine || !Function || !MaxAtomicBits || !Feature)
    return false;
  auto *TM = reinterpret_cast<llvm::TargetMachine *>(Machine);
  auto *F = llvm::dyn_cast<llvm::Function>(llvm::unwrap(Function));
  if (!F)
    return false;
  const auto *ST = TM->getSubtargetImpl(*F);
  if (!ST || !ST->getTargetLowering())
    return false;
  *MaxAtomicBits = ST->getTargetLowering()->getMaxAtomicSizeInBitsSupported();
  for (const auto &KV : ST->getAllProcessorFeatures()) {
    llvm::StringRef Name(KV.Key);
    Feature(Name.data(), Name.size(), ST->getFeatureBits()[KV.Value], Context);
  }
  return true;
}
