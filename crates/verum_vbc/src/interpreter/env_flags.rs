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

/// Names, cache indices and capacity derive from the same private roster.
/// Cache indices are process-local and are never serialized.
macro_rules! interpreter_flags {
    ($($flag:ident => $name:literal),+ $(,)?) => {
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub(crate) enum Flag {
            $(#[doc = $name] $flag,)+
        }
        impl Flag {
            const COUNT: usize = [$($name),+].len();

            fn name(self) -> &'static str {
                match self { $(Self::$flag => $name,)+ }
            }

            fn index(self) -> usize { self as usize }
        }
    };
}

interpreter_flags! {
    CbgrLegacyAlloc => "VERUM_CBGR_LEGACY_ALLOC",
    DebugFs => "VERUM_DEBUG_FS",
    DisableUnitDynDispatch => "VERUM_DISABLE_UNIT_DYN_DISPATCH",
    SuffixCompatLegacy => "VERUM_SUFFIX_COMPAT_LEGACY",
    TraceAsptr => "VERUM_TRACE_ASPTR",
    TraceCallmEq => "VERUM_TRACE_CALLM_EQ",
    TraceCallmFail => "VERUM_TRACE_CALLM_FAIL",
    TraceCallmFlow => "VERUM_TRACE_CALLM_FLOW",
    TraceCalls => "VERUM_TRACE_CALLS",
    TraceDeref => "VERUM_TRACE_DEREF",
    TraceDispatch => "VERUM_TRACE_DISPATCH",
    TraceDropfn => "VERUM_TRACE_DROPFN",
    TraceEncbuf => "VERUM_TRACE_ENCBUF",
    TraceEnvstub => "VERUM_TRACE_ENVSTUB",
    TraceEqRuntime => "VERUM_TRACE_EQ_RUNTIME",
    TraceFatrefRoute => "VERUM_TRACE_FATREF_ROUTE",
    TraceFieldaddr => "VERUM_TRACE_FIELDADDR",
    TraceGetf => "VERUM_TRACE_GETF",
    TraceGvd => "VERUM_TRACE_GVD",
    TraceHasher => "VERUM_TRACE_HASHER",
    TraceListrepr => "VERUM_TRACE_LISTREPR",
    TraceMatchtag => "VERUM_TRACE_MATCHTAG",
    TracePc => "VERUM_TRACE_PC",
    TracePcDecode => "VERUM_TRACE_PC_DECODE",
    TracePool => "VERUM_TRACE_POOL",
    TraceProcess => "VERUM_TRACE_PROCESS",
    TraceProtocolDispatch => "VERUM_TRACE_PROTOCOL_DISPATCH",
    TracePtrwrite => "VERUM_TRACE_PTRWRITE",
    TracePushStr => "VERUM_TRACE_PUSH_STR",
    TracePushStrX => "VERUM_TRACE_PUSH_STR_X",
    TraceSpecfact => "VERUM_TRACE_SPECFACT",
    TraceStaticmut => "VERUM_TRACE_STATICMUT",
    TraceStaticCall => "VERUM_TRACE_STATIC_CALL",
    TraceTcp => "VERUM_TRACE_TCP",
    TrapSelfref => "VERUM_TRAP_SELFREF",
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
