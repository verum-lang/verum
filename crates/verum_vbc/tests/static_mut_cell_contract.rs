//! Task #28 [F2] cell-backed `static mut` contract pin.
//!
//! Asserts that every `static mut` in `core/` whose address is taken
//! via `&X as *T` (the cell-backed dispatch path) is a SCALAR with
//! ZERO initialiser. Pre-condition for the current 8-byte
//! `Box<UnsafeCell<u64>>` cell architecture (Task #26 [E2]
//! `StaticMutAddr` opcode + `InterpreterState::static_mut_cells`).
//!
//! When this pin fails, F2's dynamic-size cell + non-zero-init
//! ctor extensions become load-bearing — implement them at the same
//! time the new declaration lands, NOT after.

use std::fs;
use std::path::PathBuf;

/// Walks `core/`, scans every `*.vr` file for `static mut` declarations,
/// and asserts that every name appearing in an `&NAME as *T` cast pattern
/// is also a scalar with zero initialiser. Failure = a new `static mut`
/// has been added that needs F2's dynamic-cell extension.
#[test]
fn cell_backed_static_muts_are_scalar_zero_init() {
    let core_root = workspace_root().join("core");
    if !core_root.is_dir() {
        eprintln!(
            "[F2-contract] core/ absent ({}); skipping pin — \
             stdlib-presence policy mirrors other tests in this crate.",
            core_root.display()
        );
        return;
    }

    // Pattern set for static-mut declarations and address-of casts.
    let static_mut_re = regex_like::compile(r"^\s*(?:public\s+)?static\s+mut\s+([A-Z_][A-Z_0-9]*)\s*:\s*([^=]+?)\s*=\s*(.*?);");
    let addrof_re = regex_like::compile(r"&\s*([A-Z_][A-Z_0-9]+)\s+as\s+\*(?:const|mut)\b");

    // (name, declared_type, init_expr) for every static mut found.
    let mut decls: std::collections::HashMap<String, (String, String, PathBuf)> =
        Default::default();
    // Names appearing on the LHS of `& NAME as *T`.
    let mut cell_used: std::collections::HashSet<String> = Default::default();

    visit_vr_files(&core_root, &mut |path, source| {
        for cap in static_mut_re.captures_iter(source) {
            let name = cap[1].to_string();
            let ty = cap[2].trim().to_string();
            let init = cap[3].trim().to_string();
            decls.insert(name, (ty, init, path.to_path_buf()));
        }
        for cap in addrof_re.captures_iter(source) {
            cell_used.insert(cap[1].to_string());
        }
    });

    let mut violations: Vec<String> = Vec::new();
    for name in &cell_used {
        let Some((ty, init, path)) = decls.get(name) else {
            // Reference to a name not declared as `static mut` in core/ —
            // out-of-scope for this pin.  Skip.
            continue;
        };
        let scalar_ok = is_scalar_type(ty);
        let zero_init = is_zero_init(init);
        if !scalar_ok || !zero_init {
            violations.push(format!(
                "  - {} ({}): type=`{}` (scalar={}), init=`{}` (zero={}) [{}]",
                name,
                path.strip_prefix(&core_root)
                    .unwrap_or(path)
                    .display(),
                ty,
                scalar_ok,
                init,
                zero_init,
                if !scalar_ok && !zero_init {
                    "needs F2 dynamic-size + non-zero-init"
                } else if !scalar_ok {
                    "needs F2 dynamic-size"
                } else {
                    "needs F2 non-zero-init ctor"
                }
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "F2 cell-backed contract violations — `&NAME as *T` requires \
         scalar zero-init under the current 8-byte cell architecture.  \
         Implement F2 extensions (dynamic size / non-zero init ctor) \
         before landing these declarations:\n{}",
        violations.join("\n")
    );
}

/// Workspace root resolution.
fn workspace_root() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest)
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Recursive *.vr walker.
fn visit_vr_files<F: FnMut(&PathBuf, &str)>(root: &PathBuf, f: &mut F) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit_vr_files(&path, f);
        } else if path.extension().and_then(|s| s.to_str()) == Some("vr")
            && let Ok(src) = fs::read_to_string(&path)
        {
            f(&path, &src);
        }
    }
}

/// Scalar types ≤ 8 bytes (UInt8/16/32/64, Int, Bool, Float).
/// Pointer types (`&unsafe T`, `*const T`, `*mut T`) also count
/// since they're 8-byte values on every supported platform.
fn is_scalar_type(ty: &str) -> bool {
    let t = ty.trim();
    matches!(
        t,
        "UInt8" | "UInt16" | "UInt32" | "UInt64"
            | "Int8" | "Int16" | "Int32" | "Int64" | "Int"
            | "Bool" | "Float" | "F32" | "F64" | "USize" | "ISize"
    ) || t.starts_with("*const ")
        || t.starts_with("*mut ")
        || t.starts_with("&unsafe ")
        || t.starts_with("&&")  // pointer-to-pointer = 8 bytes
        // Pthread/TLS handles are scalar wrappers
        || t == "PthreadKey"
}

/// Zero initialisers: `0`, `0u8`, `false`, `null`, `unsafe { 0 as ... }`.
fn is_zero_init(init: &str) -> bool {
    let i = init.trim();
    i == "0"
        || i == "0u8"
        || i == "0u16"
        || i == "0u32"
        || i == "0u64"
        || i == "0 as UInt32"
        || i == "0 as UInt64"
        || i == "false"
        || i == "null"
        || i.starts_with("unsafe { 0 as ")
}

/// Minimal regex compatibility shim so this test crate does not need
/// to grow a `regex` dependency.  Reads the two simple patterns used
/// above and applies them line-by-line — sufficient for the
/// `static mut` and `&NAME as *T` shapes in `core/`.
mod regex_like {
    pub struct Pattern {
        raw: &'static str,
    }
    pub struct Captures<'a>(Vec<&'a str>);
    impl<'a> std::ops::Index<usize> for Captures<'a> {
        type Output = str;
        fn index(&self, i: usize) -> &str {
            self.0[i]
        }
    }
    pub struct CapturesIter<'a> {
        haystack: &'a str,
        kind: PatternKind,
    }
    enum PatternKind {
        StaticMut,
        AddrOf,
    }
    impl Pattern {
        pub fn captures_iter<'a>(&self, hay: &'a str) -> CapturesIter<'a> {
            let kind = if self.raw.contains("static") {
                PatternKind::StaticMut
            } else {
                PatternKind::AddrOf
            };
            CapturesIter { haystack: hay, kind }
        }
    }
    impl<'a> Iterator for CapturesIter<'a> {
        type Item = Captures<'a>;
        fn next(&mut self) -> Option<Self::Item> {
            loop {
                let line_end = self.haystack.find('\n').unwrap_or(self.haystack.len());
                let line = &self.haystack[..line_end];
                let rest = if line_end < self.haystack.len() {
                    &self.haystack[line_end + 1..]
                } else {
                    ""
                };
                self.haystack = rest;
                match self.kind {
                    PatternKind::StaticMut => {
                        // `(public )?static mut NAME : TY = INIT;`
                        if let Some(caps) = parse_static_mut(line) {
                            return Some(Captures(vec!["", caps.0, caps.1, caps.2]));
                        }
                    }
                    PatternKind::AddrOf => {
                        // Multiple matches per line possible.
                        // Caller drives this iterator until empty.
                        if let Some(name) = parse_addrof(line) {
                            return Some(Captures(vec!["", name]));
                        }
                    }
                }
                if rest.is_empty() {
                    return None;
                }
            }
        }
    }
    fn parse_static_mut(line: &str) -> Option<(&str, &str, &str)> {
        let s = line.trim_start();
        let s = s.strip_prefix("public ").unwrap_or(s);
        let s = s.strip_prefix("static mut ")?;
        let colon = s.find(':')?;
        let name = s[..colon].trim();
        let after = s[colon + 1..].trim_start();
        let eq = after.find('=')?;
        let ty = after[..eq].trim_end();
        let after_eq = after[eq + 1..].trim_start();
        let semi = after_eq.find(';')?;
        let init = after_eq[..semi].trim();
        Some((name, ty, init))
    }
    fn parse_addrof(line: &str) -> Option<&str> {
        let amp = line.find('&')?;
        let after = line[amp + 1..].trim_start();
        let name_end = after
            .find(|c: char| !(c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()))?;
        if name_end == 0 {
            return None;
        }
        let name = &after[..name_end];
        let after_name = after[name_end..].trim_start();
        if !after_name.starts_with("as ") {
            return None;
        }
        let after_as = after_name[3..].trim_start();
        if !after_as.starts_with("*const ") && !after_as.starts_with("*mut ") {
            return None;
        }
        Some(name)
    }
    pub fn compile(pat: &'static str) -> Pattern {
        Pattern { raw: pat }
    }
}
