//! Unified type error wrapper for Phase 2 error consolidation.

use crate::{TypeError, KindError, ProjectionError, CapabilityError};
use crate::specialization_selection::SpecializationError;

/// Unified wrapper for type system errors.
#[derive(Debug)]
pub enum UnifiedTypeError {
    Type(TypeError),
    Specialization(SpecializationError),
    Kind(KindError),
    Projection(ProjectionError),
    Capability(CapabilityError),
}

impl std::fmt::Display for UnifiedTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Type(e) => write!(f, "{}", e),
            Self::Specialization(e) => write!(f, "specialization: {:?}", e),
            Self::Kind(e) => write!(f, "kind: {:?}", e),
            Self::Projection(e) => write!(f, "projection: {:?}", e),
            Self::Capability(e) => write!(f, "capability: {:?}", e),
        }
    }
}

impl std::error::Error for UnifiedTypeError {}

impl From<TypeError> for UnifiedTypeError {
    fn from(e: TypeError) -> Self { Self::Type(e) }
}

impl From<SpecializationError> for UnifiedTypeError {
    fn from(e: SpecializationError) -> Self { Self::Specialization(e) }
}

impl From<KindError> for UnifiedTypeError {
    fn from(e: KindError) -> Self { Self::Kind(e) }
}

impl From<ProjectionError> for UnifiedTypeError {
    fn from(e: ProjectionError) -> Self { Self::Projection(e) }
}

impl From<CapabilityError> for UnifiedTypeError {
    fn from(e: CapabilityError) -> Self { Self::Capability(e) }
}
