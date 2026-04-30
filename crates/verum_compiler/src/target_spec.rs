//! Target specification: decomposed view of a Rust-style target triple.
//!
//! The `target_triple` field on `CompilerOptions` is the canonical
//! source of cross-compilation intent — but a triple alone (e.g.
//! `"aarch64-unknown-linux-gnu"`) is opaque. Every `@cfg(...)`
//! evaluation site needs the FOUR semantic axes:
//!
//!   * `target_os` — `"linux"`, `"macos"`, `"windows"`, `"wasi"`
//!   * `target_arch` — `"x86_64"`, `"aarch64"`, `"riscv64"`,
//!                     `"wasm32"`, …
//!   * `target_pointer_width` — `"32"` / `"64"` (string per Rust's
//!     cfg vocabulary)
//!   * `target_endian` — `"little"` / `"big"`
//!
//! Pre-A4 the compiler hardcoded `cfg!(target_os = "...")` —
//! evaluating the COMPILER's host, not the target. Cross-compiling
//! from macOS to Linux silently dropped Linux modules. This module
//! gives every cfg site one canonical answer.

use verum_common::Text;

/// Decomposed target spec, derived from a target triple or the
/// process's host configuration when no triple was specified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSpec {
    /// `target_os = "..."`
    pub os: Text,
    /// `target_arch = "..."`
    pub arch: Text,
    /// `target_pointer_width = "32"` or `"64"`
    pub pointer_width: Text,
    /// `target_endian = "little"` or `"big"`
    pub endian: Text,
    /// The full triple (preserved for diagnostics + LLVM forwarding).
    pub triple: Text,
}

impl TargetSpec {
    /// Build the spec from a triple string. Returns the host spec
    /// when the triple is `None` or unrecognised — that preserves
    /// the legacy host-equals-target behaviour for native builds.
    pub fn from_triple(triple: Option<&str>) -> Self {
        match triple {
            Some(t) => Self::parse_triple(t),
            None => Self::host(),
        }
    }

    /// Parse a Rust-style target triple into its semantic components.
    ///
    /// Supported triple shapes (the four that cover ~99 % of real
    /// cross-compile targets):
    ///
    ///   * `<arch>-<vendor>-<os>` — three components, e.g.
    ///     `"aarch64-unknown-linux"`.
    ///   * `<arch>-<vendor>-<os>-<env>` — four components, e.g.
    ///     `"aarch64-unknown-linux-gnu"`.
    ///   * `wasm32-unknown-unknown` and friends — the special-case
    ///     "unknown" os is normalised to `"wasi"` when the arch
    ///     starts with `wasm`, so cfg gates targeting WASI work.
    ///
    /// Unknown components fall back to the host equivalents — a
    /// best-effort default so a partially-spelled triple still
    /// produces a usable spec.
    pub fn parse_triple(triple: &str) -> Self {
        let host = Self::host();
        let parts: Vec<&str> = triple.split('-').collect();

        let arch_str = parts.first().copied().unwrap_or("").to_string();
        let arch = if arch_str.is_empty() { host.arch.as_str().to_string() } else { arch_str.clone() };

        // OS is parts[2] for 3-component triples; for 4+ it's parts[2]
        // (vendor is parts[1]). For 2-component triples like
        // `wasm32-wasi` it's parts[1].
        let os_raw = match parts.len() {
            2 => parts[1].to_string(),
            3 | 4 => parts[2].to_string(),
            _ => host.os.as_str().to_string(),
        };
        let os = match os_raw.as_str() {
            "darwin" | "macos" => "macos".to_string(),
            "linux" => "linux".to_string(),
            "windows" => "windows".to_string(),
            "wasi" => "wasi".to_string(),
            "unknown" if arch.starts_with("wasm") => "wasi".to_string(),
            "" => host.os.as_str().to_string(),
            other => other.to_string(),
        };

        let pointer_width = pointer_width_for_arch(&arch).to_string();
        let endian = endian_for_arch(&arch).to_string();

        Self {
            os: Text::from(os),
            arch: Text::from(arch),
            pointer_width: Text::from(pointer_width),
            endian: Text::from(endian),
            triple: Text::from(triple),
        }
    }

    /// Spec for the compiler's host platform. Read from `cfg!`
    /// macros at compile time so the host's identity is captured
    /// once, deterministically.
    pub fn host() -> Self {
        let os = if cfg!(target_os = "macos") { "macos" }
            else if cfg!(target_os = "linux") { "linux" }
            else if cfg!(target_os = "windows") { "windows" }
            else if cfg!(target_os = "wasi") { "wasi" }
            else { "unknown" };

        let arch = if cfg!(target_arch = "x86_64") { "x86_64" }
            else if cfg!(target_arch = "aarch64") { "aarch64" }
            else if cfg!(target_arch = "riscv64") { "riscv64" }
            else if cfg!(target_arch = "wasm32") { "wasm32" }
            else { "unknown" };

        let pointer_width = if cfg!(target_pointer_width = "64") { "64" } else { "32" };
        let endian = if cfg!(target_endian = "little") { "little" } else { "big" };

        Self {
            os: Text::from(os),
            arch: Text::from(arch),
            pointer_width: Text::from(pointer_width),
            endian: Text::from(endian),
            triple: Text::from(format!("{}-unknown-{}", arch, os)),
        }
    }

    /// Evaluate a `key = "value"` cfg predicate against this target.
    /// Returns `Some(true|false)` when the predicate is one of the
    /// four target axes; `None` for predicates this evaluator
    /// doesn't recognise (callers fall back to feature-flag /
    /// custom-cfg evaluators).
    pub fn matches_predicate(&self, key: &str, value: &str) -> Option<bool> {
        match key {
            "target_os" => Some(self.os.as_str() == value),
            "target_arch" => Some(self.arch.as_str() == value),
            "target_pointer_width" => Some(self.pointer_width.as_str() == value),
            "target_endian" => Some(self.endian.as_str() == value),
            _ => None,
        }
    }

    /// Convenience: match a single textual predicate of the form
    /// `key = "value"`. Returns `None` for non-target-axis predicates.
    pub fn matches_textual(&self, predicate: &str) -> Option<bool> {
        // Tokens: `target_arch = "x86_64"` or `target_arch="x86_64"`.
        let mut iter = predicate.splitn(2, '=');
        let key = iter.next()?.trim();
        let value_raw = iter.next()?.trim();
        let value = value_raw.trim_matches('"');
        self.matches_predicate(key, value)
    }
}

fn pointer_width_for_arch(arch: &str) -> &'static str {
    match arch {
        "x86_64" | "aarch64" | "riscv64" | "powerpc64" | "powerpc64le"
        | "s390x" | "mips64" | "mips64el" | "wasm64" | "loongarch64" => "64",
        "i386" | "i486" | "i586" | "i686" | "arm" | "armv7" | "thumbv7em"
        | "riscv32" | "powerpc" | "mips" | "mipsel" | "wasm32" => "32",
        _ => "64", // Conservative default: assume 64-bit if unknown.
    }
}

fn endian_for_arch(arch: &str) -> &'static str {
    match arch {
        "powerpc" | "powerpc64" | "s390x" | "mips" | "mips64" | "sparc"
        | "sparc64" => "big",
        _ => "little",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_aarch64_linux_triple() {
        let s = TargetSpec::parse_triple("aarch64-unknown-linux-gnu");
        assert_eq!(s.os.as_str(), "linux");
        assert_eq!(s.arch.as_str(), "aarch64");
        assert_eq!(s.pointer_width.as_str(), "64");
        assert_eq!(s.endian.as_str(), "little");
    }

    #[test]
    fn parse_x86_64_macos_triple() {
        let s = TargetSpec::parse_triple("x86_64-apple-darwin");
        assert_eq!(s.os.as_str(), "macos");
        assert_eq!(s.arch.as_str(), "x86_64");
        assert_eq!(s.pointer_width.as_str(), "64");
    }

    #[test]
    fn parse_riscv32_imac_triple() {
        let s = TargetSpec::parse_triple("riscv32-unknown-linux");
        assert_eq!(s.arch.as_str(), "riscv32");
        assert_eq!(s.pointer_width.as_str(), "32");
        assert_eq!(s.os.as_str(), "linux");
    }

    #[test]
    fn parse_wasm32_unknown() {
        let s = TargetSpec::parse_triple("wasm32-unknown-unknown");
        assert_eq!(s.arch.as_str(), "wasm32");
        assert_eq!(s.pointer_width.as_str(), "32");
        assert_eq!(s.os.as_str(), "wasi");
    }

    #[test]
    fn parse_powerpc64_big_endian() {
        let s = TargetSpec::parse_triple("powerpc64-unknown-linux");
        assert_eq!(s.arch.as_str(), "powerpc64");
        assert_eq!(s.endian.as_str(), "big");
        assert_eq!(s.pointer_width.as_str(), "64");
    }

    #[test]
    fn matches_predicate_target_os() {
        let s = TargetSpec::parse_triple("aarch64-unknown-linux-gnu");
        assert_eq!(s.matches_predicate("target_os", "linux"), Some(true));
        assert_eq!(s.matches_predicate("target_os", "macos"), Some(false));
    }

    #[test]
    fn matches_predicate_target_arch() {
        let s = TargetSpec::parse_triple("aarch64-unknown-linux-gnu");
        assert_eq!(s.matches_predicate("target_arch", "aarch64"), Some(true));
        assert_eq!(s.matches_predicate("target_arch", "x86_64"), Some(false));
    }

    #[test]
    fn matches_predicate_target_pointer_width() {
        let s = TargetSpec::parse_triple("riscv32-unknown-linux");
        assert_eq!(s.matches_predicate("target_pointer_width", "32"), Some(true));
        assert_eq!(s.matches_predicate("target_pointer_width", "64"), Some(false));
    }

    #[test]
    fn matches_predicate_target_endian() {
        let s = TargetSpec::parse_triple("powerpc64-unknown-linux");
        assert_eq!(s.matches_predicate("target_endian", "big"), Some(true));
        assert_eq!(s.matches_predicate("target_endian", "little"), Some(false));
    }

    #[test]
    fn matches_predicate_unknown_returns_none() {
        let s = TargetSpec::host();
        assert_eq!(s.matches_predicate("target_feature", "avx2"), None);
        assert_eq!(s.matches_predicate("custom_flag", "true"), None);
    }

    #[test]
    fn matches_textual_with_quotes() {
        let s = TargetSpec::parse_triple("aarch64-unknown-linux-gnu");
        assert_eq!(s.matches_textual("target_os = \"linux\""), Some(true));
        assert_eq!(s.matches_textual("target_os=\"macos\""), Some(false));
    }

    #[test]
    fn from_triple_none_uses_host() {
        let s = TargetSpec::from_triple(None);
        let h = TargetSpec::host();
        assert_eq!(s.os, h.os);
        assert_eq!(s.arch, h.arch);
    }
}
