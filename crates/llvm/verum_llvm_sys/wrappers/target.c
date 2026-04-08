/* llvm-c/Target.h helper functions wrappers.
 *
 * The LLVMInitializeAll* functions and friends are defined `static inline`, so
 * we can't bind directly to them (the function body is generated via macro),
 * so here are some wrappers.
 *
 * These use verum_llvm_* naming to match our Rust FFI declarations.
 */
#include <llvm-c/Target.h>

void verum_llvm_initialize_all_targets(void) {
    LLVMInitializeAllTargets();
}

void verum_llvm_initialize_all_target_infos(void) {
    LLVMInitializeAllTargetInfos();
}

void verum_llvm_initialize_all_target_mcs(void) {
    LLVMInitializeAllTargetMCs();
}

void verum_llvm_initialize_all_asm_printers(void) {
    LLVMInitializeAllAsmPrinters();
}

void verum_llvm_initialize_all_asm_parsers(void) {
    LLVMInitializeAllAsmParsers();
}

void verum_llvm_initialize_native_target(void) {
    LLVMInitializeNativeTarget();
}

void verum_llvm_initialize_native_asm_printer(void) {
    LLVMInitializeNativeAsmPrinter();
}

void verum_llvm_initialize_native_asm_parser(void) {
    LLVMInitializeNativeAsmParser();
}

/* Legacy names for compatibility with llvm-sys.rs */
void LLVM_InitializeAllTargetInfos(void) {
    LLVMInitializeAllTargetInfos();
}

void LLVM_InitializeAllTargets(void) {
    LLVMInitializeAllTargets();
}

void LLVM_InitializeAllTargetMCs(void) {
    LLVMInitializeAllTargetMCs();
}

void LLVM_InitializeAllAsmPrinters(void) {
    LLVMInitializeAllAsmPrinters();
}

void LLVM_InitializeAllAsmParsers(void) {
    LLVMInitializeAllAsmParsers();
}

void LLVM_InitializeAllDisassemblers(void) {
    LLVMInitializeAllDisassemblers();
}

/* These functions return true on failure. */
LLVMBool LLVM_InitializeNativeTarget(void) {
    return LLVMInitializeNativeTarget();
}

LLVMBool LLVM_InitializeNativeAsmParser(void) {
    return LLVMInitializeNativeAsmParser();
}

LLVMBool LLVM_InitializeNativeAsmPrinter(void) {
    return LLVMInitializeNativeAsmPrinter();
}

LLVMBool LLVM_InitializeNativeDisassembler(void) {
    return LLVMInitializeNativeDisassembler();
}

/* ========================================================================
 * Shims for LLVM-C constant expression APIs removed in LLVM 17+.
 *
 * LLVMConstMul, LLVMConstNSWMul, LLVMConstNUWMul were removed because
 * LLVM no longer supports arbitrary constant expressions. These shims
 * compute the result at compile time using the LLVM-C integer API.
 * ======================================================================== */
#include <llvm-c/Core.h>

LLVMValueRef LLVMConstMul(LLVMValueRef LHS, LLVMValueRef RHS) {
    /* Both operands must be constant integers of the same type */
    unsigned long long lhs_val = LLVMConstIntGetZExtValue(LHS);
    unsigned long long rhs_val = LLVMConstIntGetZExtValue(RHS);
    LLVMTypeRef ty = LLVMTypeOf(LHS);
    return LLVMConstInt(ty, lhs_val * rhs_val, 0);
}

LLVMValueRef LLVMConstNSWMul(LLVMValueRef LHS, LLVMValueRef RHS) {
    return LLVMConstMul(LHS, RHS);
}

LLVMValueRef LLVMConstNUWMul(LLVMValueRef LHS, LLVMValueRef RHS) {
    return LLVMConstMul(LHS, RHS);
}
