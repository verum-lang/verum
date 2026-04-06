//! Content-Addressed Storage (CAS) Layer
//!
//! Provides persistent, content-addressed storage for the semantic artifact cache.
//! Artifacts are stored by their Blake3 hash, enabling deduplication across projects.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                     Content-Addressed Storage (CAS)                         │
//! │  ┌─────────────────┐   ┌──────────────────┐   ┌───────────────────────────┐ │
//! │  │  ContentStore   │   │  ArtifactStore   │   │  StorageBackend (trait)   │ │
//! │  │  - hash_to_path │   │  - type_store    │   │  - LocalFS                │ │
//! │  │  - read/write   │   │  - fn_store      │   │  - (future: S3, Redis)    │ │
//! │  │  - gc_eviction  │   │  - verify_store  │   │                           │ │
//! │  └─────────────────┘   └──────────────────┘   └───────────────────────────┘ │
//! └─────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Directory Structure
//!
//! ```text
//! .verum/cas/
//! ├── objects/              # Content-addressed objects (2-level fanout)
//! │   ├── 00/
//! │   │   └── ab3f8...      # Object file (hash = 00ab3f8...)
//! │   ├── 01/
//! │   └── ...
//! ├── types/                # Type definition index
//! │   └── index.bin
//! ├── functions/            # Function signature/body index
//! │   └── index.bin
//! ├── verification/         # Verification result index
//! │   └── index.bin
//! └── metadata.json         # CAS metadata (version, stats)
//! ```
//!
//! ## Features
//!
//! - **Two-level fanout**: Objects stored in `objects/XX/YYY...` for filesystem efficiency
//! - **Zstd compression**: Artifacts compressed with zstd level 3 by default
//! - **Atomic writes**: Write to temp file, then atomic rename
//! - **LRU eviction**: Optional garbage collection based on access time
//! - **Cross-project sharing**: Global cache directory option
//!
//! Semantic artifact cache: content-addressed storage using semantic hashes
//! for deduplication and cross-project artifact reuse.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::hash::{hash_bytes, HashValue};
use crate::semantic_query::{
    CachedFunctionInfo, CachedTypeInfo, SemanticKey, SemanticKind, VerificationResult,
};
use verum_common::{List, Maybe, Text};

// ============================================================================
// Constants
// ============================================================================

/// CAS format version for compatibility
const CAS_VERSION: u32 = 1;

/// Magic bytes for object files
const OBJECT_MAGIC: &[u8; 4] = b"VCAS";

/// Magic bytes for index files
const INDEX_MAGIC: &[u8; 4] = b"VCIX";

/// Default zstd compression level
const DEFAULT_COMPRESSION_LEVEL: i32 = 3;

/// Maximum object size (100MB)
const MAX_OBJECT_SIZE: usize = 100 * 1024 * 1024;

// ============================================================================
// Storage Backend Trait
// ============================================================================

/// Storage backend abstraction for content-addressed storage.
///
/// This trait enables pluggable backends (local filesystem, S3, Redis, etc.)
pub trait StorageBackend: Send + Sync {
    /// Check if an object exists by hash.
    fn exists(&self, hash: &HashValue) -> io::Result<bool>;

    /// Read an object by hash.
    fn read(&self, hash: &HashValue) -> io::Result<Vec<u8>>;

    /// Write an object with the given hash.
    fn write(&self, hash: &HashValue, data: &[u8]) -> io::Result<()>;

    /// Delete an object by hash.
    fn delete(&self, hash: &HashValue) -> io::Result<()>;

    /// List all object hashes (for GC).
    fn list_objects(&self) -> io::Result<Vec<HashValue>>;

    /// Get object metadata (size, access time) if available.
    fn metadata(&self, hash: &HashValue) -> io::Result<ObjectMetadata>;
}

/// Metadata for a stored object.
#[derive(Debug, Clone)]
pub struct ObjectMetadata {
    /// Size in bytes (compressed)
    pub size: u64,
    /// Creation/modification time
    pub modified: SystemTime,
    /// Last access time (if tracked)
    pub accessed: Option<SystemTime>,
}

// ============================================================================
// Local Filesystem Backend
// ============================================================================

/// Local filesystem storage backend with two-level fanout.
pub struct LocalFsBackend {
    /// Base directory for objects
    objects_dir: PathBuf,
    /// Compression level (0 = disabled)
    compression_level: i32,
    /// Whether to track access times
    track_access: bool,
}

impl LocalFsBackend {
    /// Create a new local filesystem backend.
    pub fn new(base_dir: &Path) -> Self {
        Self {
            objects_dir: base_dir.join("objects"),
            compression_level: DEFAULT_COMPRESSION_LEVEL,
            track_access: true,
        }
    }

    /// Set compression level (0 to disable, 1-19 for zstd).
    pub fn with_compression(mut self, level: i32) -> Self {
        self.compression_level = level;
        self
    }

    /// Disable access time tracking.
    pub fn without_access_tracking(mut self) -> Self {
        self.track_access = false;
        self
    }

    /// Convert a hash to a file path using two-level fanout.
    fn hash_to_path(&self, hash: &HashValue) -> PathBuf {
        let hex = hash.to_hex();
        let fanout = &hex[0..2]; // First 2 hex chars (1 byte)
        let filename = &hex[2..]; // Remaining chars
        self.objects_dir.join(fanout).join(filename)
    }

    /// Ensure parent directory exists.
    fn ensure_parent(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(())
    }

    /// Write data with atomic rename.
    fn atomic_write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        self.ensure_parent(path)?;

        // Write to temp file
        let temp_path = path.with_extension("tmp");
        {
            let mut file = File::create(&temp_path)?;
            file.write_all(data)?;
            file.sync_all()?;
        }

        // Atomic rename
        fs::rename(&temp_path, path)?;
        Ok(())
    }

    /// Compress data with zstd.
    fn compress(&self, data: &[u8]) -> io::Result<Vec<u8>> {
        if self.compression_level == 0 {
            // No compression - just prepend magic and uncompressed marker
            let mut result = Vec::with_capacity(5 + data.len());
            result.extend_from_slice(OBJECT_MAGIC);
            result.push(0); // Compression flag: 0 = none
            result.extend_from_slice(data);
            return Ok(result);
        }

        // Compress with zstd
        let compressed = zstd::encode_all(data, self.compression_level)?;

        // Prepend header
        let mut result = Vec::with_capacity(5 + compressed.len());
        result.extend_from_slice(OBJECT_MAGIC);
        result.push(1); // Compression flag: 1 = zstd
        result.extend_from_slice(&compressed);
        Ok(result)
    }

    /// Decompress data.
    fn decompress(&self, data: &[u8]) -> io::Result<Vec<u8>> {
        if data.len() < 5 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Object too small",
            ));
        }

        // Verify magic
        if &data[0..4] != OBJECT_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid object magic bytes",
            ));
        }

        let compression_flag = data[4];
        let payload = &data[5..];

        match compression_flag {
            0 => Ok(payload.to_vec()), // Uncompressed
            1 => {
                // Zstd compressed
                zstd::decode_all(payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unknown compression type: {}", compression_flag),
            )),
        }
    }
}

impl StorageBackend for LocalFsBackend {
    fn exists(&self, hash: &HashValue) -> io::Result<bool> {
        Ok(self.hash_to_path(hash).exists())
    }

    fn read(&self, hash: &HashValue) -> io::Result<Vec<u8>> {
        let path = self.hash_to_path(hash);
        let compressed = fs::read(&path)?;

        // Update access time if tracking
        if self.track_access {
            // Touch the file (platform-specific, best effort)
            let _ = filetime::set_file_atime(&path, filetime::FileTime::now());
        }

        self.decompress(&compressed)
    }

    fn write(&self, hash: &HashValue, data: &[u8]) -> io::Result<()> {
        if data.len() > MAX_OBJECT_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Object too large: {} > {}", data.len(), MAX_OBJECT_SIZE),
            ));
        }

        let path = self.hash_to_path(hash);

        // Skip if already exists (content-addressable, so must be identical)
        if path.exists() {
            return Ok(());
        }

        let compressed = self.compress(data)?;
        self.atomic_write(&path, &compressed)
    }

    fn delete(&self, hash: &HashValue) -> io::Result<()> {
        let path = self.hash_to_path(hash);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        Ok(())
    }

    fn list_objects(&self) -> io::Result<Vec<HashValue>> {
        let mut objects = Vec::new();

        if !self.objects_dir.exists() {
            return Ok(objects);
        }

        for fanout_entry in fs::read_dir(&self.objects_dir)? {
            let fanout_entry = fanout_entry?;
            if !fanout_entry.file_type()?.is_dir() {
                continue;
            }

            let fanout = fanout_entry
                .file_name()
                .to_string_lossy()
                .to_string();

            for obj_entry in fs::read_dir(fanout_entry.path())? {
                let obj_entry = obj_entry?;
                if !obj_entry.file_type()?.is_file() {
                    continue;
                }

                let rest = obj_entry.file_name().to_string_lossy().to_string();
                let full_hex = format!("{}{}", fanout, rest);

                if let Ok(hash) = HashValue::from_hex(&full_hex) {
                    objects.push(hash);
                }
            }
        }

        Ok(objects)
    }

    fn metadata(&self, hash: &HashValue) -> io::Result<ObjectMetadata> {
        let path = self.hash_to_path(hash);
        let meta = fs::metadata(&path)?;

        Ok(ObjectMetadata {
            size: meta.len(),
            modified: meta.modified().unwrap_or(UNIX_EPOCH),
            accessed: meta.accessed().ok(),
        })
    }
}

// ============================================================================
// Artifact Store
// ============================================================================

/// Content-addressed artifact store for semantic cache persistence.
pub struct ArtifactStore {
    /// Storage backend
    backend: Box<dyn StorageBackend>,
    /// Base directory for indices
    base_dir: PathBuf,
    /// Type index: name → hash
    type_index: RwLock<HashMap<String, HashValue>>,
    /// Function index: name → hash
    function_index: RwLock<HashMap<String, HashValue>>,
    /// Verification index: semantic key → hash
    verification_index: RwLock<HashMap<SemanticKey, HashValue>>,
    /// Statistics
    stats: RwLock<ArtifactStoreStats>,
}

/// Statistics for the artifact store.
#[derive(Debug, Clone, Default)]
pub struct ArtifactStoreStats {
    /// Total objects stored
    pub objects_stored: u64,
    /// Total bytes stored (compressed)
    pub bytes_stored: u64,
    /// Cache hits
    pub hits: u64,
    /// Cache misses
    pub misses: u64,
    /// Objects deleted by GC
    pub gc_deleted: u64,
}

impl ArtifactStore {
    /// Create a new artifact store with local filesystem backend.
    pub fn new(base_dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(base_dir)?;

        let backend = Box::new(LocalFsBackend::new(base_dir));

        let store = Self {
            backend,
            base_dir: base_dir.to_path_buf(),
            type_index: RwLock::new(HashMap::new()),
            function_index: RwLock::new(HashMap::new()),
            verification_index: RwLock::new(HashMap::new()),
            stats: RwLock::new(ArtifactStoreStats::default()),
        };

        // Load indices
        store.load_indices()?;

        Ok(store)
    }

    /// Create with a custom backend.
    pub fn with_backend(base_dir: &Path, backend: Box<dyn StorageBackend>) -> io::Result<Self> {
        fs::create_dir_all(base_dir)?;

        let store = Self {
            backend,
            base_dir: base_dir.to_path_buf(),
            type_index: RwLock::new(HashMap::new()),
            function_index: RwLock::new(HashMap::new()),
            verification_index: RwLock::new(HashMap::new()),
            stats: RwLock::new(ArtifactStoreStats::default()),
        };

        store.load_indices()?;
        Ok(store)
    }

    // ========================================================================
    // Type Operations
    // ========================================================================

    /// Store a type definition.
    pub fn store_type(&self, name: &str, info: &CachedTypeInfo) -> io::Result<HashValue> {
        let serialized = self.serialize_type(info)?;
        let hash = hash_bytes(&serialized);

        self.backend.write(&hash, &serialized)?;

        if let Ok(mut index) = self.type_index.write() {
            index.insert(name.to_string(), hash);
        }

        if let Ok(mut stats) = self.stats.write() {
            stats.objects_stored += 1;
            stats.bytes_stored += serialized.len() as u64;
        }

        Ok(hash)
    }

    /// Load a type definition by name.
    pub fn load_type(&self, name: &str) -> io::Result<Option<CachedTypeInfo>> {
        let hash = {
            if let Ok(index) = self.type_index.read() {
                index.get(name).copied()
            } else {
                return Ok(None);
            }
        };

        let Some(hash) = hash else {
            if let Ok(mut stats) = self.stats.write() {
                stats.misses += 1;
            }
            return Ok(None);
        };

        let data = self.backend.read(&hash)?;
        let info = self.deserialize_type(&data)?;

        if let Ok(mut stats) = self.stats.write() {
            stats.hits += 1;
        }

        Ok(Some(info))
    }

    /// Load a type definition by hash.
    pub fn load_type_by_hash(&self, hash: &HashValue) -> io::Result<Option<CachedTypeInfo>> {
        if !self.backend.exists(hash)? {
            return Ok(None);
        }

        let data = self.backend.read(hash)?;
        let info = self.deserialize_type(&data)?;
        Ok(Some(info))
    }

    // ========================================================================
    // Function Operations
    // ========================================================================

    /// Store a function definition.
    pub fn store_function(&self, name: &str, info: &CachedFunctionInfo) -> io::Result<HashValue> {
        let serialized = self.serialize_function(info)?;
        let hash = hash_bytes(&serialized);

        self.backend.write(&hash, &serialized)?;

        if let Ok(mut index) = self.function_index.write() {
            index.insert(name.to_string(), hash);
        }

        if let Ok(mut stats) = self.stats.write() {
            stats.objects_stored += 1;
            stats.bytes_stored += serialized.len() as u64;
        }

        Ok(hash)
    }

    /// Load a function definition by name.
    pub fn load_function(&self, name: &str) -> io::Result<Option<CachedFunctionInfo>> {
        let hash = {
            if let Ok(index) = self.function_index.read() {
                index.get(name).copied()
            } else {
                return Ok(None);
            }
        };

        let Some(hash) = hash else {
            if let Ok(mut stats) = self.stats.write() {
                stats.misses += 1;
            }
            return Ok(None);
        };

        let data = self.backend.read(&hash)?;
        let info = self.deserialize_function(&data)?;

        if let Ok(mut stats) = self.stats.write() {
            stats.hits += 1;
        }

        Ok(Some(info))
    }

    /// Load a function definition by hash.
    pub fn load_function_by_hash(&self, hash: &HashValue) -> io::Result<Option<CachedFunctionInfo>> {
        if !self.backend.exists(hash)? {
            return Ok(None);
        }

        let data = self.backend.read(hash)?;
        let info = self.deserialize_function(&data)?;
        Ok(Some(info))
    }

    // ========================================================================
    // Verification Operations
    // ========================================================================

    /// Store a verification result.
    pub fn store_verification(
        &self,
        key: SemanticKey,
        result: &VerificationResult,
    ) -> io::Result<HashValue> {
        let serialized = self.serialize_verification(result)?;
        let hash = hash_bytes(&serialized);

        self.backend.write(&hash, &serialized)?;

        if let Ok(mut index) = self.verification_index.write() {
            index.insert(key, hash);
        }

        if let Ok(mut stats) = self.stats.write() {
            stats.objects_stored += 1;
            stats.bytes_stored += serialized.len() as u64;
        }

        Ok(hash)
    }

    /// Load a verification result by semantic key.
    pub fn load_verification(&self, key: &SemanticKey) -> io::Result<Option<VerificationResult>> {
        let hash = {
            if let Ok(index) = self.verification_index.read() {
                index.get(key).copied()
            } else {
                return Ok(None);
            }
        };

        let Some(hash) = hash else {
            if let Ok(mut stats) = self.stats.write() {
                stats.misses += 1;
            }
            return Ok(None);
        };

        let data = self.backend.read(&hash)?;
        let info = self.deserialize_verification(&data)?;

        if let Ok(mut stats) = self.stats.write() {
            stats.hits += 1;
        }

        Ok(Some(info))
    }

    // ========================================================================
    // Raw Object Operations
    // ========================================================================

    /// Store raw bytes by hash.
    pub fn store_raw(&self, data: &[u8]) -> io::Result<HashValue> {
        let hash = hash_bytes(data);
        self.backend.write(&hash, data)?;

        if let Ok(mut stats) = self.stats.write() {
            stats.objects_stored += 1;
            stats.bytes_stored += data.len() as u64;
        }

        Ok(hash)
    }

    /// Load raw bytes by hash.
    pub fn load_raw(&self, hash: &HashValue) -> io::Result<Option<Vec<u8>>> {
        if !self.backend.exists(hash)? {
            return Ok(None);
        }
        self.backend.read(hash).map(Some)
    }

    /// Check if an object exists.
    pub fn exists(&self, hash: &HashValue) -> io::Result<bool> {
        self.backend.exists(hash)
    }

    // ========================================================================
    // Index Persistence
    // ========================================================================

    /// Save all indices to disk.
    pub fn save_indices(&self) -> io::Result<()> {
        self.save_type_index()?;
        self.save_function_index()?;
        self.save_verification_index()?;
        Ok(())
    }

    /// Load all indices from disk.
    pub fn load_indices(&self) -> io::Result<()> {
        self.load_type_index()?;
        self.load_function_index()?;
        self.load_verification_index()?;
        Ok(())
    }

    fn save_type_index(&self) -> io::Result<()> {
        let index_dir = self.base_dir.join("types");
        fs::create_dir_all(&index_dir)?;

        let path = index_dir.join("index.bin");

        let Ok(index) = self.type_index.read() else {
            return Ok(());
        };

        let file = File::create(&path)?;
        let mut writer = BufWriter::new(file);

        // Header
        writer.write_all(INDEX_MAGIC)?;
        writer.write_all(&CAS_VERSION.to_le_bytes())?;
        writer.write_all(&(index.len() as u64).to_le_bytes())?;

        // Entries
        for (name, hash) in index.iter() {
            let name_bytes = name.as_bytes();
            writer.write_all(&(name_bytes.len() as u32).to_le_bytes())?;
            writer.write_all(name_bytes)?;
            writer.write_all(hash.as_bytes())?;
        }

        writer.flush()?;
        Ok(())
    }

    fn load_type_index(&self) -> io::Result<()> {
        let path = self.base_dir.join("types").join("index.bin");
        if !path.exists() {
            return Ok(());
        }

        let file = File::open(&path)?;
        let mut reader = BufReader::new(file);

        // Read header
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if &magic != INDEX_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid type index magic",
            ));
        }

        let mut version = [0u8; 4];
        reader.read_exact(&mut version)?;
        let version = u32::from_le_bytes(version);
        if version != CAS_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Version mismatch: {} vs {}", version, CAS_VERSION),
            ));
        }

        let mut count = [0u8; 8];
        reader.read_exact(&mut count)?;
        let count = u64::from_le_bytes(count) as usize;

        // Read entries
        let mut index = HashMap::with_capacity(count);
        for _ in 0..count {
            let mut len = [0u8; 4];
            reader.read_exact(&mut len)?;
            let len = u32::from_le_bytes(len) as usize;

            let mut name_bytes = vec![0u8; len];
            reader.read_exact(&mut name_bytes)?;
            let name = String::from_utf8_lossy(&name_bytes).to_string();

            let mut hash_bytes = [0u8; 32];
            reader.read_exact(&mut hash_bytes)?;
            let hash = HashValue::from_bytes(hash_bytes);

            index.insert(name, hash);
        }

        if let Ok(mut idx) = self.type_index.write() {
            *idx = index;
        }

        Ok(())
    }

    fn save_function_index(&self) -> io::Result<()> {
        let index_dir = self.base_dir.join("functions");
        fs::create_dir_all(&index_dir)?;

        let path = index_dir.join("index.bin");

        let Ok(index) = self.function_index.read() else {
            return Ok(());
        };

        let file = File::create(&path)?;
        let mut writer = BufWriter::new(file);

        writer.write_all(INDEX_MAGIC)?;
        writer.write_all(&CAS_VERSION.to_le_bytes())?;
        writer.write_all(&(index.len() as u64).to_le_bytes())?;

        for (name, hash) in index.iter() {
            let name_bytes = name.as_bytes();
            writer.write_all(&(name_bytes.len() as u32).to_le_bytes())?;
            writer.write_all(name_bytes)?;
            writer.write_all(hash.as_bytes())?;
        }

        writer.flush()?;
        Ok(())
    }

    fn load_function_index(&self) -> io::Result<()> {
        let path = self.base_dir.join("functions").join("index.bin");
        if !path.exists() {
            return Ok(());
        }

        let file = File::open(&path)?;
        let mut reader = BufReader::new(file);

        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if &magic != INDEX_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid function index magic",
            ));
        }

        let mut version = [0u8; 4];
        reader.read_exact(&mut version)?;
        let version = u32::from_le_bytes(version);
        if version != CAS_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Function index version mismatch",
            ));
        }

        let mut count = [0u8; 8];
        reader.read_exact(&mut count)?;
        let count = u64::from_le_bytes(count) as usize;

        let mut index = HashMap::with_capacity(count);
        for _ in 0..count {
            let mut len = [0u8; 4];
            reader.read_exact(&mut len)?;
            let len = u32::from_le_bytes(len) as usize;

            let mut name_bytes = vec![0u8; len];
            reader.read_exact(&mut name_bytes)?;
            let name = String::from_utf8_lossy(&name_bytes).to_string();

            let mut hash_bytes = [0u8; 32];
            reader.read_exact(&mut hash_bytes)?;
            let hash = HashValue::from_bytes(hash_bytes);

            index.insert(name, hash);
        }

        if let Ok(mut idx) = self.function_index.write() {
            *idx = index;
        }

        Ok(())
    }

    fn save_verification_index(&self) -> io::Result<()> {
        let index_dir = self.base_dir.join("verification");
        fs::create_dir_all(&index_dir)?;

        let path = index_dir.join("index.bin");

        let Ok(index) = self.verification_index.read() else {
            return Ok(());
        };

        let file = File::create(&path)?;
        let mut writer = BufWriter::new(file);

        writer.write_all(INDEX_MAGIC)?;
        writer.write_all(&CAS_VERSION.to_le_bytes())?;
        writer.write_all(&(index.len() as u64).to_le_bytes())?;

        for (key, hash) in index.iter() {
            // Write semantic key: kind (1 byte) + hash (32 bytes)
            writer.write_all(&[key.kind() as u8])?;
            writer.write_all(key.hash().as_bytes())?;
            // Write object hash
            writer.write_all(hash.as_bytes())?;
        }

        writer.flush()?;
        Ok(())
    }

    fn load_verification_index(&self) -> io::Result<()> {
        let path = self.base_dir.join("verification").join("index.bin");
        if !path.exists() {
            return Ok(());
        }

        let file = File::open(&path)?;
        let mut reader = BufReader::new(file);

        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if &magic != INDEX_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid verification index magic",
            ));
        }

        let mut version = [0u8; 4];
        reader.read_exact(&mut version)?;
        let version = u32::from_le_bytes(version);
        if version != CAS_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Verification index version mismatch",
            ));
        }

        let mut count = [0u8; 8];
        reader.read_exact(&mut count)?;
        let count = u64::from_le_bytes(count) as usize;

        let mut index = HashMap::with_capacity(count);
        for _ in 0..count {
            // Read semantic key
            let mut kind = [0u8; 1];
            reader.read_exact(&mut kind)?;
            let kind = match kind[0] {
                0 => SemanticKind::Type,
                1 => SemanticKind::FunctionSignature,
                2 => SemanticKind::FunctionBody,
                3 => SemanticKind::Protocol,
                4 => SemanticKind::Verification,
                5 => SemanticKind::TypeCheck,
                6 => SemanticKind::Constant,
                7 => SemanticKind::Module,
                _ => continue, // Skip unknown kinds
            };

            let mut key_hash = [0u8; 32];
            reader.read_exact(&mut key_hash)?;
            let key = SemanticKey::new(HashValue::from_bytes(key_hash), kind);

            let mut obj_hash = [0u8; 32];
            reader.read_exact(&mut obj_hash)?;
            let hash = HashValue::from_bytes(obj_hash);

            index.insert(key, hash);
        }

        if let Ok(mut idx) = self.verification_index.write() {
            *idx = index;
        }

        Ok(())
    }

    // ========================================================================
    // Garbage Collection
    // ========================================================================

    /// Run garbage collection, removing objects older than max_age.
    pub fn gc(&self, max_age: Duration) -> io::Result<GcResult> {
        let now = SystemTime::now();
        let mut deleted = 0;
        let mut freed_bytes = 0u64;

        let objects = self.backend.list_objects()?;

        for hash in objects {
            if let Ok(meta) = self.backend.metadata(&hash) {
                let age = now.duration_since(meta.accessed.unwrap_or(meta.modified));
                if let Ok(age) = age {
                    if age > max_age {
                        freed_bytes += meta.size;
                        self.backend.delete(&hash)?;
                        deleted += 1;
                    }
                }
            }
        }

        // Update stats
        if let Ok(mut stats) = self.stats.write() {
            stats.gc_deleted += deleted;
        }

        // Clean up indices (remove entries that point to deleted objects)
        self.cleanup_indices()?;

        Ok(GcResult {
            deleted,
            freed_bytes,
        })
    }

    /// Remove index entries that point to non-existent objects.
    fn cleanup_indices(&self) -> io::Result<()> {
        // Cleanup type index
        if let Ok(mut index) = self.type_index.write() {
            let keys_to_remove: Vec<_> = index
                .iter()
                .filter(|(_, hash)| !self.backend.exists(hash).unwrap_or(false))
                .map(|(k, _)| k.clone())
                .collect();
            for key in keys_to_remove {
                index.remove(&key);
            }
        }

        // Cleanup function index
        if let Ok(mut index) = self.function_index.write() {
            let keys_to_remove: Vec<_> = index
                .iter()
                .filter(|(_, hash)| !self.backend.exists(hash).unwrap_or(false))
                .map(|(k, _)| k.clone())
                .collect();
            for key in keys_to_remove {
                index.remove(&key);
            }
        }

        // Cleanup verification index
        if let Ok(mut index) = self.verification_index.write() {
            let keys_to_remove: Vec<_> = index
                .iter()
                .filter(|(_, hash)| !self.backend.exists(hash).unwrap_or(false))
                .map(|(k, _)| *k)
                .collect();
            for key in keys_to_remove {
                index.remove(&key);
            }
        }

        Ok(())
    }

    // ========================================================================
    // Serialization (Simple Binary Format)
    // ========================================================================

    fn serialize_type(&self, info: &CachedTypeInfo) -> io::Result<Vec<u8>> {
        let mut buf = Vec::new();

        // Type marker
        buf.push(0x01);

        // Name
        let name_bytes = info.name.as_bytes();
        buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(name_bytes);

        // Kind
        let kind_bytes = info.kind.as_bytes();
        buf.extend_from_slice(&(kind_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(kind_bytes);

        // Generics count
        buf.extend_from_slice(&(info.generics.len() as u32).to_le_bytes());
        for generic in info.generics.iter() {
            let gen_bytes = generic.as_bytes();
            buf.extend_from_slice(&(gen_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(gen_bytes);
        }

        // Body hash
        buf.extend_from_slice(info.body_hash.as_bytes());

        // Serialized
        let ser_bytes = info.serialized.as_bytes();
        buf.extend_from_slice(&(ser_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(ser_bytes);

        Ok(buf)
    }

    fn deserialize_type(&self, data: &[u8]) -> io::Result<CachedTypeInfo> {
        let mut cursor = 0;

        // Check marker
        if data.get(cursor) != Some(&0x01) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid type marker",
            ));
        }
        cursor += 1;

        // Name
        let name_len = read_u32(&data[cursor..])?;
        cursor += 4;
        let name = String::from_utf8_lossy(&data[cursor..cursor + name_len as usize]).into_owned();
        cursor += name_len as usize;

        // Kind
        let kind_len = read_u32(&data[cursor..])?;
        cursor += 4;
        let kind = String::from_utf8_lossy(&data[cursor..cursor + kind_len as usize]).into_owned();
        cursor += kind_len as usize;

        // Generics
        let generics_count = read_u32(&data[cursor..])?;
        cursor += 4;
        let mut generics = List::new();
        for _ in 0..generics_count {
            let gen_len = read_u32(&data[cursor..])?;
            cursor += 4;
            let generic =
                String::from_utf8_lossy(&data[cursor..cursor + gen_len as usize]).into_owned();
            cursor += gen_len as usize;
            generics.push(Text::from(generic));
        }

        // Body hash
        if cursor + 32 > data.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Truncated body hash",
            ));
        }
        let body_hash = HashValue::from_slice(&data[cursor..cursor + 32]);
        cursor += 32;

        // Serialized
        let ser_len = read_u32(&data[cursor..])?;
        cursor += 4;
        let serialized =
            String::from_utf8_lossy(&data[cursor..cursor + ser_len as usize]).into_owned();

        Ok(CachedTypeInfo {
            name: Text::from(name),
            kind: Text::from(kind),
            generics,
            body_hash,
            serialized: Text::from(serialized),
        })
    }

    fn serialize_function(&self, info: &CachedFunctionInfo) -> io::Result<Vec<u8>> {
        let mut buf = Vec::new();

        // Function marker
        buf.push(0x02);

        // Name
        let name_bytes = info.name.as_bytes();
        buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(name_bytes);

        // Signature hash
        buf.extend_from_slice(info.signature_hash.as_bytes());

        // Body hash (optional)
        match &info.body_hash {
            Maybe::Some(hash) => {
                buf.push(1);
                buf.extend_from_slice(hash.as_bytes());
            }
            Maybe::None => {
                buf.push(0);
            }
        }

        // Param types
        buf.extend_from_slice(&(info.param_types.len() as u32).to_le_bytes());
        for ty in info.param_types.iter() {
            let ty_bytes = ty.as_bytes();
            buf.extend_from_slice(&(ty_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(ty_bytes);
        }

        // Return type (optional)
        match &info.return_type {
            Maybe::Some(ty) => {
                buf.push(1);
                let ty_bytes = ty.as_bytes();
                buf.extend_from_slice(&(ty_bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(ty_bytes);
            }
            Maybe::None => {
                buf.push(0);
            }
        }

        // Contexts
        buf.extend_from_slice(&(info.contexts.len() as u32).to_le_bytes());
        for ctx in info.contexts.iter() {
            let ctx_bytes = ctx.as_bytes();
            buf.extend_from_slice(&(ctx_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(ctx_bytes);
        }

        // Properties
        buf.extend_from_slice(&(info.properties.len() as u32).to_le_bytes());
        for prop in info.properties.iter() {
            let prop_bytes = prop.as_bytes();
            buf.extend_from_slice(&(prop_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(prop_bytes);
        }

        // Is meta
        buf.push(if info.is_meta { 1 } else { 0 });

        Ok(buf)
    }

    fn deserialize_function(&self, data: &[u8]) -> io::Result<CachedFunctionInfo> {
        let mut cursor = 0;

        // Check marker
        if data.get(cursor) != Some(&0x02) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid function marker",
            ));
        }
        cursor += 1;

        // Name
        let name_len = read_u32(&data[cursor..])?;
        cursor += 4;
        let name = String::from_utf8_lossy(&data[cursor..cursor + name_len as usize]).into_owned();
        cursor += name_len as usize;

        // Signature hash
        let signature_hash = HashValue::from_slice(&data[cursor..cursor + 32]);
        cursor += 32;

        // Body hash
        let body_hash = if data[cursor] == 1 {
            cursor += 1;
            let hash = HashValue::from_slice(&data[cursor..cursor + 32]);
            cursor += 32;
            Maybe::Some(hash)
        } else {
            cursor += 1;
            Maybe::None
        };

        // Param types
        let params_count = read_u32(&data[cursor..])?;
        cursor += 4;
        let mut param_types = List::new();
        for _ in 0..params_count {
            let ty_len = read_u32(&data[cursor..])?;
            cursor += 4;
            let ty = String::from_utf8_lossy(&data[cursor..cursor + ty_len as usize]).into_owned();
            cursor += ty_len as usize;
            param_types.push(Text::from(ty));
        }

        // Return type
        let return_type = if data[cursor] == 1 {
            cursor += 1;
            let ty_len = read_u32(&data[cursor..])?;
            cursor += 4;
            let ty = String::from_utf8_lossy(&data[cursor..cursor + ty_len as usize]).into_owned();
            cursor += ty_len as usize;
            Maybe::Some(Text::from(ty))
        } else {
            cursor += 1;
            Maybe::None
        };

        // Contexts
        let contexts_count = read_u32(&data[cursor..])?;
        cursor += 4;
        let mut contexts = List::new();
        for _ in 0..contexts_count {
            let ctx_len = read_u32(&data[cursor..])?;
            cursor += 4;
            let ctx =
                String::from_utf8_lossy(&data[cursor..cursor + ctx_len as usize]).into_owned();
            cursor += ctx_len as usize;
            contexts.push(Text::from(ctx));
        }

        // Properties
        let props_count = read_u32(&data[cursor..])?;
        cursor += 4;
        let mut properties = List::new();
        for _ in 0..props_count {
            let prop_len = read_u32(&data[cursor..])?;
            cursor += 4;
            let prop =
                String::from_utf8_lossy(&data[cursor..cursor + prop_len as usize]).into_owned();
            cursor += prop_len as usize;
            properties.push(Text::from(prop));
        }

        // Is meta
        let is_meta = data[cursor] == 1;

        Ok(CachedFunctionInfo {
            name: Text::from(name),
            signature_hash,
            body_hash,
            param_types,
            return_type,
            contexts,
            properties,
            is_meta,
        })
    }

    fn serialize_verification(&self, result: &VerificationResult) -> io::Result<Vec<u8>> {
        let mut buf = Vec::new();

        // Verification marker
        buf.push(0x03);

        // Success
        buf.push(if result.success { 1 } else { 0 });

        // Errors
        buf.extend_from_slice(&(result.errors.len() as u32).to_le_bytes());
        for err in result.errors.iter() {
            let err_bytes = err.as_bytes();
            buf.extend_from_slice(&(err_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(err_bytes);
        }

        // Warnings
        buf.extend_from_slice(&(result.warnings.len() as u32).to_le_bytes());
        for warn in result.warnings.iter() {
            let warn_bytes = warn.as_bytes();
            buf.extend_from_slice(&(warn_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(warn_bytes);
        }

        // Obligations
        buf.extend_from_slice(&result.obligations_satisfied.to_le_bytes());
        buf.extend_from_slice(&result.obligations_total.to_le_bytes());

        Ok(buf)
    }

    fn deserialize_verification(&self, data: &[u8]) -> io::Result<VerificationResult> {
        let mut cursor = 0;

        // Check marker
        if data.get(cursor) != Some(&0x03) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid verification marker",
            ));
        }
        cursor += 1;

        // Success
        let success = data[cursor] == 1;
        cursor += 1;

        // Errors
        let errors_count = read_u32(&data[cursor..])?;
        cursor += 4;
        let mut errors = List::new();
        for _ in 0..errors_count {
            let err_len = read_u32(&data[cursor..])?;
            cursor += 4;
            let err =
                String::from_utf8_lossy(&data[cursor..cursor + err_len as usize]).into_owned();
            cursor += err_len as usize;
            errors.push(Text::from(err));
        }

        // Warnings
        let warnings_count = read_u32(&data[cursor..])?;
        cursor += 4;
        let mut warnings = List::new();
        for _ in 0..warnings_count {
            let warn_len = read_u32(&data[cursor..])?;
            cursor += 4;
            let warn =
                String::from_utf8_lossy(&data[cursor..cursor + warn_len as usize]).into_owned();
            cursor += warn_len as usize;
            warnings.push(Text::from(warn));
        }

        // Obligations
        let obligations_satisfied = read_u32(&data[cursor..])?;
        cursor += 4;
        let obligations_total = read_u32(&data[cursor..])?;

        Ok(VerificationResult {
            success,
            errors,
            warnings,
            obligations_satisfied,
            obligations_total,
        })
    }

    // ========================================================================
    // Statistics
    // ========================================================================

    /// Get current statistics.
    pub fn stats(&self) -> ArtifactStoreStats {
        self.stats.read().map(|s| s.clone()).unwrap_or_default()
    }

    /// Get cache hit rate.
    pub fn hit_rate(&self) -> f64 {
        if let Ok(stats) = self.stats.read() {
            let total = stats.hits + stats.misses;
            if total == 0 {
                0.0
            } else {
                stats.hits as f64 / total as f64
            }
        } else {
            0.0
        }
    }
}

/// Result of garbage collection.
#[derive(Debug, Clone)]
pub struct GcResult {
    /// Number of objects deleted
    pub deleted: u64,
    /// Bytes freed
    pub freed_bytes: u64,
}

// ============================================================================
// Helpers
// ============================================================================

fn read_u32(data: &[u8]) -> io::Result<u32> {
    if data.len() < 4 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Not enough bytes for u32",
        ));
    }
    Ok(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_local_fs_backend_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFsBackend::new(temp_dir.path()).with_compression(0);

        let data = b"Hello, content-addressed world!";
        let hash = hash_bytes(data);

        // Write
        backend.write(&hash, data).unwrap();
        assert!(backend.exists(&hash).unwrap());

        // Read
        let read_data = backend.read(&hash).unwrap();
        assert_eq!(read_data, data);

        // Delete
        backend.delete(&hash).unwrap();
        assert!(!backend.exists(&hash).unwrap());
    }

    #[test]
    fn test_local_fs_backend_compression() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFsBackend::new(temp_dir.path()).with_compression(3);

        // Large data that compresses well
        let data: Vec<u8> = (0..10000).map(|i| (i % 256) as u8).collect();
        let hash = hash_bytes(&data);

        backend.write(&hash, &data).unwrap();
        let read_data = backend.read(&hash).unwrap();
        assert_eq!(read_data, data);
    }

    #[test]
    fn test_artifact_store_type() {
        let temp_dir = TempDir::new().unwrap();
        let store = ArtifactStore::new(temp_dir.path()).unwrap();

        let info = CachedTypeInfo {
            name: Text::from("MyStruct"),
            kind: Text::from("struct"),
            generics: List::from_iter([Text::from("T")]),
            body_hash: hash_bytes(b"body"),
            serialized: Text::from("type MyStruct<T> is { x: T };"),
        };

        // Store
        let hash = store.store_type("MyStruct", &info).unwrap();
        assert!(!hash.is_zero());

        // Load by name
        let loaded = store.load_type("MyStruct").unwrap().unwrap();
        assert_eq!(loaded.name.as_str(), "MyStruct");
        assert_eq!(loaded.generics.len(), 1);

        // Save and reload indices
        store.save_indices().unwrap();

        let store2 = ArtifactStore::new(temp_dir.path()).unwrap();
        let loaded2 = store2.load_type("MyStruct").unwrap().unwrap();
        assert_eq!(loaded2.name.as_str(), "MyStruct");
    }

    #[test]
    fn test_artifact_store_function() {
        let temp_dir = TempDir::new().unwrap();
        let store = ArtifactStore::new(temp_dir.path()).unwrap();

        let info = CachedFunctionInfo {
            name: Text::from("process"),
            signature_hash: hash_bytes(b"sig"),
            body_hash: Maybe::Some(hash_bytes(b"body")),
            param_types: List::from_iter([Text::from("Int"), Text::from("Text")]),
            return_type: Maybe::Some(Text::from("Result<Response, Error>")),
            contexts: List::from_iter([Text::from("Database")]),
            properties: List::from_iter([Text::from("Async")]),
            is_meta: false,
        };

        let hash = store.store_function("process", &info).unwrap();
        assert!(!hash.is_zero());

        let loaded = store.load_function("process").unwrap().unwrap();
        assert_eq!(loaded.name.as_str(), "process");
        assert_eq!(loaded.param_types.len(), 2);
        assert!(loaded.return_type.is_some());
    }

    #[test]
    fn test_artifact_store_verification() {
        let temp_dir = TempDir::new().unwrap();
        let store = ArtifactStore::new(temp_dir.path()).unwrap();

        let key = SemanticKey::for_verification(hash_bytes(b"item"));
        let result = VerificationResult {
            success: true,
            errors: List::new(),
            warnings: List::from_iter([Text::from("Unused variable x")]),
            obligations_satisfied: 5,
            obligations_total: 5,
        };

        let hash = store.store_verification(key, &result).unwrap();
        assert!(!hash.is_zero());

        let loaded = store.load_verification(&key).unwrap().unwrap();
        assert!(loaded.success);
        assert_eq!(loaded.warnings.len(), 1);
        assert_eq!(loaded.obligations_satisfied, 5);
    }

    #[test]
    fn test_artifact_store_gc() {
        let temp_dir = TempDir::new().unwrap();
        let store = ArtifactStore::new(temp_dir.path()).unwrap();

        // Store some data
        let info = CachedTypeInfo {
            name: Text::from("OldType"),
            kind: Text::from("struct"),
            generics: List::new(),
            body_hash: hash_bytes(b"old"),
            serialized: Text::from("type OldType is ();"),
        };
        store.store_type("OldType", &info).unwrap();

        // Run GC with 0 max_age (delete everything)
        let result = store.gc(Duration::from_secs(0)).unwrap();
        assert!(result.deleted >= 1);
    }

    #[test]
    fn test_hash_to_path_fanout() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFsBackend::new(temp_dir.path());

        let hash = HashValue::from_hex(
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
        )
        .unwrap();

        let path = backend.hash_to_path(&hash);

        // Should be: objects/ab/cdef...
        assert!(path.to_string_lossy().contains("objects"));
        assert!(path.to_string_lossy().contains("ab"));
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("cdef"));
    }
}
