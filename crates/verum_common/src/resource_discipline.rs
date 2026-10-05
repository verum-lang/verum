//! Declaration-owned usage constraints shared by source, archives and execution tiers.
//! A discipline is neither a Copy/Clone capability nor an owning cleanup ticket.

/// Missing provenance must not become permission to duplicate or destroy a value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum ResourceDiscipline {
    #[default]
    Unknown = 0,
    Unrestricted = 1,
    Affine = 2,
    Linear = 3,
}

impl ResourceDiscipline {
    /// Combine a declaration with its owned components; missing facts stay unknown.
    pub const fn with_component(self, component: Self) -> Self {
        match (self, component) {
            (Self::Unknown, _) | (_, Self::Unknown) => Self::Unknown,
            (Self::Linear, _) | (_, Self::Linear) => Self::Linear,
            (Self::Affine, _) | (_, Self::Affine) => Self::Affine,
            _ => Self::Unrestricted,
        }
    }

    pub const fn from_wire(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Unknown),
            1 => Some(Self::Unrestricted),
            2 => Some(Self::Affine),
            3 => Some(Self::Linear),
            _ => None,
        }
    }
}
