//! Explicit entry-source parsing policy shared by execution and editor clients.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceMode {
    /// Preserve source declarations: modules, or scripts with a shebang.
    #[default]
    Source,
    /// Permit top-level statements in this entry source, not its imports.
    Script,
}

impl SourceMode {
    pub const fn as_str(self) -> &'static str {
        match self { Self::Source => "source", Self::Script => "script" }
    }
    pub const fn language_id(self) -> &'static str {
        match self { Self::Source => "verum", Self::Script => "verum-script" }
    }
    pub fn from_language_id(id: &str) -> Self {
        if id == "verum-script" { Self::Script } else { Self::Source }
    }
}

impl std::str::FromStr for SourceMode {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "source" => Ok(Self::Source),
            "script" => Ok(Self::Script),
            _ => Err("source mode must be source or script"),
        }
    }
}
