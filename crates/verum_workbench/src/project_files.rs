//! Bounded, lazy project discovery. The editor and agents consume the same paths.
use crate::workspace::Workspace;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::path::Path;
use verum_common::{List, Text};

const DIRECTORY_LIMIT: usize = 10_000;
const SEARCH_VISIT_LIMIT: usize = 50_000;
const SEARCH_MATCH_LIMIT: usize = 200;

#[derive(Clone, Serialize)]
pub struct Entry {
    pub path: Text,
    pub kind: &'static str,
    pub size: u64,
}
#[derive(Serialize)]
pub struct Listing {
    pub root: std::path::PathBuf,
    pub entries: List<Entry>,
    pub truncated: bool,
}
pub fn exposed(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path).components().all(|component| {
            let std::path::Component::Normal(name) = component else {
                return false;
            };
            name.to_str().is_some_and(|name| {
                !name.starts_with('.') && !matches!(name, "target" | "node_modules" | "llvm")
            })
        })
}
impl Workspace {
    pub fn browse(&self, relative: &str) -> Result<Listing> {
        ensure!(
            relative.is_empty() || exposed(relative),
            "Select an exposed project directory"
        );
        let directory = if relative.is_empty() {
            self.root.clone()
        } else {
            self.document_path(relative)?
        };
        ensure!(directory.is_dir(), "Select a project directory");
        let mut entries = List::new();
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path
                .strip_prefix(&self.root)?
                .to_str()
                .context("Project paths must be UTF-8")?
                .to_owned();
            #[cfg(windows)]
            let relative = relative.replace('\\', "/");
            if !exposed(&relative) {
                continue;
            }
            let ty = entry.file_type()?;
            if !ty.is_dir() && !ty.is_file() {
                continue;
            }
            ensure!(
                entries.len() < DIRECTORY_LIMIT,
                "Directory exceeds 10000 entries; narrow the workspace"
            );
            entries.push(Entry {
                path: relative.into(),
                kind: if ty.is_dir() { "directory" } else { "file" },
                size: if ty.is_file() {
                    entry.metadata()?.len()
                } else {
                    0
                },
            });
        }
        entries.sort_by(|a, b| {
            (a.kind != "directory")
                .cmp(&(b.kind != "directory"))
                .then_with(|| a.path.cmp(&b.path))
        });
        Ok(Listing {
            root: self.root.clone(),
            entries,
            truncated: false,
        })
    }
    pub fn search_files(&self, query: &str) -> Result<Listing> {
        let query = query.trim().to_lowercase();
        ensure!(
            !query.is_empty() && query.len() <= 256,
            "Search needs 1..256 characters"
        );
        let mut pending = List::new();
        pending.push(Text::new());
        let mut matches = List::new();
        let mut visited = 0;
        let mut truncated = false;
        'search: while let Some(directory) = pending.pop() {
            let listing = self.browse(&directory)?;
            for entry in listing.entries {
                visited += 1;
                if visited > SEARCH_VISIT_LIMIT {
                    truncated = true;
                    break 'search;
                }
                if entry.kind == "directory" {
                    pending.push(entry.path);
                } else if entry.path.to_lowercase().contains(&query) {
                    if matches.len() == SEARCH_MATCH_LIMIT {
                        truncated = true;
                        break 'search;
                    }
                    matches.push(entry);
                }
            }
        }
        matches.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Listing {
            root: self.root.clone(),
            entries: matches,
            truncated,
        })
    }
}
