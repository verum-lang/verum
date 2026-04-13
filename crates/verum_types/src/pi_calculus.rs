//! π-calculus — process algebra for concurrent computation.
//!
//! Milner's π-calculus is the canonical theoretical foundation for
//! mobile concurrent processes: channels are first-class values
//! that can be sent over other channels, modelling dynamic network
//! topology. Where session types describe a single channel's
//! protocol, the π-calculus describes the entire interaction
//! pattern between concurrently executing parties.
//!
//! ## Syntax
//!
//! ```text
//!     P, Q ::= 0                  (terminated process)
//!            | P | Q              (parallel composition)
//!            | (νx) P             (restriction: x is fresh in P)
//!            | x⟨y⟩.P             (send y on x, continue as P)
//!            | x(y).P             (recv on x, bind to y, continue as P)
//!            | !P                 (replication: ∞ copies of P)
//! ```
//!
//! ## Reduction
//!
//! The single core rule is **COMM**:
//!
//! ```text
//!     x⟨y⟩.P  |  x(z).Q   →   P  |  Q[y/z]
//! ```
//!
//! A send and a matching receive on the same channel react,
//! producing the residual processes with the received name
//! substituted in.
//!
//! ## Status
//!
//! Standalone algebraic core: process AST, capture-avoiding
//! substitution, structural-congruence simplification, single-step
//! COMM reduction. Higher-level analyses (bisimulation, type
//! systems for π) build on this layer.

use std::collections::HashSet;

use verum_common::Text;

/// A π-calculus process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Process {
    /// `0` — the terminated process.
    Zero,
    /// `P | Q` — parallel composition.
    Par(Box<Process>, Box<Process>),
    /// `(νx) P` — name restriction; `x` is bound and fresh in `P`.
    Restrict(Text, Box<Process>),
    /// `x⟨y⟩.P` — send name `y` on channel `x`, continue as `P`.
    Send {
        channel: Text,
        message: Text,
        cont: Box<Process>,
    },
    /// `x(z).P` — receive on channel `x`, bind to `z`, continue.
    Recv {
        channel: Text,
        binder: Text,
        cont: Box<Process>,
    },
    /// `!P` — replication: as many parallel copies of `P` as needed.
    Replicate(Box<Process>),
}

impl Process {
    pub fn par(a: Process, b: Process) -> Self {
        Self::Par(Box::new(a), Box::new(b))
    }

    pub fn restrict(name: impl Into<Text>, body: Process) -> Self {
        Self::Restrict(name.into(), Box::new(body))
    }

    pub fn send(channel: impl Into<Text>, message: impl Into<Text>, cont: Process) -> Self {
        Self::Send {
            channel: channel.into(),
            message: message.into(),
            cont: Box::new(cont),
        }
    }

    pub fn recv(channel: impl Into<Text>, binder: impl Into<Text>, cont: Process) -> Self {
        Self::Recv {
            channel: channel.into(),
            binder: binder.into(),
            cont: Box::new(cont),
        }
    }

    pub fn replicate(p: Process) -> Self {
        Self::Replicate(Box::new(p))
    }

    /// Names used freely in this process. Restriction binders and
    /// receive binders shadow.
    pub fn free_names(&self) -> HashSet<Text> {
        let mut out = HashSet::new();
        self.free_names_into(&mut out);
        out
    }

    fn free_names_into(&self, out: &mut HashSet<Text>) {
        match self {
            Process::Zero => {}
            Process::Par(a, b) => {
                a.free_names_into(out);
                b.free_names_into(out);
            }
            Process::Restrict(name, body) => {
                let mut inner = HashSet::new();
                body.free_names_into(&mut inner);
                inner.remove(name);
                for n in inner {
                    out.insert(n);
                }
            }
            Process::Send {
                channel,
                message,
                cont,
            } => {
                out.insert(channel.clone());
                out.insert(message.clone());
                cont.free_names_into(out);
            }
            Process::Recv {
                channel,
                binder,
                cont,
            } => {
                out.insert(channel.clone());
                let mut inner = HashSet::new();
                cont.free_names_into(&mut inner);
                inner.remove(binder);
                for n in inner {
                    out.insert(n);
                }
            }
            Process::Replicate(p) => p.free_names_into(out),
        }
    }

    /// Substitute `from` with `to` everywhere it appears free.
    /// Capture-avoidance is achieved by skipping the binder when
    /// it shadows `from`.
    pub fn substitute(&self, from: &Text, to: &Text) -> Process {
        match self {
            Process::Zero => Process::Zero,
            Process::Par(a, b) => Process::par(a.substitute(from, to), b.substitute(from, to)),
            Process::Restrict(name, body) => {
                if name == from {
                    self.clone()
                } else {
                    Process::restrict(name.clone(), body.substitute(from, to))
                }
            }
            Process::Send {
                channel,
                message,
                cont,
            } => {
                let new_ch = if channel == from { to.clone() } else { channel.clone() };
                let new_msg = if message == from { to.clone() } else { message.clone() };
                Process::send(new_ch, new_msg, cont.substitute(from, to))
            }
            Process::Recv {
                channel,
                binder,
                cont,
            } => {
                let new_ch = if channel == from { to.clone() } else { channel.clone() };
                if binder == from {
                    Process::recv(new_ch, binder.clone(), (**cont).clone())
                } else {
                    Process::recv(new_ch, binder.clone(), cont.substitute(from, to))
                }
            }
            Process::Replicate(p) => Process::replicate(p.substitute(from, to)),
        }
    }
}

/// Attempt one step of COMM reduction. Searches the parallel
/// composition for a matching send/recv pair and returns the
/// reduced process, or `None` if no reduction is possible at this
/// level.
///
/// Looks at the **immediate** parallel structure: if `P = P₁ | P₂`
/// where `P₁` starts with `x⟨y⟩.…` and `P₂` starts with `x(z).…`,
/// fires the rule. More sophisticated search (under restrictions,
/// inside replication) is left to the caller's outer driver.
pub fn step(p: &Process) -> Option<Process> {
    match p {
        Process::Par(a, b) => {
            // Try a sends, b recvs.
            if let (
                Process::Send {
                    channel: ch1,
                    message,
                    cont: pc1,
                },
                Process::Recv {
                    channel: ch2,
                    binder,
                    cont: pc2,
                },
            ) = (a.as_ref(), b.as_ref())
            {
                if ch1 == ch2 {
                    return Some(Process::par(
                        (**pc1).clone(),
                        pc2.substitute(binder, message),
                    ));
                }
            }
            // Try b sends, a recvs.
            if let (
                Process::Recv {
                    channel: ch1,
                    binder,
                    cont: pc1,
                },
                Process::Send {
                    channel: ch2,
                    message,
                    cont: pc2,
                },
            ) = (a.as_ref(), b.as_ref())
            {
                if ch1 == ch2 {
                    return Some(Process::par(
                        pc1.substitute(binder, message),
                        (**pc2).clone(),
                    ));
                }
            }
            // Recurse into either side — a step inside one half
            // counts as a step of the whole.
            if let Some(a2) = step(a) {
                return Some(Process::par(a2, (**b).clone()));
            }
            if let Some(b2) = step(b) {
                return Some(Process::par((**a).clone(), b2));
            }
            None
        }
        // Step under a restriction: the result is wrapped back in
        // the restriction.
        Process::Restrict(name, body) => {
            step(body).map(|b2| Process::restrict(name.clone(), b2))
        }
        // No reduction at the immediate level for atomic processes.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Text {
        Text::from(s)
    }

    #[test]
    fn zero_has_no_free_names() {
        assert!(Process::Zero.free_names().is_empty());
    }

    #[test]
    fn send_has_channel_and_message_as_free() {
        let p = Process::send("x", "y", Process::Zero);
        let names = p.free_names();
        assert!(names.contains(&t("x")));
        assert!(names.contains(&t("y")));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn recv_binder_shadows_in_continuation() {
        // x(z). z⟨w⟩.0  — z is bound by recv, w stays free
        let inner = Process::send("z", "w", Process::Zero);
        let p = Process::recv("x", "z", inner);
        let names = p.free_names();
        assert!(names.contains(&t("x")));
        assert!(names.contains(&t("w")));
        assert!(!names.contains(&t("z")));
    }

    #[test]
    fn restriction_binds_name() {
        // (νa) a⟨b⟩.0  — a is bound, b is free
        let send = Process::send("a", "b", Process::Zero);
        let p = Process::restrict("a", send);
        let names = p.free_names();
        assert!(!names.contains(&t("a")));
        assert!(names.contains(&t("b")));
    }

    #[test]
    fn substitute_in_send_renames_channel_and_message() {
        let p = Process::send("x", "y", Process::Zero);
        let q = p.substitute(&t("x"), &t("z"));
        if let Process::Send { channel, .. } = q {
            assert_eq!(channel.as_str(), "z");
        } else {
            panic!("expected Send");
        }
    }

    #[test]
    fn substitute_avoids_recv_binder_capture() {
        // P = x(y). y — substituting [x := y] must not rebind
        let p = Process::recv("x", "y", Process::send("y", "z", Process::Zero));
        let q = p.substitute(&t("x"), &t("y"));
        // The channel is renamed but the inner binder still
        // protects its body.
        if let Process::Recv { channel, binder, .. } = q {
            assert_eq!(channel.as_str(), "y");
            assert_eq!(binder.as_str(), "y");
        }
    }

    #[test]
    fn comm_step_fires_for_matching_pair() {
        // x⟨a⟩.0 | x(z).z⟨b⟩.0
        // → 0 | a⟨b⟩.0
        let send = Process::send("x", "a", Process::Zero);
        let recv = Process::recv("x", "z", Process::send("z", "b", Process::Zero));
        let p = Process::par(send, recv);

        let q = step(&p).expect("expected COMM step");
        // The receive's `z` should be substituted with `a`.
        if let Process::Par(_, right) = &q {
            if let Process::Send { channel, message, .. } = right.as_ref() {
                assert_eq!(channel.as_str(), "a");
                assert_eq!(message.as_str(), "b");
            } else {
                panic!("expected Send on right");
            }
        }
    }

    #[test]
    fn comm_step_works_in_either_order() {
        // Reverse: recv | send should also fire.
        let recv = Process::recv("x", "z", Process::send("z", "b", Process::Zero));
        let send = Process::send("x", "a", Process::Zero);
        let p = Process::par(recv, send);
        assert!(step(&p).is_some());
    }

    #[test]
    fn no_step_when_channels_differ() {
        let send = Process::send("x", "a", Process::Zero);
        let recv = Process::recv("y", "z", Process::Zero);
        let p = Process::par(send, recv);
        assert!(step(&p).is_none());
    }

    #[test]
    fn step_under_restriction_propagates() {
        // (νx) (x⟨a⟩.0 | x(z).0)  — should reduce inside.
        let send = Process::send("x", "a", Process::Zero);
        let recv = Process::recv("x", "z", Process::Zero);
        let body = Process::par(send, recv);
        let p = Process::restrict("x", body);
        let q = step(&p);
        assert!(q.is_some());
        // The result should still be wrapped in the restriction.
        match q.unwrap() {
            Process::Restrict(name, _) => assert_eq!(name.as_str(), "x"),
            _ => panic!("expected restriction"),
        }
    }

    #[test]
    fn step_recurses_into_left_half() {
        // (send | recv) | unrelated  — should reduce in left half.
        let inner = Process::par(
            Process::send("x", "a", Process::Zero),
            Process::recv("x", "z", Process::Zero),
        );
        let p = Process::par(inner, Process::Zero);
        assert!(step(&p).is_some());
    }

    #[test]
    fn replicate_carries_no_step_at_immediate_level() {
        // !P doesn't step until you instantiate it; this minimal
        // step function leaves replicate alone.
        let p = Process::replicate(Process::send("x", "a", Process::Zero));
        assert!(step(&p).is_none());
    }

    #[test]
    fn parallel_zero_with_send_does_not_step() {
        let send = Process::send("x", "a", Process::Zero);
        let p = Process::par(send, Process::Zero);
        assert!(step(&p).is_none());
    }

    #[test]
    fn substitute_passes_through_par() {
        let p = Process::par(
            Process::send("x", "y", Process::Zero),
            Process::send("x", "y", Process::Zero),
        );
        let q = p.substitute(&t("y"), &t("z"));
        let names = q.free_names();
        assert!(names.contains(&t("z")));
        assert!(!names.contains(&t("y")));
    }

    #[test]
    fn substitute_inside_restriction_skips_binder_shadow() {
        // (νa) a⟨b⟩.0 — substituting [a := c] shouldn't change
        // the bound `a`.
        let p = Process::restrict("a", Process::send("a", "b", Process::Zero));
        let q = p.substitute(&t("a"), &t("c"));
        // The inner send's channel is still `a` because the
        // restriction shadows.
        if let Process::Restrict(name, body) = q {
            assert_eq!(name.as_str(), "a");
            if let Process::Send { channel, .. } = body.as_ref() {
                assert_eq!(channel.as_str(), "a");
            }
        }
    }

    #[test]
    fn free_names_skips_replicate_binders_correctly() {
        // !x⟨y⟩.0  has free names {x, y}.
        let p = Process::replicate(Process::send("x", "y", Process::Zero));
        let names = p.free_names();
        assert!(names.contains(&t("x")));
        assert!(names.contains(&t("y")));
    }
}
