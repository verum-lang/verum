//! The implicit prelude, as an AST fact.
//!
//! Every Verum module implicitly mounts `core.prelude.*` unless it
//! opts out (`@![no_implicit_prelude]`). That sentence is LANGUAGE
//! semantics, so the injector that makes it true lives here, next to
//! the AST it rewrites — not in any one consumer. The compiler
//! pipeline injects it for user compiles, and the VBC test harness
//! injects the SAME mount so a stdlib file compiles under the same
//! ambient vocabulary it sees in production (measured: btree.vr,
//! mount-free by design, called the prelude's `List.from` and the
//! harness — which skipped injection — died "undefined function").
//!
//! `VERUM_NO_IMPLICIT_PRELUDE=1` is the process-level escape hatch
//! (diagnostics + A/B isolation): treat every module as opted out.

use crate::decl::{MountDecl, Visibility};
use crate::span::Span;
use crate::{Ident, Item, ItemKind, MountTree, MountTreeKind, Path, PathSegment};
use verum_common::{List, Maybe, Text};

/// Prepend the implicit `mount core.prelude.*;` unless the module
/// opted out, the process opted out, or the module already mounts
/// `core.prelude` itself (idempotence — the explicit declaration
/// stays the single import site).
pub fn inject_implicit_prelude_mount(module: &mut crate::Module) {
    if module.has_no_implicit_prelude() {
        return;
    }
    if std::env::var_os("VERUM_NO_IMPLICIT_PRELUDE").is_some() {
        return;
    }
    let already_mounted = module.items.iter().any(|item| {
        let ItemKind::Mount(decl) = &item.kind else {
            return false;
        };
        let path = match &decl.tree.kind {
            MountTreeKind::Glob(p) | MountTreeKind::Path(p) => p,
            MountTreeKind::Nested { prefix, .. } => prefix,
            MountTreeKind::File { .. } => return false,
        };
        let mut names = path.segments.iter().filter_map(|s| match s {
            PathSegment::Name(id) => Some(id.name.as_str()),
            _ => None,
        });
        names.next() == Some("core") && names.next() == Some("prelude")
    });
    if already_mounted {
        return;
    }

    let span = Span::new(0, 0, module.file_id);
    let mut segments = List::new();
    segments.push(PathSegment::Name(Ident::new(Text::from("core"), span)));
    segments.push(PathSegment::Name(Ident::new(Text::from("prelude"), span)));
    let tree = MountTree {
        kind: MountTreeKind::Glob(Path::new(segments, span)),
        alias: Maybe::None,
        span,
    };
    let decl = MountDecl {
        visibility: Visibility::Private,
        tree,
        alias: Maybe::None,
        span,
    };
    module.items.insert(0, Item::new(ItemKind::Mount(decl), span));
}
