#![cfg(unix)]
use std::{os::unix::fs::PermissionsExt, time::Duration};
use verum_lsp::cli_format::{FmtSettings, format_via_cli};

#[tokio::test]
async fn cancelling_formatting_kills_the_child_process() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("formatter");
    let pid_file = root.path().join("pid");
    std::fs::write(&binary, "#!/bin/sh\necho $$ > \"$4\"\nexec sleep 60\n").unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let task_path = pid_file.clone();
    let request = tokio::spawn(async move {
        format_via_cli(
            "fn main() {}",
            &FmtSettings {
                enabled: true,
                binary: Some(binary),
            },
            task_path.to_str(),
        )
        .await
    });
    let pid = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(pid) = std::fs::read_to_string(&pid_file) {
                if let Ok(pid) = pid.trim().parse::<u32>() {
                    break pid;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    let pid = pid.expect("formatter started");
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = tokio::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stderr(std::process::Stdio::null())
                .status()
                .await
                .unwrap();
            if !status.success() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("cancelled formatter must not survive its request");
}
