//! Structural universe levels and universal ordering over natural valuations.
//!
//! Variable identifiers are opaque. Successor and maximum retain their operands;
//! neither an identifier nor an unresolved constraint denotes a concrete level.

use std::fmt;
use verum_common::{List, Map};

pub const MAX_LEVEL_NODES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UniverseLevel {
    Concrete(u32),
    Variable(u32),
    Max(Box<UniverseLevel>, Box<UniverseLevel>),
    Succ(Box<UniverseLevel>),
}

/// `max(constant, variable₁ + offset₁, …)` with sorted, unique variable IDs.
/// Instances are produced only by checked normalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalLevel {
    constant: u64,
    terms: List<(u32, u64)>,
}

impl NormalLevel {
    pub fn constant(&self) -> u64 {
        self.constant
    }
    pub fn terms(&self) -> &[(u32, u64)] {
        &self.terms
    }
    pub fn minimum(&self) -> u64 {
        self.terms
            .iter()
            .fold(self.constant, |value, (_, offset)| value.max(*offset))
    }
    /// Universal validity, not the existence of one satisfying assignment.
    pub fn is_less_or_equal(&self, other: &Self) -> bool {
        self.constant <= other.minimum()
            && self.terms.iter().all(|(variable, offset)| {
                other
                    .terms
                    .binary_search_by_key(variable, |(id, _)| *id)
                    .is_ok_and(|index| *offset <= other.terms[index].1)
            })
    }
    fn successor(mut self) -> Option<Self> {
        self.constant = self.constant.checked_add(1)?;
        for (_, offset) in &mut self.terms {
            *offset = offset.checked_add(1)?;
        }
        Some(self)
    }
    pub fn evaluate(&self, assignment: &Map<u32, u64>) -> Option<u64> {
        let mut value = self.constant;
        for (variable, offset) in &self.terms {
            value = value.max(assignment.get(variable)?.checked_add(*offset)?);
        }
        Some(value)
    }
}

impl UniverseLevel {
    pub const TYPE: Self = Self::Concrete(0);
    pub const TYPE1: Self = Self::Concrete(1);
    pub const TYPE2: Self = Self::Concrete(2);
    pub fn concrete(n: u32) -> Self {
        Self::Concrete(n)
    }
    pub fn variable(id: u32) -> Self {
        Self::Variable(id)
    }
    pub fn succ(&self) -> Self {
        if let Self::Concrete(n) = self {
            if let Some(next) = n.checked_add(1) {
                return Self::Concrete(next);
            }
        }
        Self::Succ(Box::new(self.clone()))
    }
    pub fn max(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Concrete(a), Self::Concrete(b)) => Self::Concrete((*a).max(*b)),
            (Self::Concrete(0), _) => other.clone(),
            (_, Self::Concrete(0)) => self.clone(),
            (Self::Variable(a), Self::Variable(b)) if a == b => self.clone(),
            _ if std::ptr::eq(self, other) => self.clone(),
            _ => Self::Max(Box::new(self.clone()), Box::new(other.clone())),
        }
    }
    /// Preserve a model value exactly, or refuse its representation when it
    /// would exceed the expression budget used by normalization.
    pub fn from_value(value: u64) -> Option<Self> {
        if let Ok(value) = u32::try_from(value) {
            return Some(Self::Concrete(value));
        }
        let successors = value.checked_sub(u64::from(u32::MAX))?;
        if successors >= MAX_LEVEL_NODES as u64 {
            return None;
        }
        let mut level = Self::Concrete(u32::MAX);
        for _ in 0..successors {
            level = Self::Succ(Box::new(level));
        }
        Some(level)
    }
    /// Linear traversal without distributive expansion. Duplicate variable
    /// terms retain the greatest offset; identifiers are never compared as values.
    pub fn normalize(&self) -> Option<NormalLevel> {
        let mut work = List::from_single((self, 0u64));
        let mut remaining = MAX_LEVEL_NODES;
        let mut constant = 0;
        let mut offsets: Map<u32, u64> = Map::new();
        while let Some((level, offset)) = work.pop() {
            remaining = remaining.checked_sub(1)?;
            match level {
                Self::Concrete(n) => constant = constant.max(u64::from(*n).checked_add(offset)?),
                Self::Variable(id) => {
                    let value = offsets.entry(*id).or_insert(0);
                    *value = (*value).max(offset);
                }
                Self::Succ(inner) => work.push((inner, offset.checked_add(1)?)),
                Self::Max(a, b) => {
                    work.push((a, offset));
                    work.push((b, offset));
                }
            }
        }
        let mut terms: List<_> = offsets.into_iter().collect();
        terms.sort_unstable_by_key(|(variable, _)| *variable);
        Some(NormalLevel { constant, terms })
    }
    pub fn evaluate(&self, assignment: &Map<u32, u64>) -> Option<u64> {
        self.normalize()?.evaluate(assignment)
    }
    pub fn is_less_or_equal(&self, other: &Self) -> bool {
        match (self.normalize(), other.normalize()) {
            (Some(left), Some(right)) => left.is_less_or_equal(&right),
            _ => false,
        }
    }
    pub fn is_less_than(&self, other: &Self) -> bool {
        match (
            self.normalize().and_then(NormalLevel::successor),
            other.normalize(),
        ) {
            (Some(left), Some(right)) => left.is_less_or_equal(&right),
            _ => false,
        }
    }
    pub fn equivalent(&self, other: &Self) -> bool {
        match (self.normalize(), other.normalize()) {
            (Some(left), Some(right)) => {
                left.is_less_or_equal(&right) && right.is_less_or_equal(&left)
            }
            _ => false,
        }
    }
    pub fn variables(&self) -> List<u32> {
        let mut variables = List::new();
        let mut work = List::from_single(self);
        while let Some(level) = work.pop() {
            match level {
                Self::Concrete(_) => {}
                Self::Variable(id) => variables.push(*id),
                Self::Succ(inner) => work.push(inner),
                Self::Max(a, b) => {
                    work.push(a);
                    work.push(b);
                }
            }
        }
        variables.sort_unstable();
        variables.dedup();
        variables
    }
    /// Conservative lower bound; exhaustion yields zero. Use normalize to
    /// distinguish that case explicitly instead of interpreting it as equality.
    pub fn lower_bound(&self) -> u64 {
        self.normalize().map_or(0, |level| level.minimum())
    }
    pub fn is_polymorphic(&self) -> bool {
        self.normalize().is_none_or(|level| !level.terms.is_empty())
    }
}
impl Default for UniverseLevel {
    fn default() -> Self {
        Self::TYPE
    }
}
impl fmt::Display for UniverseLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Concrete(0) => f.write_str("Type"),
            Self::Concrete(value) => {
                f.write_str("Type")?;
                for digit in value.to_string().chars() {
                    f.write_str(match digit {
                        '0' => "₀",
                        '1' => "₁",
                        '2' => "₂",
                        '3' => "₃",
                        '4' => "₄",
                        '5' => "₅",
                        '6' => "₆",
                        '7' => "₇",
                        '8' => "₈",
                        '9' => "₉",
                        _ => return Err(fmt::Error),
                    })?;
                }
                Ok(())
            }
            Self::Variable(id) => write!(f, "u{id}"),
            Self::Succ(inner) => write!(f, "succ({inner})"),
            Self::Max(a, b) => write!(f, "max({a}, {b})"),
        }
    }
}
