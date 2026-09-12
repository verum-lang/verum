//! **T1461 — a mount alias carries the canonical name, and the lookup
//! must hand it back.**
//!
//! `external_function_names` records a cross-module call under the
//! spelling the CONSUMING module used; for a mounted function that is
//! `<consumer module>.<leaf>`, which no descriptor carries because the
//! body lives under the DECLARING path. Measured on the shipped
//! archive:
//!
//! ```text
//! external_function_names[536870915] =
//!     'core.security.aead.aes_gcm.aes128_encrypt_block'
//! mount_aliases: same spelling -> fid 27860
//!     (canonical 'core.security.cipher.aes.aes128_encrypt_block')
//! ```
//!
//! Three consumers of that one function record three alias spellings
//! and all three point at fid 27860. So nothing is lost — the truth
//! sits one table over, and every reader that only knows the first
//! table asks for a body that was never meant to exist under that name.
//!
//! BOTH POLARITIES, because a lookup that always answers is not a
//! lookup: a recorded alias resolves, and a spelling nobody aliased
//! resolves to nothing rather than to the nearest thing.

use verum_vbc::module::{FunctionId, VbcModule};

const ALIAS: &str = "core.security.aead.aes_gcm.aes128_encrypt_block";
const CANON: &str = "core.security.cipher.aes.aes128_encrypt_block";

fn module_with_alias() -> VbcModule {
    let mut m = VbcModule::new("alias_probe".to_string());
    let a = m.intern_string(ALIAS);
    let c = m.intern_string(CANON);
    m.mount_aliases.push((a, FunctionId(27860), c));
    m
}

#[test]
fn an_alias_resolves_to_the_declaring_spelling_and_its_id() {
    let m = module_with_alias();
    let (fid, canon) = m
        .mount_alias_target(ALIAS)
        .expect("the alias is recorded, so the lookup must find it");
    assert_eq!(canon, CANON, "the canonical spelling is what a name-keyed reader needs");
    assert_eq!(
        fid,
        FunctionId(27860),
        "the id comes back too — a caller that can use it should not have to \
         look it up again, even though a cross-assembly reader must prefer \
         the name (ids are renumbered by the merge)"
    );
}

#[test]
fn a_spelling_nobody_aliased_resolves_to_nothing() {
    // The control. Without it the test above passes for a lookup that
    // returns the only row it has, whatever it was asked.
    let m = module_with_alias();
    assert!(
        m.mount_alias_target("core.somewhere.else.aes128_encrypt_block").is_none(),
        "an unaliased spelling must answer None, not the nearest row"
    );
    assert!(
        m.mount_alias_target(CANON).is_none(),
        "the CANONICAL name is not itself an alias — asking with it must not \
         loop back, or a resolver could chase its own tail"
    );
}

#[test]
fn an_empty_alias_table_answers_none() {
    let m = VbcModule::new("no_aliases".to_string());
    assert!(m.mount_alias_target(ALIAS).is_none());
}
