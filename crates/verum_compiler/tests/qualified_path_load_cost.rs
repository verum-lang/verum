//! T0870 — reaching a name through its full path must not cost more
//! than reaching it through a `mount`.
//!
//! Both spellings name the same function and produce the same program:
//!
//! ```verum
//! mount core.time.Duration;   fn main() { Duration.from_secs(30); }
//! fn main() { core.time.Duration.from_secs(30); }
//! ```
//!
//! Measured before the fix, on that one-line program: the mounted
//! spelling loaded **11** archive modules (9,886 functions) and ran in
//! under a second; the qualified spelling loaded **582** modules
//! (48,815 functions) and took **6 minutes 19 seconds**. The docs teach
//! the qualified form as the way to use a name once without importing
//! it, so a documented idiom read as a hang.
//!
//! The cost is asserted as a MODULE COUNT rather than a wall-clock
//! time. A timing assertion on a shared machine measures the
//! neighbours; the module count is the thing that actually differs, and
//! it is stable.

use verum_fast_parser::Parser;

/// Load the embedded archive against a source module and report how many
/// archive modules the lazy loader pulled in.
fn modules_loaded(code: &str) -> usize {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse should succeed");

    let mut codegen = verum_vbc::codegen::VbcCodegen::new();
    codegen.register_builtin_variants();

    let archive = verum_compiler::embedded_stdlib_vbc::get_runtime_archive()
        .expect("the embedded runtime archive must be available for this gate");

    let cache = verum_compiler::archive_ctx_loader::ArchiveCtxCache::new();
    let (fn_modules, _type_modules) = cache.apply_lazy_with_types(archive, &mut codegen, &module);
    fn_modules
}

const MOUNTED: &str = r#"
mount core.time.Duration;

fn main() {
    let d = Duration.from_secs(30);
    print("x");
}
"#;

const QUALIFIED: &str = r#"
fn main() {
    let d = core.time.Duration.from_secs(30);
    print("x");
}
"#;

/// The control. If the mounted spelling ever stops being frugal, the
/// ratio below would pass for the wrong reason.
#[test]
fn the_mounted_spelling_loads_a_handful_of_modules() {
    let mounted = modules_loaded(MOUNTED);
    assert!(
        mounted > 0,
        "the mounted spelling must load SOMETHING — a zero here means \
         the harness never reached the archive"
    );
    assert!(
        mounted <= 60,
        "the mounted spelling loaded {mounted} modules; it used to load 11, \
         and this gate compares against it"
    );
}

#[test]
fn a_qualified_path_costs_the_same_order_as_a_mount() {
    let mounted = modules_loaded(MOUNTED);
    let qualified = modules_loaded(QUALIFIED);

    // Deliberately generous: the point is to catch a cliff, not to pin
    // an exact number that legitimate stdlib growth would break.
    assert!(
        qualified <= mounted * 3,
        "reaching a name through its full path loaded {qualified} archive \
         modules where the mounted spelling loaded {mounted}. Both name the \
         same function. The gap was 582 vs 11 when this was first measured, \
         and it read as a hang."
    );
}
