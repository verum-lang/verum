//! T1665: real workspace previews and source archives share manifest authority.
//! This exercises library handlers, not a dispatched workspace-publish command
//! or authenticated registry service. Set VERUM_T1665_FIXTURE_ROOT to a
//! case-sensitive filesystem to require distinct-name precedence coverage.

use super::{Config, create_member_cog, publish};
use crate::error::CliError;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;
use verum_common::{List, Map, Text};

const CHILD: &str = "commands::workspace::publication_manifests::isolated_workspace_case";
const CASE_ENV: &str = "VERUM_T1665_WORKSPACE_CASE";
const ROOT_ENV: &str = "VERUM_T1665_FIXTURE_ROOT";
const SOURCE: &str = "module fixture.lib;\npublic fn marker() -> Int { 7 }\n";

fn fixture() -> (TempDir, bool) {
    let root = std::env::var_os(ROOT_ENV);
    let mut builder = tempfile::Builder::new();
    builder.prefix("verum-t1665-");
    let dir = match &root {
        Some(path) => builder.tempdir_in(path),
        None => builder.tempdir(),
    }
    .expect("create isolated workspace fixture");
    let probe = dir.path().join("case-probe");
    fs::create_dir(&probe).unwrap();
    fs::write(probe.join("lower"), b"first").unwrap();
    fs::write(probe.join("LOWER"), b"second").unwrap();
    let distinct = fs::read_dir(&probe).unwrap().count() == 2
        && fs::read(probe.join("lower")).unwrap() == b"first"
        && fs::read(probe.join("LOWER")).unwrap() == b"second";
    fs::remove_dir_all(probe).unwrap();
    println!("T1665_CASE_DISTINCT={distinct}");
    assert!(
        root.is_none() || distinct,
        "{ROOT_ENV} must support distinct filenames differing only by case"
    );
    (dir, distinct)
}

fn manifest(name: &str) -> Text {
    format!("[cog]\nname = \"{name}\"\nversion = \"1.2.3\"\n").into()
}

fn member(root: &Path, dir: &str, filename: &str, name: &str) -> Text {
    let path = root.join(dir);
    fs::create_dir_all(path.join("src")).unwrap();
    let bytes = manifest(name);
    fs::write(path.join(filename), bytes.as_bytes()).unwrap();
    fs::write(path.join("src/lib.vr"), SOURCE).unwrap();
    bytes
}

fn workspace(root: &Path, members: &[&str]) {
    let members = members
        .iter()
        .map(|member| format!("\"{member}\""))
        .collect::<List<_>>()
        .join(", ");
    fs::write(
        root.join(Config::MANIFEST_FILENAME),
        format!(
            "[cog]\nname = \"fixture-workspace\"\nversion = \"1.0.0\"\n\
             [workspace]\nmembers = [{members}]\n"
        ),
    )
    .unwrap();
}

#[test]
fn canonical_only_member_is_included_in_preview() {
    child_case("canonical", 1, 0, 0);
}

#[test]
fn legacy_only_member_is_included_in_preview() {
    child_case("legacy", 1, 0, 0);
}

#[test]
fn actual_new_project_is_publishable_without_renaming_its_manifest() {
    child_case("scaffold", 1, 0, 0);
}

#[test]
fn canonical_manifest_is_the_preview_authority() {
    child_case("precedence", 1, 0, 0);
}

#[test]
fn invalid_canonical_manifest_is_failed_without_legacy_fallback() {
    child_case("invalid", 0, 1, 0);
}

#[test]
fn missing_member_is_skipped_and_excluded_from_preview_total() {
    child_case("missing", 0, 0, 1);
}

#[test]
fn mixed_workspace_reports_ready_failed_and_skipped_members() {
    child_case("mixed", 2, 1, 1);
}

fn child_case(case: &str, ready: usize, failed: usize, skipped: usize) {
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD, "--nocapture"])
        .env(CASE_ENV, case)
        .output()
        .unwrap();
    let output = format!(
        "{}\n{}",
        std::str::from_utf8(&result.stdout).unwrap(),
        std::str::from_utf8(&result.stderr).unwrap()
    );
    println!("T1665_CASE={case}\n{output}");
    assert!(result.status.success(), "{case}: {output}");
    for name in match case {
        "canonical" | "precedence" => &["canonical-member"][..],
        "legacy" => &["legacy-member"][..],
        "scaffold" => &["scaffold-member"][..],
        "mixed" => &["canonical-member", "legacy-member"][..],
        _ => &[],
    } {
        assert!(
            output.contains(&format!("Would publish {name} v")),
            "{output}"
        );
    }
    assert!(!output.contains("Would publish shadow-member"), "{output}");
    assert!(!output.contains("Would publish invalid-member"), "{output}");
    assert!(!output.contains("Would publish missing-member"), "{output}");
    if failed == 0 {
        assert!(
            output.contains(&format!("Would publish {ready} packages")),
            "{output}"
        );
    } else {
        assert!(
            output.contains(&format!("Failed to publish {failed} packages")),
            "{output}"
        );
    }
    assert!(
        output.contains(&format!(
            "Publication summary: {ready} ready, {failed} failed, {skipped} skipped"
        )),
        "{output}"
    );
    if case == "scaffold" {
        assert!(output.contains("T1665_NEW_MANIFEST="), "{output}");
    }
}

#[test]
fn isolated_workspace_case() {
    let Ok(case) = std::env::var(CASE_ENV) else {
        return;
    };
    let (fixture, distinct) = fixture();
    let root = fixture.path();
    std::env::set_current_dir(root).unwrap();
    crate::ui::init(false, false, "never").unwrap();
    match case.as_str() {
        "canonical" | "precedence" => {
            member(root, "member", "Verum.toml", "canonical-member");
            if case == "precedence" {
                add_legacy_shadow(&root.join("member"), distinct);
            }
            workspace(root, &["member"]);
        }
        "legacy" => {
            member(root, "member", "verum.toml", "legacy-member");
            workspace(root, &["member"]);
        }
        "scaffold" => {
            crate::commands::new::execute(
                "scaffold-member",
                Some("application"),
                "library",
                false,
                Some("member"),
            )
            .unwrap();
            // Inspect the actual directory entry, not a case-folded exists()
            // probe. T1668 owns migration of the scaffold's filename spelling.
            let names = fs::read_dir(root.join("member"))
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .filter(|name| name == "Verum.toml" || name == "verum.toml")
                .collect::<List<_>>();
            assert_eq!(names.len(), 1);
            println!("T1665_NEW_MANIFEST={}", names[0].to_string_lossy());
            workspace(root, &["member"]);
        }
        "invalid" => {
            member(root, "member", "Verum.toml", "invalid-member");
            fs::write(root.join("member/Verum.toml"), "[cog]\nname = [").unwrap();
            add_legacy_shadow(&root.join("member"), distinct);
            workspace(root, &["member"]);
        }
        "missing" => {
            fs::create_dir(root.join("missing")).unwrap();
            workspace(root, &["missing"]);
        }
        "mixed" => {
            member(root, "canonical", "Verum.toml", "canonical-member");
            member(root, "legacy", "verum.toml", "legacy-member");
            member(root, "invalid", "Verum.toml", "invalid-member");
            fs::write(root.join("invalid/Verum.toml"), "[cog]\nname = [").unwrap();
            fs::create_dir(root.join("missing")).unwrap();
            workspace(root, &["canonical", "legacy", "invalid", "missing"]);
        }
        _ => panic!("unknown workspace fixture case: {case}"),
    }
    let result = publish(true);
    if matches!(case.as_str(), "invalid" | "mixed") {
        let error = result.expect_err("a malformed member must fail the preview");
        assert!(error.to_string().contains("Failed to publish 1 packages"));
    } else {
        result.expect("valid or explicitly skipped members must complete the preview");
    }
    for entry in fs::read_dir(root).unwrap() {
        assert!(!entry.unwrap().path().join("target").exists());
    }
}

fn add_legacy_shadow(member: &Path, distinct: bool) {
    if distinct {
        fs::write(
            member.join("verum.toml"),
            manifest("shadow-member").as_bytes(),
        )
        .unwrap();
        println!("T1665_DISTINCT_PRECEDENCE=exercised");
    } else {
        // Do not overwrite the canonical file through its case-folded alias.
        // Full T1665 precedence acceptance requires ROOT_ENV on a distinct FS.
        println!("T1665_DISTINCT_PRECEDENCE=unavailable-on-case-folded-filesystem");
    }
}

#[test]
fn canonical_member_archive_preserves_manifest_and_source_bytes() {
    archive_case("Verum.toml", false);
}

#[test]
fn legacy_member_archive_uses_the_canonical_entry_name() {
    archive_case("verum.toml", false);
}

#[test]
fn member_archive_uses_the_selected_canonical_manifest() {
    archive_case("Verum.toml", true);
}

fn archive_case(filename: &str, with_shadow: bool) {
    let (fixture, distinct) = fixture();
    let bytes = member(fixture.path(), "member", filename, "canonical-member");
    let member = fixture.path().join("member");
    if with_shadow {
        add_legacy_shadow(&member, distinct);
    }
    let config = Config::load(&member).unwrap();
    let archive_path = create_member_cog(&member, &config).unwrap();
    let file = fs::File::open(archive_path).unwrap();
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut entries = Map::<Text, List<u8>>::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path: Text = entry.path().unwrap().to_string_lossy().into_owned().into();
        let mut contents = List::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let count = entry.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            contents.extend_from_slice(&buffer[..count]);
        }
        assert!(
            entries.insert(path, contents).is_none(),
            "duplicate archive entry"
        );
    }
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries
            .get(&Text::from("Verum.toml"))
            .expect("source archive must contain the canonical manifest entry")
            .as_slice(),
        bytes.as_bytes()
    );
    assert_eq!(
        entries[&Text::from("src/lib.vr")].as_slice(),
        SOURCE.as_bytes()
    );
}

#[test]
fn manifest_removed_after_loading_cannot_produce_an_incomplete_archive() {
    let (fixture, _) = fixture();
    member(fixture.path(), "member", "Verum.toml", "removed-member");
    let member = fixture.path().join("member");
    let config = Config::load(&member).unwrap();
    fs::remove_file(Config::manifest_path(&member)).unwrap();
    let error = create_member_cog(&member, &config)
        .expect_err("missing manifest must fail packaging instead of being silently omitted");
    assert!(matches!(error, CliError::Io(_)), "{error}");
}
