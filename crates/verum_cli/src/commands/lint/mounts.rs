use super::{FixEdit, LintIssue};
use verum_ast::{FileId, ItemKind, MountTree, MountTreeKind, Visibility};

pub(super) struct Binding {
    pub name: String,
    pub start: usize,
    end: usize,
}
pub(super) struct LineMount {
    pub bindings: Vec<Binding>,
    pub start: usize,
    pub end: usize,
}

impl LineMount {
    /// The lexical rule only offers edits for an entire private, standalone mount.
    /// Complex trees, inline declarations and comments within a tree require a
    /// richer source refactoring than an edit on this line.
    pub fn parse(line: &str) -> Option<Self> {
        let module = verum_fast_parser::VerumParser::new()
            .parse_module_str(line, FileId::new(0))
            .ok()?;
        let [item] = module.items.as_slice() else {
            return None;
        };
        let ItemKind::Mount(mount) = &item.kind else {
            return None;
        };
        if mount.visibility != Visibility::Private || !item.attributes.is_empty() {
            return None;
        }
        let (start, end) = (mount.span.start as usize, mount.span.end as usize);
        let statement = line.get(start..end)?;
        if statement.contains("//")
            || statement.contains("/*")
            || !statement.trim_end().ends_with(';')
            || !line[..start].trim().is_empty()
        {
            return None;
        }
        let binding = |tree: &MountTree| {
            let MountTreeKind::Path(path) = &tree.kind else {
                return None;
            };
            let name = tree
                .alias
                .as_ref()
                .map(|alias| alias.name.as_str())
                .unwrap_or(path.last_segment_name());
            let start = tree.span.start as usize;
            let end = tree
                .alias
                .as_ref()
                .map(|a| a.span.end)
                .unwrap_or(tree.span.end) as usize;
            line.get(start..end)?;
            Some(Binding {
                name: name.to_owned(),
                start,
                end,
            })
        };
        let bindings = match &mount.tree.kind {
            MountTreeKind::Path(_) => vec![binding(&mount.tree)?],
            MountTreeKind::Nested { trees, .. } => {
                trees.iter().map(binding).collect::<Option<Vec<_>>>()?
            }
            _ => return None,
        };
        Some(Self {
            bindings,
            start,
            end,
        })
    }

    pub fn edits(&self, line: &str, issue: &LintIssue) -> Option<Vec<FixEdit>> {
        let target = issue.column.checked_sub(1)?;
        let (start, end_line, end_column) = if target == self.start {
            if line[self.end..].trim().is_empty() {
                (0, issue.line + 1, 1)
            } else {
                (self.start, issue.line, self.end + 1)
            }
        } else {
            let binding = self
                .bindings
                .iter()
                .find(|binding| binding.start == target)?;
            let tail = line[binding.end..].trim_start();
            // Drop the following separator if present. Removing a final member
            // leaves a legal trailing comma and never overlaps a sibling edit.
            let end = tail
                .strip_prefix(',')
                .map(|rest| line.len() - rest.trim_start().len())
                .unwrap_or(binding.end);
            (binding.start, issue.line, end + 1)
        };
        Some(vec![FixEdit {
            start_line: issue.line,
            start_column: start + 1,
            end_line,
            end_column,
            new_text: String::new(),
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::super::{apply_fixes, lint_source};
    use std::path::Path;

    #[test]
    fn non_ascii_bindings_are_matched_on_identifier_boundaries() {
        assert!(!super::super::contains_word("xλ λ_δ", "λ"));
        assert!(super::super::contains_word("let δ = λ();", "λ"));
    }

    #[test]
    fn fixing_group_members_preserves_used_bindings_aliases_and_comments() {
        let source = "mount demo.{Keep, Drop, Other as Local, Tail}; // imports\r\nfn probe() { Keep(); Local(); }\r\n";
        let issues = lint_source(Path::new("main.vr"), source, None);
        let unused = issues
            .iter()
            .filter(|issue| issue.rule == "unused-import")
            .collect();
        assert_eq!(
            apply_fixes(source, &unused),
            "mount demo.{Keep, Other as Local, }; // imports\r\nfn probe() { Keep(); Local(); }\r\n"
        );
        let only = "mount demo.Single; // retain reason\nfn probe() {}\n";
        let issues = lint_source(Path::new("main.vr"), only, None);
        let unused = issues
            .iter()
            .filter(|issue| issue.rule == "unused-import")
            .collect();
        assert_eq!(
            apply_fixes(only, &unused),
            " // retain reason\nfn probe() {}\n"
        );
    }

    #[test]
    fn fixes_preserve_interpolated_imports_shadowed_reads_and_match_coverage() {
        let source = "mount demo.helper;\nfn probe(value: Int) {\n    let x = 1;\n    let x = 2;\n    print(f\"{helper()} {x}\");\n    match value {\n        _ => (),\n    }\n}\n";
        let issues = lint_source(Path::new("main.vr"), source, None);
        assert!(!issues.iter().any(|issue| issue.rule == "unused-import"));
        assert!(issues.iter().any(|issue| issue.rule == "shadow-binding"));
        let fixes = issues.iter().filter(|issue| issue.fixable).collect();
        assert_eq!(apply_fixes(source, &fixes), source);
    }
}
