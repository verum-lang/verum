use super::*;
use crate::commands::lint_engine::{self, CorpusCtx, CorpusFile};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use tower_lsp::lsp_types::{DiagnosticSeverity, Position, Range, TextEdit};
use verum_lsp::lint_report::{Finding, LintDocument, LintReport};

/// An inspection describes findings, coverage and exact edits. A process error
/// means the inspection itself failed. No baseline, cache or source is written.
pub fn inspect_workspace(profile: Option<&str>) -> Result<()> {
    let report = inspect_at(Path::new("."), profile)?;
    serde_json::to_writer(std::io::stdout().lock(), &report)
        .map_err(|e| CliError::Custom(e.to_string()))?;
    Ok(())
}

fn inspect_at(root: &Path, profile: Option<&str>) -> Result<LintReport> {
    let mut packages: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    let mut directories = HashMap::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || entry.file_name().to_str().is_some_and(|name| {
                    !name.starts_with('.') && !matches!(name, "target" | "llvm" | "node_modules")
                })
        })
    {
        let entry = entry?;
        if !entry.file_type().is_file() || !is_verum_file(entry.path()) {
            continue;
        }
        let parent = entry.path().parent().expect("WalkDir source has a parent");
        let package = directories.entry(parent.to_owned()).or_insert_with(|| {
            parent
                .ancestors()
                .take_while(|dir| dir.starts_with(root))
                .find(|dir| {
                    LINT_MANIFESTS.iter().any(|name| dir.join(name).is_file())
                        || dir.join(".verum/lint.toml").is_file()
                })
                .unwrap_or(root)
                .to_owned()
        });
        packages
            .entry(package.clone())
            .or_default()
            .push(entry.into_path());
    }

    let mut files = Vec::new();
    for (package, paths) in packages {
        let mut config = load_lint_config_at(&package);
        if let Some(profile) = profile {
            apply_profile(&mut config, profile)?;
        }
        let analyzed = paths
            .into_par_iter()
            .map(|path| -> Result<_> {
                let relative = path
                    .strip_prefix(&package)
                    .expect("source belongs to package");
                let source = fs::read_to_string(&path)
                    .map_err(|e| CliError::Custom(format!("{}: {e}", path.display())))?;
                let (issues, module) = lint_content_with_module(relative, &source, &config)?;
                let lines = compute_line_offsets_from_str(&source);
                let findings = issues
                    .iter()
                    .filter_map(|issue| finding(issue, &source, &lines, &config))
                    .collect();
                let doc = LintDocument {
                    path: path
                        .strip_prefix(root)
                        .expect("source belongs to workspace")
                        .to_owned(),
                    source_hash: hex::encode(Sha256::digest(source.as_bytes())),
                    parsed: module.is_some(),
                    findings,
                };
                let corpus = module.map(|module| CorpusFile {
                    path: relative.to_owned(),
                    source,
                    module,
                });
                Ok((doc, corpus))
            })
            .collect::<Result<Vec<_>>>()?;
        let (mut documents, corpus): (Vec<_>, Vec<_>) = analyzed.into_iter().unzip();
        let corpus: Vec<_> = corpus.into_iter().flatten().collect();
        let package_prefix = package.strip_prefix(root).expect("nested package");
        let mut by_path: HashMap<_, _> = documents
            .iter_mut()
            .map(|doc| {
                (
                    doc.path
                        .strip_prefix(package_prefix)
                        .expect("nested document")
                        .to_owned(),
                    doc,
                )
            })
            .collect();
        let mut cross_by_file: HashMap<_, Vec<_>> = HashMap::new();
        for issue in lint_engine::run_cross_file(&CorpusCtx {
            files: &corpus,
            config: Some(&config),
        }) {
            cross_by_file
                .entry(issue.file.clone())
                .or_default()
                .push(issue);
        }
        for file in &corpus {
            if let Some(issues) = cross_by_file.remove(&file.path) {
                let lines = compute_line_offsets_from_str(&file.source);
                let doc = by_path.get_mut(&file.path).expect("corpus document");
                doc.findings.extend(
                    issues
                        .iter()
                        .filter_map(|issue| finding(issue, &file.source, &lines, &config)),
                );
            }
        }
        if !cross_by_file.is_empty() {
            return Err(CliError::Custom(
                "Cross-file rule reported a source outside its corpus".into(),
            ));
        }
        for doc in &mut documents {
            doc.findings.sort_by(|a, b| {
                (&a.position, &a.rule, &a.message).cmp(&(&b.position, &b.rule, &b.message))
            });
        }
        files.extend(documents);
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(LintReport {
        schema_version: 1,
        files,
    })
}

fn finding(
    issue: &LintIssue,
    source: &str,
    lines: &[usize],
    config: &LintConfig,
) -> Option<Finding> {
    let severity = match config.effective_level_for_file(issue.rule, &issue.file, issue.level)? {
        LintLevel::Error => DiagnosticSeverity::ERROR,
        LintLevel::Warning => DiagnosticSeverity::WARNING,
        LintLevel::Info => DiagnosticSeverity::INFORMATION,
        LintLevel::Hint => DiagnosticSeverity::HINT,
        LintLevel::Off => return None,
    };
    let position = |line: usize, column: usize| -> Option<Position> {
        let offset = line_col_to_offset(source, lines, line, column)?;
        if !source.is_char_boundary(offset)
            || source.as_bytes().get(offset) == Some(&b'\n')
                && offset > 0
                && source.as_bytes()[offset - 1] == b'\r'
        {
            return None;
        }
        Some(verum_common::span_utils::lsp::offset_to_lsp_position(
            offset, source,
        ))
    };
    let edits = synthesize_fix_edits_for(issue, source)
        .and_then(|edits| {
            edits
                .into_iter()
                .map(|edit| {
                    Some(TextEdit::new(
                        Range::new(
                            position(edit.start_line, edit.start_column)?,
                            position(edit.end_line, edit.end_column)?,
                        ),
                        edit.new_text,
                    ))
                })
                .collect::<Option<Vec<_>>>()
        })
        .unwrap_or_default();
    Some(Finding {
        rule: issue.rule.into(),
        severity,
        message: issue.message.clone(),
        position: position(issue.line, issue.column),
        edits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_binds_edits_to_file_bytes_and_obeys_nested_package_config() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("child/src")).unwrap();
        fs::create_dir(root.join("target")).unwrap();
        fs::write(
            root.join("child/Verum.toml"),
            "[lint]\ndisable = \"todo-in-code\"\n",
        )
        .unwrap();
        let source = "fn hello() {}\r\n// 💡 FIXME\r\n";
        for path in ["root.vr", "child/src/inner.vr", "target/generated.vr"] {
            fs::write(root.join(path), source).unwrap();
        }
        fs::write(root.join("broken.vr"), "fn broken( -> {").unwrap();
        let report = inspect_at(root, None).unwrap();
        assert_eq!(report.files.len(), 3);
        assert!(!report.files[0].parsed);
        assert!(
            !report.files[1]
                .findings
                .iter()
                .any(|f| f.rule == "todo-in-code")
        );
        let doc = &report.files[2];
        assert_eq!(
            doc.source_hash,
            hex::encode(Sha256::digest(source.as_bytes()))
        );
        let finding = doc
            .findings
            .iter()
            .find(|f| f.rule == "todo-in-code")
            .unwrap();
        assert_eq!(finding.position, Some(Position::new(1, 6)));
        let edit = &finding.edits[0];
        assert_eq!(
            edit.range,
            Range::new(Position::new(1, 6), Position::new(1, 11))
        );
        assert_eq!(fs::read_to_string(root.join("root.vr")).unwrap(), source);
    }
}
