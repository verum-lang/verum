//! One source-order traversal of complete paths inside grouped mounts.
use crate::{
    Ident, Path, Span,
    decl::{MountTree, MountTreeKind},
};
use verum_common::{List, Maybe};

/// Whether a mount leaf names an item or a module's exported surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountPathKind {
    /// A named item, optionally renamed at the mount site.
    Named,
    /// Every eligible export of the named module.
    Glob,
}

/// A complete path with every enclosing group prefix retained.
#[derive(Debug)]
pub struct MountPath<'a> {
    /// The unresolved path; relative markers remain for the ordinary resolver.
    pub path: Path,
    /// The leaf's binding rename, if present.
    pub alias: Maybe<&'a Ident>,
    /// Named item or glob surface.
    pub kind: MountPathKind,
    /// The original leaf location.
    pub span: Span,
}

#[derive(Debug)]
enum Step<'a> {
    Tree(&'a MountTree),
    LeavePrefix,
}

/// Iterates path-based leaves without recursion or module lookups.
/// File mounts are handled separately by the session loader, not this iterator.
#[derive(Debug)]
pub struct MountPaths<'a> {
    work: List<Step<'a>>,
    prefixes: List<&'a Path>,
}

/// Visit complete named/glob paths in source order. Construction and iteration
/// perform no registry reads, so consumers retain control of observed lookups.
pub fn mount_paths(tree: &MountTree) -> MountPaths<'_> {
    MountPaths {
        work: List::from_iter([Step::Tree(tree)]),
        prefixes: List::new(),
    }
}

impl<'a> MountPaths<'a> {
    /// Visit leaves satisfying the predicate before constructing their full paths.
    /// The predicate sees the original leaf path, kind and rename; prefixes and
    /// source spans on every yielded path are identical to unfiltered iteration.
    pub fn filter_leaves(
        mut self,
        mut include: impl FnMut(&Path, MountPathKind, Option<&Ident>) -> bool,
    ) -> impl Iterator<Item = MountPath<'a>> {
        std::iter::from_fn(move || self.next_where(&mut include))
    }

    fn next_where(
        &mut self,
        include: &mut impl FnMut(&Path, MountPathKind, Option<&Ident>) -> bool,
    ) -> Maybe<MountPath<'a>> {
        while let Some(step) = self.work.pop() {
            let tree = match step {
                Step::Tree(tree) => tree,
                Step::LeavePrefix => {
                    self.prefixes.pop();
                    continue;
                }
            };
            let (leaf, kind) = match &tree.kind {
                MountTreeKind::Path(path) => (path, MountPathKind::Named),
                MountTreeKind::Glob(path) => (path, MountPathKind::Glob),
                MountTreeKind::Nested { prefix, trees } => {
                    self.prefixes.push(prefix);
                    self.work.push(Step::LeavePrefix);
                    self.work.extend(trees.iter().rev().map(Step::Tree));
                    continue;
                }
                MountTreeKind::File { .. } => continue,
            };
            if !include(leaf, kind, tree.alias.as_ref()) {
                continue;
            }
            let path = if self.prefixes.is_empty() {
                leaf.clone()
            } else {
                Path::new(
                    self.prefixes
                        .iter()
                        .copied()
                        .chain(std::iter::once(leaf))
                        .flat_map(|part| part.segments.iter().cloned())
                        .collect(),
                    tree.span,
                )
            };
            return Some(MountPath {
                path,
                alias: tree.alias.as_ref(),
                kind,
                span: tree.span,
            });
        }
        None
    }
}

impl<'a> Iterator for MountPaths<'a> {
    type Item = MountPath<'a>;

    fn next(&mut self) -> Maybe<Self::Item> {
        self.next_where(&mut |_, _, _| true)
    }
}
