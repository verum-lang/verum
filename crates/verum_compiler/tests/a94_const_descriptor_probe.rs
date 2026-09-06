//! Throwaway probe (A94): what does a PRIVATE constant's descriptor
//! actually carry — name, module_path, origin_module_path — as
//! `archive_to_core_metadata` produces it?
//!
//! The marking pass keys on `(declaring module, name)`, and a first
//! attempt keyed on `module_path` matched nothing because that is the
//! archive ENTRY path. Keying on `origin_module_path` fixed TYPES and
//! left CONSTANTS untouched, so one of the two halves of the key is
//! still not what this probe will print.

use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_vbc::archive::read_archive_from_file;

#[test]
#[ignore]
fn dump_private_const_descriptors() {
    let path = std::env::var("VBCA").expect("set VBCA=/path/to/runtime.vbca");
    let archive = read_archive_from_file(&path).expect("archive readable");
    let meta = archive_to_core_metadata(&archive);

    let witnesses = ["K", "H_INIT", "LF", "RCODE_REFUSED", "CLOCK_TAI"];
    for (key, fd) in meta.functions.iter() {
        let k = key.as_str();
        let hit = witnesses.iter().any(|w| k == *w || k.ends_with(&format!(".{w}")));
        if !hit {
            continue;
        }
        println!(
            "key=`{}` name=`{}` module_path=`{}` origin=`{:?}` is_const={} is_public={}",
            k,
            fd.name.as_str(),
            fd.module_path.as_str(),
            fd.origin_module_path,
            fd.is_const,
            fd.is_public,
        );
    }
}
