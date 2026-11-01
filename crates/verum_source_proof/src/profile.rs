//! Source proof selection and report binding shared by frontends.
use crate::{SourceBinding, SourceReport, SourceRequest, SourceSpan, elaborator_identity, sha256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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

impl SourceSpan {
    /// Validate a half-open UTF-8 range, including diagnostic insertion points.
    pub fn matches_source(&self, source: &str) -> bool {
        source
            .get(self.start as usize..self.end as usize)
            .is_some_and(|text| sha256(text.as_bytes()) == self.sha256)
    }
}

fn bound_to_source(binding: &SourceBinding, source: &str, hash: &str, required: bool) -> bool {
    binding.sha256 == hash
        && match &binding.span {
            Some(span) => span.start < span.end && span.matches_source(source),
            None => !required,
        }
}

impl SourceReport {
    /// Validate an isolated, configured worker's response for this request.
    /// This is protocol validation, not a second proof checker or a signature:
    /// the configured executable remains trusted to perform native admission.
    pub fn validate_for(&self, request: &SourceRequest) -> Result<SourceOutcome, &'static str> {
        let source_hash = sha256(request.source.as_bytes());
        if !SourceProfile::ALL
            .iter()
            .any(|p| p.as_str() == request.profile)
            || self.profile != request.profile
            || self.source.declaration != request.declaration
            || !bound_to_source(&self.source, &request.source, &source_hash, false)
            || self.elaborator != elaborator_identity()
            || self
                .diagnostic_span
                .as_ref()
                .is_some_and(|span| !span.matches_source(&request.source))
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
                || self.source.span.is_none()
            {
                return Err("Verified source report lacks consistent native evidence");
            }
            if let Some(implementation) = &self.implementation {
                let dependencies: BTreeMap<_, _> = implementation
                    .dependencies
                    .iter()
                    .map(|binding| (binding.declaration.as_str(), binding))
                    .collect();
                let captured = |binding: &SourceBinding| {
                    dependencies
                        .get(binding.declaration.as_str())
                        .is_some_and(|candidate| *candidate == binding)
                };
                if implementation.semantics != self.profile
                    || dependencies.len() != implementation.dependencies.len()
                    || !captured(&self.source)
                    || dependencies.values().any(|binding| {
                        !bound_to_source(binding, &request.source, &source_hash, true)
                    })
                    || implementation.obligations.is_empty()
                {
                    return Err(
                        "Function proof dependencies do not bind exact source declarations",
                    );
                }
                for obligation in &implementation.obligations {
                    if !captured(&obligation.owner) {
                        return Err("Proof obligation owner is not a captured dependency");
                    }
                    use crate::finite::ObligationKind;
                    match (&obligation.kind, &obligation.call, &obligation.callee) {
                        (ObligationKind::FunctionContract, None, None) => {}
                        (
                            ObligationKind::BodyCall
                            | ObligationKind::PreconditionCall
                            | ObligationKind::PostconditionCall,
                            Some(call),
                            Some(callee),
                        ) if captured(callee)
                            && call.start < call.end
                            && call.matches_source(&request.source)
                            && obligation.owner.span.as_ref().is_some_and(|owner| {
                                owner.start <= call.start && call.end <= owner.end
                            }) => {}
                        _ => {
                            return Err(
                                "Proof obligation has an invalid call range or callee binding",
                            );
                        }
                    }
                }
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
