//! Exact identity for values embedded in types, separate from numeric equality.
//!
//! IEEE signed zeros and NaN payloads have observable representations. Ordered
//! collections retain their canonical key order. An explicit work list compares every admitted
//! node without recursion, rendering names or an undecidable-result fallback.
use verum_common::{ConstValue, List};

pub(crate) fn same_constant_identity(left: &ConstValue, right: &ConstValue) -> bool {
    let mut pending = List::from_single((left, right));
    while let Some((left, right)) = pending.pop() {
        if std::ptr::eq(left, right) {
            continue;
        }
        match (left, right) {
            (ConstValue::Unit, ConstValue::Unit) => {}
            (ConstValue::Bool(a), ConstValue::Bool(b)) if a == b => {}
            (ConstValue::Int(a), ConstValue::Int(b)) if a == b => {}
            (ConstValue::UInt(a), ConstValue::UInt(b)) if a == b => {}
            (ConstValue::Float(a), ConstValue::Float(b)) if a.to_bits() == b.to_bits() => {}
            (ConstValue::Char(a), ConstValue::Char(b)) if a == b => {}
            (ConstValue::Text(a), ConstValue::Text(b)) if a == b => {}
            (ConstValue::Bytes(a), ConstValue::Bytes(b)) if a == b => {}
            (ConstValue::Array(a), ConstValue::Array(b))
            | (ConstValue::Tuple(a), ConstValue::Tuple(b)) => {
                if a.len() != b.len() {
                    return false;
                }
                pending.extend(a.iter().zip(b.iter()));
            }
            (ConstValue::Maybe(a), ConstValue::Maybe(b)) => match (a.as_deref(), b.as_deref()) {
                (Some(a), Some(b)) => pending.push((a, b)),
                (None, None) => {}
                _ => return false,
            },
            (ConstValue::Map(a), ConstValue::Map(b)) => {
                if a.len() != b.len() {
                    return false;
                }
                for ((key_a, value_a), (key_b, value_b)) in a.iter().zip(b.iter()) {
                    if key_a != key_b {
                        return false;
                    }
                    pending.push((value_a, value_b));
                }
            }
            (ConstValue::Set(a), ConstValue::Set(b)) if a.iter().eq(b.iter()) => {}
            (
                ConstValue::Unit
                | ConstValue::Bool(_)
                | ConstValue::Int(_)
                | ConstValue::UInt(_)
                | ConstValue::Float(_)
                | ConstValue::Char(_)
                | ConstValue::Text(_)
                | ConstValue::Bytes(_)
                | ConstValue::Array(_)
                | ConstValue::Tuple(_)
                | ConstValue::Maybe(_)
                | ConstValue::Map(_)
                | ConstValue::Set(_),
                _,
            ) => return false,
        }
    }
    true
}

#[cfg(test)]
#[path = "../tests/unit/value_constant_identity.rs"]
mod tests;
