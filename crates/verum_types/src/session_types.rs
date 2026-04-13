//! Session Types — typed protocols for bidirectional channels.
//!
//! A *session type* describes the sequence of sends, receives, and
//! choices that two communicating parties must perform on a shared
//! channel. Every channel end carries a session type indexing its
//! remaining protocol; the type system then enforces at compile
//! time that both ends follow compatible (dual) protocols, ruling
//! out deadlocks, protocol mismatches, and linearity violations.
//!
//! ## The session calculus
//!
//! ```text
//!     S ::= !T. S          (send value of type T, then continue as S)
//!         | ?T. S          (recv value of type T, then continue as S)
//!         | S₁ ⊕ S₂        (offer the other party a choice of S₁ or S₂)
//!         | S₁ & S₂        (select between S₁ and S₂ ourselves)
//!         | end            (protocol complete)
//! ```
//!
//! ## Duality
//!
//! Two protocols `P` and `Q` are *dual* when they perform
//! mirror-image actions:
//!
//! ```text
//!     dual(!T. S)   = ?T. dual(S)
//!     dual(?T. S)   = !T. dual(S)
//!     dual(S ⊕ T)   = dual(S) & dual(T)
//!     dual(S & T)   = dual(S) ⊕ dual(T)
//!     dual(end)     = end
//! ```
//!
//! `dual(dual(P)) = P` — duality is an involution.
//!
//! ## Usage
//!
//! Session types integrate with the rest of Verum's type system as
//! type-level descriptors on channel values. The channel's current
//! protocol advances after every send/recv/offer/select:
//!
//! ```text
//!     chan : Channel<!Int. ?Bool. end>
//!     chan.send(42)    // chan now has type Channel<?Bool. end>
//!     let b = chan.recv()  // chan now has type Channel<end>
//!     chan.close()     // matched by `end`
//! ```
//!
//! Linearity of channels (each handle used exactly once per step)
//! is enforced by the QTT usage tracker — session types compose
//! cleanly with `Quantity::One` on the channel handle.
//!
//! ## Implementation
//!
//! This module is the pure algebraic layer — `Protocol` values, the
//! `dual` function, a compatibility predicate, and a protocol
//! stepper. Integration into the surface syntax (parse
//! `Channel<!Int. ?Bool. end>`) is a future step; the current API
//! is driven programmatically by compiler phases and tests.

use verum_common::Text;

/// A payload type carried by send or receive actions. Kept abstract
/// as an opaque textual name — the structural type system handles
/// actual type checking separately.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PayloadType {
    pub name: Text,
}

impl PayloadType {
    pub fn new(name: impl Into<Text>) -> Self {
        Self { name: name.into() }
    }
}

/// The core session-type algebra.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Protocol {
    /// Send a value of `payload` type, then continue as `rest`.
    ///
    /// Written `!T. S` in standard session-calculus notation.
    Send {
        payload: PayloadType,
        rest: Box<Protocol>,
    },

    /// Receive a value of `payload` type, then continue as `rest`.
    ///
    /// Written `?T. S`.
    Recv {
        payload: PayloadType,
        rest: Box<Protocol>,
    },

    /// Offer the other party a choice between `left` and `right`.
    /// We wait for their selection before continuing.
    ///
    /// Written `S₁ ⊕ S₂` (plus).
    Offer {
        left: Box<Protocol>,
        right: Box<Protocol>,
    },

    /// Select between `left` and `right` ourselves. The other party
    /// offers (dual action).
    ///
    /// Written `S₁ & S₂` (with).
    Select {
        left: Box<Protocol>,
        right: Box<Protocol>,
    },

    /// Protocol complete — no more actions.
    End,
}

impl Protocol {
    pub fn send(payload: impl Into<Text>, rest: Protocol) -> Self {
        Protocol::Send {
            payload: PayloadType::new(payload),
            rest: Box::new(rest),
        }
    }

    pub fn recv(payload: impl Into<Text>, rest: Protocol) -> Self {
        Protocol::Recv {
            payload: PayloadType::new(payload),
            rest: Box::new(rest),
        }
    }

    pub fn offer(left: Protocol, right: Protocol) -> Self {
        Protocol::Offer {
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    pub fn select(left: Protocol, right: Protocol) -> Self {
        Protocol::Select {
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    pub fn end() -> Self {
        Protocol::End
    }

    /// Return the dual protocol. Duality is an involution:
    /// `dual(dual(P)) == P`.
    pub fn dual(&self) -> Protocol {
        match self {
            Protocol::Send { payload, rest } => Protocol::Recv {
                payload: payload.clone(),
                rest: Box::new(rest.dual()),
            },
            Protocol::Recv { payload, rest } => Protocol::Send {
                payload: payload.clone(),
                rest: Box::new(rest.dual()),
            },
            Protocol::Offer { left, right } => Protocol::Select {
                left: Box::new(left.dual()),
                right: Box::new(right.dual()),
            },
            Protocol::Select { left, right } => Protocol::Offer {
                left: Box::new(left.dual()),
                right: Box::new(right.dual()),
            },
            Protocol::End => Protocol::End,
        }
    }

    /// Two protocols are compatible iff one is the dual of the other.
    /// This is the deadlock-freedom predicate for a two-party
    /// channel: if both ends use compatible protocols, every action
    /// one side performs is matched by the other.
    pub fn compatible_with(&self, other: &Protocol) -> bool {
        &self.dual() == other
    }

    /// Is this protocol complete?
    pub fn is_end(&self) -> bool {
        matches!(self, Protocol::End)
    }

    /// Length of the protocol measured in actions (send/recv count
    /// as 1; offer/select count the *longer* branch; end is 0).
    pub fn depth(&self) -> usize {
        match self {
            Protocol::End => 0,
            Protocol::Send { rest, .. } | Protocol::Recv { rest, .. } => {
                1 + rest.depth()
            }
            Protocol::Offer { left, right }
            | Protocol::Select { left, right } => {
                1 + left.depth().max(right.depth())
            }
        }
    }

    /// Advance the protocol by one action, returning the remaining
    /// protocol after the action has occurred. Returns `Err` if
    /// the action does not match the protocol's expected next step.
    pub fn step(&self, action: &Action) -> Result<Protocol, SessionError> {
        match (self, action) {
            (Protocol::Send { payload, rest }, Action::Send { payload: a }) => {
                if payload == a {
                    Ok(*rest.clone())
                } else {
                    Err(SessionError::PayloadMismatch {
                        expected: payload.clone(),
                        actual: a.clone(),
                    })
                }
            }
            (Protocol::Recv { payload, rest }, Action::Recv { payload: a }) => {
                if payload == a {
                    Ok(*rest.clone())
                } else {
                    Err(SessionError::PayloadMismatch {
                        expected: payload.clone(),
                        actual: a.clone(),
                    })
                }
            }
            (Protocol::Offer { left, right }, Action::OfferLeft) => {
                Ok(*left.clone())
            }
            (Protocol::Offer { left, right }, Action::OfferRight) => {
                let _ = left;
                Ok(*right.clone())
            }
            (Protocol::Select { left, right }, Action::SelectLeft) => {
                let _ = right;
                Ok(*left.clone())
            }
            (Protocol::Select { left, right }, Action::SelectRight) => {
                let _ = left;
                Ok(*right.clone())
            }
            (Protocol::End, Action::Close) => Ok(Protocol::End),
            (proto, act) => Err(SessionError::ActionMismatch {
                expected_shape: proto_shape(proto),
                observed: action_shape(act),
            }),
        }
    }

    /// Chain of actions: repeatedly advance the protocol. Returns
    /// the final remaining protocol, or the first error encountered.
    pub fn run(&self, actions: &[Action]) -> Result<Protocol, SessionError> {
        let mut current = self.clone();
        for a in actions {
            current = current.step(a)?;
        }
        Ok(current)
    }
}

/// A single action performed on one end of a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Send { payload: PayloadType },
    Recv { payload: PayloadType },
    OfferLeft,
    OfferRight,
    SelectLeft,
    SelectRight,
    Close,
}

impl Action {
    pub fn send(payload: impl Into<Text>) -> Self {
        Action::Send {
            payload: PayloadType::new(payload),
        }
    }

    pub fn recv(payload: impl Into<Text>) -> Self {
        Action::Recv {
            payload: PayloadType::new(payload),
        }
    }
}

/// Errors raised when a protocol step fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    /// Action matched the expected shape but carried the wrong
    /// payload type (e.g., sent `Int` when the protocol expected
    /// `Bool`).
    PayloadMismatch {
        expected: PayloadType,
        actual: PayloadType,
    },
    /// Action shape did not match (e.g., tried to send when the
    /// protocol expected a receive).
    ActionMismatch {
        expected_shape: &'static str,
        observed: &'static str,
    },
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PayloadMismatch { expected, actual } => write!(
                f,
                "session type: expected payload {}, got {}",
                expected.name.as_str(),
                actual.name.as_str()
            ),
            Self::ActionMismatch {
                expected_shape,
                observed,
            } => write!(
                f,
                "session type: expected {} action, got {}",
                expected_shape, observed
            ),
        }
    }
}

impl std::error::Error for SessionError {}

fn proto_shape(p: &Protocol) -> &'static str {
    match p {
        Protocol::Send { .. } => "send",
        Protocol::Recv { .. } => "recv",
        Protocol::Offer { .. } => "offer (external choice)",
        Protocol::Select { .. } => "select (internal choice)",
        Protocol::End => "end",
    }
}

fn action_shape(a: &Action) -> &'static str {
    match a {
        Action::Send { .. } => "send",
        Action::Recv { .. } => "recv",
        Action::OfferLeft | Action::OfferRight => "offer-branch",
        Action::SelectLeft | Action::SelectRight => "select-branch",
        Action::Close => "close",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A small echo protocol: client sends Int, server replies Int, done.
    fn echo_client() -> Protocol {
        Protocol::send("Int", Protocol::recv("Int", Protocol::end()))
    }

    fn echo_server() -> Protocol {
        Protocol::recv("Int", Protocol::send("Int", Protocol::end()))
    }

    #[test]
    fn duality_is_involution() {
        let p = echo_client();
        assert_eq!(p.dual().dual(), p);
    }

    #[test]
    fn echo_client_and_server_are_dual() {
        let c = echo_client();
        let s = echo_server();
        assert_eq!(c.dual(), s);
        assert_eq!(s.dual(), c);
    }

    #[test]
    fn compatible_iff_dual() {
        assert!(echo_client().compatible_with(&echo_server()));
        assert!(echo_server().compatible_with(&echo_client()));

        // Client is not compatible with itself — it would try to
        // both send at once, deadlock.
        assert!(!echo_client().compatible_with(&echo_client()));
    }

    #[test]
    fn end_duals_to_end() {
        assert_eq!(Protocol::End.dual(), Protocol::End);
    }

    #[test]
    fn offer_duals_to_select() {
        let p = Protocol::offer(
            Protocol::recv("A", Protocol::end()),
            Protocol::recv("B", Protocol::end()),
        );
        match p.dual() {
            Protocol::Select { left, right } => {
                assert!(matches!(*left, Protocol::Send { .. }));
                assert!(matches!(*right, Protocol::Send { .. }));
            }
            _ => panic!("offer's dual must be select"),
        }
    }

    #[test]
    fn depth_counts_longest_branch() {
        let p = Protocol::offer(
            Protocol::send("A", Protocol::send("B", Protocol::end())), // depth 2
            Protocol::recv("C", Protocol::end()),                       // depth 1
        );
        assert_eq!(p.depth(), 3); // 1 for the offer + max(2, 1)
    }

    #[test]
    fn step_send_advances() {
        let p = echo_client();
        let after = p.step(&Action::send("Int")).unwrap();
        // Remaining protocol: recv Int, then end.
        assert_eq!(
            after,
            Protocol::recv("Int", Protocol::end())
        );
    }

    #[test]
    fn step_recv_advances() {
        let p = echo_server();
        let after = p.step(&Action::recv("Int")).unwrap();
        assert_eq!(
            after,
            Protocol::send("Int", Protocol::end())
        );
    }

    #[test]
    fn step_with_wrong_payload_errors() {
        let p = echo_client();
        let err = p.step(&Action::send("Bool")).unwrap_err();
        assert!(matches!(err, SessionError::PayloadMismatch { .. }));
    }

    #[test]
    fn step_with_wrong_shape_errors() {
        let p = echo_client(); // expects send first
        let err = p.step(&Action::recv("Int")).unwrap_err();
        assert!(matches!(err, SessionError::ActionMismatch { .. }));
    }

    #[test]
    fn full_echo_client_run_reaches_end() {
        let p = echo_client();
        let done = p
            .run(&[Action::send("Int"), Action::recv("Int"), Action::Close])
            .unwrap();
        assert!(done.is_end());
    }

    #[test]
    fn full_echo_server_run_reaches_end() {
        let p = echo_server();
        let done = p
            .run(&[Action::recv("Int"), Action::send("Int"), Action::Close])
            .unwrap();
        assert!(done.is_end());
    }

    #[test]
    fn offer_left_branch_steps_correctly() {
        let p = Protocol::offer(
            Protocol::recv("A", Protocol::end()),
            Protocol::recv("B", Protocol::end()),
        );
        let after = p.step(&Action::OfferLeft).unwrap();
        assert_eq!(after, Protocol::recv("A", Protocol::end()));
    }

    #[test]
    fn select_right_branch_steps_correctly() {
        let p = Protocol::select(
            Protocol::send("A", Protocol::end()),
            Protocol::send("B", Protocol::end()),
        );
        let after = p.step(&Action::SelectRight).unwrap();
        assert_eq!(after, Protocol::send("B", Protocol::end()));
    }

    #[test]
    fn three_step_protocol_progression() {
        // send Int, recv Bool, send Text, end
        let p = Protocol::send(
            "Int",
            Protocol::recv(
                "Bool",
                Protocol::send("Text", Protocol::end()),
            ),
        );

        let s1 = p.step(&Action::send("Int")).unwrap();
        assert!(matches!(s1, Protocol::Recv { .. }));

        let s2 = s1.step(&Action::recv("Bool")).unwrap();
        assert!(matches!(s2, Protocol::Send { .. }));

        let s3 = s2.step(&Action::send("Text")).unwrap();
        assert!(s3.is_end());
    }

    #[test]
    fn choice_protocols_dualize_correctly() {
        // Client: select between sending Int or Bool.
        // Server: offer to receive Int or Bool.
        let client = Protocol::select(
            Protocol::send("Int", Protocol::end()),
            Protocol::send("Bool", Protocol::end()),
        );
        let server = Protocol::offer(
            Protocol::recv("Int", Protocol::end()),
            Protocol::recv("Bool", Protocol::end()),
        );
        assert!(client.compatible_with(&server));
    }

    #[test]
    fn deep_duality_involution() {
        let deep = Protocol::send(
            "A",
            Protocol::recv(
                "B",
                Protocol::offer(
                    Protocol::send("C", Protocol::end()),
                    Protocol::recv("D", Protocol::end()),
                ),
            ),
        );
        assert_eq!(deep.dual().dual(), deep);
    }
}
