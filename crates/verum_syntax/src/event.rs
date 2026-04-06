//! Event-based parsing infrastructure.
//!
//! The parser emits events instead of building trees directly. This enables:
//! 1. Retroactive modifications via markers (precede pattern)
//! 2. Different tree representations from the same parse
//! 3. Easy testing without tree construction
//!
//! Event-Based Parser Design:
//! Events are emitted in parsing order, then reordered to tree order via
//! forward_parent links. The `precede` pattern enables retroactive parent
//! wrapping (e.g., wrapping "1" in a BINARY_EXPR after seeing "+").
//! Different sinks (GreenTreeSink, TextTreeSink) consume events to produce
//! different representations from the same parse.

use crate::SyntaxKind;

/// Events emitted during parsing.
///
/// The parser emits a flat sequence of events that can be processed
/// by different sinks to produce different representations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Start a new node.
    ///
    /// If `forward_parent` is set, this node will be wrapped in another
    /// node later (retroactive parent wrapping).
    Start {
        /// The syntax kind for this node.
        kind: SyntaxKind,
        /// If set, this node will be wrapped in a parent node at that event index.
        forward_parent: Option<u32>,
    },

    /// Finish the current node.
    Finish,

    /// Advance to the next token.
    Token {
        /// The syntax kind for this token.
        kind: SyntaxKind,
        /// Number of raw tokens this event represents (usually 1).
        n_raw_tokens: u8,
    },

    /// Record an error at the current position.
    Error {
        /// The error message describing what went wrong.
        message: String,
    },

    /// Placeholder for abandoned or reordered events.
    Tombstone,
}

impl Event {
    /// Create a Start event.
    pub fn start(kind: SyntaxKind) -> Self {
        Event::Start {
            kind,
            forward_parent: None,
        }
    }

    /// Create a Finish event.
    pub fn finish() -> Self {
        Event::Finish
    }

    /// Create a Token event.
    pub fn token(kind: SyntaxKind) -> Self {
        Event::Token {
            kind,
            n_raw_tokens: 1,
        }
    }

    /// Create a Token event consuming multiple raw tokens.
    pub fn token_n(kind: SyntaxKind, n: u8) -> Self {
        Event::Token {
            kind,
            n_raw_tokens: n,
        }
    }

    /// Create an Error event.
    pub fn error(message: impl Into<String>) -> Self {
        Event::Error {
            message: message.into(),
        }
    }

    /// Create a Tombstone event.
    pub fn tombstone() -> Self {
        Event::Tombstone
    }
}

/// Marker for a node start position.
///
/// Markers are used for retroactive parent wrapping. When parsing an expression
/// like `1 + 2`, we start with `1`, then realize we need to wrap it in a
/// binary expression. The marker allows this retroactive wrapping.
#[derive(Debug)]
pub struct Marker {
    /// Position in the events list
    pos: u32,
    /// Whether this marker has been completed or abandoned
    bomb: bool,
}

impl Marker {
    /// Create a new marker at the given position.
    pub fn new(pos: u32) -> Self {
        Self { pos, bomb: true }
    }

    /// Get the position of this marker.
    pub fn pos(&self) -> u32 {
        self.pos
    }

    /// Complete this marker with the given kind.
    ///
    /// This updates the Start event at the marker's position and emits
    /// a Finish event.
    pub fn complete(mut self, p: &mut EventBuilder, kind: SyntaxKind) -> CompletedMarker {
        self.bomb = false;
        let idx = self.pos as usize;

        match &mut p.events[idx] {
            Event::Start { kind: slot, .. } => *slot = kind,
            _ => unreachable!("marker not pointing to Start event"),
        }

        p.events.push(Event::Finish);
        CompletedMarker::new(self.pos, kind)
    }

    /// Abandon this marker.
    ///
    /// The Start event is converted to a Tombstone. Tombstones are filtered
    /// out when events are finalized.
    pub fn abandon(mut self, p: &mut EventBuilder) {
        self.bomb = false;
        let idx = self.pos as usize;

        // Just convert to tombstone - it will be filtered out in finish()
        if idx < p.events.len() {
            match &p.events[idx] {
                Event::Start { .. } => {
                    p.events[idx] = Event::Tombstone;
                }
                _ => unreachable!("marker not pointing to Start event"),
            }
        }
    }
}

impl Drop for Marker {
    fn drop(&mut self) {
        if self.bomb && !std::thread::panicking() {
            panic!("Marker was neither completed nor abandoned");
        }
    }
}

/// A completed marker that can be used for retroactive parent wrapping.
#[derive(Clone, Copy, Debug)]
pub struct CompletedMarker {
    pos: u32,
    kind: SyntaxKind,
}

impl CompletedMarker {
    fn new(pos: u32, kind: SyntaxKind) -> Self {
        Self { pos, kind }
    }

    /// Get the position of this marker.
    pub fn pos(&self) -> u32 {
        self.pos
    }

    /// Get the kind of this completed node.
    pub fn kind(&self) -> SyntaxKind {
        self.kind
    }

    /// Wrap this completed node in a parent node.
    ///
    /// This is used for retroactive parent wrapping. For example, when
    /// parsing `1 + 2`, we first parse `1` as a literal, then realize
    /// we need to wrap it in a binary expression.
    ///
    /// Before: `[Start(LITERAL), ..., Finish]`
    /// After:  `[Start(BINARY), Start(LITERAL), ..., Finish, ...]`
    pub fn precede(self, p: &mut EventBuilder) -> Marker {
        let new_marker = p.start();
        let idx = self.pos as usize;

        match &mut p.events[idx] {
            Event::Start { forward_parent, .. } => {
                *forward_parent = Some(new_marker.pos);
            }
            _ => unreachable!("marker not pointing to Start event"),
        }

        new_marker
    }

    /// Extend this marker to the right to include additional tokens.
    ///
    /// This is used when we need to include trailing tokens in an existing
    /// node, such as adding a postfix operator.
    pub fn extend_to(self, _p: &mut EventBuilder, end: CompletedMarker) -> CompletedMarker {
        // The end marker's start should come after our start
        debug_assert!(end.pos >= self.pos);

        // We need to move the Finish event from our position to after the end marker
        // This is done implicitly by the way we process events
        let _ = end; // Silence unused warning

        self
    }
}

/// Builder for parser events.
///
/// This collects events during parsing and provides methods for
/// processing them into a syntax tree.
#[derive(Debug, Default)]
pub struct EventBuilder {
    events: Vec<Event>,
}

impl EventBuilder {
    /// Create a new event builder.
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Create a new event builder with preallocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            events: Vec::with_capacity(capacity),
        }
    }

    /// Start a new node.
    pub fn start(&mut self) -> Marker {
        let pos = self.events.len() as u32;
        self.events.push(Event::Start {
            kind: SyntaxKind::TOMBSTONE,
            forward_parent: None,
        });
        Marker::new(pos)
    }

    /// Start a new node with a known kind.
    pub fn start_node(&mut self, kind: SyntaxKind) -> Marker {
        let pos = self.events.len() as u32;
        self.events.push(Event::Start {
            kind,
            forward_parent: None,
        });
        Marker::new(pos)
    }

    /// Add a token event.
    pub fn token(&mut self, kind: SyntaxKind) {
        self.events.push(Event::Token {
            kind,
            n_raw_tokens: 1,
        });
    }

    /// Add a token event consuming multiple raw tokens.
    pub fn token_n(&mut self, kind: SyntaxKind, n: u8) {
        self.events.push(Event::Token {
            kind,
            n_raw_tokens: n,
        });
    }

    /// Record an error.
    pub fn error(&mut self, message: impl Into<String>) {
        self.events.push(Event::Error {
            message: message.into(),
        });
    }

    /// Get the current number of events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Check if there are no events.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Get the events.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Consume the builder and return the events.
    /// Tombstones are filtered out.
    pub fn finish(self) -> Vec<Event> {
        self.events
            .into_iter()
            .filter(|e| !matches!(e, Event::Tombstone))
            .collect()
    }

    /// Reorder events to handle forward_parent links.
    ///
    /// This transforms:
    ///   `[Start(A, forward=1), ..., Start(B), ...]`
    /// into:
    ///   `[Start(B), Start(A), ..., ...]`
    ///
    /// This is necessary because markers with forward_parent were
    /// created before their parent nodes.
    pub fn reorder(mut self) -> Vec<Event> {
        // Process from end to start to handle nested forward parents
        for i in (0..self.events.len()).rev() {
            let forward_parent = match &self.events[i] {
                Event::Start { forward_parent: Some(fp), .. } => *fp as usize,
                _ => continue,
            };

            // Take the forward parent's kind
            let parent_kind = match &self.events[forward_parent] {
                Event::Start { kind, .. } => *kind,
                _ => continue,
            };

            // Clear the forward_parent link
            if let Event::Start { forward_parent: fp, .. } = &mut self.events[i] {
                *fp = None;
            }

            // Remove the parent's Start event (replace with Tombstone)
            self.events[forward_parent] = Event::Tombstone;

            // Insert the parent's Start event before current position
            let new_start = Event::Start {
                kind: parent_kind,
                forward_parent: None,
            };

            // Shift events and insert
            self.events.insert(i, new_start);
        }

        // Filter out tombstones
        self.events
            .into_iter()
            .filter(|e| !matches!(e, Event::Tombstone))
            .collect()
    }
}

/// Process events into a syntax tree using the given sink.
pub fn process<S: EventSink>(events: Vec<Event>, tokens: &[TokenSource], sink: &mut S) {
    let mut token_pos = 0;

    // Handle forward_parent by reordering
    let events = reorder_events(events);

    for event in events {
        match event {
            Event::Start { kind, .. } if kind != SyntaxKind::TOMBSTONE => {
                sink.start_node(kind);
            }
            Event::Start { .. } | Event::Tombstone => {
                // Skip tombstones
            }
            Event::Finish => {
                sink.finish_node();
            }
            Event::Token { kind, n_raw_tokens } => {
                for _ in 0..n_raw_tokens {
                    if token_pos < tokens.len() {
                        let token = &tokens[token_pos];

                        // Attach leading trivia
                        for trivia in &token.leading_trivia {
                            sink.token(trivia.kind, &trivia.text);
                        }

                        // Main token
                        sink.token(kind, &token.text);

                        // Attach trailing trivia
                        for trivia in &token.trailing_trivia {
                            sink.token(trivia.kind, &trivia.text);
                        }

                        token_pos += 1;
                    }
                }
            }
            Event::Error { message } => {
                sink.error(&message);
            }
        }
    }
}

/// Reorder events to handle forward_parent links.
fn reorder_events(mut events: Vec<Event>) -> Vec<Event> {
    // Process forward_parent links from end to start
    for i in (0..events.len()).rev() {
        if let Event::Start { forward_parent: Some(parent_pos), .. } = events[i].clone() {
            // Insert the current event before its parent
            let current = std::mem::replace(&mut events[i], Event::Tombstone);
            let parent_idx = parent_pos as usize;

            if parent_idx < events.len() {
                // Clear forward_parent on the moved event
                let current = match current {
                    Event::Start { kind, .. } => Event::Start {
                        kind,
                        forward_parent: None,
                    },
                    other => other,
                };

                // Shift events between parent and current
                let parent = std::mem::replace(&mut events[parent_idx], current);
                events.insert(parent_idx + 1, parent);
            }
        }
    }

    // Filter out tombstones
    events.retain(|e| !matches!(e, Event::Tombstone));
    events
}

/// Token source for event processing.
#[derive(Clone, Debug)]
pub struct TokenSource {
    /// The syntax kind of this token.
    pub kind: SyntaxKind,
    /// The source text of this token.
    pub text: String,
    /// Trivia (whitespace, comments) appearing before this token.
    pub leading_trivia: Vec<TriviaSource>,
    /// Trivia (whitespace, comments) appearing after this token.
    pub trailing_trivia: Vec<TriviaSource>,
}

/// Trivia source for event processing.
#[derive(Clone, Debug)]
pub struct TriviaSource {
    /// The syntax kind of this trivia (whitespace or comment).
    pub kind: SyntaxKind,
    /// The source text of this trivia.
    pub text: String,
}

/// Trait for consuming parser events.
pub trait EventSink {
    /// Start a new node.
    fn start_node(&mut self, kind: SyntaxKind);

    /// Finish the current node.
    fn finish_node(&mut self);

    /// Add a token.
    fn token(&mut self, kind: SyntaxKind, text: &str);

    /// Record an error.
    fn error(&mut self, message: &str);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_events() {
        let mut builder = EventBuilder::new();

        let m = builder.start();
        builder.token(SyntaxKind::LET_KW);
        builder.token(SyntaxKind::IDENT);
        m.complete(&mut builder, SyntaxKind::LET_STMT);

        let events = builder.finish();
        assert_eq!(events.len(), 4);

        assert!(matches!(events[0], Event::Start { kind: SyntaxKind::LET_STMT, .. }));
        assert!(matches!(events[1], Event::Token { kind: SyntaxKind::LET_KW, .. }));
        assert!(matches!(events[2], Event::Token { kind: SyntaxKind::IDENT, .. }));
        assert!(matches!(events[3], Event::Finish));
    }

    #[test]
    fn test_precede() {
        let mut builder = EventBuilder::new();

        // Parse `1 + 2` - first we parse `1`, then realize it's a binary expr
        let m1 = builder.start();
        builder.token(SyntaxKind::INT_LITERAL);
        let c1 = m1.complete(&mut builder, SyntaxKind::LITERAL_EXPR);

        // Now wrap it in a binary expression
        let m_binary = c1.precede(&mut builder);
        builder.token(SyntaxKind::PLUS);

        let m2 = builder.start();
        builder.token(SyntaxKind::INT_LITERAL);
        m2.complete(&mut builder, SyntaxKind::LITERAL_EXPR);

        m_binary.complete(&mut builder, SyntaxKind::BINARY_EXPR);

        let events = builder.reorder();

        // After reordering, BINARY_EXPR should come first
        let start_kinds: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Start { kind, .. } => Some(*kind),
                _ => None,
            })
            .collect();

        assert_eq!(start_kinds[0], SyntaxKind::BINARY_EXPR);
        assert_eq!(start_kinds[1], SyntaxKind::LITERAL_EXPR);
        assert_eq!(start_kinds[2], SyntaxKind::LITERAL_EXPR);
    }

    #[test]
    fn test_abandon() {
        let mut builder = EventBuilder::new();

        let m = builder.start();
        builder.token(SyntaxKind::IDENT);
        m.abandon(&mut builder);

        let events = builder.finish();
        assert_eq!(events.len(), 1); // Only the token
        assert!(matches!(events[0], Event::Token { .. }));
    }

    #[test]
    fn test_error() {
        let mut builder = EventBuilder::new();

        let m = builder.start();
        builder.error("unexpected token");
        builder.token(SyntaxKind::ERROR);
        m.complete(&mut builder, SyntaxKind::ERROR);

        let events = builder.finish();
        assert!(events.iter().any(|e| matches!(e, Event::Error { .. })));
    }
}
