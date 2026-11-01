//! Source proof selection and report binding shared by frontends.
use crate::{SourceReport, SourceRequest, elaborator_identity, sha256};
use serde::{Deserialize, Serialize};
use verum_proof::admission::{self, ReplayOutcome};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceProfile {
    #[default]
    #[serde(rename = "verum-source-pi-id/1")]
    Logical,
    #[serde(rename = "verum-source-finite/1")]
    Finite,
}

impl SourceProfile {
    pub const ALL: [Self; 2] = [Self::Logical, Self::Finite];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Logical => crate::PROFILE,
            Self::Finite => crate::finite::PROFILE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceOutcome {
    Verified,
    Rejected,
    Unsupported,
    Inconclusive,
    Unavailable,
}

impl SourceReport {
    /// Validate an isolated, configured worker's response for this request.
    /// This is protocol validation, not a second proof checker or a signature:
    /// the configured executable remains trusted to perform native admission.
    pub fn validate_for(&self, request: &SourceRequest) -> Result<SourceOutcome, &'static str> {
        if !SourceProfile::ALL
            .iter()
            .any(|p| p.as_str() == request.profile)
            || self.profile != request.profile
            || self.source.declaration != request.declaration
            || self.source.sha256 != sha256(request.source.as_bytes())
            || self.elaborator != elaborator_identity()
        {
            return Err(
                "Proof report does not match the requested profile, declaration, source or elaborator",
            );
        }
        let outcome = match self.status.as_str() {
            "verified" => SourceOutcome::Verified,
            "rejected" => SourceOutcome::Rejected,
            "unsupported" => SourceOutcome::Unsupported,
            "inconclusive" => SourceOutcome::Inconclusive,
            "unavailable" => SourceOutcome::Unavailable,
            _ => return Err("Unknown source proof outcome"),
        };
        if let Some(native) = &self.native {
            if native.profile != admission::PROFILE
                || native.checker != admission::checker_identity()
                || native.limits != Default::default()
                || !native.is_consistent()
            {
                return Err(
                    "Proof worker returned an inconsistent or different native checker report",
                );
            }
        }
        if outcome == SourceOutcome::Verified {
            let judgment = self
                .judgment
                .as_ref()
                .ok_or("Verified report has no judgment")?;
            if !self.native.as_ref().is_some_and(|native| native.accepted)
                || self.diagnostic.is_some()
                || judgment.profile != admission::PROFILE
                || judgment.statement != judgment.certificate.claimed_type
                || self.implementation.is_some() != (self.profile == crate::finite::PROFILE)
            {
                return Err("Verified source report lacks consistent native evidence");
            }
        } else {
            if self.judgment.is_some() || self.implementation.is_some() || self.diagnostic.is_none()
            {
                return Err(
                    "Refused source report contains success evidence or lacks a diagnostic",
                );
            }
            if let Some(native) = &self.native {
                let expected = match native.outcome {
                    ReplayOutcome::Rejected => SourceOutcome::Rejected,
                    ReplayOutcome::Unsupported => SourceOutcome::Unsupported,
                    ReplayOutcome::ResourceExhausted | ReplayOutcome::Inconclusive => {
                        SourceOutcome::Inconclusive
                    }
                    ReplayOutcome::Accepted => {
                        return Err("Refused report contains native acceptance");
                    }
                };
                if expected != outcome {
                    return Err("Source outcome changes the native refusal category");
                }
            }
        }
        Ok(outcome)
    }
}
