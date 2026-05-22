use crate::Error;
use verum_mlir_sys::{
    MlirDiagnosticSeverity, MlirDiagnosticSeverity_MlirDiagnosticError,
    MlirDiagnosticSeverity_MlirDiagnosticNote, MlirDiagnosticSeverity_MlirDiagnosticRemark,
    MlirDiagnosticSeverity_MlirDiagnosticWarning,
};

/// Diagnostic severity.
#[derive(Clone, Copy, Debug)]
pub enum DiagnosticSeverity {
    Error,
    Note,
    Remark,
    Warning,
}

// NOTE: bindgen types the `MlirDiagnosticSeverity` enum — and its
// `MlirDiagnosticSeverity_*` constants — according to the target ABI:
// `u32` on Linux/macOS, but `i32` on Windows/MSVC (plain C enums are
// signed there). Targeting the bindgen type alias instead of a hard-coded
// integer keeps this `TryFrom` impl correct on every platform —
// `mlirDiagnosticGetSeverity` returns that very alias, so
// `verum_mlir/src/diagnostic.rs` calls `try_from` without any cast.
impl TryFrom<MlirDiagnosticSeverity> for DiagnosticSeverity {
    type Error = Error;

    fn try_from(severity: MlirDiagnosticSeverity) -> Result<Self, Error> {
        #[allow(non_upper_case_globals)]
        Ok(match severity {
            MlirDiagnosticSeverity_MlirDiagnosticError => Self::Error,
            MlirDiagnosticSeverity_MlirDiagnosticNote => Self::Note,
            MlirDiagnosticSeverity_MlirDiagnosticRemark => Self::Remark,
            MlirDiagnosticSeverity_MlirDiagnosticWarning => Self::Warning,
            _ => return Err(Error::UnknownDiagnosticSeverity(severity as u32)),
        })
    }
}
