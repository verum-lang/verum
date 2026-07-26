//! Guards the List slot-2 offset against re-hardcoding.
//!
//! `LIST_PTR_OFFSET` (verum_common::layout) is the single authority for where
//! a List's backing pointer lives. Tier-1 codegen names it 28 times; the
//! interpreter historically did not name it at all, reaching the same address
//! by hand.
//!
//! Three spellings existed, and only one was actually dangerous:
//!
//!   * `OBJECT_HEADER_SIZE + 16`  — hardcodes `size_of::<Value>() == 8`. If
//!     Value ever changes width this silently addresses the wrong slot while
//!     every other form adapts. Converted to `heap::LIST_PTR_OFFSET`.
//!   * `OBJECT_HEADER_SIZE + 2 * size_of::<Value>()` — width-safe.
//!   * `data_ptr.add(2)` on a `*const Value` — width-safe (scales by element).
//!
//! This pin covers the dangerous one. It deliberately does NOT forbid the
//! width-safe spellings: they are invisible to a layout realign (T0293) and so
//! still want converting eventually, but they are not defects, and a blanket
//! ban would fire on every third-element access of any Value array — the same
//! over-match that nearly saw Stopwatch's `accumulated` field converted to a
//! List constant.

use std::path::{Path, PathBuf};

fn interpreter_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("interpreter")
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn no_hardcoded_list_slot2_byte_offset_in_interpreter() {
    let mut files = Vec::new();
    rs_files(&interpreter_dir(), &mut files);
    assert!(!files.is_empty(), "found no interpreter sources to scan");

    let mut offenders: Vec<String> = Vec::new();
    for f in &files {
        let Ok(src) = std::fs::read_to_string(f) else { continue };
        for (i, line) in src.lines().enumerate() {
            // The literal form only. `+ 2 * size_of::<Value>()` is width-safe
            // and deliberately not matched.
            // Skip comments: the heap.rs doc block deliberately QUOTES the old
            // spelling to explain why it was replaced, and a scan that counts
            // prose as code flags its own documentation.
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            if line.contains("OBJECT_HEADER_SIZE + 16") {
                offenders.push(format!("{}:{}", f.display(), i + 1));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "hardcoded byte offset for a heap object's slot 2 reintroduced at:\n  {}\n\n\
         If this addresses a List backing pointer, use `heap::LIST_PTR_OFFSET` \
         so a layout realign reaches it (T0293).\n\
         If it addresses the third field of some OTHER record type, do NOT use \
         the List constant — derive from size_of::<Value>() instead, or the \
         next List realign will silently move your field too.",
        offenders.join("\n  ")
    );
}

#[test]
fn list_ptr_offset_is_reachable_from_the_interpreter_heap_module() {
    // The conversion above depends on the re-export existing. If someone
    // removes it, the sites would have to go back to hand arithmetic.
    let heap = std::fs::read_to_string(interpreter_dir().join("heap.rs"))
        .expect("interpreter/heap.rs is readable");
    assert!(
        heap.contains("pub const LIST_PTR_OFFSET"),
        "interpreter::heap no longer re-exports LIST_PTR_OFFSET; the List \
         slot-2 sites depend on it (T0293)"
    );
}
