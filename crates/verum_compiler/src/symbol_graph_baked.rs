//! The archive's call-graph index, in a form that is READ rather than
//! DERIVED at compiler start-up (T0753).
//!
//! # Why this file exists
//!
//! `SymbolGraph` answers four questions about the baked standard
//! library — which entry defines a qualified name, which qualified
//! names share a leaf, which share a first segment, and what each
//! function calls.  None of those answers depend on the program being
//! compiled, yet the graph was rebuilt on every process start by
//! decoding all 590 archive entries and disassembling every function
//! body.  Measured on a program whose whole text is
//! `fn main() { print("hello") }`: 350–810 ms, which was 87 % of that
//! program's archive-load phase, and the bulk of a ~700 MB peak.
//!
//! The bake already writes two sidecars next to `runtime.vbca`; this
//! is the third.  The rule it serves is the standing one: nothing
//! derivable at bake time may be re-derived at load time.
//!
//! # Format
//!
//! One byte block, all integers little-endian `u32`, every section
//! `u32`-aligned.  Strings live in a single blob addressed by an
//! offset table, and every other section stores string INDICES — so a
//! name that appears as a descriptor, as a callee of forty functions
//! and as a leaf key is stored once.
//!
//! ```text
//!   header (see HEADER_LEN)
//!   str_off    u32[n_strings + 1]   offsets into str_blob
//!   str_blob   bytes                concatenated UTF-8, no separators
//!   func_name  u32[n_funcs]         string idx, SORTED by string bytes
//!   func_mod   u32[n_funcs]         archive entry index, parallel to func_name
//!   edge_off   u32[n_funcs + 1]     CSR range into edge_tgt
//!   edge_tgt   u32[n_edges]         callee string idx
//!   leaf_key   u32[n_leaf]          string idx, SORTED
//!   leaf_off   u32[n_leaf + 1]      CSR range into leaf_val
//!   leaf_val   u32[...]             function index (into func_name)
//!   pref_key   u32[n_pref]          string idx, SORTED
//!   pref_off   u32[n_pref + 1]
//!   pref_val   u32[...]
//!   entry_name u32[n_entries]       string idx, in ARCHIVE ORDER
//!   entry_sort u32[n_entries]       entry indices, SORTED by name
//! ```
//!
//! Lookups are binary searches over a sorted index array, comparing
//! the strings they address: O(log n) with n ≈ 44 000, against the
//! `HashMap`'s O(1) — sixteen string compares per lookup, on a path
//! that performs a few tens of thousands of them, against a build
//! step that cost hundreds of milliseconds.
//!
//! # One representation, two sources
//!
//! [`BakedSymbolGraph`] is also what the FALLBACK path produces: when
//! no sidecar is embedded (a compiler built without a bake, or an
//! archive loaded from a file), the graph is scanned from the archive
//! and then encoded into these same bytes.  Two producers, one reader
//! — so the sidecar path can never drift away from the path that
//! still has to work without it.

use std::borrow::Cow;

/// Section offsets and counts, in `u32` slots.
const MAGIC: &[u8; 4] = b"VSG1";

/// Bump when the layout below changes.  The bake mixes this into the
/// precompile schema version, so a stale sidecar is regenerated rather
/// than misread.
pub const FORMAT_VERSION: u32 = 1;

const H_MAGIC: usize = 0;
const H_VERSION: usize = 1;
const H_N_STRINGS: usize = 2;
const H_N_FUNCS: usize = 3;
const H_N_ENTRIES: usize = 4;
const H_N_LEAF: usize = 5;
const H_N_PREF: usize = 6;
const H_OFF_STR_OFF: usize = 7;
const H_OFF_STR_BLOB: usize = 8;
const H_OFF_FUNC_NAME: usize = 9;
const H_OFF_FUNC_MOD: usize = 10;
const H_OFF_EDGE_OFF: usize = 11;
const H_OFF_EDGE_TGT: usize = 12;
const H_OFF_LEAF_KEY: usize = 13;
const H_OFF_LEAF_OFF: usize = 14;
const H_OFF_LEAF_VAL: usize = 15;
const H_OFF_PREF_KEY: usize = 16;
const H_OFF_PREF_OFF: usize = 17;
const H_OFF_PREF_VAL: usize = 18;
const H_OFF_ENTRY_NAME: usize = 19;
const H_OFF_ENTRY_SORT: usize = 20;
const H_TOTAL_LEN: usize = 21;
/// Number of `u32` slots in the header.
const HEADER_SLOTS: usize = 22;
const HEADER_LEN: usize = HEADER_SLOTS * 4;

/// One function as the encoder sees it: its qualified descriptor name,
/// the archive entry that defines it, and the callee names its body
/// emits.
pub struct EncodedFunction {
    pub name: String,
    pub module: u32,
    pub callees: Vec<String>,
}

/// Serialise the graph.  `entries` is the archive's entry-name list in
/// ARCHIVE ORDER — entry indices are positions in it, and the loader
/// uses them to address `archive.index`, so the order is part of the
/// contract, not a detail.
///
/// `funcs` must already carry the caller's first-wins discipline: a
/// name repeated across entries keeps its first `module`, matching
/// `register_function`'s rule.  The encoder does not re-decide it; it
/// keeps the FIRST occurrence of a duplicated name and drops later
/// ones, which is the same rule expressed once more so a careless
/// caller cannot produce two rows for one name.
///
/// `leaf_index` and `prefix_index` arrive READY rather than being
/// derived here, and that is deliberate: the scanner indexes a
/// function under its descriptor spelling but NOT under the canonical
/// re-spelling it also registers, so deriving the two indexes from the
/// function list would widen every leaf fanout by the alias set.  The
/// encoder must not have an opinion about which names are indexable —
/// only the scanner knows.
pub fn encode(
    entries: &[String],
    funcs: &[EncodedFunction],
    leaf_index: &[(String, Vec<String>)],
    prefix_index: &[(String, Vec<String>)],
) -> Vec<u8> {
    use std::collections::HashMap;

    // ── string interning ────────────────────────────────────────────
    //
    // Owned, because the sections below are built in several passes
    // and a borrowed interner would tie the whole encoder to the
    // caller's lifetimes for no gain: this runs once, at bake time.
    let mut strings: Vec<String> = Vec::new();
    let mut string_idx: HashMap<String, u32> = HashMap::new();
    macro_rules! intern {
        ($s:expr) => {{
            let s: &str = $s;
            match string_idx.get(s) {
                Some(i) => *i,
                None => {
                    let i = strings.len() as u32;
                    strings.push(s.to_string());
                    string_idx.insert(s.to_string(), i);
                    i
                }
            }
        }};
    }

    let entry_name_idx: Vec<u32> = entries.iter().map(|e| intern!(e.as_str())).collect();

    // ── functions, deduplicated first-wins ──────────────────────────
    //
    // The caller already applies `register_function`'s first-wins rule
    // when it collects these rows; stating it again here means a
    // careless caller cannot produce two rows for one name and get a
    // graph whose binary search finds whichever landed later.
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut func_rows: Vec<(u32 /*name idx*/, u32 /*module*/, &[String])> = Vec::new();
    for f in funcs {
        if !seen.insert(f.name.as_str()) {
            continue;
        }
        let ni = intern!(f.name.as_str());
        func_rows.push((ni, f.module, f.callees.as_slice()));
    }
    for f in funcs {
        for c in &f.callees {
            let _ = intern!(c.as_str());
        }
    }

    // Sort function rows by name bytes so the reader can binary-search.
    func_rows.sort_by(|a, b| strings[a.0 as usize].cmp(&strings[b.0 as usize]));

    // Post-sort position of each function name — the leaf/prefix value
    // lists address functions by their SORTED position.
    let mut pos_by_name: HashMap<&str, u32> = HashMap::with_capacity(func_rows.len());
    for (pos, row) in func_rows.iter().enumerate() {
        pos_by_name.insert(strings[row.0 as usize].as_str(), pos as u32);
    }
    let pos_by_name: HashMap<String, u32> = pos_by_name
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

    // ── leaf / prefix indices, as the scanner recorded them ─────────
    let mut leaf_map: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut pref_map: HashMap<u32, Vec<u32>> = HashMap::new();
    for (key, names) in leaf_index {
        let ki = intern!(key.as_str());
        let vals: Vec<u32> = names
            .iter()
            .filter_map(|n| pos_by_name.get(n.as_str()).copied())
            .collect();
        leaf_map.entry(ki).or_default().extend(vals);
    }
    for (key, names) in prefix_index {
        let ki = intern!(key.as_str());
        let vals: Vec<u32> = names
            .iter()
            .filter_map(|n| pos_by_name.get(n.as_str()).copied())
            .collect();
        pref_map.entry(ki).or_default().extend(vals);
    }

    let sort_keys = |map: &HashMap<u32, Vec<u32>>, strings: &[String]| -> Vec<u32> {
        let mut keys: Vec<u32> = map.keys().copied().collect();
        keys.sort_by(|a, b| strings[*a as usize].cmp(&strings[*b as usize]));
        keys
    };
    let leaf_keys = sort_keys(&leaf_map, &strings);
    let pref_keys = sort_keys(&pref_map, &strings);

    let mut entry_sort: Vec<u32> = (0..entries.len() as u32).collect();
    entry_sort.sort_by(|a, b| entries[*a as usize].cmp(&entries[*b as usize]));

    // ── layout ──────────────────────────────────────────────────────
    let n_strings = strings.len();
    let n_funcs = func_rows.len();
    let n_entries = entries.len();
    let n_leaf = leaf_keys.len();
    let n_pref = pref_keys.len();
    let n_edges: usize = func_rows.iter().map(|r| r.2.len()).sum();
    let blob_len: usize = strings.iter().map(|s| s.len()).sum();

    let mut out: Vec<u8> = Vec::with_capacity(HEADER_LEN + blob_len + (n_edges + n_funcs) * 8);
    out.resize(HEADER_LEN, 0);

    let mut header = [0u32; HEADER_SLOTS];
    header[H_MAGIC] = u32::from_le_bytes(*MAGIC);
    header[H_VERSION] = FORMAT_VERSION;
    header[H_N_STRINGS] = n_strings as u32;
    header[H_N_FUNCS] = n_funcs as u32;
    header[H_N_ENTRIES] = n_entries as u32;
    header[H_N_LEAF] = n_leaf as u32;
    header[H_N_PREF] = n_pref as u32;

    fn push_u32s(out: &mut Vec<u8>, vals: impl Iterator<Item = u32>) -> u32 {
        let at = out.len() as u32;
        for v in vals {
            out.extend_from_slice(&v.to_le_bytes());
        }
        at
    }

    // str_off + str_blob
    let mut acc: u32 = 0;
    let mut str_offsets: Vec<u32> = Vec::with_capacity(n_strings + 1);
    for s in &strings {
        str_offsets.push(acc);
        acc += s.len() as u32;
    }
    str_offsets.push(acc);
    header[H_OFF_STR_OFF] = push_u32s(&mut out, str_offsets.into_iter());
    header[H_OFF_STR_BLOB] = out.len() as u32;
    for s in &strings {
        out.extend_from_slice(s.as_bytes());
    }
    while out.len() % 4 != 0 {
        out.push(0);
    }

    header[H_OFF_FUNC_NAME] = push_u32s(&mut out, func_rows.iter().map(|r| r.0));
    header[H_OFF_FUNC_MOD] = push_u32s(&mut out, func_rows.iter().map(|r| r.1));

    let mut edge_off: Vec<u32> = Vec::with_capacity(n_funcs + 1);
    let mut acc: u32 = 0;
    for row in &func_rows {
        edge_off.push(acc);
        acc += row.2.len() as u32;
    }
    edge_off.push(acc);
    header[H_OFF_EDGE_OFF] = push_u32s(&mut out, edge_off.into_iter());
    let edge_targets: Vec<u32> = func_rows
        .iter()
        .flat_map(|r| r.2.iter())
        .map(|c| string_idx[c.as_str()])
        .collect();
    header[H_OFF_EDGE_TGT] = push_u32s(&mut out, edge_targets.into_iter());

    let write_index = |keys: &[u32], map: &HashMap<u32, Vec<u32>>, out: &mut Vec<u8>| {
        let k_at = push_u32s(out, keys.iter().copied());
        let mut offs: Vec<u32> = Vec::with_capacity(keys.len() + 1);
        let mut acc: u32 = 0;
        for k in keys {
            offs.push(acc);
            acc += map[k].len() as u32;
        }
        offs.push(acc);
        let o_at = push_u32s(out, offs.into_iter());
        let vals: Vec<u32> = keys.iter().flat_map(|k| map[k].iter().copied()).collect();
        let v_at = push_u32s(out, vals.into_iter());
        (k_at, o_at, v_at)
    };
    let (lk, lo, lv) = write_index(&leaf_keys, &leaf_map, &mut out);
    header[H_OFF_LEAF_KEY] = lk;
    header[H_OFF_LEAF_OFF] = lo;
    header[H_OFF_LEAF_VAL] = lv;
    let (pk, po, pv) = write_index(&pref_keys, &pref_map, &mut out);
    header[H_OFF_PREF_KEY] = pk;
    header[H_OFF_PREF_OFF] = po;
    header[H_OFF_PREF_VAL] = pv;

    header[H_OFF_ENTRY_NAME] = push_u32s(&mut out, entry_name_idx.into_iter());
    header[H_OFF_ENTRY_SORT] = push_u32s(&mut out, entry_sort.into_iter());
    header[H_TOTAL_LEN] = out.len() as u32;

    for (i, v) in header.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

/// A symbol graph backed by encoded bytes — either the embedded
/// sidecar (`&'static [u8]`, no copy, paged in by the OS as it is
/// touched) or a freshly encoded fallback (owned).
pub struct BakedSymbolGraph {
    bytes: Cow<'static, [u8]>,
}

impl BakedSymbolGraph {
    /// Wrap already-encoded bytes.  Returns `None` when they are not a
    /// graph this build can read — a missing sidecar, a truncated file,
    /// or one written by a different format version.  Every caller has
    /// a working fallback, so a rejected sidecar costs time, never
    /// correctness.
    pub fn from_bytes(bytes: Cow<'static, [u8]>) -> Option<Self> {
        if bytes.len() < HEADER_LEN {
            return None;
        }
        let g = Self { bytes };
        if g.slot(H_MAGIC) != u32::from_le_bytes(*MAGIC)
            || g.slot(H_VERSION) != FORMAT_VERSION
            || g.slot(H_TOTAL_LEN) as usize != g.bytes.len()
        {
            return None;
        }
        Some(g)
    }

    /// Encode and wrap in one step — the fallback path's constructor.
    pub fn from_parts(
        entries: &[String],
        funcs: &[EncodedFunction],
        leaf_index: &[(String, Vec<String>)],
        prefix_index: &[(String, Vec<String>)],
    ) -> Self {
        Self {
            bytes: Cow::Owned(encode(entries, funcs, leaf_index, prefix_index)),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    fn slot(&self, i: usize) -> u32 {
        let at = i * 4;
        u32::from_le_bytes([
            self.bytes[at],
            self.bytes[at + 1],
            self.bytes[at + 2],
            self.bytes[at + 3],
        ])
    }

    fn u32_at(&self, byte_off: u32, i: u32) -> u32 {
        let at = byte_off as usize + i as usize * 4;
        u32::from_le_bytes([
            self.bytes[at],
            self.bytes[at + 1],
            self.bytes[at + 2],
            self.bytes[at + 3],
        ])
    }

    /// The `i`-th interned string.
    fn s(&self, i: u32) -> &str {
        let off_tab = self.slot(H_OFF_STR_OFF);
        let blob = self.slot(H_OFF_STR_BLOB) as usize;
        let start = blob + self.u32_at(off_tab, i) as usize;
        let end = blob + self.u32_at(off_tab, i + 1) as usize;
        // The encoder only ever writes valid UTF-8 here, and the
        // header check rejects a file whose length disagrees with its
        // own table, so a corrupt blob cannot reach this point with a
        // plausible length.  Still not `unchecked`: a wrong answer
        // here is a silently wrong compile.
        std::str::from_utf8(&self.bytes[start..end]).unwrap_or("")
    }

    pub fn function_count(&self) -> usize {
        self.slot(H_N_FUNCS) as usize
    }

    pub fn entry_count(&self) -> usize {
        self.slot(H_N_ENTRIES) as usize
    }

    /// Binary-search a sorted array of string indices for `needle`.
    fn find_sorted(&self, keys_off: u32, n: u32, needle: &str) -> Option<u32> {
        let (mut lo, mut hi) = (0u32, n);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let s = self.s(self.u32_at(keys_off, mid));
            match s.cmp(needle) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(mid),
            }
        }
        None
    }

    /// Position of `name` in the sorted function table.
    pub fn function_index(&self, name: &str) -> Option<u32> {
        self.find_sorted(self.slot(H_OFF_FUNC_NAME), self.slot(H_N_FUNCS), name)
    }

    pub fn function_name(&self, idx: u32) -> &str {
        self.s(self.u32_at(self.slot(H_OFF_FUNC_NAME), idx))
    }

    /// Archive entry index that defines `name`, first-wins.
    pub fn module_of(&self, name: &str) -> Option<u32> {
        let i = self.function_index(name)?;
        Some(self.u32_at(self.slot(H_OFF_FUNC_MOD), i))
    }

    pub fn module_of_index(&self, idx: u32) -> u32 {
        self.u32_at(self.slot(H_OFF_FUNC_MOD), idx)
    }

    /// Callee names emitted by the body of function `idx`.
    pub fn callees(&self, idx: u32) -> impl Iterator<Item = &str> + '_ {
        let off = self.slot(H_OFF_EDGE_OFF);
        let start = self.u32_at(off, idx);
        let end = self.u32_at(off, idx + 1);
        let tgt = self.slot(H_OFF_EDGE_TGT);
        (start..end).map(move |i| self.s(self.u32_at(tgt, i)))
    }

    fn index_lookup(
        &self,
        keys_off: u32,
        n: u32,
        offs_off: u32,
        vals_off: u32,
        key: &str,
    ) -> Option<impl Iterator<Item = u32> + '_> {
        let k = self.find_sorted(keys_off, n, key)?;
        let start = self.u32_at(offs_off, k);
        let end = self.u32_at(offs_off, k + 1);
        Some((start..end).map(move |i| self.u32_at(vals_off, i)))
    }

    /// Function indices whose qualified name ends in `.<leaf>`.
    pub fn leaf_matches(&self, leaf: &str) -> Option<impl Iterator<Item = u32> + '_> {
        self.index_lookup(
            self.slot(H_OFF_LEAF_KEY),
            self.slot(H_N_LEAF),
            self.slot(H_OFF_LEAF_OFF),
            self.slot(H_OFF_LEAF_VAL),
            leaf,
        )
    }

    /// How many qualified names share this leaf — the number the
    /// fanout cap is compared against, answered without materialising
    /// the list.
    pub fn leaf_match_count(&self, leaf: &str) -> usize {
        let keys = self.slot(H_OFF_LEAF_KEY);
        let Some(k) = self.find_sorted(keys, self.slot(H_N_LEAF), leaf) else {
            return 0;
        };
        let offs = self.slot(H_OFF_LEAF_OFF);
        (self.u32_at(offs, k + 1) - self.u32_at(offs, k)) as usize
    }

    /// Function indices whose qualified name starts with `<prefix>.`.
    pub fn prefix_matches(&self, prefix: &str) -> Option<impl Iterator<Item = u32> + '_> {
        self.index_lookup(
            self.slot(H_OFF_PREF_KEY),
            self.slot(H_N_PREF),
            self.slot(H_OFF_PREF_OFF),
            self.slot(H_OFF_PREF_VAL),
            prefix,
        )
    }

    /// True when SOME archive symbol is spelled exactly `name`, either
    /// as a leaf of a qualified name or as a whole descriptor name.
    pub fn carries_simple_name(&self, name: &str) -> bool {
        self.find_sorted(self.slot(H_OFF_LEAF_KEY), self.slot(H_N_LEAF), name)
            .is_some()
            || self.function_index(name).is_some()
    }

    pub fn entry_name(&self, idx: u32) -> &str {
        self.s(self.u32_at(self.slot(H_OFF_ENTRY_NAME), idx))
    }

    /// Archive entry index for an entry NAME (`core.text`).
    pub fn entry_index(&self, name: &str) -> Option<u32> {
        let sort = self.slot(H_OFF_ENTRY_SORT);
        let n = self.slot(H_N_ENTRIES);
        let (mut lo, mut hi) = (0u32, n);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let e = self.u32_at(sort, mid);
            match self.entry_name(e).cmp(name) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(e),
            }
        }
        None
    }

    /// Longest-dot-prefix home-module resolution for a qualified name
    /// with no function descriptor (variant constructor, FFI extern,
    /// re-export spelling).  `a.b.c.D` tries `a.b.c`, then `a.b`, then
    /// `a`.  Bounded by segment count — no fanout.
    pub fn home_module_of(&self, name: &str) -> Option<u32> {
        let mut prefix = name;
        while let Some(pos) = prefix.rfind('.') {
            prefix = &prefix[..pos];
            if let Some(idx) = self.entry_index(prefix) {
                return Some(idx);
            }
        }
        None
    }
}
