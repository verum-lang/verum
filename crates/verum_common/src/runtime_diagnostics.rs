//! Versioned runtime observations shared by the interpreter, CLI and tools.
//! These records describe observed execution, not a memory-safety proof.
use crate::{CbgrViolationKind, List, Text};
use serde::{Deserialize, Serialize};

mod methods;
pub use methods::*;

pub const MAX_CBGR_EVENTS: usize = 256;
pub const MAX_RUNTIME_REPORT_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CbgrSchema {
    #[serde(rename = "verum.cbgr.dispatch/1")]
    DispatchV1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeReportSchema {
    #[serde(rename = "verum.runtime-report/1")]
    V1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CbgrTargetKind {
    Register,
    Address,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CbgrSourceLocation {
    pub path: Text,
    /// One-based coordinates from an exact bytecode/source-map entry.
    pub line: u32,
    pub column: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CbgrEvent {
    pub kind: CbgrViolationKind,
    pub target_kind: CbgrTargetKind,
    /// Decimal register or opaque hex address; never dereference this value.
    pub target: Text,
    /// Decimal strings preserve full-width counters across JSON consumers.
    pub expected: Option<Text>,
    pub actual: Option<Text>,
    pub function_id: Option<u32>,
    pub function: Option<Text>,
    pub pc: u32,
    pub opcode: u8,
    pub source: Option<CbgrSourceLocation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CbgrReport {
    pub schema: CbgrSchema,
    pub capacity: usize,
    pub checks_enabled_at_start: bool,
    pub truncated: bool,
    pub events: List<CbgrEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeObservation {
    Captured,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeReport {
    pub schema: RuntimeReportSchema,
    pub run_id: Text,
    /// Bytes submitted by the caller, before an optional script shebang.
    pub source_sha256: Text,
    /// Bytes prepared by the CLI. This does not identify dependency sources.
    pub prepared_source_sha256: Text,
    pub wrapper_lines: u32,
    /// Present only if the compiler held the exact prepared entry bytes at
    /// this path. Cached bytecode without that source identity has no mapping.
    pub entry_source_path: Option<Text>,
    pub compiler_version: Text,
    pub observation: RuntimeObservation,
    pub cbgr: Option<CbgrReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method_dispatch: Option<MethodDispatchReport>,
}

impl RuntimeReport {
    /// Validate untrusted transport data before displaying it as this run's
    /// observations. This establishes identity and bounds, not producer trust.
    pub fn validate_binding(&self, run_id: &str, source_hash: &str) -> Result<(), &'static str> {
        if self.run_id.as_str() != run_id || self.source_sha256.as_str() != source_hash {
            return Err("Runtime report belongs to another run or source");
        }
        let hash = |value: &str| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        };
        if !hash(&self.source_sha256)
            || !hash(&self.prepared_source_sha256)
            || self.run_id.is_empty()
            || self.run_id.len() > 128
            || self.wrapper_lines > 1
            || self.compiler_version.len() > 128
            || self
                .entry_source_path
                .as_ref()
                .is_some_and(|path| path.is_empty() || path.len() > 2048)
        {
            return Err("Invalid runtime report identity");
        }
        if (self.observation == RuntimeObservation::Captured) != self.cbgr.is_some() {
            return Err("Runtime report observation and payload disagree");
        }
        if self.method_dispatch.as_ref().is_some_and(|report|
            self.observation != RuntimeObservation::Captured || !report.validate()) {
            return Err("Invalid runtime method dispatch profile");
        }
        if let Some(report) = &self.cbgr {
            if report.capacity > MAX_CBGR_EVENTS || report.events.len() > report.capacity {
                return Err("Runtime report exceeds capture bounds");
            }
            for event in &report.events {
                let valid_target = match event.target_kind {
                    CbgrTargetKind::Register => event.target.parse::<u32>().is_ok(),
                    CbgrTargetKind::Address => {
                        event.target.as_str().strip_prefix("0x").is_some_and(|hex| {
                            !hex.is_empty()
                                && hex.len() <= 16
                                && u64::from_str_radix(hex, 16).is_ok()
                        })
                    }
                };
                if !valid_target
                    || event.target.len() > 20
                    || [&event.expected, &event.actual]
                        .into_iter()
                        .flatten()
                        .any(|value| value.len() > 20 || value.parse::<u64>().is_err())
                    || event
                        .function
                        .as_ref()
                        .is_some_and(|name| name.chars().count() > 256)
                    || event.source.as_ref().is_some_and(|source| {
                        source.path.is_empty()
                            || source.path.len() > 2048
                            || source.line == 0
                            || source.column == 0
                    })
                {
                    return Err("Invalid CBGR observation");
                }
            }
        }
        Ok(())
    }
}
