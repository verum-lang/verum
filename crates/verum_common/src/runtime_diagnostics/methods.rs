//! Inclusive wall-clock observations at decoded VBC method calls.
use crate::{List, Text};
use serde::{Deserialize, Serialize};

pub const MAX_METHOD_DISPATCH_SITES: usize = 64;
pub const MAX_DISPATCH_NAME_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MethodDispatchSchema {
    #[serde(rename = "verum.method-dispatch/1")]
    V1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MethodDispatchOutcome {
    /// Control entered this bytecode frame. Its subsequent execution is excluded.
    EnteredFrame {
        function_id: u32,
    },
    /// The handler completed without leaving a new frame on the stack.
    CompletedInline,
    /// The handler staged a dereferenced receiver and rewound to this CallM.
    Retried,
    Returned,
    Yielded,
    /// Includes errors from nested synchronous work performed by the handler.
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MethodDispatchRow {
    /// Local to this capture; two loaded modules can share a display name.
    pub module_instance: usize,
    pub module: Option<Text>,
    pub function_id: u32,
    pub function: Option<Text>,
    /// Byte offset of the actual CallM opcode within the caller.
    pub pc: u32,
    pub selector_id: u32,
    pub selector: Option<Text>,
    pub outcome: MethodDispatchOutcome,
    #[serde(with = "decimal")]
    pub count: u64,
    /// Includes synchronous nested work. Overlapping rows cannot be added to get CPU time.
    #[serde(with = "decimal")]
    pub inclusive_ns: u64,
    #[serde(with = "decimal")]
    pub max_ns: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// The first distinct site/outcome rows are retained. Monotonic counts and
/// nanosecond totals saturate at u64::MAX, and serialize as decimal strings.
pub struct MethodDispatchReport {
    pub schema: MethodDispatchSchema,
    pub capacity: usize,
    /// Calls whose site/outcome could not be stored. Retained rows continue counting.
    #[serde(with = "decimal")]
    pub dropped_calls: u64,
    pub rows: List<MethodDispatchRow>,
}

impl MethodDispatchReport {
    pub fn validate(&self) -> bool {
        self.capacity <= MAX_METHOD_DISPATCH_SITES
            && self.rows.len() <= self.capacity
            && (self.dropped_calls == 0 || self.rows.len() == self.capacity)
            && self.rows.iter().all(|r| {
                r.count != 0
                    && r.max_ns <= r.inclusive_ns
                    && r.inclusive_ns <= r.max_ns.saturating_mul(r.count)
                    && r.module_instance < self.rows.len()
                    && [&r.module, &r.function, &r.selector]
                        .into_iter()
                        .flatten()
                        .all(|s| s.len() <= MAX_DISPATCH_NAME_BYTES)
            })
    }
}

// Decimal strings retain full precision across JSON/JavaScript consumers.
mod decimal {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};
    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(v)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        let value = String::deserialize(d)?;
        if value.is_empty() || value.len() > 20 || !value.bytes().all(|c| c.is_ascii_digit()) {
            return Err(D::Error::custom("invalid unsigned decimal counter"));
        }
        value.parse().map_err(D::Error::custom)
    }
}
