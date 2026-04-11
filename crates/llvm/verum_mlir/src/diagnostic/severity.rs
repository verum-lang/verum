use crate::Error;
use verum_mlir_sys::{
    MlirDiagnosticSeverity_MlirDiagnosticError, MlirDiagnosticSeverity_MlirDiagnosticNote,
    MlirDiagnosticSeverity_MlirDiagnosticRemark, MlirDiagnosticSeverity_MlirDiagnosticWarning,
};

/// Diagnostic severity.
#[derive(Clone, Copy, Debug)]
pub enum DiagnosticSeverity {
    Error,
    Note,
    Remark,
    Warning,
}

// NOTE: `mlirDiagnosticGetSeverity` in the regenerated `verum_mlir_sys`
// bindings (LLVM 21) returns `u32` rather than the historical `i32`, and
// the `MlirDiagnosticSeverity_*` constants are also `u32` in the new
// bindings. The TryFrom impl below therefore targets `u32` — otherwise
// `verum_mlir/src/diagnostic.rs:30` (which calls `try_from` on a `u32`
// value returned by the FFI) fails to compile with E0277.
impl TryFrom<u32> for DiagnosticSeverity {
    type Error = Error;

    fn try_from(severity: u32) -> Result<Self, Error> {
        #[allow(non_upper_case_globals)]
        Ok(match severity {
            MlirDiagnosticSeverity_MlirDiagnosticError => Self::Error,
            MlirDiagnosticSeverity_MlirDiagnosticNote => Self::Note,
            MlirDiagnosticSeverity_MlirDiagnosticRemark => Self::Remark,
            MlirDiagnosticSeverity_MlirDiagnosticWarning => Self::Warning,
            _ => return Err(Error::UnknownDiagnosticSeverity(severity)),
        })
    }
}
