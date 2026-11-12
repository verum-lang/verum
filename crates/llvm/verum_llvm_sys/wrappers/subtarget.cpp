// Compiler-host binding to LLVM metadata, never linked into a Verum program.
// No target tables or runtime operations live here: LLVM owns these facts.
#include "llvm-c/TargetMachine.h"
#include "llvm/CodeGen/TargetLowering.h"
#include "llvm/CodeGen/TargetSubtargetInfo.h"
#include "llvm/IR/Function.h"
#include "llvm/MC/TargetRegistry.h"
#include "llvm/Target/TargetMachine.h"

// Validate against LLVM's own catalog before constructing a machine: some
// backends abort for an unknown CPU, others merely warn and ignore a feature.
extern "C" bool verum_llvm_validate_subtarget(
    LLVMTargetRef Target, const char *Triple, const char *CPU, const char *Features) {
  if (!Target || !Triple || !CPU || !Features)
    return false;
  const auto *T = reinterpret_cast<const llvm::Target *>(Target);
  std::unique_ptr<llvm::MCSubtargetInfo> Catalog(T->createMCSubtargetInfo(Triple, "", ""));
  if (!Catalog || (*CPU && !Catalog->isCPUStringValid(CPU)))
    return false;
  llvm::StringRef Rest(Features);
  while (!Rest.empty()) {
    auto [Feature, Tail] = Rest.split(',');
    Rest = Tail;
    if (Feature.size() < 2 || (Feature.front() != '+' && Feature.front() != '-'))
      return false;
    auto Name = Feature.drop_front();
    if (!llvm::any_of(Catalog->getAllProcessorFeatures(),
                     [&](const auto &KV) { return Name == KV.Key; }))
      return false;
  }
  return true;
}

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
