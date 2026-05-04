//! Structured cfg-expression keys for multi-variant VBC archives.
//!
//! Stdlib functions with `#[cfg(target_os = "macos")]` /
//! `#[cfg(target_arch = "aarch64")]` branches are compiled into one
//! multi-variant entry per function (see [`crate::module::VbcVariant`]).
//! Each variant carries a [`CfgKey`] describing the cfg expression of
//! the source arm; the runtime/AOT loader builds a [`CfgKey`] from the
//! active target triple and calls [`CfgKey::matches`] to pick the
//! winning arm.
//!
//! The structure is deliberately small (5 fields, all optional or
//! enum-tagged) and **content-deduplicated**: the precompiler emits
//! one [`CfgKey`] per unique cfg expression in stdlib (~10 entries for
//! the current 96 cfg-conditional files), then variants reference it
//! by 16-bit ID. This keeps per-function variant overhead at 12 bytes
//! (`{cfg_key_id: u16, _padding: u16, bytecode_offset: u32, bytecode_length: u32}`)
//! regardless of how rich the cfg expression is.
//!
//! # Why a structured key, not a string
//!
//! Matching `String == String` is cache-unfriendly and offers no useful
//! relationship beyond equality. A structured key supports:
//!
//! * O(1) HashMap lookup keyed by a fixed-size struct.
//! * Subset semantics: `cfg(target_os = "macos")` matches *every*
//!   active triple where `os = darwin`, regardless of arch / ptr-width.
//! * Round-trip via `serde` with stable enum tags — newer compiler
//!   versions can deserialise older archives without losing keys they
//!   don't understand (`TargetArch::Other(..)` carries the original
//!   token text).

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::types::StringId;

/// Operating-system axis of a `cfg(target_os = ...)` expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetOs {
    Darwin,
    Linux,
    Windows,
    FreeBsd,
    OpenBsd,
    NetBsd,
    Wasi,
    None, // bare-metal / embedded
    /// Anything else — token text stored in the module string-pool so
    /// older readers can still print the value.
    Other(StringId),
}

/// Architecture axis of a `cfg(target_arch = ...)` expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetArch {
    X86_64,
    Aarch64,
    Arm,
    Riscv64,
    PowerPc64,
    S390x,
    Wasm32,
    Wasm64,
    Bpfel,
    Bpfeb,
    Other(StringId),
}

/// Pointer-width axis. Most `cfg(target_pointer_width = "N")` expressions
/// resolve to one of these three; `Custom(u8)` accommodates rare 16-bit
/// embedded targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PtrWidth {
    Bits32,
    Bits64,
    Custom(u8),
}

/// Endian-ness axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Endian {
    Little,
    Big,
}

/// A structured cfg expression — the conjunction of every named
/// constraint. `None` on a field means "any" (the variant matches
/// regardless of the active value of that axis).
///
/// # Matching rules
///
/// Given a precompiled [`CfgKey`] (the source variant's cfg expression)
/// and an `active` [`CfgKey`] (the target triple's resolved values),
/// the source key *matches* the active key when every constrained axis
/// in the source is satisfied by the active key:
///
/// ```ignore
/// source.os.is_none() || source.os == active.os
/// ```
///
/// applied to every field. Features in the source are matched as a
/// subset: every required feature must be in `active.features`.
///
/// The *empty* `CfgKey` (all `None`, no features) matches every active
/// triple — that is the "universal" function variant equivalent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct CfgKey {
    pub os: Option<TargetOs>,
    pub arch: Option<TargetArch>,
    pub ptr_width: Option<PtrWidth>,
    pub endian: Option<Endian>,
    /// Required cfg features (e.g. `target_feature = "neon"`).
    /// Variants without features list have an empty SmallVec.
    #[serde(default)]
    pub features: SmallVec<[StringId; 2]>,
}

impl CfgKey {
    /// Construct a fully-resolved key for the given target triple.
    /// Used by the loader at archive-open time; runs once per process.
    ///
    /// String-pooled fields (`Other` arms, `features`) take the
    /// provided interner so the resulting key is comparable to the
    /// archive's stored keys. The interner is the linker's own
    /// string-pool — see `crate::linker`.
    pub fn for_triple(triple: &str, intern: &mut dyn FnMut(&str) -> StringId) -> Self {
        let lower = triple.to_ascii_lowercase();
        let mut key = CfgKey::default();
        key.os = Some(parse_os(&lower, intern));
        key.arch = Some(parse_arch(&lower, intern));
        key.ptr_width = Some(parse_ptr_width(&lower, key.arch));
        key.endian = Some(parse_endian(&lower, key.arch));
        key
    }

    /// Source variant matches `active`?
    ///
    /// `self` is the variant's stored cfg constraint; `active` is the
    /// target triple's resolved key. Returns true iff every constrained
    /// axis in `self` agrees with `active`.
    pub fn matches(&self, active: &CfgKey) -> bool {
        match_axis(self.os, active.os) == AxisMatch::Match
            && match_axis(self.arch, active.arch) == AxisMatch::Match
            && match_axis(self.ptr_width, active.ptr_width) == AxisMatch::Match
            && match_axis(self.endian, active.endian) == AxisMatch::Match
            && self.features.iter().all(|f| active.features.contains(f))
    }

    /// True for the all-`None` empty key — the "universal variant"
    /// marker.
    pub fn is_universal(&self) -> bool {
        self.os.is_none()
            && self.arch.is_none()
            && self.ptr_width.is_none()
            && self.endian.is_none()
            && self.features.is_empty()
    }
}

#[derive(PartialEq, Eq)]
enum AxisMatch {
    Match,
    Conflict,
}

fn match_axis<T: Eq>(constraint: Option<T>, active: Option<T>) -> AxisMatch {
    match (constraint, active) {
        (None, _) => AxisMatch::Match,
        (Some(c), Some(a)) if c == a => AxisMatch::Match,
        // Constraint set, active unknown — counts as conflict (loader
        // should never have an unknown axis for the host triple).
        _ => AxisMatch::Conflict,
    }
}

fn parse_os(triple: &str, intern: &mut dyn FnMut(&str) -> StringId) -> TargetOs {
    if triple.contains("darwin") || triple.contains("macos") || triple.contains("ios") {
        TargetOs::Darwin
    } else if triple.contains("linux") || triple.contains("android") {
        TargetOs::Linux
    } else if triple.contains("windows") {
        TargetOs::Windows
    } else if triple.contains("freebsd") {
        TargetOs::FreeBsd
    } else if triple.contains("openbsd") {
        TargetOs::OpenBsd
    } else if triple.contains("netbsd") {
        TargetOs::NetBsd
    } else if triple.contains("wasi") {
        TargetOs::Wasi
    } else if triple.contains("none") || triple.contains("unknown") && triple.contains("eabi") {
        TargetOs::None
    } else {
        // Pull the OS slot out of the triple — it's the third
        // dash-separated token by convention (`arch-vendor-os-env`).
        let token = triple.split('-').nth(2).unwrap_or(triple);
        TargetOs::Other(intern(token))
    }
}

fn parse_arch(triple: &str, intern: &mut dyn FnMut(&str) -> StringId) -> TargetArch {
    let head = triple.split('-').next().unwrap_or(triple);
    match head {
        "x86_64" | "amd64" => TargetArch::X86_64,
        "aarch64" | "arm64" => TargetArch::Aarch64,
        "arm" | "armv7" | "armv6" | "armv4t" => TargetArch::Arm,
        "riscv64" | "riscv64gc" => TargetArch::Riscv64,
        "powerpc64" | "powerpc64le" | "ppc64" | "ppc64le" => TargetArch::PowerPc64,
        "s390x" => TargetArch::S390x,
        "wasm32" => TargetArch::Wasm32,
        "wasm64" => TargetArch::Wasm64,
        "bpfel" => TargetArch::Bpfel,
        "bpfeb" => TargetArch::Bpfeb,
        other => TargetArch::Other(intern(other)),
    }
}

fn parse_ptr_width(triple: &str, arch: Option<TargetArch>) -> PtrWidth {
    if let Some(a) = arch {
        return match a {
            TargetArch::X86_64
            | TargetArch::Aarch64
            | TargetArch::Riscv64
            | TargetArch::PowerPc64
            | TargetArch::S390x
            | TargetArch::Wasm64 => PtrWidth::Bits64,
            TargetArch::Arm | TargetArch::Wasm32 | TargetArch::Bpfel | TargetArch::Bpfeb => {
                PtrWidth::Bits32
            }
            TargetArch::Other(_) => {
                // Best-effort substring scan.
                if triple.contains("64") {
                    PtrWidth::Bits64
                } else {
                    PtrWidth::Bits32
                }
            }
        };
    }
    PtrWidth::Bits64
}

fn parse_endian(triple: &str, arch: Option<TargetArch>) -> Endian {
    match arch {
        Some(TargetArch::PowerPc64) if !triple.contains("le") => Endian::Big,
        Some(TargetArch::S390x) | Some(TargetArch::Bpfeb) => Endian::Big,
        _ => Endian::Little,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_intern() -> impl FnMut(&str) -> StringId {
        let mut counter: u32 = 0;
        let mut seen: HashMap<String, StringId> = HashMap::new();
        move |s| {
            if let Some(id) = seen.get(s) {
                *id
            } else {
                let id = StringId(counter);
                counter += 1;
                seen.insert(s.to_string(), id);
                id
            }
        }
    }

    #[test]
    fn host_triple_resolves() {
        let mut intern = make_intern();
        let key = CfgKey::for_triple("aarch64-apple-darwin", &mut intern);
        assert_eq!(key.os, Some(TargetOs::Darwin));
        assert_eq!(key.arch, Some(TargetArch::Aarch64));
        assert_eq!(key.ptr_width, Some(PtrWidth::Bits64));
        assert_eq!(key.endian, Some(Endian::Little));
    }

    #[test]
    fn linux_x86_64_resolves() {
        let mut intern = make_intern();
        let key = CfgKey::for_triple("x86_64-unknown-linux-gnu", &mut intern);
        assert_eq!(key.os, Some(TargetOs::Linux));
        assert_eq!(key.arch, Some(TargetArch::X86_64));
        assert_eq!(key.ptr_width, Some(PtrWidth::Bits64));
    }

    #[test]
    fn windows_resolves() {
        let mut intern = make_intern();
        let key = CfgKey::for_triple("x86_64-pc-windows-msvc", &mut intern);
        assert_eq!(key.os, Some(TargetOs::Windows));
        assert_eq!(key.arch, Some(TargetArch::X86_64));
    }

    #[test]
    fn os_only_constraint_matches_any_arch() {
        let mut intern = make_intern();
        let darwin_only = CfgKey {
            os: Some(TargetOs::Darwin),
            ..CfgKey::default()
        };
        let active_aarch64 = CfgKey::for_triple("aarch64-apple-darwin", &mut intern);
        let active_x86 = CfgKey::for_triple("x86_64-apple-darwin", &mut intern);
        assert!(darwin_only.matches(&active_aarch64));
        assert!(darwin_only.matches(&active_x86));
    }

    #[test]
    fn arch_mismatch_rejects() {
        let mut intern = make_intern();
        let aarch64_only = CfgKey {
            arch: Some(TargetArch::Aarch64),
            ..CfgKey::default()
        };
        let active_x86 = CfgKey::for_triple("x86_64-apple-darwin", &mut intern);
        assert!(!aarch64_only.matches(&active_x86));
    }

    #[test]
    fn universal_matches_everyone() {
        let mut intern = make_intern();
        let universal = CfgKey::default();
        assert!(universal.is_universal());
        let active = CfgKey::for_triple("x86_64-pc-windows-msvc", &mut intern);
        assert!(universal.matches(&active));
    }

    #[test]
    fn ppc64_big_endian_default() {
        let mut intern = make_intern();
        let key = CfgKey::for_triple("powerpc64-unknown-linux-gnu", &mut intern);
        assert_eq!(key.endian, Some(Endian::Big));
        let key_le = CfgKey::for_triple("powerpc64le-unknown-linux-gnu", &mut intern);
        assert_eq!(key_le.endian, Some(Endian::Little));
    }

    #[test]
    fn feature_subset_match() {
        let mut intern = make_intern();
        let neon_id = intern("neon");
        let aes_id = intern("aes");
        let _other = intern("other");
        let needs_neon = CfgKey {
            features: SmallVec::from_buf([neon_id, StringId(0)]),
            ..CfgKey::default()
        };
        // Truncate to single feature to keep the test focused.
        let mut needs_neon = needs_neon;
        needs_neon.features.clear();
        needs_neon.features.push(neon_id);

        let mut active = CfgKey::default();
        active.features.push(neon_id);
        active.features.push(aes_id);
        assert!(needs_neon.matches(&active));
    }
}
