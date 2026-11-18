//! Operator-approved delivery of an exact, completed provider message.
use crate::{
    Studio,
    agent_runtime::TurnIdentity,
    codex::Events,
    workspace::{digest, private_dir},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use uuid::Uuid;
use verum_common::Text;

const PAGE: usize = 16;
const TEXT_LIMIT: usize = 16 * 1024;

pub(super) fn shareable_text(text: &str) -> bool {
    !text.is_empty() && text.len() <= TEXT_LIMIT
}
const PACKET_LIMIT: u64 = 160 * 1024;

#[cfg(test)]
#[path = "../tests/handoffs/store.rs"]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Address {
    connection_id: Uuid,
    thread_id: Text,
}

impl Address {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.thread_id.is_empty() && self.thread_id.len() <= 1024,
            "Invalid conversation ID"
        );
        Ok(())
    }

    fn key(&self) -> Result<Text> {
        Ok(digest(&serde_json::to_vec(self)?))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    from: Address,
    to: Address,
    seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Message {
    id: Text,
    from: Address,
    to: Address,
    seq: u64,
    turn_id: Text,
    item_id: Text,
    text: Text,
    sha256: Text,
}

impl Message {
    fn identity(
        from: &Address,
        to: &Address,
        seq: u64,
        turn: &Text,
        item: &Text,
        hash: &Text,
    ) -> Result<Text> {
        Ok(digest(&serde_json::to_vec(&(
            from, to, seq, turn, item, hash,
        ))?))
    }

    fn observe(source: &Events, target: &Events, selection: &Selection) -> Result<Self> {
        let Selection { from, to, seq } = selection;
        ensure!(from != to, "Choose a different recipient");
        ensure!(
            from.connection_id == source.connection()
                && to.connection_id == target.connection()
                && target.live_thread(&to.thread_id),
            "Conversation is no longer active"
        );
        let params = source.completed_message(&from.thread_id, *seq)?;
        let bounded = |v: &Value, limit| -> Result<Text> {
            let value = v.as_str().context("Incomplete provider message")?;
            ensure!(
                !value.is_empty() && value.len() <= limit,
                "Message field is empty or too large"
            );
            Ok(value.into())
        };
        let turn_id = bounded(&params["turnId"], 1024)?;
        let item_id = bounded(&params["item"]["id"], 1024)?;
        let text = bounded(&params["item"]["text"], TEXT_LIMIT)?;
        let sha256 = digest(text.as_bytes());
        let id = Self::identity(from, to, *seq, &turn_id, &item_id, &sha256)?;
        Ok(Self {
            id,
            from: from.clone(),
            to: to.clone(),
            seq: *seq,
            turn_id,
            item_id,
            text,
            sha256,
        })
    }

    fn filename(&self) -> Result<String> {
        Ok(format!(
            "{}.{}.{}.json",
            self.from.key()?,
            self.to.key()?,
            self.id
        ))
    }

    fn deliver(&self, dir: &Path) -> Result<()> {
        let destination = dir.join(self.filename()?);
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        file.write_all(&serde_json::to_vec(self)?)?;
        file.as_file().sync_all()?;
        if let Err(error) = file.persist_noclobber(&destination) {
            if error.error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(error.error.into());
            }
            ensure!(
                read_packet(&destination)? == *self,
                "Stored message differs from the approved message"
            );
        }
        #[cfg(unix)]
        std::fs::File::open(dir)?.sync_all()?;
        Ok(())
    }
}

fn read_packet(path: &Path) -> Result<Message> {
    let meta = path.symlink_metadata()?;
    ensure!(
        meta.is_file() && meta.len() <= PACKET_LIMIT,
        "Invalid stored message"
    );
    let file = std::fs::File::open(path)?;
    let message: Message = serde_json::from_reader(file.take(PACKET_LIMIT + 1))?;
    let id = Message::identity(
        &message.from,
        &message.to,
        message.seq,
        &message.turn_id,
        &message.item_id,
        &message.sha256,
    )?;
    let filename = message.filename()?;
    ensure!(
        message.id == id
            && message.sha256 == digest(message.text.as_bytes())
            && path
                .file_name()
                .is_some_and(|name| name == filename.as_str()),
        "Stored message identity mismatch"
    );
    Ok(message)
}

fn valid_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

enum Visibility<T> {
    Project,
    Participant(T),
    Recipient(T),
}
impl<T> Visibility<T> {
    fn includes<U: ?Sized>(&self, from: &U, to: &U) -> bool
    where
        T: PartialEq<U>,
    {
        match self {
            Self::Project => true,
            Self::Participant(owner) => owner == from || owner == to,
            Self::Recipient(owner) => owner == to,
        }
    }
}
fn page(dir: &Path, audience: Visibility<Address>, after: Option<&str>) -> Result<Value> {
    ensure!(
        after.is_none_or(|cursor| cursor
            .split_once('.')
            .is_some_and(|(time, id)| time.len() == 32
                && u128::from_str_radix(time, 16).is_ok()
                && valid_key(id))),
        "Invalid inbox cursor"
    );
    let keys = match &audience {
        Visibility::Project => Visibility::Project,
        Visibility::Participant(address) => Visibility::Participant(address.key()?.to_string()),
        Visibility::Recipient(address) => Visibility::Recipient(address.key()?.to_string()),
    };
    let mut names = BTreeSet::new();
    for entry in dir.read_dir()? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        let mut parts = stem.split('.');
        let (Some(from), Some(to), Some(id), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if keys.includes(from, to) && valid_key(from) && valid_key(to) && valid_key(id) {
            // Delivery never overwrites the file. Retrying an old message must not move it
            // to the front of the inbox. The digest breaks ties at filesystem precision.
            let time = entry
                .metadata()?
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            let position = format!("{time:032x}.{id}");
            if after.is_some_and(|last| position.as_str() >= last) {
                continue;
            }
            names.insert((position, name.to_owned()));
            if names.len() > PAGE + 1 {
                names.pop_first();
            }
        }
    }
    let more = names.len() > PAGE;
    let mut messages = Vec::new();
    let mut next = None;
    for (position, name) in names.into_iter().rev().take(PAGE) {
        let message = read_packet(&dir.join(name))?;
        ensure!(
            audience.includes(&message.from, &message.to),
            "Message recipient mismatch"
        );
        messages.push(message);
        if more {
            next = Some(position);
        }
    }
    Ok(json!({"messages":messages,"next":next}))
}

pub(super) struct Handoffs {
    data_root: Option<PathBuf>,
    project: Text,
    previews: Mutex<HashMap<Text, (Instant, Message)>>,
}

impl Handoffs {
    pub(super) fn for_workspace(workspace: &Path) -> Self {
        Self {
            data_root: crate::data_home(),
            project: digest(workspace.as_os_str().as_encoded_bytes()),
            previews: Mutex::default(),
        }
    }

    fn directory(&self) -> Result<PathBuf> {
        let root = self
            .data_root
            .as_deref()
            .context("Studio application data directory is unavailable")?;
        ensure!(
            root.is_absolute(),
            "Studio application data directory must be absolute"
        );
        private_dir(root, &["VerumStudio", "handoffs", self.project.as_str()])
    }
}

impl Studio {
    async fn observe_handoff(&self, selection: &Selection) -> Result<Message> {
        selection.from.validate()?;
        selection.to.validate()?;
        let a = self.connection_events(selection.from.connection_id).await?;
        if selection.from.connection_id == selection.to.connection_id {
            let a = a.lock().await;
            return Message::observe(&a, &a, selection);
        }
        let b = self.connection_events(selection.to.connection_id).await?;
        // Both endpoints must be live in the same observation. Lock in connection order.
        if selection.from.connection_id < selection.to.connection_id {
            let a = a.lock().await;
            let b = b.lock().await;
            Message::observe(&a, &b, selection)
        } else {
            let b = b.lock().await;
            let a = a.lock().await;
            Message::observe(&a, &b, selection)
        }
    }

    pub(super) async fn preview_handoff(&self, params: Value) -> Result<Value> {
        let selection = serde_json::from_value(params)?;
        let message = self.observe_handoff(&selection).await?;
        let mut pending = self.handoffs.previews.lock().await;
        pending.retain(|_, (time, _)| time.elapsed() < Duration::from_secs(300));
        ensure!(
            pending.len() < 128 || pending.contains_key(&message.id),
            "Too many pending handoffs"
        );
        pending.insert(message.id.clone(), (Instant::now(), message.clone()));
        Ok(serde_json::to_value(message)?)
    }

    pub(super) async fn send_handoff(&self, params: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Approval {
            id: Text,
        }
        let approval: Approval = serde_json::from_value(params)?;
        let (time, message) = self
            .handoffs
            .previews
            .lock()
            .await
            .remove(&approval.id)
            .context("Handoff was already confirmed or has expired")?;
        ensure!(
            time.elapsed() < Duration::from_secs(300),
            "Handoff preview expired"
        );
        let observed = self
            .observe_handoff(&Selection {
                from: message.from.clone(),
                to: message.to.clone(),
                seq: message.seq,
            })
            .await?;
        ensure!(observed == message, "Handoff changed since review");
        let directory = self.handoffs.directory()?;
        tokio::task::spawn_blocking(move || {
            message.deliver(&directory)?;
            Ok(serde_json::to_value(message)?)
        })
        .await?
    }

    pub(super) async fn deliver_reviewed_handoff(
        &self,
        params: Value,
        scope: &crate::agent_runtime::Scope,
    ) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Delivery {
            connection_id: Uuid,
            thread_id: Text,
            seq: u64,
        }
        let request: Delivery = serde_json::from_value(params)?;
        let origin = scope.review_origin()?.owner;
        let selection = Selection {
            from: Address {
                connection_id: origin.connection.as_str().parse()?,
                thread_id: origin.conversation,
            },
            to: Address {
                connection_id: request.connection_id,
                thread_id: request.thread_id,
            },
            seq: request.seq,
        };
        let message = self.observe_handoff(&selection).await?;
        let directory = self.handoffs.directory()?;
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            scope.admit(|| {
                message.deliver(&directory)?;
                Ok(serde_json::to_value(message)?)
            })
        })
        .await?
    }

    pub(super) async fn handoff_history(&self, params: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Query {
            after: Option<Text>,
        }
        let query: Query = serde_json::from_value(params)?;
        let directory = self.handoffs.directory()?;
        tokio::task::spawn_blocking(move || {
            page(
                &directory,
                Visibility::Project,
                query.after.as_ref().map(|x| x.as_str()),
            )
        })
        .await?
    }

    pub(super) async fn list_handoffs(
        &self,
        params: Value,
        turn: Option<&TurnIdentity>,
    ) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Query {
            connection_id: Option<Uuid>,
            thread_id: Option<Text>,
            after: Option<Text>,
        }
        let query: Query = serde_json::from_value(params)?;
        let owner = if let Some(turn) = turn {
            ensure!(
                query.connection_id.is_none() && query.thread_id.is_none(),
                "Inbox address is host-owned"
            );
            Address {
                connection_id: turn.connection.as_str().parse()?,
                thread_id: turn.conversation.clone(),
            }
        } else {
            Address {
                connection_id: query.connection_id.context("Missing connectionId")?,
                thread_id: query.thread_id.context("Missing threadId")?,
            }
        };
        owner.validate()?;
        let directory = self.handoffs.directory()?;
        let incoming_only = turn.is_some();
        let recent = if incoming_only {
            let events = self.connection_events(owner.connection_id).await?;
            let events = events.lock().await;
            ensure!(
                events.live_thread(&owner.thread_id),
                "Conversation is no longer active"
            );
            Some(events.recent_messages(&owner.thread_id))
        } else {
            None
        };
        let mut result = tokio::task::spawn_blocking(move || {
            page(
                &directory,
                if incoming_only {
                    Visibility::Recipient(owner)
                } else {
                    Visibility::Participant(owner)
                },
                query.after.as_ref().map(|x| x.as_str()),
            )
        })
        .await??;
        if let Some(recent) = recent {
            result["recentMessages"] = json!(recent);
        }
        Ok(result)
    }
}
