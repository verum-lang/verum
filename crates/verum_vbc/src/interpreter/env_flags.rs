//! Process-lifetime cache for the interpreter's `VERUM_*` trace and
//! behaviour flags (T0852 perf follow-up).
//!
//! The dispatch loop and several per-instruction handlers used to call
//! `std::env::var(...)` on EVERY instruction — `getenv` takes the
//! process-global environment lock, and the sampled hot loop spent
//! ~70% of its wall time in `__ulock_wait` + `__findenv_locked`
//! (850 ns per interpreted ADD).  Environment flags are read ONCE per
//! process here; the documented contract of every `VERUM_TRACE_*` /
//! behaviour toggle is "set before launch", which this cache makes
//! literal.
//!
//! `is_set` answers presence (the `var(..).is_ok()` /
//! `var_os(..).is_some()` forms); `get` answers the value for
//! filter-style flags (`VERUM_TRACE_PC=<substr>`).

use std::sync::OnceLock;

/// Every cached flag.  Adding a variant is the ONLY step — the name
/// table and the cache size follow it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flag {
    /// `VERUM_CBGR_LEGACY_ALLOC`
    CbgrLegacyAlloc,
    /// `VERUM_CBGR_LEGACY_INT_REFS`
    CbgrLegacyIntRefs,
    /// `VERUM_DEBUG_FS`
    DebugFs,
    /// `VERUM_DISABLE_UNIT_DYN_DISPATCH`
    DisableUnitDynDispatch,
    /// `VERUM_SUFFIX_COMPAT_LEGACY`
    SuffixCompatLegacy,
    /// `VERUM_TRACE_ASPTR`
    TraceAsptr,
    /// `VERUM_TRACE_CALLM_EQ`
    TraceCallmEq,
    /// `VERUM_TRACE_CALLM_FAIL`
    TraceCallmFail,
    /// `VERUM_TRACE_CALLM_FLOW`
    TraceCallmFlow,
    /// `VERUM_TRACE_CALLS`
    TraceCalls,
    /// `VERUM_TRACE_DEREF`
    TraceDeref,
    /// `VERUM_TRACE_DISPATCH`
    TraceDispatch,
    /// `VERUM_TRACE_DROPFN`
    TraceDropfn,
    /// `VERUM_TRACE_ENCBUF`
    TraceEncbuf,
    /// `VERUM_TRACE_ENVSTUB`
    TraceEnvstub,
    /// `VERUM_TRACE_EQ_RUNTIME`
    TraceEqRuntime,
    /// `VERUM_TRACE_FATREF_ROUTE`
    TraceFatrefRoute,
    /// `VERUM_TRACE_FIELDADDR`
    TraceFieldaddr,
    /// `VERUM_TRACE_GETF`
    TraceGetf,
    /// `VERUM_TRACE_GVD`
    TraceGvd,
    /// `VERUM_TRACE_HASHER`
    TraceHasher,
    /// `VERUM_TRACE_LISTREPR`
    TraceListrepr,
    /// `VERUM_TRACE_MATCHTAG`
    TraceMatchtag,
    /// `VERUM_TRACE_PC`
    TracePc,
    /// `VERUM_TRACE_PC_DECODE`
    TracePcDecode,
    /// `VERUM_TRACE_POOL`
    TracePool,
    /// `VERUM_TRACE_PROCESS`
    TraceProcess,
    /// `VERUM_TRACE_PROTOCOL_DISPATCH`
    TraceProtocolDispatch,
    /// `VERUM_TRACE_PTRWRITE`
    TracePtrwrite,
    /// `VERUM_TRACE_PUSH_STR`
    TracePushStr,
    /// `VERUM_TRACE_PUSH_STR_X`
    TracePushStrX,
    /// `VERUM_TRACE_STATICMUT`
    TraceStaticmut,
    /// `VERUM_TRACE_STATIC_CALL`
    TraceStaticCall,
    /// `VERUM_TRACE_TCP`
    TraceTcp,
    /// `VERUM_TRAP_SELFREF`
    TrapSelfref,
}

impl Flag {
    const COUNT: usize = 36;

    fn name(self) -> &'static str {
        match self {
            Flag::CbgrLegacyAlloc => "VERUM_CBGR_LEGACY_ALLOC",
            Flag::CbgrLegacyIntRefs => "VERUM_CBGR_LEGACY_INT_REFS",
            Flag::DebugFs => "VERUM_DEBUG_FS",
            Flag::DisableUnitDynDispatch => "VERUM_DISABLE_UNIT_DYN_DISPATCH",
            Flag::SuffixCompatLegacy => "VERUM_SUFFIX_COMPAT_LEGACY",
            Flag::TraceAsptr => "VERUM_TRACE_ASPTR",
            Flag::TraceCallmEq => "VERUM_TRACE_CALLM_EQ",
            Flag::TraceCallmFail => "VERUM_TRACE_CALLM_FAIL",
            Flag::TraceCallmFlow => "VERUM_TRACE_CALLM_FLOW",
            Flag::TraceCalls => "VERUM_TRACE_CALLS",
            Flag::TraceDeref => "VERUM_TRACE_DEREF",
            Flag::TraceDispatch => "VERUM_TRACE_DISPATCH",
            Flag::TraceDropfn => "VERUM_TRACE_DROPFN",
            Flag::TraceEncbuf => "VERUM_TRACE_ENCBUF",
            Flag::TraceEnvstub => "VERUM_TRACE_ENVSTUB",
            Flag::TraceEqRuntime => "VERUM_TRACE_EQ_RUNTIME",
            Flag::TraceFatrefRoute => "VERUM_TRACE_FATREF_ROUTE",
            Flag::TraceFieldaddr => "VERUM_TRACE_FIELDADDR",
            Flag::TraceGetf => "VERUM_TRACE_GETF",
            Flag::TraceGvd => "VERUM_TRACE_GVD",
            Flag::TraceHasher => "VERUM_TRACE_HASHER",
            Flag::TraceListrepr => "VERUM_TRACE_LISTREPR",
            Flag::TraceMatchtag => "VERUM_TRACE_MATCHTAG",
            Flag::TracePc => "VERUM_TRACE_PC",
            Flag::TracePcDecode => "VERUM_TRACE_PC_DECODE",
            Flag::TracePool => "VERUM_TRACE_POOL",
            Flag::TraceProcess => "VERUM_TRACE_PROCESS",
            Flag::TraceProtocolDispatch => "VERUM_TRACE_PROTOCOL_DISPATCH",
            Flag::TracePtrwrite => "VERUM_TRACE_PTRWRITE",
            Flag::TracePushStr => "VERUM_TRACE_PUSH_STR",
            Flag::TracePushStrX => "VERUM_TRACE_PUSH_STR_X",
            Flag::TraceStaticmut => "VERUM_TRACE_STATICMUT",
            Flag::TraceStaticCall => "VERUM_TRACE_STATIC_CALL",
            Flag::TraceTcp => "VERUM_TRACE_TCP",
            Flag::TrapSelfref => "VERUM_TRAP_SELFREF",
        }
    }

    fn index(self) -> usize {
        match self {
            Flag::CbgrLegacyAlloc => 0,
            Flag::CbgrLegacyIntRefs => 1,
            Flag::DebugFs => 2,
            Flag::DisableUnitDynDispatch => 3,
            Flag::SuffixCompatLegacy => 5,
            Flag::TraceAsptr => 6,
            Flag::TraceCallmEq => 7,
            Flag::TraceCallmFail => 8,
            Flag::TraceCallmFlow => 9,
            Flag::TraceCalls => 10,
            Flag::TraceDeref => 11,
            Flag::TraceDispatch => 12,
            Flag::TraceDropfn => 13,
            Flag::TraceEncbuf => 14,
            Flag::TraceEnvstub => 15,
            Flag::TraceEqRuntime => 16,
            Flag::TraceFatrefRoute => 17,
            Flag::TraceFieldaddr => 18,
            Flag::TraceGetf => 19,
            Flag::TraceGvd => 20,
            Flag::TraceHasher => 21,
            Flag::TraceListrepr => 22,
            Flag::TraceMatchtag => 23,
            Flag::TracePc => 24,
            Flag::TracePcDecode => 25,
            Flag::TracePool => 26,
            Flag::TraceProcess => 27,
            Flag::TraceProtocolDispatch => 28,
            Flag::TracePtrwrite => 29,
            Flag::TracePushStr => 30,
            Flag::TracePushStrX => 31,
            Flag::TraceStaticmut => 32,
            Flag::TraceStaticCall => 33,
            Flag::TraceTcp => 34,
            Flag::TrapSelfref => 35,
        }
    }
}

static CACHE: [OnceLock<Option<String>>; Flag::COUNT] =
    [const { OnceLock::new() }; Flag::COUNT];

/// The flag's value, read from the environment once per process.
pub(crate) fn get(flag: Flag) -> Option<&'static str> {
    CACHE[flag.index()]
        .get_or_init(|| std::env::var(flag.name()).ok())
        .as_deref()
}

/// Whether the flag is present at all (any value, including empty).
#[inline]
pub(crate) fn is_set(flag: Flag) -> bool {
    get(flag).is_some()
}
