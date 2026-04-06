//! Async passes.

verum_mlir_macro::passes!(
    "Async",
    [
        mlirCreateAsyncAsyncFuncToAsyncRuntimePass,
        mlirCreateAsyncAsyncParallelForPass,
        mlirCreateAsyncAsyncRuntimePolicyBasedRefCountingPass,
        mlirCreateAsyncAsyncRuntimeRefCountingPass,
        mlirCreateAsyncAsyncRuntimeRefCountingOptPass,
        mlirCreateAsyncAsyncToAsyncRuntimePass,
    ]
);
