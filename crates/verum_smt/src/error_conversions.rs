//! Conversions from verum_smt::Error to verum_error::VerumError
//!
//! This module implements the `From` trait to convert SMT verification errors
//! to the unified VerumError type, enabling seamless error propagation across
//! crate boundaries.

use crate::{Error, VerificationError};
use verum_error::unified::VerumError;

impl From<Error> for VerumError {
    fn from(err: Error) -> Self {
        match err {
            Error::Timeout { timeout_ms } => VerumError::VerificationTimeout { timeout_ms },
            Error::Verification(v) => {
                // Extract detailed information from VerificationError
                match v {
                    VerificationError::CannotProve {
                        constraint,
                        counterexample,
                        ..
                    } => VerumError::VerificationFailed {
                        reason: format!("cannot prove: {}", constraint).into(),
                        counterexample: counterexample.map(|ce| format!("{:?}", ce).into()),
                    },
                    VerificationError::Timeout {
                        constraint,
                        timeout,
                        ..
                    } => VerumError::VerificationFailed {
                        reason: format!(
                            "verification timeout for: {} (after {:?})",
                            constraint, timeout
                        ).into(),
                        counterexample: None,
                    },
                    VerificationError::Translation(trans_err) => VerumError::Other {
                        message: format!("SMT translation error: {}", trans_err).into(),
                    },
                    VerificationError::SolverError(msg) => VerumError::Other {
                        message: format!("SMT solver error: {}", msg).into(),
                    },
                    VerificationError::Unknown(msg) => VerumError::VerificationFailed {
                        reason: format!("unknown verification result: {}", msg).into(),
                        counterexample: None,
                    },
                }
            }
            Error::Translation(trans_err) => VerumError::Other {
                message: format!("SMT translation error: {}", trans_err).into(),
            },
            Error::Unsupported(feature) => VerumError::UnsupportedSMT { feature: feature.into() },
            Error::ContextError(msg) => VerumError::Other {
                message: format!("Z3 context error: {}", msg).into(),
            },
            Error::Internal(msg) => VerumError::Other {
                message: format!("SMT internal error: {}", msg).into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::VerificationCost;
    use std::time::Duration;
    use verum_common::Text;

    #[test]
    fn test_timeout_conversion() {
        let smt_err = Error::Timeout { timeout_ms: 5000 };
        let verum_err: VerumError = smt_err.into();

        match verum_err {
            VerumError::VerificationTimeout { timeout_ms } => {
                assert_eq!(timeout_ms, 5000);
            }
            _ => panic!("Expected VerificationTimeout variant"),
        }
    }

    #[test]
    fn test_unsupported_conversion() {
        let smt_err = Error::Unsupported("nonlinear arithmetic".to_string());
        let verum_err: VerumError = smt_err.into();

        match verum_err {
            VerumError::UnsupportedSMT { feature } => {
                assert_eq!(feature, Text::from("nonlinear arithmetic"));
            }
            _ => panic!("Expected UnsupportedSMT variant"),
        }
    }

    #[test]
    fn test_verification_error_conversion() {
        let ver_err = VerificationError::CannotProve {
            constraint: "x > 0".into(),
            counterexample: None,
            cost: VerificationCost::new("test".into(), Duration::from_secs(1), false),
            suggestions: Vec::new().into(),
        };
        let smt_err = Error::Verification(ver_err);
        let verum_err: VerumError = smt_err.into();

        match verum_err {
            VerumError::VerificationFailed { reason, .. } => {
                assert!(reason.contains("cannot prove"));
                assert!(reason.contains("x > 0"));
            }
            _ => panic!("Expected VerificationFailed variant"),
        }
    }

    #[test]
    fn test_context_error_conversion() {
        let smt_err = Error::ContextError("failed to initialize Z3".to_string());
        let verum_err: VerumError = smt_err.into();

        match verum_err {
            VerumError::Other { message } => {
                assert!(message.contains("Z3 context error"));
            }
            _ => panic!("Expected Other variant"),
        }
    }
}
