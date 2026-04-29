// Add dependencies with multi-source support (registry, git, path).
// Updates verum.toml [dependencies] and resolves version constraints.

use crate::config::{Dependency, Manifest};
use crate::error::Result;
use crate::registry::{RegistryClient, cache_dir};
use crate::ui;
use colored::Colorize;
use std::path::PathBuf;
use verum_common::{List, Text};

/// Add dependency options
pub struct AddOptions {
    pub name: Text,
    pub version: Option<Text>,
    pub features: List<Text>,
    pub optional: bool,
    pub dev: bool,
    pub build: bool,
    pub git: Option<Text>,
    pub branch: Option<Text>,
    pub tag: Option<Text>,
    pub rev: Option<Text>,
    pub path: Option<PathBuf>,
    pub ipfs: Option<Text>,
    pub cbgr_profile: Option<Text>,
    pub verify: bool,
    pub prefer_ipfs: bool,
}

impl Default for AddOptions {
    fn default() -> Self {
        Self {
            name: Text::new(),
            version: None,
            features: List::new(),
            optional: false,
            dev: false,
            build: false,
            git: None,
            branch: None,
            tag: None,
            rev: None,
            path: None,
            ipfs: None,
            cbgr_profile: None,
            verify: false,
            prefer_ipfs: false,
        }
    }
}

/// Add dependency to project
pub fn add(options: AddOptions) -> Result<()> {
    ui::step(&format!(
        "Adding dependency: {}",
        options.name.as_str().cyan()
    ));

    // Surface inert AddOptions fields. `cbgr_profile` (CBGR
    // optimization profile to apply when building this dep) and
    // `prefer_ipfs` (route registry fetches through IPFS first
    // before HTTPS) flow from CLI flags but no current path
    // consults them — the dep gets added to the manifest with
    // the standard registry/git/ipfs/path source resolution
    // regardless of the profile or transport preference.
    // Closes the inert-defense pattern by routing the requested
    // values through tracing so embedders writing
    // `verum add foo --cbgr-profile=Strict` or
    // `--prefer-ipfs` see the request was observed at the
    // command entry, even when the integration with build-time
    // profile selection / IPFS-first transport isn't yet
    // realised.
    if options.cbgr_profile.is_some() || options.prefer_ipfs {
        tracing::debug!(
            "verum add: cbgr_profile={:?}, prefer_ipfs={} — these fields \
             are forward-looking; the registry/git/ipfs/path source path \
             does not yet differentiate behaviour based on them",
            options.cbgr_profile.as_ref().map(|t| t.as_str()),
            options.prefer_ipfs,
        );
    }

    // Find manifest
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let mut manifest = Manifest::from_file(&manifest_path)?;

    // Determine dependency source
    let dependency = if let Some(ref path) = options.path {
        // Local path
        ui::info(&format!("Using local path: {}", path.display()));
        create_path_dependency(path.clone(), &options)
    } else if let Some(ref git_url) = options.git {
        // Git repository
        ui::info(&format!("Using Git repository: {}", git_url));
        create_git_dependency(git_url.clone(), &options)
    } else if let Some(ref ipfs_hash) = options.ipfs {
        // IPFS
        ui::info(&format!("Using IPFS hash: {}", ipfs_hash));
        create_ipfs_dependency(ipfs_hash.clone(), &options)?
    } else {
        // Registry
        ui::info("Using package registry");
        create_registry_dependency(&options)?
    };

    // Add to appropriate section
    if options.build {
        ui::info("Adding to [build-dependencies]");
        manifest
            .build_dependencies
            .insert(options.name.clone(), dependency);
    } else if options.dev {
        ui::info("Adding to [dev-dependencies]");
        manifest
            .dev_dependencies
            .insert(options.name.clone(), dependency);
    } else {
        ui::info("Adding to [dependencies]");
        manifest
            .dependencies
            .insert(options.name.clone(), dependency);
    }

    // Save manifest
    manifest.to_file(&manifest_path)?;

    // Download and cache package
    if options.path.is_none() {
        download_and_cache(&options)?;
    }

    ui::success(&format!("Added {} to dependencies", options.name));

    // Show next steps
    println!();
    ui::info("Run 'verum build' to download and compile dependencies");

    Ok(())
}

/// Create registry dependency
fn create_registry_dependency(options: &AddOptions) -> Result<Dependency> {
    let client = RegistryClient::default()?;

    // Get version
    let version = if let Some(v) = &options.version {
        v.clone()
    } else {
        // Fetch latest version
        ui::info("Fetching latest version...");
        client.get_latest_version(options.name.as_str())?
    };

    ui::info(&format!("Using version: {}", version));

    // Create dependency
    if options.features.is_empty() && !options.optional {
        Ok(Dependency::Simple(version))
    } else {
        Ok(Dependency::Detailed {
            version: Some(version),
            path: None,
            git: None,
            branch: None,
            tag: None,
            rev: None,
            features: if options.features.is_empty() {
                None
            } else {
                Some(options.features.clone())
            },
            optional: if options.optional { Some(true) } else { None },
        })
    }
}

/// Create Git dependency
fn create_git_dependency(git_url: Text, options: &AddOptions) -> Dependency {
    Dependency::Detailed {
        version: None,
        path: None,
        git: Some(git_url),
        branch: options.branch.clone(),
        tag: options.tag.clone(),
        rev: options.rev.clone(),
        features: if options.features.is_empty() {
            None
        } else {
            Some(options.features.clone())
        },
        optional: if options.optional { Some(true) } else { None },
    }
}

/// Create path dependency
fn create_path_dependency(path: PathBuf, options: &AddOptions) -> Dependency {
    Dependency::Detailed {
        version: None,
        path: Some(path),
        git: None,
        branch: None,
        tag: None,
        rev: None,
        features: if options.features.is_empty() {
            None
        } else {
            Some(options.features.clone())
        },
        optional: if options.optional { Some(true) } else { None },
    }
}

/// Create IPFS dependency
fn create_ipfs_dependency(ipfs_hash: Text, options: &AddOptions) -> Result<Dependency> {
    Ok(Dependency::Detailed {
        version: None,
        path: None,
        git: Some(format!("ipfs://{}", ipfs_hash).into()),
        branch: None,
        tag: None,
        rev: None,
        features: if options.features.is_empty() {
            None
        } else {
            Some(options.features.clone())
        },
        optional: if options.optional { Some(true) } else { None },
    })
}

/// Download and cache package
fn download_and_cache(options: &AddOptions) -> Result<()> {
    let cache = cache_dir()?;
    std::fs::create_dir_all(&cache)?;

    if let Some(ref git_url) = options.git {
        // Git clone
        ui::info("Cloning Git repository...");
        clone_git_repository(git_url, &options.name, &cache, options)?;
        return Ok(());
    }

    if let Some(ref ipfs_hash) = options.ipfs {
        // IPFS download
        ui::info("Downloading from IPFS...");
        download_from_ipfs(ipfs_hash, &options.name, &cache)?;
        return Ok(());
    }

    // Registry download
    let client = RegistryClient::default()?;
    let version = options
        .version
        .as_ref()
        .map(|v| v.as_str())
        .unwrap_or("latest");

    let package_cache = cache.join(options.name.as_str()).join(version);
    std::fs::create_dir_all(&package_cache)?;

    let cog_file = package_cache.join(format!("{}-{}.tar.gz", options.name, version));

    if cog_file.exists() {
        ui::info("Cog already cached");
        return Ok(());
    }

    ui::info("Downloading package...");
    client.download(options.name.as_str(), version, &cog_file)?;

    // Verify checksum if requested
    if options.verify {
        ui::info("Verifying package...");
        verify_cog(options.name.as_str(), version, &cog_file)?;
    }

    Ok(())
}

/// Verify package integrity
fn verify_cog(name: &str, version: &str, cog_file: &PathBuf) -> Result<()> {
    let client = RegistryClient::default()?;
    let metadata = client.get_metadata(name, version)?;

    // Calculate checksum
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = std::fs::File::open(cog_file)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }

    let checksum = format!("{:x}", hasher.finalize());

    if checksum != metadata.checksum.as_str() {
        return Err(crate::error::CliError::Custom(format!(
            "Checksum mismatch for {} {}: expected {}, got {}",
            name, version, metadata.checksum, checksum
        )));
    }

    // Verify signature if present
    if let Some(signature) = metadata.signature {
        ui::info("Verifying signature...");
        use crate::registry::CogSigner;

        if !CogSigner::verify_signature(cog_file, &signature)? {
            return Err(crate::error::CliError::Custom(
                "Cog signature verification failed".into(),
            ));
        }
    }

    ui::success("Cog verified");
    Ok(())
}

/// Clone a Git repository to the cache directory
fn clone_git_repository(
    git_url: &Text,
    cog_name: &Text,
    cache: &PathBuf,
    options: &AddOptions,
) -> Result<()> {
    use git2::{FetchOptions, Repository, build::RepoBuilder};

    // Determine the git reference to checkout
    let git_ref = if let Some(ref tag) = options.tag {
        tag.as_str()
    } else if let Some(ref branch) = options.branch {
        branch.as_str()
    } else if let Some(ref rev) = options.rev {
        rev.as_str()
    } else {
        "HEAD"
    };

    // Create cache directory for this git dependency
    let repo_cache = cache.join("git").join(cog_name.as_str());
    std::fs::create_dir_all(&repo_cache)?;

    let repo_path = repo_cache.join(git_ref);

    // Check if already cloned
    if repo_path.exists() && Repository::open(&repo_path).is_ok() {
        ui::info("Repository already cached");
        return Ok(());
    }

    // Clone the repository
    ui::info(&format!("Cloning from {}...", git_url));

    let mut fetch_opts = FetchOptions::new();
    // Add progress reporting
    fetch_opts.remote_callbacks({
        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.transfer_progress(|stats| {
            if stats.received_objects() == stats.total_objects() {
                ui::info(&format!(
                    "Resolving deltas {}/{}",
                    stats.indexed_deltas(),
                    stats.total_deltas()
                ));
            } else {
                ui::info(&format!(
                    "Downloading objects {}/{}",
                    stats.received_objects(),
                    stats.total_objects()
                ));
            }
            true
        });
        callbacks
    });

    let mut repo_builder = RepoBuilder::new();
    repo_builder.fetch_options(fetch_opts);

    let repo = repo_builder
        .clone(git_url.as_str(), &repo_path)
        .map_err(|e| {
            crate::error::CliError::GitError(format!("Failed to clone repository: {}", e))
        })?;

    // Checkout the specified reference if not HEAD
    if git_ref != "HEAD" {
        ui::info(&format!("Checking out {}...", git_ref));

        // Try to find the reference as a branch, tag, or commit
        let obj = repo.revparse_single(git_ref).map_err(|e| {
            crate::error::CliError::GitError(format!("Failed to find ref '{}': {}", git_ref, e))
        })?;

        repo.checkout_tree(&obj, None).map_err(|e| {
            crate::error::CliError::GitError(format!("Failed to checkout ref '{}': {}", git_ref, e))
        })?;

        // Set HEAD to the checked out reference
        repo.set_head_detached(obj.id())
            .map_err(|e| crate::error::CliError::GitError(format!("Failed to set HEAD: {}", e)))?;
    }

    ui::success(&format!("Cloned {} to cache", cog_name));
    Ok(())
}

/// Download a package from IPFS using public gateways
fn download_from_ipfs(ipfs_hash: &Text, cog_name: &Text, cache: &PathBuf) -> Result<()> {
    // List of public IPFS gateways to try
    let gateways = [
        "https://ipfs.io/ipfs/",
        "https://cloudflare-ipfs.com/ipfs/",
        "https://dweb.link/ipfs/",
        "https://gateway.pinata.cloud/ipfs/",
    ];

    // Create cache directory for IPFS packages
    let ipfs_cache = cache.join("ipfs").join(cog_name.as_str());
    std::fs::create_dir_all(&ipfs_cache)?;

    let cog_file = ipfs_cache.join(format!("{}.tar.gz", ipfs_hash));

    // Check if already downloaded
    if cog_file.exists() {
        ui::info("Cog already cached");
        return Ok(());
    }

    // Try each gateway until one succeeds
    let mut last_error = None;

    for gateway in &gateways {
        let url = format!("{}{}", gateway, ipfs_hash);
        ui::info(&format!("Trying gateway: {}", gateway));

        match download_from_url(&url, &cog_file) {
            Ok(_) => {
                ui::success(&format!("Downloaded {} from IPFS", cog_name));
                return Ok(());
            }
            Err(e) => {
                ui::info(&format!("Gateway failed: {}", e));
                last_error = Some(e);
            }
        }
    }

    // All gateways failed
    Err(last_error
        .unwrap_or_else(|| crate::error::CliError::Network("All IPFS gateways failed".into())))
}

/// Download a file from a URL using reqwest
fn download_from_url(url: &str, dest: &PathBuf) -> Result<()> {
    use reqwest::blocking::Client;
    use std::io::{Read, Write};

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| {
            crate::error::CliError::Network(format!("Failed to create HTTP client: {}", e))
        })?;

    let mut response = client
        .get(url)
        .send()
        .map_err(|e| crate::error::CliError::Network(format!("HTTP request failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(crate::error::CliError::Network(format!(
            "HTTP request failed with status: {}",
            response.status()
        )));
    }

    let mut file = std::fs::File::create(dest)?;
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = response.read(&mut buffer).map_err(|e| {
            crate::error::CliError::Network(format!("Failed to read response: {}", e))
        })?;

        if bytes_read == 0 {
            break;
        }

        file.write_all(&buffer[..bytes_read])?;
    }

    Ok(())
}
