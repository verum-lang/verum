//! `verum cache` — inspect and manage the script-mode VBC cache.
//!
//! P5.2 — surfaces the [`crate::script::cache::ScriptCache`] persistent
//! cache (default: `~/.verum/script-cache/`) to end users via four
//! subcommands:
//!
//!   - `path`     — print the cache root.
//!   - `list`     — table of cached entries with size and last-access.
//!   - `clear`    — delete every cache entry (`--yes` to skip prompt).
//!   - `gc`       — evict least-recently-used until under a budget.
//!   - `show`     — dump meta.toml for one entry by hex key prefix.
//!
//! All subcommands accept `--root <PATH>` to point at a non-default
//! cache root (test-rigs and CI invocations rely on this).
//!
//! The on-disk format is documented in [`crate::script::cache`]; this
//! module is purely UX glue.

use crate::error::{CliError, Result};
use crate::script::cache::{CacheError, CacheKey, CacheMeta, ScriptCache};
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(clap::Subcommand)]
pub enum CacheCommands {
    /// Print the cache root directory.
    Path {
        /// Override the cache root (default: `$HOME/.verum/script-cache`).
        #[clap(long, value_name = "DIR")]
        root: Option<PathBuf>,
    },
    /// List cached entries (newest access first by default).
    List {
        /// Override the cache root.
        #[clap(long, value_name = "DIR")]
        root: Option<PathBuf>,
        /// Sort key: `accessed` (default), `created`, `size`, `key`.
        #[clap(long, default_value = "accessed")]
        sort: String,
        /// Maximum rows to print. Use 0 for unbounded.
        #[clap(long, default_value = "100")]
        limit: usize,
        /// Emit one JSON object per line (newline-delimited JSON) instead
        /// of the human-readable table — for scripting.
        #[clap(long)]
        json: bool,
    },
    /// Delete every entry in the cache.
    Clear {
        /// Override the cache root.
        #[clap(long, value_name = "DIR")]
        root: Option<PathBuf>,
        /// Skip the confirmation prompt.
        #[clap(long, short = 'y')]
        yes: bool,
    },
    /// Evict least-recently-accessed entries until total size is at most
    /// `--max-size` bytes.
    Gc {
        /// Override the cache root.
        #[clap(long, value_name = "DIR")]
        root: Option<PathBuf>,
        /// Target maximum cache size, in bytes. Suffixes K / M / G are
        /// recognised (powers of 1024). 0 means evict everything.
        #[clap(long, value_name = "BYTES", default_value = "256M")]
        max_size: String,
        /// Print what would be evicted without removing anything.
        #[clap(long)]
        dry_run: bool,
    },
    /// Print metadata for a single entry.
    Show {
        /// Override the cache root.
        #[clap(long, value_name = "DIR")]
        root: Option<PathBuf>,
        /// 64-character blake3 hex key, or any unique prefix (≥ 4 chars).
        key: String,
    },
}

pub fn execute(cmd: CacheCommands) -> Result<()> {
    match cmd {
        CacheCommands::Path { root } => path(root),
        CacheCommands::List {
            root,
            sort,
            limit,
            json,
        } => list(root, &sort, limit, json),
        CacheCommands::Clear { root, yes } => clear(root, yes),
        CacheCommands::Gc {
            root,
            max_size,
            dry_run,
        } => gc(root, &max_size, dry_run),
        CacheCommands::Show { root, key } => show(root, &key),
    }
}

fn open(root: Option<PathBuf>) -> Result<ScriptCache> {
    match root {
        Some(p) => ScriptCache::at(p),
        None => ScriptCache::at_default(),
    }
    .map_err(into_cli_err)
}

fn into_cli_err(e: CacheError) -> CliError {
    match e {
        CacheError::Io { source, .. } => CliError::Io(source),
        CacheError::InvalidKey { name } => {
            CliError::InvalidArgument(format!("invalid cache key {name:?}"))
        }
        CacheError::InvalidMeta { reason, .. } => CliError::InvalidArgument(reason),
    }
}

fn path(root: Option<PathBuf>) -> Result<()> {
    let cache = open(root)?;
    println!("{}", cache.root().display());
    Ok(())
}

fn list(root: Option<PathBuf>, sort: &str, limit: usize, json: bool) -> Result<()> {
    let cache = open(root)?;
    let mut entries = cache.list().map_err(into_cli_err)?;
    sort_entries(&mut entries, sort)?;
    let total: u64 = entries.iter().map(|(_, m)| m.vbc_len).sum();

    let take = if limit == 0 { entries.len() } else { limit.min(entries.len()) };

    if json {
        let mut out = io::stdout().lock();
        for (key, meta) in entries.iter().take(take) {
            writeln!(
                out,
                r#"{{"key":"{}","vbc_len":{},"source_path":{},"compiler":{},"created_at":{},"accessed_at":{}}}"#,
                key.to_hex(),
                meta.vbc_len,
                json_str(&meta.source_path),
                json_str(&meta.compiler_version),
                meta.created_at,
                meta.last_accessed_at,
            )
            .map_err(CliError::Io)?;
        }
        return Ok(());
    }

    if entries.is_empty() {
        println!("(cache empty: {})", cache.root().display());
        return Ok(());
    }
    println!(
        "cache root : {}\nentries    : {}\ntotal size : {}",
        cache.root().display(),
        entries.len(),
        format_bytes(total),
    );
    println!();
    println!(
        "{:<16}  {:>10}  {:>12}  {:<20}  {}",
        "KEY", "SIZE", "ACCESSED", "COMPILER", "SOURCE"
    );
    for (key, meta) in entries.iter().take(take) {
        let hex = key.to_hex();
        let short = &hex[..16];
        println!(
            "{:<16}  {:>10}  {:>12}  {:<20}  {}",
            short,
            format_bytes(meta.vbc_len),
            relative_time(meta.last_accessed_at),
            truncate(&meta.compiler_version, 20),
            truncate(&meta.source_path, 60),
        );
    }
    if take < entries.len() {
        println!("\n({} more, pass --limit 0 to see all)", entries.len() - take);
    }
    Ok(())
}

fn sort_entries(entries: &mut [(CacheKey, CacheMeta)], key: &str) -> Result<()> {
    match key {
        "accessed" => entries.sort_by(|a, b| b.1.last_accessed_at.cmp(&a.1.last_accessed_at)),
        "created" => entries.sort_by(|a, b| b.1.created_at.cmp(&a.1.created_at)),
        "size" => entries.sort_by(|a, b| b.1.vbc_len.cmp(&a.1.vbc_len)),
        "key" => entries.sort_by(|a, b| a.0.cmp(&b.0)),
        other => {
            return Err(CliError::InvalidArgument(format!(
                "--sort must be one of accessed|created|size|key, got {other:?}"
            )));
        }
    }
    Ok(())
}

fn clear(root: Option<PathBuf>, yes: bool) -> Result<()> {
    let cache = open(root)?;
    let entries = cache.list().map_err(into_cli_err)?;
    if entries.is_empty() {
        println!("cache already empty: {}", cache.root().display());
        return Ok(());
    }
    if !yes && !confirm(&format!(
        "Remove {} cache entries from {}?",
        entries.len(),
        cache.root().display()
    ))? {
        println!("aborted");
        return Ok(());
    }
    let n = cache.clear().map_err(into_cli_err)?;
    println!("removed {n} entries");
    Ok(())
}

fn gc(root: Option<PathBuf>, max_size: &str, dry_run: bool) -> Result<()> {
    let cache = open(root)?;
    let max_bytes = parse_size(max_size)?;
    if dry_run {
        let mut entries = cache.list().map_err(into_cli_err)?;
        entries.sort_by_key(|(_, m)| m.last_accessed_at);
        let mut total: u64 = entries.iter().map(|(_, m)| m.vbc_len + 256).sum();
        let mut would_evict = 0usize;
        let mut would_free = 0u64;
        for (_, meta) in &entries {
            if total <= max_bytes {
                break;
            }
            let weight = meta.vbc_len + 256;
            total = total.saturating_sub(weight);
            would_free += weight;
            would_evict += 1;
        }
        println!(
            "dry-run: would evict {} entries, freeing {}",
            would_evict,
            format_bytes(would_free)
        );
        return Ok(());
    }
    let evicted = cache.gc_to_size(max_bytes).map_err(into_cli_err)?;
    println!("evicted {evicted} entries (target: {})", format_bytes(max_bytes));
    Ok(())
}

fn show(root: Option<PathBuf>, prefix: &str) -> Result<()> {
    if prefix.len() < 4 {
        return Err(CliError::InvalidArgument(
            "key prefix must be at least 4 characters".to_string(),
        ));
    }
    let cache = open(root)?;
    let entries = cache.list().map_err(into_cli_err)?;
    let matches: Vec<_> = entries
        .into_iter()
        .filter(|(k, _)| k.to_hex().starts_with(prefix))
        .collect();
    match matches.len() {
        0 => Err(CliError::InvalidArgument(format!(
            "no cache entry matches prefix {prefix:?}"
        ))),
        1 => {
            let (key, meta) = matches.into_iter().next().expect("len==1");
            println!("key                : {}", key.to_hex());
            println!("schema_version     : {}", meta.schema_version);
            println!("source_path        : {}", meta.source_path);
            println!("source_len         : {} bytes", meta.source_len);
            println!("compiler_version  : {}", meta.compiler_version);
            println!("created_at         : {}", meta.created_at);
            println!("last_accessed_at   : {}", meta.last_accessed_at);
            println!(
                "vbc_len            : {} bytes ({})",
                meta.vbc_len,
                format_bytes(meta.vbc_len)
            );
            Ok(())
        }
        n => {
            let mut msg = format!("prefix {prefix:?} is ambiguous: {n} entries match\n");
            for (k, _) in matches.iter().take(8) {
                msg.push_str(&format!("  {}\n", k.to_hex()));
            }
            if n > 8 {
                msg.push_str(&format!("  ... ({} more)\n", n - 8));
            }
            Err(CliError::InvalidArgument(msg))
        }
    }
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt} [y/N] ");
    io::stdout().flush().map_err(CliError::Io)?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).map_err(CliError::Io)?;
    Ok(matches!(buf.trim(), "y" | "Y" | "yes" | "YES"))
}

fn parse_size(input: &str) -> Result<u64> {
    let s = input.trim();
    if s.is_empty() {
        return Err(CliError::InvalidArgument(
            "--max-size cannot be empty".to_string(),
        ));
    }
    let (num, mult) = match s.as_bytes().last().copied() {
        Some(b'K') | Some(b'k') => (&s[..s.len() - 1], 1024u64),
        Some(b'M') | Some(b'm') => (&s[..s.len() - 1], 1024u64 * 1024),
        Some(b'G') | Some(b'g') => (&s[..s.len() - 1], 1024u64 * 1024 * 1024),
        Some(b'0'..=b'9') => (s, 1u64),
        _ => {
            return Err(CliError::InvalidArgument(format!(
                "--max-size {input:?} is not a valid byte count (use e.g. 256M, 1G, 0)"
            )));
        }
    };
    let n: u64 = num.trim().parse().map_err(|_| {
        CliError::InvalidArgument(format!("--max-size {input:?} is not numeric"))
    })?;
    n.checked_mul(mult).ok_or_else(|| {
        CliError::InvalidArgument(format!("--max-size {input:?} overflows u64"))
    })
}

fn format_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if n >= GB {
        format!("{:.2} GiB", n as f64 / GB as f64)
    } else if n >= MB {
        format!("{:.2} MiB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.2} KiB", n as f64 / KB as f64)
    } else {
        format!("{n} B")
    }
}

fn relative_time(epoch_secs: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let delta = now.saturating_sub(epoch_secs);
    if delta < 60 {
        format!("{delta}s ago")
    } else if delta < 3600 {
        format!("{}m ago", delta / 60)
    } else if delta < 86_400 {
        format!("{}h ago", delta / 3600)
    } else {
        format!("{}d ago", delta / 86_400)
    }
}

fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_size_handles_suffixes() {
        assert_eq!(parse_size("0").unwrap(), 0);
        assert_eq!(parse_size("512").unwrap(), 512);
        assert_eq!(parse_size("4K").unwrap(), 4 * 1024);
        assert_eq!(parse_size("4k").unwrap(), 4 * 1024);
        assert_eq!(parse_size("256M").unwrap(), 256 * 1024 * 1024);
        assert_eq!(parse_size("2G").unwrap(), 2 * 1024 * 1024 * 1024);
    }

    #[test]
    fn parse_size_rejects_garbage() {
        assert!(parse_size("").is_err());
        assert!(parse_size("abc").is_err());
        assert!(parse_size("12X").is_err());
        assert!(parse_size("1.5M").is_err());
    }

    #[test]
    fn parse_size_rejects_overflow() {
        assert!(parse_size("99999999999999999999G").is_err());
    }

    #[test]
    fn format_bytes_picks_unit() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.00 KiB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MiB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.00 GiB");
    }

    #[test]
    fn truncate_keeps_short_strings() {
        assert_eq!(truncate("hi", 10), "hi");
        assert_eq!(truncate("0123456789", 10), "0123456789");
        assert_eq!(truncate("01234567890", 10), "012345678…");
    }

    #[test]
    fn json_str_escapes_quotes_backslash_control() {
        assert_eq!(json_str(r#"plain"#), r#""plain""#);
        assert_eq!(json_str(r#"a "b" c"#), r#""a \"b\" c""#);
        assert_eq!(json_str("a\\b"), r#""a\\b""#);
        assert_eq!(json_str("a\nb"), r#""a\nb""#);
        assert_eq!(json_str("a\x01b"), r#""a\u0001b""#);
    }

    #[test]
    fn sort_entries_validates_key() {
        let mut entries: Vec<(CacheKey, CacheMeta)> = Vec::new();
        assert!(sort_entries(&mut entries, "accessed").is_ok());
        assert!(sort_entries(&mut entries, "created").is_ok());
        assert!(sort_entries(&mut entries, "size").is_ok());
        assert!(sort_entries(&mut entries, "key").is_ok());
        assert!(sort_entries(&mut entries, "bogus").is_err());
    }

    #[test]
    fn relative_time_formats_recent() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // 30s ago
        let s = relative_time(now - 30);
        assert!(s.ends_with("s ago"), "{s}");
        // 5m ago
        let s = relative_time(now - 5 * 60);
        assert!(s.ends_with("m ago"), "{s}");
        // 3h ago
        let s = relative_time(now - 3 * 3600);
        assert!(s.ends_with("h ago"), "{s}");
        // 2d ago
        let s = relative_time(now - 2 * 86_400);
        assert!(s.ends_with("d ago"), "{s}");
    }
}
