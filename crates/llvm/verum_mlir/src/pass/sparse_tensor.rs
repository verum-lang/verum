//! Sparse tensor passes.

verum_mlir_macro::passes!(
    "SparseTensor",
    [
        mlirCreateSparseTensorLowerForeachToSCF,
        mlirCreateSparseTensorLowerSparseOpsToForeach,
        mlirCreateSparseTensorPreSparsificationRewrite,
        mlirCreateSparseTensorSparseBufferRewrite,
        mlirCreateSparseTensorSparseGPUCodegen,
        mlirCreateSparseTensorSparseReinterpretMap,
        mlirCreateSparseTensorSparseTensorCodegen,
        mlirCreateSparseTensorSparseTensorConversionPass,
        mlirCreateSparseTensorSparseVectorization,
        mlirCreateSparseTensorSparsificationAndBufferization,
        mlirCreateSparseTensorSparsificationPass,
        mlirCreateSparseTensorStageSparseOperations,
        mlirCreateSparseTensorStorageSpecifierToLLVM,
    ]
);
