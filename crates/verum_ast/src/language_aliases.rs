//! Immutable language-owned alias declarations shared by semantic consumers.
//! These are language declarations, not ambient standard-library imports.
use crate::{
    Span, Type, TypeKind,
    ty::{Ident, Path},
};
use std::sync::LazyLock;
use verum_common::Heap;

/// The target is scoped in the language registrar, never in a caller module.
#[derive(Debug)]
pub struct LanguageTypeAlias {
    /// Exact source spelling registered in the ambient language scope.
    pub name: &'static str,
    /// Declaration-owned target; consumers may not resolve it in a caller scope.
    pub target: Type,
}

/// Existing low-level language aliases. Consumers translate these AST types;
/// they must not infer alias authority from a populated name table or layout.
pub fn declarations() -> &'static [LanguageTypeAlias] {
    static ALIASES: LazyLock<[LanguageTypeAlias; 3]> = LazyLock::new(|| {
        let span = Span::dummy();
        [
            LanguageTypeAlias {
                name: "RawPtr",
                target: Type::new(
                    TypeKind::Pointer {
                        mutable: true,
                        inner: Heap::new(Type::int(span)),
                    },
                    span,
                ),
            },
            LanguageTypeAlias {
                name: "u32",
                target: Type::new(
                    TypeKind::Path(Path::single(Ident::new("UInt32", span))),
                    span,
                ),
            },
            LanguageTypeAlias {
                name: "Epoch",
                target: Type::int(span),
            },
        ]
    });
    &*ALIASES
}

/// Only the exact unqualified spelling names a language declaration.
pub fn lookup(name: &str) -> Option<&'static LanguageTypeAlias> {
    declarations()
        .iter()
        .find(|declaration| declaration.name == name)
}
