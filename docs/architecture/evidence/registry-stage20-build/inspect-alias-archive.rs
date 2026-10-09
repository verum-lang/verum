//! Read-only audit using the production archive and module decoders.
use verum_vbc::archive::read_archive_from_file;
use verum_vbc::types::TypeRef;

fn main() {
    let path = std::env::args().nth(1).expect("archive path");
    let archive = read_archive_from_file(path).expect("read archive");
    for entry in &archive.index {
        let module = archive.load_module(&entry.name).expect("decode archive module");
        for ty in &module.types {
            if module.get_string(ty.name) != Some("IoResult") { continue; }
            let head = match ty.alias_target.as_ref() {
                Some(TypeRef::Concrete(id)) => Some(*id),
                Some(TypeRef::Instantiated { base, .. }) => Some(*base),
                _ => None,
            };
            let declaration = head.and_then(|id| module.get_type(id));
            println!("entry={} alias_id={} owner={:?} target={:?} carried={:?} head_name={:?} head_owner={:?}",
                entry.name, ty.id.0, ty.origin_module.and_then(|id| module.get_string(id)),
                ty.alias_target, ty.alias_target_name.and_then(|id| module.get_string(id)),
                declaration.and_then(|d| module.get_string(d.name)),
                declaration.and_then(|d| d.origin_module).and_then(|id| module.get_string(id)));
        }
    }
}
