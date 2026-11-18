//! Capabilities exposed by Studio's implemented provider adapters.
use crate::Studio;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

#[cfg(test)]
#[path = "../tests/providers/discovery.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Driver {
    #[default]
    Codex,
    #[serde(rename = "agy")]
    Antigravity,
}

impl Driver {
    const ALL: &[Self] = &[Self::Codex, Self::Antigravity];

    fn name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Antigravity => "Antigravity",
        }
    }

    fn capabilities(self) -> Value {
        // The local driver's contract is independent of whether an account has signed in.
        match self {
            Self::Codex => {
                json!({"reviewedTools":true, "workspaceContext":true, "modelSelection":true})
            }
            Self::Antigravity => {
                json!({"reviewedTools":false, "workspaceContext":false, "modelSelection":false})
            }
        }
    }
}

fn executable_file(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn executable_available(path: &Path) -> bool {
    let available = |path: &Path| {
        if executable_file(path) {
            return true;
        }
        #[cfg(windows)]
        if path.extension().is_none() {
            return executable_file(&path.with_extension("exe"));
        }
        false
    };
    if path.components().count() > 1 || path.is_absolute() {
        return available(path);
    }
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|base| available(&base.join(path))))
}

impl Studio {
    pub(super) async fn provider_catalog(&self) -> Value {
        let mut providers = Vec::with_capacity(Driver::ALL.len());
        for &driver in Driver::ALL {
            let (executable, live) = match driver {
                Driver::Codex => (
                    &self.config.codex,
                    self.agent
                        .lock()
                        .await
                        .as_ref()
                        .filter(|p| p.connected())
                        .map(|p| p.events.clone()),
                ),
                Driver::Antigravity => (
                    &self.agy_executable,
                    self.agy
                        .lock()
                        .await
                        .as_ref()
                        .filter(|p| p.connected())
                        .map(|p| p.events.clone()),
                ),
            };
            let connection = match live {
                Some(events) => Some(events.lock().await.connection()),
                None => None,
            };
            providers.push(json!({"id":driver,"name":driver.name(),"executableAvailable":executable_available(executable),
                "connectionId":connection,"capabilities":driver.capabilities()}));
        }
        json!({"providers":providers})
    }
}
