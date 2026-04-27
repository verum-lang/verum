//! Script-mode dispatch for `verum`.
//!
//! Verum can be invoked in two distinct modes:
//!
//! 1. **Project mode** — `verum run`, `verum build`, etc., dispatched through
//!    clap subcommands and operating on a `verum.toml`-rooted project tree.
//! 2. **Script mode** — `verum path/to/file.vr [args…]` or, via shebang,
//!    `./file.vr [args…]`. The `.vr` file is parsed, compiled, and executed
//!    directly. No `verum.toml` is required.
//!
//! Script mode is signalled by a `#!` line at byte 0 of the source file (the
//! POSIX-shebang convention). [`verum_lexer::strip_shebang`] strips the line
//! before tokenisation; this module is responsible only for deciding whether
//! `verum`'s argv looks like a script invocation, and if so, rewriting argv so
//! that downstream clap parsing sees `verum run <file> <args…>`.
//!
//! Why argv rewriting and not a clap fallback? Two reasons:
//!
//! - Shebang invocations (`./file.vr`) hit the kernel as
//!   `verum file.vr [args…]`. Clap would reject `file.vr` as an unknown
//!   subcommand. A pre-clap rewrite is the simplest, most robust fix.
//! - The rewrite is safe: `run` is an existing subcommand whose arity already
//!   accepts `<file> [args…]`. We are not inventing a new dispatch path.
//!
//! # Invariants
//!
//! - The rewrite ONLY fires when argv[1] points at an existing file with a
//!   `.vr` extension OR a `#!` shebang. Existing subcommands win
//!   unconditionally; if `argv[1]` matches a known clap subcommand name, no
//!   rewrite happens.
//! - If `argv[1]` is a flag (starts with `-`), no rewrite happens.
//! - If the file does not exist, no rewrite happens — clap's normal "unknown
//!   subcommand" error reaches the user.

pub mod frontmatter;

use std::ffi::OsString;
use std::path::Path;

/// Subcommand names registered by `verum`. If `argv[1]` matches any of these,
/// we never rewrite — the user is invoking the subcommand directly.
///
/// Maintained in sync with `enum Commands` in `main.rs`. Adding a subcommand
/// requires adding its CLI name here as well; the rewrite is conservative —
/// missing entries cause script mode to (incorrectly) fire on a real
/// subcommand path. A test below catches the drift.
const KNOWN_SUBCOMMANDS: &[&str] = &[
    "add",
    "analyze",
    "audit",
    "bench",
    "build",
    "cache",
    "check",
    "clean",
    "config",
    "dap",
    "deps",
    "diagnose",
    "doc",
    "doctor",
    "explain",
    "export",
    "file",
    "fmt",
    "help",
    "hooks",
    "init",
    "lex_mask",
    "lint",
    "lint_baseline",
    "lint_cache",
    "lint_engine",
    "lint_human",
    "lsp",
    "mod",
    "new",
    "owl2",
    "playbook",
    "profile",
    "property",
    "publish",
    "remove",
    "repl",
    "run",
    "search",
    "smt_check",
    "smt_info",
    "smt_stats",
    "test",
    "toolchain",
    "tree",
    "update",
    "verify",
    "version",
    "watch",
    "workspace",
];

/// Decide whether the supplied argv looks like a script invocation, and if so,
/// rewrite it to `verum run <file> -- <args…>`. Returns the (possibly
/// unchanged) argv to feed into clap.
///
/// `argv` must include `argv[0]` (the binary path), as is conventional.
///
/// # Arg-passing semantics
///
/// In a shebang invocation (`./hello.vr foo bar`), the OS hands `verum` the
/// argv `["verum", "hello.vr", "foo", "bar"]`. The user expects `foo` and
/// `bar` to reach the script, not to act as `verum`-level flags. We therefore
/// insert `--` after the script path so `foo bar` are unambiguously parsed as
/// trailing script arguments. Users who want `verum`-level flags can still
/// use the explicit `verum run [flags] hello.vr [-- args]` form.
pub fn rewrite_argv_for_script_mode(argv: Vec<OsString>) -> Vec<OsString> {
    if argv.len() < 2 {
        return argv;
    }
    if !looks_like_script_invocation(&argv[1]) {
        return argv;
    }
    // Build: [argv[0], "run", argv[1], "--", argv[2..]…]
    let mut rewritten = Vec::with_capacity(argv.len() + 2);
    let mut iter = argv.into_iter();
    rewritten.push(iter.next().unwrap()); // argv[0]
    rewritten.push(OsString::from("run"));
    rewritten.push(iter.next().unwrap()); // argv[1] = script path
    // Only insert `--` if there are remaining args AND they don't already
    // start with `--`. Avoids `verum hello.vr -- foo` becoming
    // `verum run hello.vr -- -- foo` (still parses but ugly).
    let rest: Vec<_> = iter.collect();
    let has_explicit_separator = rest.first().map(|s| s == "--").unwrap_or(false);
    if !rest.is_empty() && !has_explicit_separator {
        rewritten.push(OsString::from("--"));
    }
    rewritten.extend(rest);
    rewritten
}

/// True iff `arg` should trigger a script-mode rewrite. The check is
/// deliberately strict: false positives would shadow legitimate subcommands.
///
/// Conditions, AND-joined:
/// - Not a flag (does not start with `-`).
/// - Not a known subcommand name.
/// - Names an existing file (regular file, accessible).
/// - File has `.vr` extension OR begins with a `#!` shebang.
fn looks_like_script_invocation(arg: &OsString) -> bool {
    let s = match arg.to_str() {
        Some(s) => s,
        None => return false,
    };
    if s.starts_with('-') {
        return false;
    }
    if KNOWN_SUBCOMMANDS.binary_search(&s).is_ok() {
        return false;
    }
    let path = Path::new(s);
    if !path.is_file() {
        return false;
    }
    let has_vr_ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("vr"))
        .unwrap_or(false);
    if has_vr_ext {
        return true;
    }
    // Even without a `.vr` extension, an executable file with a `#!verum`-ish
    // shebang counts. We read at most 2 bytes — the shebang signature.
    has_shebang(path)
}

/// Returns true iff the file's first two bytes are `#!`.
fn has_shebang(path: &Path) -> bool {
    use std::fs::File;
    use std::io::Read;
    match File::open(path) {
        Ok(mut f) => {
            let mut buf = [0u8; 2];
            match f.read(&mut buf) {
                Ok(2) => &buf == b"#!",
                _ => false,
            }
        }
        Err(_) => false,
    }
}

/// Returns true iff `argv[1]` (when present) is a script invocation — i.e. the
/// rewrite would fire. Useful for diagnostics that want to know what mode is
/// active without consuming argv.
pub fn is_script_invocation(argv: &[OsString]) -> bool {
    argv.len() >= 2 && looks_like_script_invocation(&argv[1])
}

/// Validation invariant for `KNOWN_SUBCOMMANDS`: the array must be sorted.
/// `binary_search` above relies on it.
#[cfg(test)]
fn known_subcommands_sorted() -> bool {
    KNOWN_SUBCOMMANDS.windows(2).all(|w| w[0] < w[1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn argv(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    fn write_temp(name: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "verum_script_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn known_subcommands_array_is_sorted() {
        assert!(
            known_subcommands_sorted(),
            "KNOWN_SUBCOMMANDS must be sorted alphabetically for binary_search"
        );
    }

    #[test]
    fn no_rewrite_when_no_args() {
        let a = argv(&["verum"]);
        assert_eq!(rewrite_argv_for_script_mode(a.clone()), a);
    }

    #[test]
    fn no_rewrite_for_known_subcommand() {
        let a = argv(&["verum", "build"]);
        assert_eq!(rewrite_argv_for_script_mode(a.clone()), a);
    }

    #[test]
    fn no_rewrite_for_known_subcommand_with_args() {
        let a = argv(&["verum", "test", "--release"]);
        assert_eq!(rewrite_argv_for_script_mode(a.clone()), a);
    }

    #[test]
    fn no_rewrite_for_flag() {
        let a = argv(&["verum", "--version"]);
        assert_eq!(rewrite_argv_for_script_mode(a.clone()), a);
    }

    #[test]
    fn no_rewrite_for_short_flag() {
        let a = argv(&["verum", "-h"]);
        assert_eq!(rewrite_argv_for_script_mode(a.clone()), a);
    }

    #[test]
    fn no_rewrite_for_nonexistent_file() {
        let a = argv(&["verum", "/no/such/path/script.vr"]);
        assert_eq!(rewrite_argv_for_script_mode(a.clone()), a);
    }

    #[test]
    fn rewrite_for_vr_extension_file() {
        let p = write_temp("script.vr", "fn main() {}\n");
        let a = argv(&["verum", p.to_str().unwrap()]);
        let r = rewrite_argv_for_script_mode(a.clone());
        assert_eq!(r.len(), 3);
        assert_eq!(r[0], OsString::from("verum"));
        assert_eq!(r[1], OsString::from("run"));
        assert_eq!(r[2], a[1]);
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn rewrite_for_vr_extension_with_explicit_separator() {
        // The user already wrote `--`; we must not double it.
        let p = write_temp("greet.vr", "fn main() {}\n");
        let a = argv(&["verum", p.to_str().unwrap(), "--", "alice", "bob"]);
        let r = rewrite_argv_for_script_mode(a);
        assert_eq!(r.len(), 6);
        assert_eq!(r[1], OsString::from("run"));
        assert_eq!(r[3], OsString::from("--"));
        assert_eq!(r[4], OsString::from("alice"));
        assert_eq!(r[5], OsString::from("bob"));
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn rewrite_inserts_separator_for_trailing_script_args() {
        // The shebang case: `./hello.vr foo bar` arrives as
        // ["verum", "hello.vr", "foo", "bar"]; we must rewrite so `foo bar`
        // are unambiguous trailing-args, not parsed as verum flags.
        let p = write_temp("greet3.vr", "fn main() {}\n");
        let a = argv(&["verum", p.to_str().unwrap(), "foo", "bar"]);
        let r = rewrite_argv_for_script_mode(a);
        assert_eq!(r.len(), 6);
        assert_eq!(r[1], OsString::from("run"));
        assert_eq!(r[3], OsString::from("--"));
        assert_eq!(r[4], OsString::from("foo"));
        assert_eq!(r[5], OsString::from("bar"));
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn rewrite_no_separator_if_no_script_args() {
        // Plain `verum hello.vr` — no trailing args, no `--` needed.
        let p = write_temp("greet4.vr", "fn main() {}\n");
        let a = argv(&["verum", p.to_str().unwrap()]);
        let r = rewrite_argv_for_script_mode(a);
        assert_eq!(r.len(), 3);
        assert_eq!(r[1], OsString::from("run"));
        assert_ne!(r.last().unwrap(), &OsString::from("--"));
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn rewrite_for_shebang_no_extension() {
        use std::os::unix::fs::PermissionsExt;
        let p = write_temp("greet", "#!/usr/bin/env verum\nfn main() {}\n");
        // Make it executable (simulating the chmod +x case).
        let mut perms = fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&p, perms).unwrap();
        let a = argv(&["verum", p.to_str().unwrap()]);
        let r = rewrite_argv_for_script_mode(a);
        assert_eq!(r.len(), 3);
        assert_eq!(r[1], OsString::from("run"));
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn rewrite_for_shebang_file_no_extension_any_platform() {
        // Same as above but without chmod — the rewrite logic only inspects
        // file content, not permission bits, so this works on every platform.
        let p = write_temp("greet2", "#!/usr/bin/env verum\nfn main() {}\n");
        let a = argv(&["verum", p.to_str().unwrap()]);
        let r = rewrite_argv_for_script_mode(a);
        assert_eq!(r.len(), 3);
        assert_eq!(r[1], OsString::from("run"));
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn no_rewrite_for_non_vr_non_shebang_file() {
        let p = write_temp("notes.txt", "just some text\n");
        let a = argv(&["verum", p.to_str().unwrap()]);
        assert_eq!(rewrite_argv_for_script_mode(a.clone()), a);
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn is_script_invocation_matches_rewrite() {
        let p = write_temp("s.vr", "fn main() {}\n");
        let a = argv(&["verum", p.to_str().unwrap()]);
        assert!(is_script_invocation(&a));
        let b = argv(&["verum", "build"]);
        assert!(!is_script_invocation(&b));
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }
}
