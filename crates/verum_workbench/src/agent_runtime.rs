//! Provider-independent ownership of a turn and its reviewed operations.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::watch;
use verum_common::{List, Map, Text};

#[cfg(all(test, unix))]
#[path = "agent_runtime_tests.rs"]
mod tests;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TurnIdentity {
    pub connection: Text,
    pub conversation: Text,
    pub turn: Text,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum End {
    Interrupted,
    Completed,
    Disconnected,
    Retired,
    Resolved,
    Abandoned,
}
impl End {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::Interrupted => "Agent turn interrupted",
            Self::Completed => "Agent turn completed",
            Self::Disconnected => "Agent provider disconnected",
            Self::Retired => "Agent conversation retired",
            Self::Resolved => "Provider withdrew the reviewed request",
            Self::Abandoned => "Agent request abandoned before delivery",
        }
    }
}

struct State {
    ended: Option<End>,
    children: List<Weak<Inner>>,
}
struct Inner {
    identity: TurnIdentity,
    parent: Option<Scope>,
    state: Mutex<State>,
    ended: watch::Sender<Option<End>>,
}
#[derive(Clone)]
pub(crate) struct Scope(Arc<Inner>);
impl Scope {
    fn new(identity: TurnIdentity, parent: Option<Self>) -> Self {
        Self(Arc::new(Inner {
            identity,
            parent,
            state: Mutex::new(State {
                ended: None,
                children: List::new(),
            }),
            ended: watch::channel(None).0,
        }))
    }

    pub(crate) fn identity(&self) -> &TurnIdentity {
        &self.0.identity
    }

    /// Mutation admission and cancellation share a lock. Callers must acquire
    /// async resource locks first and perform only synchronous work inside it.
    pub(crate) fn admit<T>(&self, action: impl FnOnce() -> Result<T>) -> Result<T> {
        let mut action = Some(action);
        self.admit_inner(&mut || action.take().expect("admission runs once")())
    }
    fn admit_inner<T>(&self, action: &mut dyn FnMut() -> Result<T>) -> Result<T> {
        let mut local = || {
            let state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
            ensure!(state.ended.is_none(), "Agent operation has expired");
            action()
        };
        match &self.0.parent {
            Some(parent) => parent.admit_inner(&mut local),
            None => local(),
        }
    }

    pub(crate) fn child(&self) -> Result<Self> {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        ensure!(state.ended.is_none(), "Agent turn has expired");
        state.children.retain(|child| child.strong_count() != 0);
        let child = Self::new(self.0.identity.clone(), Some(self.clone()));
        state.children.push(Arc::downgrade(&child.0));
        Ok(child)
    }

    pub(crate) fn end(&self, reason: End) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.ended.is_some() {
            return;
        }
        state.ended = Some(reason);
        self.0.ended.send_replace(Some(reason));
        for child in state.children.drain(..).filter_map(|child| child.upgrade()) {
            Self(child).end(reason);
        }
    }

    pub(crate) async fn cancelled(&self) -> End {
        let mut receiver = self.0.ended.subscribe();
        loop {
            if let Some(reason) = *receiver.borrow_and_update() {
                return reason;
            }
            if receiver.changed().await.is_err() {
                return End::Disconnected;
            }
        }
    }

    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub(crate) fn guard(&self) -> RequestGuard {
        RequestGuard(self.clone())
    }
}

/// Dropping an HTTP responder must not detach the operation it approved.
pub(crate) struct RequestGuard(Scope);
impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.0.end(End::Abandoned);
    }
}

struct Registry {
    closed: bool,
    turns: Map<(Text, Text), Scope>,
}
#[derive(Clone)]
pub(crate) struct Turns {
    connection: Text,
    registry: Arc<Mutex<Registry>>,
}
impl Turns {
    pub(crate) fn new(connection: uuid::Uuid) -> Self {
        Self {
            connection: connection.to_string().into(),
            registry: Arc::new(Mutex::new(Registry {
                closed: false,
                turns: Map::new(),
            })),
        }
    }

    /// Retain ended IDs until disconnect: late calls cannot resurrect a turn.
    pub(crate) fn get(&self, conversation: &str, turn: &str) -> Result<Scope> {
        ensure!(
            !conversation.is_empty() && conversation.len() <= 1024,
            "Invalid provider conversation ID"
        );
        ensure!(
            !turn.is_empty() && turn.len() <= 1024,
            "Invalid provider turn ID"
        );
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        ensure!(!registry.closed, "Provider connection has expired");
        let key = (Text::from(conversation), Text::from(turn));
        if let Some(scope) = registry.turns.get(&key) {
            return Ok(scope.clone());
        }
        ensure!(
            registry.turns.len() < 4096,
            "Provider turn limit reached; reconnect before continuing"
        );
        let scope = Scope::new(
            TurnIdentity {
                connection: self.connection.clone(),
                conversation: key.0.clone(),
                turn: key.1.clone(),
            },
            None,
        );
        registry.turns.insert(key, scope.clone());
        Ok(scope)
    }

    pub(crate) fn end(&self, conversation: &str, turn: &str, reason: End) -> Result<()> {
        self.get(conversation, turn)?.end(reason);
        Ok(())
    }
    pub(crate) fn retire(&self, conversation: &str) {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        for ((owner, _), scope) in &registry.turns {
            if owner == conversation {
                scope.end(End::Retired);
            }
        }
    }
    pub(crate) fn disconnect(&self) {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry.closed = true;
        for scope in registry.turns.values() {
            scope.end(End::Disconnected);
        }
    }
}
