//! Linalg passes.

verum_mlir_macro::passes!(
    "Linalg",
    [
        // spell-checker: disable-next-line
        mlirCreateLinalgConvertElementwiseToLinalgPass,
        mlirCreateLinalgConvertLinalgToAffineLoopsPass,
        mlirCreateLinalgConvertLinalgToLoopsPass,
        mlirCreateLinalgConvertLinalgToParallelLoopsPass,
        mlirCreateLinalgLinalgBlockPackMatmul,
        mlirCreateLinalgLinalgDetensorizePass,
        mlirCreateLinalgLinalgElementwiseOpFusionPass,
        mlirCreateLinalgLinalgFoldUnitExtentDimsPass,
        mlirCreateLinalgLinalgGeneralizeNamedOpsPass,
        mlirCreateLinalgLinalgInlineScalarOperandsPass,
        mlirCreateLinalgLinalgNamedOpConversionPass,
        mlirCreateLinalgLinalgSpecializeGenericOpsPass,
    ]
);
