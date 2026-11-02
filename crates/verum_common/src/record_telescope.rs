//! Ordered dependencies of the fields of a dependent record. Each slot is a
//! declaration position. Names and receiver identities are resolved by callers.

use std::fmt;

pub const MAX_FIELDS: usize = 4096;
pub const MAX_DEPENDENCIES: usize = 16384;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TelescopeError {
    SizeLimit,
    UnknownField(usize),
    NotEarlier { field: usize, dependency: usize },
    UnorderedDependencies(usize),
    WrongPrefix { field: usize, length: usize },
    MissingWitness(usize),
}

impl fmt::Display for TelescopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SizeLimit => write!(f, "record telescope exceeds its field or dependency limit"),
            Self::UnknownField(i) => write!(f, "unknown record field slot {i}"),
            Self::NotEarlier { field, dependency } => {
                write!(f, "field {field} depends on non-earlier slot {dependency}")
            }
            Self::UnorderedDependencies(i) => write!(
                f,
                "dependencies of field {i} must be distinct and increasing"
            ),
            Self::WrongPrefix { field, length } => write!(
                f,
                "field {field} requires exactly {field} previous values, got {length}"
            ),
            Self::MissingWitness(i) => {
                write!(f, "no stable value supplied for record field slot {i}")
            }
        }
    }
}

impl std::error::Error for TelescopeError {}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FieldProjection<R> {
    pub receiver: R,
    pub field: usize,
}

/// A field in a right-associated Sigma ending in its last component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectionSelector {
    pub snd_steps: usize,
    pub take_fst: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordTelescope {
    rows: Vec<Vec<usize>>,
    used_by_later: Vec<bool>,
}

impl RecordTelescope {
    /// Validate an already canonical dependency graph, as used by replay.
    pub fn new(rows: Vec<Vec<usize>>) -> Result<Self, TelescopeError> {
        if rows.len() > MAX_FIELDS {
            return Err(TelescopeError::SizeLimit);
        }
        let mut used_by_later = vec![false; rows.len()];
        let mut remaining = MAX_DEPENDENCIES;
        for (field, row) in rows.iter().enumerate() {
            remaining = remaining
                .checked_sub(row.len())
                .ok_or(TelescopeError::SizeLimit)?;
            if row.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(TelescopeError::UnorderedDependencies(field));
            }
            for &dependency in row {
                if dependency >= field {
                    return Err(TelescopeError::NotEarlier { field, dependency });
                }
                used_by_later[dependency] = true;
            }
        }
        Ok(Self {
            rows,
            used_by_later,
        })
    }

    /// Collect distinct references from a type visitor. Repeated occurrences
    /// of a binder require one substitution. Budget the input before sorting.
    pub fn from_dependencies<I, J>(fields: I) -> Result<Self, TelescopeError>
    where
        I: IntoIterator<Item = J>,
        J: IntoIterator<Item = usize>,
    {
        let mut rows = Vec::new();
        let mut remaining = MAX_DEPENDENCIES;
        for dependencies in fields {
            if rows.len() == MAX_FIELDS {
                return Err(TelescopeError::SizeLimit);
            }
            let mut row = Vec::new();
            for dependency in dependencies {
                remaining = remaining.checked_sub(1).ok_or(TelescopeError::SizeLimit)?;
                row.push(dependency);
            }
            row.sort_unstable();
            row.dedup();
            rows.push(row);
        }
        Self::new(rows)
    }

    pub fn rows(&self) -> &[Vec<usize>] {
        &self.rows
    }

    pub fn dependencies(&self, field: usize) -> Result<&[usize], TelescopeError> {
        self.rows
            .get(field)
            .map(Vec::as_slice)
            .ok_or(TelescopeError::UnknownField(field))
    }

    pub fn selector(&self, field: usize) -> Result<ProjectionSelector, TelescopeError> {
        self.dependencies(field)?;
        Ok(ProjectionSelector {
            snd_steps: field,
            take_fst: field + 1 < self.rows.len(),
        })
    }

    pub fn construction_substitution<T: Clone>(
        &self,
        field: usize,
        prefix: &[Option<T>],
    ) -> Result<Vec<(usize, T)>, TelescopeError> {
        let row = self.dependencies(field)?;
        if prefix.len() != field {
            return Err(TelescopeError::WrongPrefix {
                field,
                length: prefix.len(),
            });
        }
        row.iter()
            .map(|&slot| {
                let value = prefix[slot]
                    .clone()
                    .ok_or(TelescopeError::MissingWitness(slot))?;
                Ok((slot, value))
            })
            .collect()
    }

    pub fn projection_substitution<R: Clone>(
        &self,
        field: usize,
        receiver: &R,
    ) -> Result<Vec<(usize, FieldProjection<R>)>, TelescopeError> {
        Ok(self
            .dependencies(field)?
            .iter()
            .map(|&slot| {
                (
                    slot,
                    FieldProjection {
                        receiver: receiver.clone(),
                        field: slot,
                    },
                )
            })
            .collect())
    }

    /// A partial write is allowed only where it cannot invalidate another
    /// slot's type. The new value still has to check at the specialized type.
    pub fn may_mutate(&self, field: usize) -> Result<bool, TelescopeError> {
        self.used_by_later
            .get(field)
            .map(|used| !used)
            .ok_or(TelescopeError::UnknownField(field))
    }
}
