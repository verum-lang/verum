//! # Embedded scripting engine — Rust backing for `core.script`
//!
//! This is the runtime that backs Verum's standard-library scripting API
//! (`core.script.Engine`).  It lets a Verum **host** application compile and
//! execute Verum **scripts** at runtime, in-process — the capability a game
//! engine gets from embedding Lua, but using Verum's own VBC interpreter.
//!
//! ## Reuse-first: this module introduces no second VM
//!
//! Every heavyweight capability is delegated to machinery that already exists
//! in this crate; the engine is a thin orchestration layer:
//!
//! | Concern                | Reused primitive                                |
//! |------------------------|-------------------------------------------------|
//! | source → VBC           | installed [`CompilerHook`] (full pipeline, or    |
//! |                        | the lite `VbcCodegen` path)                      |
//! | execution + heap       | [`Interpreter`] / [`InterpreterState`]           |
//! | resource limits        | [`InterpreterConfig`] (fuel / timeout / mem)     |
//! | cooperative abort      | `InterpreterConfig::cancel_flag`                 |
//! | capability sandbox     | `PermissionRouter` (wired in Phase 1)            |
//! | cross-script interop   | shared CBGR heap + `VbcLinker` (Phase 2)         |
//!
//! ## Crate-layer note (dependency inversion)
//!
//! `verum_vbc` sits *below* `verum_compiler`, so it cannot call the full
//! source→VBC pipeline (`verum_compiler::api::compile_to_vbc`) directly.  The
//! dependency is inverted through [`install_compiler_hook`]: `verum_compiler`
//! installs the full pipeline at startup (the same path the REPL already
//! uses).  When no hook is installed — e.g. a stripped AOT binary that ships
//! no compiler — source evaluation degrades gracefully to
//! [`ScriptError::CompilerUnavailable`], while execution of already-compiled
//! modules keeps working.  This is exactly the "scripting at the
//! interpretation level for now" boundary: principled, not a workaround.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use crate::module::VbcModule;
use crate::module::FunctionId;
use crate::value::Value;

use super::state::{HostCallContext, InterpreterConfig, InterpreterState};
use super::Interpreter;

/// An installable source→VBC compiler.
///
/// The hook takes Verum source text and returns a fully-compiled, runnable
/// [`VbcModule`] (stdlib linked) or a human-readable error string.  It is
/// `Send + Sync` so a single host process can drive scripting from any thread.
pub type CompilerHook = Arc<dyn Fn(&str) -> Result<VbcModule, String> + Send + Sync>;

/// Process-global compiler hook (dependency inversion across the crate layer).
static COMPILER_HOOK: RwLock<Option<CompilerHook>> = RwLock::new(None);

/// Install the process-wide source→VBC compiler used by [`ScriptEngine`].
///
/// `verum_compiler` calls this once at startup with the full pipeline.  Tests
/// and embedders may install a lighter compiler.  Last writer wins.
pub fn install_compiler_hook(hook: CompilerHook) {
    if let Ok(mut slot) = COMPILER_HOOK.write() {
        *slot = Some(hook);
    }
}

/// Whether a source→VBC compiler is available in this process.
///
/// Hosts can probe this to decide between source-eval and precompiled-module
/// execution (the latter never needs a compiler).
pub fn compiler_hook_installed() -> bool {
    COMPILER_HOOK.read().map(|s| s.is_some()).unwrap_or(false)
}

/// Compile `source` through the installed hook.
fn compile_via_hook(source: &str) -> Result<VbcModule, ScriptError> {
    // Clone the Arc out under the read lock so compilation (which may be slow)
    // does not hold the global lock.
    //
    // The host toolchain installs the hook before scripts run, but on a cold
    // first eval that install can still be racing on another thread (the host
    // interpreter starts as the install is committed). Briefly spin for it
    // rather than spuriously reporting the compiler as unavailable; if no hook
    // is ever installed (e.g. a stripped AOT binary), this falls through to
    // `CompilerUnavailable` after a bounded wait.
    let mut hook = None;
    for attempt in 0..2_000u32 {
        {
            let slot = COMPILER_HOOK
                .read()
                .map_err(|_| ScriptError::Internal("compiler hook lock poisoned".to_string()))?;
            if slot.is_some() {
                hook = slot.clone();
                break;
            }
        }
        // Back off from a tight spin after the first handful of tries.
        if attempt < 64 {
            std::thread::yield_now();
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    match hook {
        Some(h) => h(source).map_err(ScriptError::Compile),
        None => Err(ScriptError::CompilerUnavailable),
    }
}

/// Errors surfaced by the scripting engine.
#[derive(Debug, Clone)]
pub enum ScriptError {
    /// No source→VBC compiler is installed in this process (see
    /// [`install_compiler_hook`]).  Precompiled-module execution is unaffected.
    CompilerUnavailable,
    /// The script failed to compile; carries the compiler's message.
    Compile(String),
    /// The requested entry function does not exist in the compiled script.
    EntryNotFound(String),
    /// The script trapped at runtime (panic, limit exceeded, CBGR violation…).
    Runtime(String),
    /// Internal engine error (e.g. a poisoned lock).
    Internal(String),
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScriptError::CompilerUnavailable => {
                write!(f, "source compilation is unavailable (no compiler hook installed)")
            }
            ScriptError::Compile(msg) => write!(f, "script compilation failed: {msg}"),
            ScriptError::EntryNotFound(name) => write!(f, "script entry function not found: {name}"),
            ScriptError::Runtime(msg) => write!(f, "script runtime error: {msg}"),
            ScriptError::Internal(msg) => write!(f, "scripting engine internal error: {msg}"),
        }
    }
}

impl std::error::Error for ScriptError {}

/// An isolated script-execution world.
///
/// Each engine owns its [`InterpreterConfig`] (resource limits + cancel flag),
/// a host-side global table, and the captured stdout of the most recent run.
/// In Phase 0 every run executes on a fresh [`Interpreter`] (Lua-`dostring`
/// semantics — the safest and simplest isolation).  Phase 2 will optionally
/// hold a persistent interpreter so multiple scripts share one CBGR heap for
/// zero-copy interop.
/// Capability grants for a sandboxed script — the object-capability / WASI
/// posture: a sandboxed script has NO ambient authority for file, network or
/// process operations unless the host explicitly grants it. Enforced through
/// the interpreter's `PermissionRouter` (which gates `FileSystem` / `Network` /
/// `Process` scopes); pure compute, memory and stdout are always allowed.
#[derive(Debug, Clone, Copy)]
pub struct ScriptCaps {
    /// Allow file-system access.
    pub file_io: bool,
    /// Allow network access.
    pub network: bool,
    /// Allow process / environment operations (spawn, exit, env vars).
    pub process: bool,
}

impl ScriptCaps {
    /// All authority granted (the default for a plain `Engine`).
    pub fn permissive() -> Self {
        Self {
            file_io: true,
            network: true,
            process: true,
        }
    }

    /// No ambient authority (the default for a sandboxed engine).
    pub fn restricted() -> Self {
        Self {
            file_io: false,
            network: false,
            process: false,
        }
    }

    /// Whether every capability is granted (no gating needed).
    fn is_permissive(&self) -> bool {
        self.file_io && self.network && self.process
    }
}

pub struct ScriptEngine {
    config: InterpreterConfig,
    globals: HashMap<String, ScriptValueOwned>,
    /// Host functions the host registered for scripts to call back into,
    /// by name → host-module FunctionId.
    host_fns: HashMap<String, FunctionId>,
    /// Capability grants enforced on each run (see [`ScriptCaps`]).
    caps: ScriptCaps,
    cancel: Arc<AtomicBool>,
    last_stdout: String,
}

impl ScriptEngine {
    /// Create an engine with default limits (permissive).
    pub fn new() -> Self {
        Self::with_config(InterpreterConfig::default())
    }

    /// Create an engine with explicit limits.
    ///
    /// The engine wires its own cancel flag into the config so [`interrupt`]
    /// works regardless of what the caller passed.
    ///
    /// [`interrupt`]: ScriptEngine::interrupt
    pub fn with_config(mut config: InterpreterConfig) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        config.cancel_flag = Some(cancel.clone());
        Self {
            config,
            globals: HashMap::new(),
            host_fns: HashMap::new(),
            caps: ScriptCaps::permissive(),
            cancel,
            last_stdout: String::new(),
        }
    }

    /// Grant file-system access to a sandboxed engine (chainable).
    pub fn allow_file_io(mut self) -> Self {
        self.caps.file_io = true;
        self
    }

    /// Grant network access to a sandboxed engine (chainable).
    pub fn allow_network(mut self) -> Self {
        self.caps.network = true;
        self
    }

    /// Grant process / environment access to a sandboxed engine (chainable).
    pub fn allow_process(mut self) -> Self {
        self.caps.process = true;
        self
    }

    /// Register a host function (by host-module [`FunctionId`]) that scripts
    /// can call back into via the `script_host_call_*` intrinsics.
    pub fn register(&mut self, name: impl Into<String>, func_id: FunctionId) {
        self.host_fns.insert(name.into(), func_id);
    }

    /// Create a sandboxed engine with resource limits.  Each limit is `0` for
    /// "unlimited" on that dimension.  These reuse the interpreter's existing
    /// fuel / heap / timeout enforcement (`InterpreterConfig`) — no new
    /// sandbox machinery: a runaway script aborts with
    /// `InstructionLimitExceeded` / `OutOfMemory` / `Timeout`, surfaced as a
    /// failed [`ScriptOutcome`].
    ///
    /// This is the WASI-style "no ambient authority by default" posture: the
    /// sandboxed engine enforces both resource limits AND capability gating —
    /// file / network / process operations are DENIED unless re-granted via
    /// [`allow_file_io`](Self::allow_file_io) / [`allow_network`] /
    /// [`allow_process`]. Pure compute, memory and stdout always work.
    ///
    /// [`allow_network`]: Self::allow_network
    /// [`allow_process`]: Self::allow_process
    pub fn sandboxed(memory_limit: usize, instruction_limit: u64, time_limit_ms: u64) -> Self {
        let mut config = InterpreterConfig::default();
        if memory_limit > 0 {
            config.max_heap_size = memory_limit;
        }
        if instruction_limit > 0 {
            config.max_instructions = instruction_limit;
        }
        if time_limit_ms > 0 {
            config.timeout_ms = time_limit_ms;
        }
        let mut engine = Self::with_config(config);
        engine.caps = ScriptCaps::restricted();
        engine
    }

    /// Compile `source` into a self-contained, runnable module.
    ///
    /// Requires an installed [`CompilerHook`]; otherwise returns
    /// [`ScriptError::CompilerUnavailable`].
    pub fn compile(&self, source: &str) -> Result<Arc<VbcModule>, ScriptError> {
        compile_via_hook(source).map(Arc::new)
    }

    /// Compile and run `source`, executing its `main` function.
    ///
    /// Returns `main`'s value; the script's captured stdout is available via
    /// [`last_stdout`](ScriptEngine::last_stdout).
    pub fn eval(&mut self, source: &str) -> Result<Value, ScriptError> {
        let module = self.compile(source)?;
        self.run(module, "main", &[])
    }

    /// Run `entry` from an already-compiled `module` with `args`.
    ///
    /// A fresh interpreter is created per call (Phase 0 isolation).  Stdout is
    /// captured into the engine and overwritten on each run.
    pub fn run(
        &mut self,
        module: Arc<VbcModule>,
        entry: &str,
        args: &[Value],
    ) -> Result<Value, ScriptError> {
        // Codegen registers functions under their fully-qualified name
        // (e.g. `script.main`).  `find_function_by_name` only suffix-matches
        // when the query itself contains a dot, so fall back to the unique
        // bare-suffix lookup for plain entry names like `main`.
        let func_id = module
            .find_function_by_name(entry)
            .or_else(|| module.find_function_by_unique_bare_suffix(entry))
            .ok_or_else(|| ScriptError::EntryNotFound(entry.to_string()))?;

        // Reuse the proven REPL execution path: build an interpreter with this
        // engine's limits, then call the entry. No second VM, heap, or sandbox.
        let mut interp = Interpreter::try_new_with_config(module, self.config.clone())
            .map_err(|e| ScriptError::Runtime(format!("{e:?}")))?;

        let result = interp
            .execute_function_with_args(func_id, args)
            .map_err(|e| ScriptError::Runtime(format!("{e:?}")));

        // Capture stdout regardless of success so hosts can inspect partial
        // output from a script that trapped.
        self.last_stdout = interp.state.get_stdout().to_string();
        result
    }

    /// The stdout captured from the most recent [`eval`](ScriptEngine::eval) or
    /// [`run`](ScriptEngine::run).
    pub fn last_stdout(&self) -> &str {
        &self.last_stdout
    }

    /// Set a host-provided global the next script run can read (via the
    /// `script_global_*` intrinsics).  Overwrites any previous value.
    pub fn set_global(&mut self, name: impl Into<String>, value: ScriptValueOwned) {
        self.globals.insert(name.into(), value);
    }

    /// Read a global — either one the host set, or one a script wrote during
    /// its run (available after the run completes).
    pub fn get_global(&self, name: &str) -> Option<ScriptValueOwned> {
        self.globals.get(name).cloned()
    }

    /// Request cooperative interruption of a running script (thread-safe).
    pub fn interrupt(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// Clear a pending interrupt so the engine can be reused.
    pub fn clear_interrupt(&self) {
        self.cancel.store(false, Ordering::SeqCst);
    }

    /// Whether an interrupt is currently pending.
    pub fn is_interrupted(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Read-only access to this engine's interpreter configuration.
    pub fn config(&self) -> &InterpreterConfig {
        &self.config
    }

    /// Mutable access to this engine's interpreter configuration (e.g. to
    /// tighten limits between runs).
    pub fn config_mut(&mut self) -> &mut InterpreterConfig {
        &mut self.config
    }
}

impl Default for ScriptEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// A script's return value marshaled into an owned host representation.
///
/// Phase 1 covers scalars + `Text`; `Other` stands in for heap objects
/// (List/Map/records) whose structural marshaling lands later.  Marshaling to
/// an *owned* form (rather than a borrowed interpreter `Value`) is required
/// because the script runs on a throwaway interpreter whose heap is freed once
/// the run returns — a borrowed `Text` / heap pointer would dangle.
#[derive(Debug, Clone)]
pub enum ScriptValueOwned {
    /// Unit / no meaningful value.
    Nil,
    /// Boolean.
    Bool(bool),
    /// 64-bit integer.
    Int(i64),
    /// 64-bit float.
    Float(f64),
    /// UTF-8 text, copied out of the script heap.
    Text(String),
    /// A list, structurally marshaled (elements copied out of the script heap).
    List(Vec<ScriptValueOwned>),
    /// A map, structurally marshaled as (key, value) pairs.
    Map(Vec<(ScriptValueOwned, ScriptValueOwned)>),
    /// A heap object not yet structurally marshaled (record/reference).
    Other,
}

impl ScriptValueOwned {
    /// The canonical `ScriptValue` kind tag — the single source of truth shared
    /// by `script_outcome_kind` and `script_global_kind`:
    /// `0`=Nil, `1`=Bool, `2`=Int, `3`=Float, `4`=Text, `5`=List, `6`=Map,
    /// `7`=other/opaque.
    pub fn kind(&self) -> i64 {
        match self {
            ScriptValueOwned::Nil => 0,
            ScriptValueOwned::Bool(_) => 1,
            ScriptValueOwned::Int(_) => 2,
            ScriptValueOwned::Float(_) => 3,
            ScriptValueOwned::Text(_) => 4,
            ScriptValueOwned::List(_) => 5,
            ScriptValueOwned::Map(_) => 6,
            ScriptValueOwned::Other => 7,
        }
    }
}

/// The result of running a script: its (owned) return value, captured stdout,
/// and an optional error.  Boxed behind an opaque handle by the `core.script`
/// intrinsics and read back through accessors.
pub struct ScriptOutcome {
    /// The script entry's return value (meaningful only when [`is_ok`] is true).
    ///
    /// [`is_ok`]: ScriptOutcome::is_ok
    pub value: ScriptValueOwned,
    /// The error, if the script failed to compile or trapped.
    pub error: Option<ScriptError>,
    /// Everything the script wrote to stdout during the run.
    pub stdout: String,
}

impl ScriptOutcome {
    /// Whether the script completed successfully.
    pub fn is_ok(&self) -> bool {
        self.error.is_none()
    }

    /// The canonical `ScriptValue` kind tag of [`value`](Self::value), for the
    /// host's marshaling layer. See [`ScriptValueOwned::kind`].
    pub fn kind(&self) -> i64 {
        self.value.kind()
    }

    /// Wrap a bare value in an outcome. Used for sub-handles when marshaling
    /// nested collections: each nested `List`/`Map` element becomes its own
    /// outcome the host's recursive marshaler reads with the same accessors,
    /// so nesting works to arbitrary depth.
    pub fn from_value(value: ScriptValueOwned) -> Self {
        Self {
            value,
            error: None,
            stdout: String::new(),
        }
    }

    /// Clone the `i`-th list element's owned value (Nil if absent / not a list).
    pub fn list_elem_owned(&self, i: i64) -> ScriptValueOwned {
        self.list_elem(i).cloned().unwrap_or(ScriptValueOwned::Nil)
    }

    /// Clone the `i`-th map entry's key / value (Nil if absent / not a map).
    pub fn map_key_owned(&self, i: i64) -> ScriptValueOwned {
        self.map_pair(i)
            .map(|(k, _)| k.clone())
            .unwrap_or(ScriptValueOwned::Nil)
    }
    pub fn map_value_owned(&self, i: i64) -> ScriptValueOwned {
        self.map_pair(i)
            .map(|(_, v)| v.clone())
            .unwrap_or(ScriptValueOwned::Nil)
    }

    /// The value as an integer (valid when [`kind`](Self::kind) is `2`).
    pub fn as_int(&self) -> i64 {
        match &self.value {
            ScriptValueOwned::Int(i) => *i,
            _ => 0,
        }
    }

    /// The value as a float (valid when [`kind`](Self::kind) is `3`).
    pub fn as_float(&self) -> f64 {
        match &self.value {
            ScriptValueOwned::Float(f) => *f,
            _ => 0.0,
        }
    }

    /// The value as a bool (valid when [`kind`](Self::kind) is `1`).
    pub fn as_bool(&self) -> bool {
        matches!(&self.value, ScriptValueOwned::Bool(true))
    }

    /// The value as text (valid when [`kind`](Self::kind) is `4`); `""` otherwise.
    pub fn as_text(&self) -> &str {
        match &self.value {
            ScriptValueOwned::Text(s) => s,
            _ => "",
        }
    }

    /// The number of elements when the value is a `List` (valid when
    /// [`kind`](Self::kind) is `5`); `0` otherwise.
    pub fn list_len(&self) -> i64 {
        match &self.value {
            ScriptValueOwned::List(items) => items.len() as i64,
            _ => 0,
        }
    }

    /// The `i`-th list element, or `None` if the value isn't a list / out of range.
    fn list_elem(&self, i: i64) -> Option<&ScriptValueOwned> {
        match &self.value {
            ScriptValueOwned::List(items) if i >= 0 => items.get(i as usize),
            _ => None,
        }
    }

    /// The canonical kind tag of the `i`-th list element (`0` if absent).
    pub fn list_elem_kind(&self, i: i64) -> i64 {
        self.list_elem(i).map(|e| e.kind()).unwrap_or(0)
    }

    /// The `i`-th list element as `Int` (`0` if absent or not an Int).
    pub fn list_elem_int(&self, i: i64) -> i64 {
        match self.list_elem(i) {
            Some(ScriptValueOwned::Int(n)) => *n,
            _ => 0,
        }
    }

    /// The `i`-th list element as `Float` (`0.0` if absent or not a Float).
    pub fn list_elem_float(&self, i: i64) -> f64 {
        match self.list_elem(i) {
            Some(ScriptValueOwned::Float(f)) => *f,
            _ => 0.0,
        }
    }

    /// The `i`-th list element as `Bool` (`false` if absent or not a Bool).
    pub fn list_elem_bool(&self, i: i64) -> bool {
        matches!(self.list_elem(i), Some(ScriptValueOwned::Bool(true)))
    }

    /// The `i`-th list element as text (`""` if absent or not Text).
    pub fn list_elem_text(&self, i: i64) -> &str {
        match self.list_elem(i) {
            Some(ScriptValueOwned::Text(s)) => s,
            _ => "",
        }
    }

    /// The number of entries when the value is a `Map` (kind `6`); `0` otherwise.
    pub fn map_len(&self) -> i64 {
        match &self.value {
            ScriptValueOwned::Map(entries) => entries.len() as i64,
            _ => 0,
        }
    }

    /// The `i`-th map entry as a `(key, value)` pair, or `None`.
    fn map_pair(&self, i: i64) -> Option<&(ScriptValueOwned, ScriptValueOwned)> {
        match &self.value {
            ScriptValueOwned::Map(entries) if i >= 0 => entries.get(i as usize),
            _ => None,
        }
    }

    /// Canonical kind tag of entry `i`'s key / value (`0` if absent).
    pub fn map_key_kind(&self, i: i64) -> i64 {
        self.map_pair(i).map(|(k, _)| k.kind()).unwrap_or(0)
    }
    pub fn map_value_kind(&self, i: i64) -> i64 {
        self.map_pair(i).map(|(_, v)| v.kind()).unwrap_or(0)
    }

    /// Entry `i`'s key / value as a typed scalar (default if absent / wrong type).
    pub fn map_key_int(&self, i: i64) -> i64 {
        match self.map_pair(i) {
            Some((ScriptValueOwned::Int(n), _)) => *n,
            _ => 0,
        }
    }
    pub fn map_value_int(&self, i: i64) -> i64 {
        match self.map_pair(i) {
            Some((_, ScriptValueOwned::Int(n))) => *n,
            _ => 0,
        }
    }
    pub fn map_key_float(&self, i: i64) -> f64 {
        match self.map_pair(i) {
            Some((ScriptValueOwned::Float(f), _)) => *f,
            _ => 0.0,
        }
    }
    pub fn map_value_float(&self, i: i64) -> f64 {
        match self.map_pair(i) {
            Some((_, ScriptValueOwned::Float(f))) => *f,
            _ => 0.0,
        }
    }
    pub fn map_key_bool(&self, i: i64) -> bool {
        matches!(self.map_pair(i), Some((ScriptValueOwned::Bool(true), _)))
    }
    pub fn map_value_bool(&self, i: i64) -> bool {
        matches!(self.map_pair(i), Some((_, ScriptValueOwned::Bool(true))))
    }
    pub fn map_key_text(&self, i: i64) -> &str {
        match self.map_pair(i) {
            Some((ScriptValueOwned::Text(s), _)) => s,
            _ => "",
        }
    }
    pub fn map_value_text(&self, i: i64) -> &str {
        match self.map_pair(i) {
            Some((_, ScriptValueOwned::Text(s))) => s,
            _ => "",
        }
    }

    /// The error message, if the script failed to compile or trapped.
    pub fn error_message(&self) -> Option<String> {
        self.error.as_ref().map(|e| e.to_string())
    }

    /// The script's captured stdout.
    pub fn stdout(&self) -> &str {
        &self.stdout
    }
}

impl ScriptEngine {
    /// Compile and run `source` (entry `main`), packaging the result, captured
    /// stdout, and any error into a [`ScriptOutcome`].
    ///
    /// Never returns `Err`: a compile/runtime failure becomes
    /// `ScriptOutcome { error: Some(_), .. }`.  This is the form the scripting
    /// intrinsics use, since the host reads success/failure through accessors.
    pub fn eval_to_outcome(&mut self, source: &str) -> ScriptOutcome {
        self.eval_to_outcome_with_host(source, 0)
    }

    /// Like [`eval_to_outcome`](Self::eval_to_outcome) but installs a host
    /// re-entry context: `host_state_addr` is the address of the host
    /// interpreter's `InterpreterState`, so the script's `script_host_call_*`
    /// intrinsics can call back into the host functions registered via
    /// [`register`](Self::register).  Pass `0` for no host re-entry.
    ///
    /// # Safety contract
    /// `host_state_addr`, when non-zero, must point to a live `InterpreterState`
    /// that stays valid for the whole run — which holds when the caller is the
    /// `script_engine_eval` intrinsic handler (the host is paused there).
    pub fn eval_to_outcome_with_host(
        &mut self,
        source: &str,
        host_state_addr: usize,
    ) -> ScriptOutcome {
        let module = match self.compile(source) {
            Ok(m) => m,
            Err(error) => {
                return ScriptOutcome {
                    value: ScriptValueOwned::Nil,
                    error: Some(error),
                    stdout: String::new(),
                };
            }
        };
        self.run_with_host(module, "main", &[], host_state_addr)
    }

    /// Compile `source` and run its `fn_name` entry (rather than `main`) with a
    /// host re-entry context — the "call a named script function" embedding
    /// primitive. Arguments are passed through the shared-global table (the host
    /// `set_*`s them before the call; the entry reads them via `script_global_*`)
    /// and the result comes back marshaled in the [`ScriptOutcome`], so a host
    /// can drive a script's individual functions, not only its `main`.
    pub fn call_named_with_host(
        &mut self,
        source: &str,
        fn_name: &str,
        host_state_addr: usize,
    ) -> ScriptOutcome {
        self.call_named_args_with_host(source, fn_name, &[], host_state_addr)
    }

    /// [`call_named_with_host`](Self::call_named_with_host) passing positional
    /// `args` to the entry as its function parameters (rather than routing them
    /// through the shared globals). Scalar args (`Int` / `Float` / `Bool`) are
    /// self-contained `Value`s and cross to the fresh script interpreter safely;
    /// the caller marshals the host's argument list into `args`.
    pub fn call_named_args_with_host(
        &mut self,
        source: &str,
        fn_name: &str,
        args: &[Value],
        host_state_addr: usize,
    ) -> ScriptOutcome {
        let module = match self.compile(source) {
            Ok(m) => m,
            Err(error) => {
                return ScriptOutcome {
                    value: ScriptValueOwned::Nil,
                    error: Some(error),
                    stdout: String::new(),
                };
            }
        };
        self.run_with_host(module, fn_name, args, host_state_addr)
    }

    /// Run `entry` from a compiled `module`, marshaling its result into an
    /// owned [`ScriptOutcome`] *before* the script interpreter (and its heap)
    /// is dropped — so a `Text` / heap result is copied out, not left dangling.
    pub fn run_to_outcome(
        &mut self,
        module: Arc<VbcModule>,
        entry: &str,
        args: &[Value],
    ) -> ScriptOutcome {
        self.run_with_host(module, entry, args, 0)
    }

    /// [`run_to_outcome`](Self::run_to_outcome) with a host re-entry context
    /// (see [`eval_to_outcome_with_host`](Self::eval_to_outcome_with_host)).
    fn run_with_host(
        &mut self,
        module: Arc<VbcModule>,
        entry: &str,
        args: &[Value],
        host_state_addr: usize,
    ) -> ScriptOutcome {
        let func_id = match module
            .find_function_by_name(entry)
            .or_else(|| module.find_function_by_unique_bare_suffix(entry))
        {
            Some(f) => f,
            None => {
                return ScriptOutcome {
                    value: ScriptValueOwned::Nil,
                    error: Some(ScriptError::EntryNotFound(entry.to_string())),
                    stdout: String::new(),
                };
            }
        };
        let mut interp = match Interpreter::try_new_with_config(module, self.config.clone()) {
            Ok(i) => i,
            Err(e) => {
                return ScriptOutcome {
                    value: ScriptValueOwned::Nil,
                    error: Some(ScriptError::Runtime(format!("{e:?}"))),
                    stdout: String::new(),
                };
            }
        };
        // Install the host-function bridge so the script's `script_host_call_*`
        // intrinsics can call back into the host's registered functions.
        if host_state_addr != 0 || !self.host_fns.is_empty() {
            interp.state.host_call_ctx = Some(HostCallContext {
                host_state_addr,
                host_fns: self.host_fns.clone(),
            });
        }

        // Capability gating (object-capability / WASI posture): a sandboxed
        // engine denies file / network / process authority unless granted. The
        // interpreter's PermissionRouter enforces this at the FFI / syscall /
        // process sites; pure compute, memory and stdout are unaffected.
        if !self.caps.is_permissive() {
            let caps = self.caps;
            interp.state.set_permission_policy(move |scope, _target| {
                use crate::interpreter::permission::PermissionDecision::{Allow, Deny};
                use crate::interpreter::permission::PermissionScope;
                match scope {
                    PermissionScope::FileSystem if !caps.file_io => Deny,
                    PermissionScope::Network if !caps.network => Deny,
                    PermissionScope::Process if !caps.process => Deny,
                    _ => Allow,
                }
            });
        }

        // Seed host-provided globals into the script interpreter so the
        // script can read them via the `script_global_*` intrinsics.
        for (name, owned) in &self.globals {
            let v = build_value(&mut interp, owned);
            interp.state.host_globals.insert(name.clone(), v);
        }

        let result = interp.execute_function_with_args(func_id, args);
        let stdout = interp.state.get_stdout().to_string();
        self.last_stdout = stdout.clone();

        // Read back globals (including any the script wrote) into owned form
        // while the interpreter — and its heap — is still alive.
        let read_back: Vec<(String, ScriptValueOwned)> = interp
            .state
            .host_globals
            .iter()
            .map(|(name, v)| (name.clone(), extract_owned(&interp.state, *v)))
            .collect();
        for (name, owned) in read_back {
            self.globals.insert(name, owned);
        }

        match result {
            Ok(value) => ScriptOutcome {
                value: extract_owned(&interp.state, value),
                error: None,
                stdout,
            },
            Err(e) => ScriptOutcome {
                value: ScriptValueOwned::Nil,
                error: Some(ScriptError::Runtime(format!("{e:?}"))),
                stdout,
            },
        }
    }
}

/// Build a script-interpreter `Value` from an owned host value, reconstructing
/// `Text` / `List` / `Map` on the script heap. The inverse of [`extract_owned`]:
/// lets a host seed structured globals INTO a script (and a `ScriptWorld` share
/// `List`/`Map` between scripts by value).
fn build_value(interp: &mut Interpreter, owned: &ScriptValueOwned) -> Value {
    match owned {
        ScriptValueOwned::Nil | ScriptValueOwned::Other => Value::unit(),
        ScriptValueOwned::Bool(b) => Value::from_bool(*b),
        ScriptValueOwned::Int(i) => Value::from_i64(*i),
        ScriptValueOwned::Float(f) => Value::from_f64(*f),
        ScriptValueOwned::Text(s) => interp.alloc_string(s).unwrap_or_else(|_| Value::unit()),
        ScriptValueOwned::List(items) => build_list_value(interp, items),
        ScriptValueOwned::Map(entries) => build_map_value(interp, entries),
    }
}

/// Reconstruct a `List` heap object (`[len, cap, backing]`, elements as Values
/// after the backing's header) — inverse of `InterpreterState::list_elements`.
fn build_list_value(interp: &mut Interpreter, items: &[ScriptValueOwned]) -> Value {
    use crate::interpreter::heap::OBJECT_HEADER_SIZE;
    use crate::types::TypeId;
    const VSIZE: usize = std::mem::size_of::<Value>();
    let len = items.len();
    let cap = len.max(1);
    let obj = match interp.state.heap.alloc(TypeId::LIST, 3 * VSIZE) {
        Ok(o) => o,
        Err(_) => return Value::unit(),
    };
    interp.state.record_allocation();
    let backing = match interp.state.heap.alloc_array(TypeId::LIST, cap) {
        Ok(b) => b,
        Err(_) => return Value::unit(),
    };
    interp.state.record_allocation();
    let backing_ptr = backing.as_ptr() as *mut u8;
    // Build each element first (a nested element may itself allocate; the heap
    // is append-only so `backing_ptr` stays valid) and store it.
    for (i, item) in items.iter().enumerate() {
        let ev = build_value(interp, item);
        // SAFETY: i < cap; backing has cap Value slots after its header.
        unsafe { *(backing_ptr.add(OBJECT_HEADER_SIZE + i * VSIZE) as *mut Value) = ev };
    }
    let obj_ptr = obj.as_ptr() as *mut u8;
    // SAFETY: a LIST object has 3 Value slots after its header.
    unsafe {
        let data = obj_ptr.add(OBJECT_HEADER_SIZE) as *mut Value;
        *data = Value::from_i64(len as i64);
        *data.add(1) = Value::from_i64(cap as i64);
        *data.add(2) = Value::from_ptr(backing_ptr);
    }
    Value::from_ptr(obj_ptr)
}

/// Reconstruct a `Map` heap object (`[count, cap, entries]`; entries backing
/// holds (key, value) Value pairs by open addressing, empty slot = Unit key) —
/// inverse of `InterpreterState::map_entries`, matching the `Map.insert`
/// intercept's layout. Source keys are already unique (they came from a real
/// map), so no dedup probe is needed.
fn build_map_value(
    interp: &mut Interpreter,
    entries: &[(ScriptValueOwned, ScriptValueOwned)],
) -> Value {
    use crate::interpreter::dispatch_table::handlers::memory_collections::value_hash;
    use crate::interpreter::heap::OBJECT_HEADER_SIZE;
    use crate::types::TypeId;
    const VSIZE: usize = std::mem::size_of::<Value>();
    // Power-of-two capacity keeping the load factor under 75%.
    let count = entries.len();
    let mut cap = 16usize;
    while cap < count * 2 {
        cap *= 2;
    }
    let obj = match interp.state.heap.alloc(TypeId::MAP, 4 * VSIZE) {
        Ok(o) => o,
        Err(_) => return Value::unit(),
    };
    interp.state.record_allocation();
    let entries_obj = match interp.state.heap.alloc_array(TypeId::UNIT, cap * 2) {
        Ok(e) => e,
        Err(_) => return Value::unit(),
    };
    interp.state.record_allocation();
    let entries_ptr = entries_obj.as_ptr() as *mut u8;
    let entries_data = unsafe { entries_ptr.add(OBJECT_HEADER_SIZE) as *mut Value };
    for i in 0..(cap * 2) {
        unsafe { *entries_data.add(i) = Value::unit() };
    }
    let mut live = 0usize;
    for (k_owned, v_owned) in entries {
        let key = build_value(interp, k_owned);
        let val = build_value(interp, v_owned);
        let mut idx = value_hash(key) % cap;
        loop {
            // SAFETY: idx < cap; entries backing has cap (key,value) pairs.
            if unsafe { (*entries_data.add(idx * 2)).is_unit() } {
                unsafe {
                    *entries_data.add(idx * 2) = key;
                    *entries_data.add(idx * 2 + 1) = val;
                }
                live += 1;
                break;
            }
            idx = (idx + 1) % cap;
        }
    }
    let obj_ptr = obj.as_ptr() as *mut u8;
    // SAFETY: a MAP object has 4 Value slots after its header.
    unsafe {
        let data = obj_ptr.add(OBJECT_HEADER_SIZE) as *mut Value;
        *data = Value::from_i64(live as i64);
        *data.add(1) = Value::from_i64(cap as i64);
        *data.add(2) = Value::from_ptr(entries_ptr);
        *data.add(3) = Value::from_i64(0); // tombstones
    }
    Value::from_ptr(obj_ptr)
}

/// A persistent shared-world for **zero-copy interop between scripts** (P2).
///
/// Unlike [`ScriptEngine`] — which runs each script on a throwaway interpreter,
/// so heaps don't outlive a run — a `ScriptWorld` holds ONE persistent
/// interpreter. Its heap and its shared-global table survive across evals, so
/// scripts running in the same world share data structures **by reference**: a
/// `Text` / `List` / `Map` one script stores in the shared table is read by
/// another as the SAME heap object — no copy, no serialization — with CBGR
/// generation/epoch checks catching any stale reference at ~1ns. This is the
/// interop tier that copy-at-the-boundary engines (Wasm, BEAM, V8 isolates)
/// cannot offer; CBGR is what makes the shared reference safe.
pub struct ScriptWorld {
    /// The world's persistent shared table — the source of truth for data
    /// shared between scripts, in owned form. Each `eval` runs on a FRESH
    /// interpreter (so module-local string-constant resolution is never
    /// corrupted by reuse) seeded from this table; what a script writes via
    /// `script_set_*` is captured back into it. Scalars + Text share reliably.
    /// (A future tier links scripts into one module via `VbcLinker` and runs
    /// them on one heap, so large structures can be shared truly by-reference
    /// — the zero-overhead interop tier the runtime linker already enables.)
    shared: HashMap<String, ScriptValueOwned>,
    config: InterpreterConfig,
}

impl ScriptWorld {
    /// Create an empty world.
    pub fn new() -> Self {
        Self {
            shared: HashMap::new(),
            config: InterpreterConfig::default(),
        }
    }

    /// Read a value the world currently holds in its shared table.
    pub fn get_shared(&self, name: &str) -> Option<ScriptValueOwned> {
        self.shared.get(name).cloned()
    }

    /// Compile and run `source` (entry `main`) in the world. The shared table
    /// is seeded into the script before the run and updated from it after, so a
    /// script sees what earlier scripts shared and can share its own.
    pub fn eval(&mut self, source: &str) -> ScriptOutcome {
        let module = match compile_via_hook(source) {
            Ok(m) => Arc::new(m),
            Err(error) => {
                return ScriptOutcome {
                    value: ScriptValueOwned::Nil,
                    error: Some(error),
                    stdout: String::new(),
                };
            }
        };
        let func_id = match module
            .find_function_by_name("main")
            .or_else(|| module.find_function_by_unique_bare_suffix("main"))
        {
            Some(f) => f,
            None => {
                return ScriptOutcome {
                    value: ScriptValueOwned::Nil,
                    error: Some(ScriptError::EntryNotFound("main".to_string())),
                    stdout: String::new(),
                };
            }
        };

        let mut interp = match Interpreter::try_new_with_config(module, self.config.clone()) {
            Ok(i) => i,
            Err(e) => {
                return ScriptOutcome {
                    value: ScriptValueOwned::Nil,
                    error: Some(ScriptError::Runtime(format!("{e:?}"))),
                    stdout: String::new(),
                };
            }
        };

        // Seed the shared table into the script's readable globals.
        for (name, owned) in &self.shared {
            let v = build_value(&mut interp, owned);
            interp.state.host_globals.insert(name.clone(), v);
        }

        let result = interp.execute_function_with_args(func_id, &[]);
        let stdout = interp.state.get_stdout().to_string();

        // Persist what the script wrote — owned snapshots captured AT WRITE TIME
        // (the raw heap values do not survive the eval's frame teardown).
        let writes: Vec<(String, ScriptValueOwned)> =
            interp.state.shared_writes.drain().collect();
        for (name, owned) in writes {
            self.shared.insert(name, owned);
        }

        match result {
            Ok(value) => ScriptOutcome {
                value: extract_owned(&interp.state, value),
                error: None,
                stdout,
            },
            Err(e) => ScriptOutcome {
                value: ScriptValueOwned::Nil,
                error: Some(ScriptError::Runtime(format!("{e:?}"))),
                stdout,
            },
        }
    }
}

impl Default for ScriptWorld {
    fn default() -> Self {
        Self::new()
    }
}

/// Marshal a script-interpreter `Value` into an owned [`ScriptValueOwned`],
/// copying any heap text/structure out while the interpreter heap is still
/// alive. Takes the `InterpreterState` directly (rather than `&Interpreter`) so
/// it is reusable from dispatch handlers — e.g. `script_set_*`, which only holds
/// `&mut InterpreterState`. `read_text` / `list_elements` / `map_entries` all
/// live on the state, so nothing here needs the full interpreter.
pub(crate) fn extract_owned(state: &InterpreterState, value: Value) -> ScriptValueOwned {
    if value.is_unit() || value.is_nil() {
        ScriptValueOwned::Nil
    } else if value.is_bool() {
        ScriptValueOwned::Bool(value.as_bool())
    } else if value.is_int() {
        ScriptValueOwned::Int(value.as_i64())
    } else if value.is_float() {
        ScriptValueOwned::Float(value.as_f64())
    } else if let Some(s) = state.read_text(value) {
        ScriptValueOwned::Text(s)
    } else if let Some(items) = state.list_elements(value) {
        ScriptValueOwned::List(
            items
                .into_iter()
                .map(|elem| extract_owned(state, elem))
                .collect(),
        )
    } else if let Some(pairs) = state.map_entries(value) {
        ScriptValueOwned::Map(
            pairs
                .into_iter()
                .map(|(k, v)| (extract_owned(state, k), extract_owned(state, v)))
                .collect(),
        )
    } else {
        ScriptValueOwned::Other
    }
}

#[cfg(all(test, feature = "codegen"))]
mod tests {
    use super::*;
    use crate::codegen::{CodegenConfig, VbcCodegen};
    use verum_ast::FileId;
    use verum_fast_parser::VerumParser;
    use verum_lexer::Lexer;

    /// A lightweight source→VBC hook for tests: parse + bare codegen (no
    /// stdlib link).  Sufficient for arithmetic scripts that touch no stdlib.
    fn lite_hook() -> CompilerHook {
        Arc::new(|source: &str| {
            let file_id = FileId::new(0);
            let lexer = Lexer::new(source, file_id);
            let parser = VerumParser::new();
            let module = parser
                .parse_module(lexer, file_id)
                .map_err(|errs| format!("parse error: {:?}", errs.first()))?;
            let mut codegen = VbcCodegen::with_config(CodegenConfig::new("script_test"));
            codegen
                .compile_module(&module)
                .map_err(|e| format!("codegen error: {e:?}"))
        })
    }

    #[test]
    fn eval_runs_main_and_returns_value() {
        install_compiler_hook(lite_hook());
        let mut engine = ScriptEngine::new();
        let result = engine
            .eval("fn main() -> Int { 20 + 22 }")
            .expect("eval should succeed");
        assert!(result.is_int());
        assert_eq!(result.as_i64(), 42);
    }

    #[test]
    fn run_named_function_with_args() {
        install_compiler_hook(lite_hook());
        let mut engine = ScriptEngine::new();
        let module = engine
            .compile("fn add(a: Int, b: Int) -> Int { a + b }")
            .expect("compile should succeed");
        let r = engine
            .run(module, "add", &[Value::from_i64(3), Value::from_i64(4)])
            .expect("run should succeed");
        assert_eq!(r.as_i64(), 7);
    }

    // --- P2 zero-copy foundation: link N scripts into one module + heap ---

    /// Two independently-compiled scripts merge (via `VbcLinker`) into ONE
    /// module whose functions run on ONE interpreter — a single shared heap.
    /// This is the substrate for zero-copy cross-script interop: objects one
    /// script's function creates live in the same heap another script reads.
    #[test]
    fn linker_merges_two_scripts_into_one_module() {
        install_compiler_hook(lite_hook());
        let mut engine = ScriptEngine::new();
        let a = engine.compile("fn a_fn() -> Int { 10 }").expect("compile a");
        let b = engine.compile("fn b_fn() -> Int { 20 }").expect("compile b");
        let mut linker = crate::linker::VbcLinker::new("aarch64-apple-darwin");
        linker.add_user_module((*a).clone()).expect("add a");
        linker.add_user_module((*b).clone()).expect("add b");
        let merged = Arc::new(linker.finalize());
        let ra = engine.run(merged.clone(), "a_fn", &[]).expect("run a_fn");
        let rb = engine.run(merged.clone(), "b_fn", &[]).expect("run b_fn");
        assert_eq!(ra.as_i64(), 10, "script A's function in the merged module");
        assert_eq!(rb.as_i64(), 20, "script B's function in the merged module");
    }

    /// Zero-copy substrate: run two linked scripts' functions on ONE persistent
    /// interpreter. What `store` writes, `load` reads back — the heap and the
    /// shared table survive across calls, so data is shared by reference (no
    /// serialization, no boundary copy) — the thing copy-at-the-boundary engines
    /// (Wasm, BEAM, V8 isolates) cannot offer. CBGR keeps the shared ref safe.
    #[test]
    fn linked_scripts_share_one_heap_across_calls() {
        install_compiler_hook(lite_hook());
        let mut engine = ScriptEngine::new();
        let a = engine
            .compile("fn store() -> Int { @intrinsic(\"script_set_int\", \"x\", 99); 0 }")
            .expect("compile a");
        let b = engine
            .compile("fn load() -> Int { @intrinsic(\"script_global_int\", \"x\") }")
            .expect("compile b");
        let mut linker = crate::linker::VbcLinker::new("aarch64-apple-darwin");
        linker.add_user_module((*a).clone()).expect("add a");
        linker.add_user_module((*b).clone()).expect("add b");
        let merged = Arc::new(linker.finalize());
        // ONE persistent interpreter — host_globals + heap survive across calls.
        let mut interp = Interpreter::new(merged.clone());
        let store_fn = merged
            .find_function_by_name("store")
            .or_else(|| merged.find_function_by_unique_bare_suffix("store"))
            .expect("store fn");
        let load_fn = merged
            .find_function_by_name("load")
            .or_else(|| merged.find_function_by_unique_bare_suffix("load"))
            .expect("load fn");
        interp
            .execute_function_with_args(store_fn, &[])
            .expect("run store");
        let r = interp
            .execute_function_with_args(load_fn, &[])
            .expect("run load");
        assert_eq!(r.as_i64(), 99, "load reads store's write — shared persistent state");
    }

    /// A direct cross-script CALL (script B calling a function DEFINED in script
    /// A) needs an extern/forward declaration: the compiler resolves calls
    /// within a module, so `helper()` is undefined when B is compiled alone.
    /// The linker merges DEFINED functions (see the tests above); wiring
    /// cross-script SYMBOL resolution (extern decls) is the next P2 step.
    #[test]
    fn cross_script_call_needs_extern_decl() {
        install_compiler_hook(lite_hook());
        let mut engine = ScriptEngine::new();
        let err = engine
            .compile("fn entry() -> Int { helper() + 1 }")
            .expect_err("undefined cross-script fn must not compile standalone");
        assert!(
            matches!(err, ScriptError::Compile(_)),
            "expected a compile error for the undefined cross-script call, got {err:?}"
        );
    }

    #[test]
    fn missing_entry_is_reported() {
        install_compiler_hook(lite_hook());
        let mut engine = ScriptEngine::new();
        let err = engine
            .eval("fn other() -> Int { 1 }")
            .expect_err("eval should fail: no main");
        assert!(matches!(err, ScriptError::EntryNotFound(_)));
    }

    #[test]
    fn outcome_classifies_and_extracts_value() {
        install_compiler_hook(lite_hook());
        let mut engine = ScriptEngine::new();
        let outcome = engine.eval_to_outcome("fn main() -> Int { 7 * 6 }");
        assert!(outcome.is_ok());
        assert_eq!(outcome.kind(), 2, "Int kind tag");
        assert_eq!(outcome.as_int(), 42);

        let failed = engine.eval_to_outcome("fn other() -> Int { 1 }");
        assert!(!failed.is_ok());
        assert!(failed.error.is_some());
    }

    #[test]
    fn text_result_marshals_to_owned_string() {
        install_compiler_hook(lite_hook());
        let mut engine = ScriptEngine::new();

        // Heap text (> 6 bytes) — exercises the heap reader; the string is
        // copied out before the script interpreter's heap is freed.
        let heap = engine.eval_to_outcome("fn main() -> Text { \"hello from the script\" }");
        assert!(heap.is_ok());
        assert_eq!(heap.kind(), 4, "Text kind tag");
        assert_eq!(heap.as_text(), "hello from the script");

        // Inline small string (<= 6 bytes).
        let small = engine.eval_to_outcome("fn main() -> Text { \"hi\" }");
        assert_eq!(small.kind(), 4);
        assert_eq!(small.as_text(), "hi");
    }

    /// Exercises the Phase-1 intrinsics through a serialization round-trip:
    /// the 4-register `script_engine_new_sandboxed` decode (new operand count)
    /// plus `script_outcome_kind`. Regression for the same decode-arm class as
    /// the eval round-trip test.
    #[test]
    fn phase1_sandboxed_and_kind_survive_roundtrip() {
        let hook = lite_hook();
        install_compiler_hook(hook.clone());

        let outer = r#"
            fn main() -> Int {
                let e = @intrinsic("script_engine_new_sandboxed", 0, 1000000, 0);
                let o = @intrinsic("script_engine_eval", e, "fn main() -> Int { 100 + 23 }");
                let k = @intrinsic("script_outcome_kind", o);
                @intrinsic("script_outcome_free", o);
                @intrinsic("script_engine_free", e);
                k
            }
        "#;

        let module = hook(outer).expect("host program should compile");
        let bytes = crate::serialize::serialize_module(&module).expect("serialize");
        let module = Arc::new(crate::deserialize::deserialize_module(&bytes).expect("deserialize"));
        let func_id = module
            .find_function_by_unique_bare_suffix("main")
            .expect("host main exists");
        let mut interp = Interpreter::new(module);
        let result = interp
            .execute_function(func_id)
            .expect("host program should run");
        // The inner script returns Int 123 → kind tag 2.
        assert_eq!(
            result.as_i64(),
            2,
            "sandboxed engine + outcome_kind must survive the round-trip"
        );
    }

    #[test]
    fn script_returns_list_extracts_structurally() {
        install_compiler_hook(lite_hook());
        let mut engine = ScriptEngine::new();
        // No `-> List<Int>` annotation: the lite/script compile path has no
        // stdlib, so `List` isn't a known type name there — the array literal
        // is what marshals (a runtime List object) regardless.
        let out = engine.eval_to_outcome("fn main() { [10, 20, 30] }");
        assert!(out.is_ok(), "list script: {:?}", out.error);
        assert_eq!(out.kind(), 5, "List kind tag");
        match &out.value {
            ScriptValueOwned::List(items) => {
                assert_eq!(items.len(), 3, "three elements");
                assert!(matches!(items[0], ScriptValueOwned::Int(10)));
                assert!(matches!(items[1], ScriptValueOwned::Int(20)));
                assert!(matches!(items[2], ScriptValueOwned::Int(30)));
            }
            other => panic!("expected List, got {other:?}"),
        }
        // The indexed accessors (used by the .vr marshaling layer) agree.
        assert_eq!(out.list_len(), 3);
        assert_eq!(out.list_elem_kind(0), 2, "element is Int");
        assert_eq!(out.list_elem_int(1), 20);
    }

    #[test]
    fn host_to_script_global_exchange() {
        install_compiler_hook(lite_hook());
        let mut engine = ScriptEngine::new();
        engine.set_global("limit", ScriptValueOwned::Int(99));

        // The script reads the host-seeded Int global and returns it.
        let got = engine.eval_to_outcome(
            "fn main() -> Int { @intrinsic(\"script_global_int\", \"limit\") }",
        );
        assert!(got.is_ok());
        assert_eq!(got.as_int(), 99, "script must read the host-set Int global");

        // An absent global reads as 0.
        let missing = engine
            .eval_to_outcome("fn main() -> Int { @intrinsic(\"script_global_int\", \"nope\") }");
        assert_eq!(missing.as_int(), 0);
    }

    /// The host<->script value exchange is complete over every scalar: Float
    /// and Bool round-trip just like Int and Text.
    #[test]
    fn host_to_script_float_bool_exchange() {
        install_compiler_hook(lite_hook());
        let mut engine = ScriptEngine::new();
        engine.set_global("ratio", ScriptValueOwned::Float(1.5));
        engine.set_global("flag", ScriptValueOwned::Bool(true));

        let f = engine.eval_to_outcome(
            "fn main() -> Float { @intrinsic(\"script_global_float\", \"ratio\") }",
        );
        assert_eq!(f.kind(), 3, "Float kind tag");
        assert!((f.as_float() - 1.5).abs() < 1e-9, "script reads host Float");

        let b = engine
            .eval_to_outcome("fn main() -> Bool { @intrinsic(\"script_global_bool\", \"flag\") }");
        assert_eq!(b.kind(), 1, "Bool kind tag");
        assert!(b.as_bool(), "script reads host Bool");
    }

    /// Full vertical slice of host-function callbacks: a host program defines
    /// `double`, registers it, and evals a script that calls it back via
    /// `script_host_call_int` — the call re-enters the host interpreter,
    /// runs `double(21)`, and marshals 42 back into the script.
    #[test]
    fn script_calls_registered_host_function() {
        let hook = lite_hook();
        install_compiler_hook(hook.clone());

        let host_src = r#"
            fn double(x: Int) -> Int { x * 2 }
            fn main() -> Int {
                let e = @intrinsic("script_engine_new");
                @intrinsic("script_engine_register", e, "double", double);
                let o = @intrinsic("script_engine_eval", e,
                    "fn main() -> Int { @intrinsic(\"script_host_call_int\", \"double\", 21) }");
                let n = @intrinsic("script_outcome_as_int", o);
                @intrinsic("script_outcome_free", o);
                @intrinsic("script_engine_free", e);
                n
            }
        "#;

        let module = Arc::new(hook(host_src).expect("host program should compile"));
        let func_id = module
            .find_function_by_unique_bare_suffix("main")
            .expect("host main exists");
        let mut interp = Interpreter::new(module);
        let result = interp
            .execute_function(func_id)
            .expect("host program should run");
        assert_eq!(
            result.as_i64(),
            42,
            "script's host call double(21) must re-enter the host and return 42"
        );
    }

    /// P2: scripts in one `ScriptWorld` share data through its persistent
    /// shared table — script B reads the Int and script C the Text that script
    /// A stored, and the host can read the table directly.
    #[test]
    fn world_shares_data_between_scripts() {
        install_compiler_hook(lite_hook());
        let mut world = ScriptWorld::new();

        // Script A shares an Int and a Text into the world.
        let a = world.eval(
            "fn main() { @intrinsic(\"script_set_int\", \"counter\", 41); \
             @intrinsic(\"script_set_text\", \"greeting\", \"shared text value\") }",
        );
        assert!(a.is_ok(), "script A should run: {:?}", a.error);

        // Script B reads the Int script A shared.
        let b = world.eval("fn main() -> Int { @intrinsic(\"script_global_int\", \"counter\") + 1 }");
        assert_eq!(b.as_int(), 42, "script B must see script A's shared Int");

        // Script C reads the Text script A shared.
        let c =
            world.eval("fn main() -> Text { @intrinsic(\"script_global_text\", \"greeting\") }");
        assert_eq!(c.kind(), 4, "shared value is Text");
        assert_eq!(
            c.as_text(),
            "shared text value",
            "script C must read script A's shared Text"
        );

        // The host can also read the world's shared table directly.
        assert!(matches!(
            world.get_shared("counter"),
            Some(ScriptValueOwned::Int(41))
        ));
    }

    #[test]
    fn sandbox_denies_process_capability() {
        install_compiler_hook(lite_hook());

        // A sandboxed engine has no ambient authority: the Process capability
        // (here, `exit`) is denied, so the script traps instead of running it.
        let mut sandboxed = ScriptEngine::sandboxed(0, 0, 0);
        let denied =
            sandboxed.eval_to_outcome("fn main() { @intrinsic(\"verum.process.exit\", 0) }");
        assert!(
            !denied.is_ok(),
            "sandboxed script must be denied the Process (exit) capability"
        );

        // (The grant path can't be unit-tested here: a permitted `exit` would
        // terminate the test process. The deny path proves the gate is wired.)

        // A sandboxed script that touches no gated capability runs fine.
        let ok = sandboxed.eval_to_outcome("fn main() -> Int { 2 + 3 }");
        assert!(ok.is_ok());
        assert_eq!(ok.as_int(), 5);
    }

    #[test]
    fn sandbox_instruction_limit_aborts_runaway_script() {
        install_compiler_hook(lite_hook());

        // A 5k-instruction cap aborts a script that would otherwise loop ~1M
        // times — the limit is enforced by the interpreter's existing fuel
        // counter, surfaced as a failed outcome.
        let mut bounded = ScriptEngine::sandboxed(0, 5_000, 0);
        let runaway = bounded
            .eval_to_outcome("fn main() -> Int { let mut i = 0; while i < 1000000 { i = i + 1 } i }");
        assert!(
            !runaway.is_ok(),
            "runaway script must hit the 5k instruction limit"
        );

        // The same shape, well under the cap and under default limits, runs.
        let mut unbounded = ScriptEngine::new();
        let ok = unbounded
            .eval_to_outcome("fn main() -> Int { let mut i = 0; while i < 1000 { i = i + 1 } i }");
        assert!(ok.is_ok(), "bounded script should succeed");
        assert_eq!(ok.as_int(), 1000);
    }

    /// Full vertical slice through the VBC `@intrinsic` surface: a host
    /// program creates an engine, evaluates a nested script via the
    /// `script_*` Extended sub-ops, marshals the result back, and frees the
    /// handles — exactly the path `core.script` will wrap, but exercised
    /// without a stdlib rebuild.
    #[test]
    fn end_to_end_eval_via_intrinsics() {
        let hook = lite_hook();
        install_compiler_hook(hook.clone());

        let outer = r#"
            fn main() -> Int {
                let e = @intrinsic("script_engine_new");
                let o = @intrinsic("script_engine_eval", e, "fn main() -> Int { 7 * 6 }");
                let n = @intrinsic("script_outcome_as_int", o);
                @intrinsic("script_outcome_free", o);
                @intrinsic("script_engine_free", e);
                n
            }
        "#;

        let module = Arc::new(hook(outer).expect("host program should compile"));
        let func_id = module
            .find_function_by_unique_bare_suffix("main")
            .expect("host main exists");
        let mut interp = Interpreter::new(module);
        let result = interp
            .execute_function(func_id)
            .expect("host program should run");
        assert_eq!(
            result.as_i64(),
            42,
            "nested script `7 * 6` should marshal back through the outcome handle"
        );
    }

    /// Same vertical slice, but the host module is serialized and deserialized
    /// before running — the path the stdlib actually takes (archive store +
    /// linker decode→re-encode).  Regression for the Extended-sub-op
    /// operand-loss bug: an in-memory run (above) passes even when the decoder
    /// drops the script-op operands, because it never round-trips; this test
    /// fails unless the decoder reads + advances past them.
    #[test]
    fn end_to_end_eval_survives_serialization_roundtrip() {
        let hook = lite_hook();
        install_compiler_hook(hook.clone());

        let outer = r#"
            fn main() -> Int {
                let e = @intrinsic("script_engine_new");
                let o = @intrinsic("script_engine_eval", e, "fn main() -> Int { 7 * 6 }");
                let n = @intrinsic("script_outcome_as_int", o);
                @intrinsic("script_outcome_free", o);
                @intrinsic("script_engine_free", e);
                n
            }
        "#;

        let module = hook(outer).expect("host program should compile");
        // Round-trip through (de)serialization before running.
        let bytes = crate::serialize::serialize_module(&module).expect("serialize");
        let module = crate::deserialize::deserialize_module(&bytes).expect("deserialize");
        let module = Arc::new(module);

        let func_id = module
            .find_function_by_unique_bare_suffix("main")
            .expect("host main exists");
        let mut interp = Interpreter::new(module);
        let result = interp
            .execute_function(func_id)
            .expect("host program should run");
        assert_eq!(
            result.as_i64(),
            42,
            "script-op operands must survive serialization round-trip"
        );
    }
}
