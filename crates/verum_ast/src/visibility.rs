//! Declaration-owned normalization for path-restricted visibility.
use crate::{Ident, Path, PathSegment, Span, Visibility};
use verum_common::{List, Text};

impl Visibility {
    /// Resolve a restricted scope against the module that declares the item.
    /// The result contains only absolute name segments and no source spans;
    /// importers must not resolve the same policy again in their own module.
    pub fn resolve_declared_scope(&self, declaring_module: &str) -> Result<Self, Text> {
        let Self::PublicIn(path) = self else {
            return Ok(self.clone());
        };
        let Some(first) = path.segments.first() else {
            return Err("visibility scope is empty".into());
        };
        let mut names: List<Text> = List::new();
        let mut index = 0;
        if !matches!(first, PathSegment::Name(_)) {
            names = declaring_module.split('.').map(Text::from).collect();
            if names.is_empty() || names.iter().any(|part| part.is_empty()) {
                return Err("relative visibility scope requires a declaring module".into());
            }
            match first {
                PathSegment::Cog => {
                    names.truncate(1);
                    index = 1;
                }
                PathSegment::SelfValue => index = 1,
                PathSegment::Relative => {
                    if names.len() <= 1 {
                        return Err("visibility scope escapes the declaring cog root".into());
                    }
                    names.pop();
                    index = 1;
                }
                PathSegment::Super => {
                    while matches!(path.segments.get(index), Some(PathSegment::Super)) {
                        if names.len() <= 1 {
                            return Err("visibility scope escapes the declaring cog root".into());
                        }
                        names.pop();
                        index += 1;
                    }
                }
                PathSegment::Name(_) => unreachable!(),
            }
        }
        for segment in &path.segments[index..] {
            let PathSegment::Name(ident) = segment else {
                return Err("visibility scope markers must precede name segments".into());
            };
            if ident.name.is_empty() {
                return Err("visibility scope has an empty name segment".into());
            }
            names.push(ident.name.clone());
        }
        let segments = names
            .into_iter()
            .map(|name| PathSegment::Name(Ident::new(name, Span::dummy())))
            .collect();
        Ok(Self::PublicIn(Path::new(segments, Span::dummy())))
    }
}
