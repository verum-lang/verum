//! Closed sequence values carried by generic calls. Leaves use the same exact
//! signed integer domain as scalar const arguments. No module-local pool IDs
//! or caller slots occur inside a closed value.
use crate::encoding::{decode_u8, decode_varint, encode_varint};
use crate::error::{VbcError, VbcResult};
use crate::types::{TypeId, TypeRef};
use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConstSequence {
    /// Contiguous scalar data has one representation for literals and repeats.
    Integers(Arc<[i64]>),
    /// Rows retain their own extents, including empty and ragged rows.
    Arrays(Arc<[ConstSequence]>),
}

use verum_common::const_value::{MAX_SEQUENCE_DEPTH, MAX_SEQUENCE_ELEMENTS};

pub(crate) fn sequence_domain(domain: &TypeRef) -> Option<(&TypeRef, Option<u64>)> {
    match domain {
        TypeRef::Slice(element) => Some((element, None)),
        TypeRef::Array { element, length } => Some((element, Some(*length))),
        _ => None,
    }
}

impl ConstSequence {
    pub fn len(&self) -> usize {
        match self {
            Self::Integers(values) => values.len(),
            Self::Arrays(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn validate(&self) -> VbcResult<()> {
        fn visit(value: &ConstSequence, remaining: &mut usize, depth: usize) -> VbcResult<()> {
            if depth >= MAX_SEQUENCE_DEPTH {
                return Err(VbcError::Serialization(
                    "constant sequence exceeds maximum nesting depth".into(),
                ));
            }
            *remaining = remaining.checked_sub(value.len()).ok_or_else(|| {
                VbcError::Serialization("constant sequence exceeds the total element limit".into())
            })?;
            if let ConstSequence::Arrays(children) = value {
                for child in children.iter() {
                    visit(child, remaining, depth + 1)?;
                }
            }
            Ok(())
        }
        let mut remaining = MAX_SEQUENCE_ELEMENTS;
        visit(self, &mut remaining, 0)
    }

    pub(crate) fn supported_domain(mut domain: &TypeRef) -> bool {
        for _ in 0..MAX_SEQUENCE_DEPTH {
            let Some((element, _)) = sequence_domain(domain) else {
                return false;
            };
            if *element == TypeRef::Concrete(TypeId::INT) {
                return true;
            }
            domain = element;
        }
        false
    }

    pub(crate) fn matches_domain(&self, domain: &TypeRef) -> bool {
        let Some((element, length)) = sequence_domain(domain) else {
            return false;
        };
        if length.is_some_and(|n| n != self.len() as u64) {
            return false;
        }
        match self {
            Self::Integers(_) => *element == TypeRef::Concrete(TypeId::INT),
            Self::Arrays(values) => {
                Self::supported_domain(element)
                    && values.iter().all(|value| value.matches_domain(element))
            }
        }
    }

    pub(crate) fn encode(&self, bytes: &mut Vec<u8>) {
        bytes.push(match self {
            Self::Integers(_) => 0,
            Self::Arrays(_) => 1,
        });
        encode_varint(self.len() as u64, bytes);
        match self {
            Self::Integers(values) => {
                for value in values.iter() {
                    encode_varint(*value as u64, bytes);
                }
            }
            Self::Arrays(arrays) => {
                for value in arrays.iter() {
                    value.encode(bytes);
                }
            }
        }
    }

    pub(crate) fn decode(bytes: &[u8], offset: &mut usize) -> VbcResult<Self> {
        fn read(
            bytes: &[u8],
            offset: &mut usize,
            remaining: &mut usize,
            depth: usize,
        ) -> VbcResult<ConstSequence> {
            if depth >= MAX_SEQUENCE_DEPTH {
                return Err(VbcError::Deserialization(
                    "constant sequence exceeds maximum nesting depth".into(),
                ));
            }
            let tag = decode_u8(bytes, offset)?;
            let n = decode_varint(bytes, offset)?;
            if n > *remaining as u64 {
                return Err(VbcError::TableTooLarge {
                    field: "constant_sequence_elements",
                    count: n.min(u32::MAX as u64) as u32,
                    max: MAX_SEQUENCE_ELEMENTS as u32,
                });
            }
            let count = n as usize;
            if count > bytes.len().saturating_sub(*offset) {
                return Err(VbcError::Deserialization(
                    "truncated constant sequence".into(),
                ));
            }
            *remaining -= count;
            match tag {
                0 => (0..count)
                    .map(|_| decode_varint(bytes, offset).map(|value| value as i64))
                    .collect::<VbcResult<Arc<[_]>>>()
                    .map(ConstSequence::Integers),
                1 => (0..count)
                    .map(|_| read(bytes, offset, remaining, depth + 1))
                    .collect::<VbcResult<Arc<[_]>>>()
                    .map(ConstSequence::Arrays),
                _ => Err(VbcError::Deserialization(format!(
                    "unknown constant sequence kind {tag}"
                ))),
            }
        }
        let mut remaining = MAX_SEQUENCE_ELEMENTS;
        read(bytes, offset, &mut remaining, 0)
    }
}

impl fmt::Display for ConstSequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn items<T: fmt::Display>(values: &[T], f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("[")?;
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{value}")?;
            }
            f.write_str("]")
        }
        match self {
            Self::Integers(v) => items(v, f),
            Self::Arrays(v) => items(v, f),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_or_resource_exhausting_sequences_are_rejected_before_allocation() {
        for data in [vec![0, 2, 1], vec![2, 0], vec![1, 2, 0, 0], vec![]] {
            assert!(ConstSequence::decode(&data, &mut 0).is_err());
        }
        let mut oversized = vec![0];
        encode_varint(MAX_SEQUENCE_ELEMENTS as u64 + 1, &mut oversized);
        assert!(matches!(
            ConstSequence::decode(&oversized, &mut 0),
            Err(VbcError::TableTooLarge { .. })
        ));
        let mut deep = [1, 1].repeat(MAX_SEQUENCE_DEPTH);
        deep.extend([0, 0]);
        assert!(ConstSequence::decode(&deep, &mut 0).is_err());
    }
}
