//! GPU passes.

verum_mlir_macro::passes!(
    "GPU",
    [
        // spell-checker: disable-next-line
        mlirCreateGPUGpuAsyncRegionPass,
        mlirCreateGPUGpuDecomposeMemrefsPass,
        mlirCreateGPUGpuEliminateBarriers,
        mlirCreateGPUGpuKernelOutliningPass,
        mlirCreateGPUGpuLaunchSinkIndexComputationsPass,
        mlirCreateGPUGpuMapParallelLoopsPass,
        mlirCreateGPUGpuModuleToBinaryPass,
        mlirCreateGPUGpuNVVMAttachTarget,
        mlirCreateGPUGpuROCDLAttachTarget,
        mlirCreateGPUGpuSPIRVAttachTarget,
    ]
);
