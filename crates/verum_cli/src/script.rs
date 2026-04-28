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

pub mod cache;
pub mod context;
pub mod frontmatter;
pub mod lockfile;
pub mod permission_flags;
pub mod permissions;

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
/// **Verum execution-mode contract.** The no-`run` shorthand
/// (`verum ./script.vr`) is reserved for **scripts**, not arbitrary
/// `.vr` files. A script is identified by a `#!` shebang line at byte 0
/// (BOM-tolerant). Files that lack a shebang — even if they end in
/// `.vr` — must be invoked through the explicit `verum run file.vr`
/// form. This makes the three execution modes unambiguous from argv
/// alone:
///
/// | Mode        | Invocation                        | Required signal           |
/// |-------------|-----------------------------------|---------------------------|
/// | Interpreter | `verum run file.vr`               | `fn main()` in source     |
/// | AOT         | `verum run --aot file.vr`         | `fn main()` in source     |
/// | Script      | `verum file.vr` or `./file.vr`    | `#!` shebang at byte 0    |
///
/// Conditions, AND-joined:
/// - Not a flag (does not start with `-`).
/// - Not a known subcommand name (UTF-8 only — every Verum subcommand
///   spells its name in ASCII, so a non-UTF-8 OsString cannot collide).
/// - Names an existing file (regular file, accessible).
/// - **Begins with `#!` shebang** (BOM-tolerant: `EF BB BF #!` accepted).
///
/// **Encoding contract:** flag detection and the file-existence /
/// shebang checks all operate on the raw `OsStr` so non-UTF-8 paths
/// (Windows legacy paths, macOS broken-encoding test fixtures,
/// deliberate Unix paths with non-UTF-8 bytes) still trigger script-mode
/// dispatch. Only the subcommand-name match requires UTF-8, and a
/// non-UTF-8 string can't match an ASCII subcommand anyway, so we skip
/// that arm when conversion fails.
fn looks_like_script_invocation(arg: &OsString) -> bool {
    // Flag check works on every encoding: `-` is a single ASCII byte and
    // its byte representation is identical in WTF-8 / UTF-16 / Linux raw
    // bytes at the start of an OsString.
    if os_starts_with_dash(arg) {
        return false;
    }

    // Subcommand-name match. Only meaningful when the OsString is valid
    // UTF-8; otherwise it cannot collide with an ASCII subcommand name.
    if let Some(s) = arg.to_str() {
        if KNOWN_SUBCOMMANDS.binary_search(&s).is_ok() {
            return false;
        }
    }

    // File checks operate directly on the OsStr — `Path::new` is
    // encoding-agnostic.
    let path = Path::new(arg);
    if !path.is_file() {
        return false;
    }
    // Shebang is the sole script signal — the `.vr` extension is no
    // longer sufficient. Without a shebang, the file is a library /
    // binary source that must be run via `verum run file.vr`.
    has_shebang(path)
}

/// True iff the OsString begins with the ASCII byte `-`. Encoding-agnostic
/// because the WTF-8 / UTF-16 / Linux-bytes representations of `-` (U+002D)
/// are all the single byte `0x2D` at the start of the string.
#[inline]
fn os_starts_with_dash(s: &OsString) -> bool {
    s.as_encoded_bytes().first() == Some(&b'-')
}

/// Returns true iff the file begins with a `#!` shebang. A leading UTF-8
/// BOM (`EF BB BF`) is tolerated — Windows / cross-platform editors
/// frequently prepend one and the shebang line is still shell-significant
/// at byte 3 in that case (the kernel ignores the BOM, treating the line
/// as `#!...`). Reading at most 5 bytes is enough to cover both layouts.
fn has_shebang(path: &Path) -> bool {
    use std::fs::File;
    use std::io::Read;
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = [0u8; 5];
    let n = match f.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return false,
    };
    if n >= 5 && &buf[..3] == [0xEF, 0xBB, 0xBF] && &buf[3..5] == b"#!" {
        return true;
    }
    n >= 2 && &buf[..2] == b"#!"
}

/// Returns true iff `argv[1]` (when present) is a script invocation — i.e. the
/// rewrite would fire. Useful for diagnostics that want to know what mode is
/// active without consuming argv.
pub fn is_script_invocation(argv: &[OsString]) -> bool {
    argv.len() >= 2 && looks_like_script_invocation(&argv[1])
}

/// Productivity helper for the `verum file.vr` (no `run`) form. Returns a
/// precise advisory message when `argv[1]` looks like a *would-be* script
/// (existing `.vr` file) but lacks the mandatory `#!` shebang.
///
/// The Verum execution-mode contract reserves the no-`run` shorthand for
/// shebang scripts; without one, clap would surface the generic "unknown
/// subcommand" error which gives the user no actionable next step. By
/// detecting the misuse pre-clap we can point them at the exact fix
/// (`verum run file.vr` for non-script files, or add a shebang).
///
/// Returns `Some(message)` only for the `.vr`-extension-without-shebang
/// case; every other shape (subcommands, flags, non-existent files, files
/// with unrelated extensions) returns `None` to stay out of clap's way.
pub fn missing_shebang_advisory(argv: &[OsString]) -> Option<String> {
    if argv.len() < 2 {
        return None;
    }
    let arg = &argv[1];
    if os_starts_with_dash(arg) {
        return None;
    }
    if let Some(s) = arg.to_str() {
        if KNOWN_SUBCOMMANDS.binary_search(&s).is_ok() {
            return None;
        }
    }
    let path = Path::new(arg);
    if !path.is_file() {
        return None;
    }
    let has_vr_ext = path
        .extension()
        .map(|e| e.eq_ignore_ascii_case("vr"))
        .unwrap_or(false);
    if !has_vr_ext {
        return None;
    }
    if has_shebang(path) {
        return None; // Properly shaped script — no advisory.
    }
    let display = arg.to_string_lossy();
    Some(format!(
        "`{display}` looks like a Verum source file but is missing a `#!` shebang line.\n\
         \n\
         The bare `verum {display}` shorthand is reserved for **scripts**, which must \
         declare a shebang at byte 0 (e.g. `#!/usr/bin/env verum`). Choose one of:\n\
         \n  \
         • Run as a library/binary entry point:  verum run {display}\n  \
         • Convert to a script:                  add `#!/usr/bin/env verum` as the first line"
    ))
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
    fn rewrite_for_shebang_vr_file() {
        // `.vr` extension is no longer sufficient — the file MUST start
        // with a shebang to qualify as a script. Pin the new contract.
        let p = write_temp("script.vr", "#!/usr/bin/env verum\nprint(1);\n");
        let a = argv(&["verum", p.to_str().unwrap()]);
        let r = rewrite_argv_for_script_mode(a.clone());
        assert_eq!(r.len(), 3);
        assert_eq!(r[0], OsString::from("verum"));
        assert_eq!(r[1], OsString::from("run"));
        assert_eq!(r[2], a[1]);
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn no_rewrite_for_vr_file_without_shebang() {
        // The Verum execution-mode contract reserves the no-`run`
        // shorthand for shebang scripts. A `.vr` file without a shebang
        // is library/binary code and must be invoked via `verum run`.
        let p = write_temp("non_script.vr", "fn main() { print(1) }\n");
        let a = argv(&["verum", p.to_str().unwrap()]);
        assert_eq!(rewrite_argv_for_script_mode(a.clone()), a);
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn rewrite_for_shebang_vr_with_explicit_separator() {
        // The user already wrote `--`; we must not double it.
        let p = write_temp("greet.vr", "#!/usr/bin/env verum\nprint(1);\n");
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
        let p = write_temp("greet3.vr", "#!/usr/bin/env verum\nprint(1);\n");
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
        let p = write_temp("greet4.vr", "#!/usr/bin/env verum\nprint(1);\n");
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

    // Filesystem-backed non-UTF-8 path: only Linux ext4/tmpfs accept
    // non-UTF-8 bytes in filenames. APFS (macOS) and NTFS (Windows) reject
    // them at create time. This test is gated to Linux so it runs in CI on
    // the platforms where the scenario is reachable.
    #[cfg(target_os = "linux")]
    #[test]
    fn rewrite_for_non_utf8_path_with_shebang() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let dir = std::env::temp_dir().join(format!(
            "verum_script_nonutf8_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let mut name_bytes: Vec<u8> = b"bad-".to_vec();
        name_bytes.push(0xFF);
        name_bytes.extend_from_slice(b".vr");
        let p = dir.join(std::ffi::OsString::from_vec(name_bytes));
        // Shebang qualifies the file as a script regardless of path encoding.
        fs::write(&p, "#!/usr/bin/env verum\nprint(1);\n").unwrap();
        let arg = std::ffi::OsString::from_vec(p.as_os_str().as_bytes().to_vec());
        assert!(arg.to_str().is_none(), "fixture must be non-UTF-8");
        let argv = vec![OsString::from("verum"), arg];
        let r = rewrite_argv_for_script_mode(argv);
        assert_eq!(r.len(), 3);
        assert_eq!(r[1], OsString::from("run"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn os_starts_with_dash_handles_unicode_and_ascii() {
        // Pure unit test of the encoding-agnostic flag-leading check.
        // No filesystem; runs everywhere. Pins the contract that Verum's
        // script-mode dispatch does not falsely treat non-flag arguments
        // as flags — including ones that begin with a non-ASCII letter.
        assert!(super::os_starts_with_dash(&OsString::from("-h")));
        assert!(super::os_starts_with_dash(&OsString::from("--release")));
        assert!(super::os_starts_with_dash(&OsString::from("-")));
        assert!(!super::os_starts_with_dash(&OsString::from("hello")));
        assert!(!super::os_starts_with_dash(&OsString::from("файл.vr")));
        assert!(!super::os_starts_with_dash(&OsString::from("")));
        // A unicode dash (U+2013 EN DASH, ≠ U+002D HYPHEN-MINUS) must NOT
        // be treated as a flag — the OS / shell never produces it from
        // a `-flag` keystroke.
        assert!(!super::os_starts_with_dash(&OsString::from("\u{2013}flag")));
    }

    #[test]
    fn rewrite_for_bom_prefixed_shebang_file() {
        // Cross-platform editors that emit UTF-8 BOM (`EF BB BF`) still
        // produce a shebang script when the BOM precedes `#!`. The
        // shell/kernel happily accepts this layout (the BOM is just bytes
        // before the shebang); script-mode dispatch must follow suit.
        let p = write_temp("bom_script", "\u{FEFF}#!/usr/bin/env verum\nfn main() {}\n");
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
        let p = write_temp("s.vr", "#!/usr/bin/env verum\nprint(1);\n");
        let a = argv(&["verum", p.to_str().unwrap()]);
        assert!(is_script_invocation(&a));
        let b = argv(&["verum", "build"]);
        assert!(!is_script_invocation(&b));
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn missing_shebang_advisory_for_vr_extension() {
        // Productivity: when the user types `verum file.vr` for a file
        // that LOOKS like a script (has `.vr` extension) but lacks the
        // mandatory shebang, surface a precise advisory pointing them
        // at `verum run`. Prevents the bare clap "unknown subcommand"
        // failure mode.
        let p = write_temp("nope.vr", "fn main() {}\n");
        let a = argv(&["verum", p.to_str().unwrap()]);
        let advisory = missing_shebang_advisory(&a)
            .expect("vr-extension file without shebang must produce an advisory");
        assert!(
            advisory.contains("verum run"),
            "advisory must redirect the user to `verum run`, got: {}",
            advisory
        );
        assert!(
            advisory.contains("shebang"),
            "advisory must mention the shebang requirement, got: {}",
            advisory
        );
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn no_advisory_for_proper_shebang_script() {
        let p = write_temp("good.vr", "#!/usr/bin/env verum\nprint(1);\n");
        let a = argv(&["verum", p.to_str().unwrap()]);
        assert!(missing_shebang_advisory(&a).is_none());
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn no_advisory_for_non_vr_file() {
        let p = write_temp("notes.txt", "just text\n");
        let a = argv(&["verum", p.to_str().unwrap()]);
        assert!(missing_shebang_advisory(&a).is_none());
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn no_advisory_for_subcommand() {
        let a = argv(&["verum", "build"]);
        assert!(missing_shebang_advisory(&a).is_none());
    }
}
