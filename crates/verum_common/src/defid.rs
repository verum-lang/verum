//! **Nominal identity — `DefId`** (NOMINAL-DEFID-1 S1, T0690).
//!
//! One stable, integer identity per declaration (function, type,
//! protocol, const), minted exactly once at the declaring compilation's
//! collection point and never renumbered afterwards. Archives serialize
//! it verbatim; every later phase compares ids, not strings.
//!
//! See `docs/architecture/nominal-identity.md` for the migration plan
//! this type anchors (679 measured name-keyed identity points, retired
//! stage by stage under the `check-name-census` ratchet).
//!
//! # Layout
//!
//! ```text
//! bit 63..48   OriginSpace — which compilation minted the id
//!              (0 = current compilation, 1 = embedded stdlib bake,
//!               2.. = dependency cogs in manifest order)
//! bit 47..0    dense per-origin ordinal, minted in canonical
//!              declaration-collection order
//! ```
//!
//! The origin space is what makes archive merging a TRANSLATION-FREE
//! operation for identity: a baked stdlib function keeps the exact id
//! the bake minted, in every consumer, forever. Only table *indices*
//! (positions in a module's function table) remain per-module; those
//! are addresses, not identity.
//!
//! Determinism contract: ordinals depend on collection order, so the
//! minting walk MUST be the canonical module order the bake already
//! pins for field interning (the double-bake determinism gate covers
//! drift). Two identical bakes yield identical DefIds.

use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

/// Which compilation minted a [`DefId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OriginSpace(pub u16);

impl OriginSpace {
    /// The compilation currently running (user cog / script).
    pub const CURRENT: OriginSpace = OriginSpace(0);
    /// The embedded stdlib bake.
    pub const STDLIB: OriginSpace = OriginSpace(1);
    /// First dependency-cog space; further cogs count up from here in
    /// manifest order.
    pub const FIRST_DEPENDENCY: OriginSpace = OriginSpace(2);
}

impl fmt::Display for OriginSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            OriginSpace::CURRENT => write!(f, "current"),
            OriginSpace::STDLIB => write!(f, "stdlib"),
            OriginSpace(n) => write!(f, "dep#{}", n - 2),
        }
    }
}

/// Stable nominal identity of one declaration.
///
/// `DefId(0)` is reserved as the NIL sentinel ("no declaration") so a
/// zeroed field never aliases a real stdlib declaration; the first
/// minted ordinal in every space is 1.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DefId(pub u64);

const ORDINAL_BITS: u32 = 48;
const ORDINAL_MASK: u64 = (1u64 << ORDINAL_BITS) - 1;

impl DefId {
    /// The "no declaration" sentinel. Never minted.
    pub const NIL: DefId = DefId(0);

    /// Compose from parts. Panics (debug) on ordinal overflow — 2^48
    /// declarations per origin is beyond any real program; a release
    /// build saturates into the reserved top ordinal, which the miner
    /// treats as exhaustion.
    #[inline]
    pub const fn new(origin: OriginSpace, ordinal: u64) -> DefId {
        debug_assert!(ordinal != 0, "ordinal 0 is the NIL sentinel space");
        debug_assert!(ordinal <= ORDINAL_MASK, "DefId ordinal overflow");
        DefId(((origin.0 as u64) << ORDINAL_BITS) | (ordinal & ORDINAL_MASK))
    }

    #[inline]
    pub const fn origin(self) -> OriginSpace {
        OriginSpace((self.0 >> ORDINAL_BITS) as u16)
    }

    #[inline]
    pub const fn ordinal(self) -> u64 {
        self.0 & ORDINAL_MASK
    }

    #[inline]
    pub const fn is_nil(self) -> bool {
        self.0 == 0
    }
}

impl fmt::Debug for DefId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_nil() {
            write!(f, "DefId(NIL)")
        } else {
            write!(f, "DefId({}:{})", self.origin(), self.ordinal())
        }
    }
}

impl fmt::Display for DefId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Mints dense ordinals for ONE origin space.
///
/// Thread-safe (the bake collects modules in parallel phases); minting
/// order within a space must still follow the canonical walk for
/// determinism — the minter enforces uniqueness, the CALLER owns order.
#[derive(Debug)]
pub struct DefIdMinter {
    origin: OriginSpace,
    next: AtomicU64,
}

impl DefIdMinter {
    pub fn new(origin: OriginSpace) -> DefIdMinter {
        DefIdMinter {
            origin,
            next: AtomicU64::new(1),
        }
    }

    /// Resume minting after `last_minted` (archive load: the spelling
    /// table records how far the bake got; later same-space minting —
    /// e.g. bake-time synthetic wrappers — continues, never reuses).
    pub fn resuming_after(origin: OriginSpace, last_minted: u64) -> DefIdMinter {
        DefIdMinter {
            origin,
            next: AtomicU64::new(last_minted.saturating_add(1)),
        }
    }

    pub fn origin(&self) -> OriginSpace {
        self.origin
    }

    /// Mint the next id. Saturates at the space's top ordinal on
    /// exhaustion (2^48 declarations) — the caller surfaces that as a
    /// hard error; ids are never recycled.
    pub fn mint(&self) -> DefId {
        let ord = self.next.fetch_add(1, Ordering::Relaxed);
        DefId::new(self.origin, ord.min(ORDINAL_MASK))
    }

    /// How many ids have been minted so far.
    pub fn minted(&self) -> u64 {
        self.next.load(Ordering::Relaxed).saturating_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_round_trips() {
        let id = DefId::new(OriginSpace::STDLIB, 0x1234_5678_9ABC);
        assert_eq!(id.origin(), OriginSpace::STDLIB);
        assert_eq!(id.ordinal(), 0x1234_5678_9ABC);
        assert!(!id.is_nil());
        assert_eq!(format!("{id:?}"), "DefId(stdlib:20015998343868)");
    }

    #[test]
    fn nil_is_reserved_and_distinct() {
        assert!(DefId::NIL.is_nil());
        let first = DefIdMinter::new(OriginSpace::CURRENT).mint();
        assert_ne!(first, DefId::NIL);
        assert_eq!(first.ordinal(), 1);
    }

    #[test]
    fn spaces_never_collide() {
        let a = DefIdMinter::new(OriginSpace::CURRENT);
        let b = DefIdMinter::new(OriginSpace::STDLIB);
        let (x, y) = (a.mint(), b.mint());
        assert_eq!(x.ordinal(), y.ordinal());
        assert_ne!(x, y, "same ordinal, different origin — distinct ids");
    }

    #[test]
    fn minting_is_dense_and_resume_continues() {
        let m = DefIdMinter::new(OriginSpace::CURRENT);
        let ids: Vec<u64> = (0..5).map(|_| m.mint().ordinal()).collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);
        assert_eq!(m.minted(), 5);
        let r = DefIdMinter::resuming_after(OriginSpace::CURRENT, 5);
        assert_eq!(r.mint().ordinal(), 6);
    }

    #[test]
    fn ordering_is_origin_major() {
        let cur = DefId::new(OriginSpace::CURRENT, 999);
        let std = DefId::new(OriginSpace::STDLIB, 1);
        assert!(cur < std, "origin dominates ordering — stable sort keys");
    }
}
