// Cog registry enterprise features: private registries, proxy support, offline mode, compliance

use crate::error::{CliError, Result};
use reqwest::Proxy;
use reqwest::blocking::{Client, ClientBuilder};
use std::path::{Path, PathBuf};
use std::time::Duration;
use verum_common::{List, Text};

/// Enterprise configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EnterpriseConfig {
    /// HTTP/HTTPS proxy
    pub proxy: Option<ProxyConfig>,

    /// Offline mode
    pub offline: bool,

    /// Corporate registry mirrors
    pub mirrors: List<MirrorConfig>,

    /// Cog allow/deny lists
    pub access_control: AccessControl,

    /// Audit logging
    pub audit: AuditConfig,

    /// Compliance policies
    pub compliance: ComplianceConfig,
}

/// Proxy configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProxyConfig {
    /// Proxy URL (http://proxy:port or https://proxy:port)
    pub url: Text,

    /// Username for authentication
    pub username: Option<Text>,

    /// Password for authentication
    pub password: Option<Text>,

    /// Bypass proxy for these domains
    pub no_proxy: List<Text>,
}

/// Mirror configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MirrorConfig {
    /// Mirror name
    pub name: Text,

    /// Mirror URL
    pub url: Text,

    /// Priority (lower = higher priority)
    pub priority: u32,

    /// Only use for specific packages
    pub packages: Option<List<Text>>,
}

/// Access control configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AccessControl {
    /// Allow list (if set, only these packages allowed)
    pub allow_list: Option<List<Text>>,

    /// Deny list (these packages forbidden)
    pub deny_list: List<Text>,

    /// Require signature verification
    pub require_signature: bool,

    /// Allowed licenses
    pub allowed_licenses: Option<List<Text>>,
}

/// Audit configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditConfig {
    /// Enable audit logging
    pub enabled: bool,

    /// Audit log file path
    pub log_file: PathBuf,

    /// Log retention days
    pub retention_days: u32,

    /// Include in audit log
    pub log_level: AuditLevel,
}

/// Audit level
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditLevel {
    /// Log all operations
    All,

    /// Log only package changes
    Changes,

    /// Log only security events
    Security,
}

/// Compliance configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ComplianceConfig {
    /// SBOM (Software Bill of Materials) generation
    pub generate_sbom: bool,

    /// SBOM format (spdx, cyclonedx)
    pub sbom_format: SbomFormat,

    /// Vulnerability scanning required
    pub require_vulnerability_scan: bool,

    /// Maximum allowed severity
    pub max_severity: Text,

    /// License compliance checks
    pub license_compliance: bool,
}

/// SBOM format
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SbomFormat {
    Spdx,
    CycloneDx,
}

impl Default for EnterpriseConfig {
    fn default() -> Self {
        Self {
            proxy: None,
            offline: false,
            mirrors: List::new(),
            access_control: AccessControl {
                allow_list: None,
                deny_list: List::new(),
                require_signature: false,
                allowed_licenses: None,
            },
            audit: AuditConfig {
                enabled: false,
                log_file: PathBuf::from("/var/log/verum/audit.log"),
                retention_days: 90,
                log_level: AuditLevel::Changes,
            },
            compliance: ComplianceConfig {
                generate_sbom: false,
                sbom_format: SbomFormat::Spdx,
                require_vulnerability_scan: false,
                max_severity: "high".into(),
                license_compliance: false,
            },
        }
    }
}

/// Enterprise client with proxy and offline support
pub struct EnterpriseClient {
    config: EnterpriseConfig,
    client: Option<Client>,
}

impl EnterpriseClient {
    /// Get a reference to the enterprise configuration
    pub fn config(&self) -> &EnterpriseConfig {
        &self.config
    }

    /// Create new enterprise client
    pub fn new(config: EnterpriseConfig) -> Result<Self> {
        let client = if !config.offline {
            Some(Self::build_client(&config)?)
        } else {
            None
        };

        // Phase-not-realised tracing for inert AuditConfig fields.
        // `audit.enabled` IS consumed at cog_manager.rs:190/263/332
        // (gates `security::log_action` calls), but three companion
        // fields land on the manifest without reaching production
        // code paths:
        //
        // - `audit.log_file` (PathBuf) — `security::save_audit_log`
        //   takes a path argument from the caller; no caller threads
        //   the manifest's value through. Audit entries currently
        //   live in-memory only (Vec<AuditEntry>) and never flush
        //   to the configured file.
        //
        // - `audit.retention_days` (u32) — no rotation/eviction
        //   pass exists; old entries persist indefinitely.
        //
        // - `audit.log_level` (AuditLevel: All/Changes/Security) —
        //   `log_action` records every call regardless of the
        //   configured level; no filtering happens at the recorder.
        //
        // Surface a debug trace when the user has set any of these
        // to a non-default-equivalent value so embedders writing
        // `[audit] log_file = "/var/log/verum.log"` see the value
        // was observed but not threaded through.
        if config.audit.enabled {
            let default_log_file = std::path::PathBuf::from("verum-audit.log");
            let log_file_overridden = config.audit.log_file != default_log_file;
            let retention_overridden = config.audit.retention_days != 90;
            let log_level_overridden =
                !matches!(config.audit.log_level, AuditLevel::All);
            if log_file_overridden || retention_overridden || log_level_overridden {
                tracing::debug!(
                    "EnterpriseConfig.audit surface: log_file={:?}, retention_days={}, \
                     log_level={:?} (these fields land on the AuditConfig and are \
                     parsed from the manifest, but `security::log_action` records \
                     in-memory only — no caller flushes to log_file, no rotation \
                     pass honours retention_days, and no level-filtering applies \
                     log_level. Forward-looking; only `audit.enabled` reaches the \
                     recorder gate today.)",
                    config.audit.log_file,
                    config.audit.retention_days,
                    config.audit.log_level,
                );
            }
        }

        Ok(Self { config, client })
    }

    /// Build HTTP client with proxy support
    fn build_client(config: &EnterpriseConfig) -> Result<Client> {
        let mut builder = ClientBuilder::new()
            .timeout(Duration::from_secs(30))
            .user_agent("verum-cli/1.0.0");

        // Configure proxy
        if let Some(proxy_config) = &config.proxy {
            let mut proxy = Proxy::all(proxy_config.url.as_str())
                .map_err(|e| CliError::Custom(format!("Invalid proxy URL: {}", e)))?;

            // Add authentication if configured
            if let (Some(username), Some(password)) =
                (&proxy_config.username, &proxy_config.password)
            {
                proxy = proxy.basic_auth(username.as_str(), password.as_str());
            }

            builder = builder.proxy(proxy);

            // Configure no_proxy
            if !proxy_config.no_proxy.is_empty() {
                builder = builder.no_proxy();
            }
        }

        builder
            .build()
            .map_err(|e| CliError::Custom(format!("Failed to build HTTP client: {}", e)))
    }

    /// Check if cog is allowed
    pub fn is_cog_allowed(&self, cog_name: &str) -> bool {
        // Check deny list first
        if self
            .config
            .access_control
            .deny_list
            .contains(&cog_name.into())
        {
            return false;
        }

        // Check allow list if configured
        if let Some(allow_list) = &self.config.access_control.allow_list {
            return allow_list.contains(&cog_name.into());
        }

        true
    }

    /// Whether the access-control policy requires every installed
    /// cog to ship a verified signature.
    ///
    /// Surfaces `EnterpriseConfig.access_control.require_signature`
    /// as a public read so callers driving install / publish flows
    /// can branch on the policy without re-reading the config.
    pub fn requires_signature(&self) -> bool {
        self.config.access_control.require_signature
    }

    /// Combined access check: cog name passes the allow / deny lists
    /// AND, when `require_signature = true`, the caller has confirmed
    /// the cog ships a verified signature.
    ///
    /// This is the load-bearing wiring for
    /// `AccessControl.require_signature`. Pre-fix the field was
    /// inert: enterprises that set the flag in `enterprise.toml`
    /// would still install unsigned cogs because no code path
    /// consulted the flag. Now the policy is enforced at every
    /// gate that calls this method.
    ///
    /// Callers that don't have signature info yet should call
    /// `is_cog_allowed` for the name-only check and
    /// `requires_signature` to decide whether to look up the
    /// signature before proceeding.
    pub fn is_cog_allowed_with_signature(
        &self,
        cog_name: &str,
        has_valid_signature: bool,
    ) -> bool {
        if !self.is_cog_allowed(cog_name) {
            return false;
        }
        if self.requires_signature() && !has_valid_signature {
            return false;
        }
        true
    }

    /// Check if license is allowed
    pub fn is_license_allowed(&self, license: &str) -> bool {
        if let Some(allowed) = &self.config.access_control.allowed_licenses {
            return allowed.iter().any(|l| license.contains(l.as_str()));
        }

        true
    }

    /// Whether the license-compliance gate is enabled at the
    /// compliance layer. When `false` (the default) the install
    /// pipeline must skip license checks entirely so a project
    /// without an explicit compliance policy doesn't accidentally
    /// reject cogs whose licenses happen to match a partial-string
    /// rule in `allowed_licenses`.
    ///
    /// Pre-fix this field was set in defaults / parsed from
    /// `enterprise.toml` but no consumer read it — the license check
    /// in `CogManager::install` always fired when `enterprise` was
    /// configured, regardless of whether the operator opted into
    /// license compliance. Wiring lets the field act as the
    /// documented kill-switch.
    pub fn license_compliance_enabled(&self) -> bool {
        self.config.compliance.license_compliance
    }

    /// Whether a vulnerability of the given severity exceeds the
    /// `compliance.max_severity` policy.
    ///
    /// `max_severity` (default `"high"`) caps the SEVERITY allowed
    /// through the install gate when `require_vulnerability_scan`
    /// is also true. Returns `true` (= blocked) when:
    ///
    /// * the actual severity rank > the configured ceiling, or
    /// * the configured ceiling string is not a recognised severity
    ///   (fail-closed: an unrecognised policy value blocks everything
    ///   so a typo doesn't silently disable the gate).
    ///
    /// Pre-fix the install path treated `require_vulnerability_scan
    /// = true` as "block on ANY vulnerability". Now `max_severity =
    /// "high"` means Low/Medium/High vulnerabilities pass through
    /// (the operator was warned but the install isn't blocked) and
    /// only Critical vulnerabilities trip the gate. Operators who
    /// want strict mode set `max_severity = "low"` to revert.
    pub fn vulnerability_exceeds_policy(
        &self,
        severity: &super::types::Severity,
    ) -> bool {
        use super::types::Severity;
        let ceiling = match self.config.compliance.max_severity.as_str().to_ascii_lowercase().as_str() {
            "low" => 1u8,
            "medium" => 2,
            "high" => 3,
            "critical" => 4,
            _ => return true, // fail-closed on unrecognised policy string
        };
        let actual = match severity {
            Severity::Low => 1u8,
            Severity::Medium => 2,
            Severity::High => 3,
            Severity::Critical => 4,
        };
        actual > ceiling
    }

    /// Get best mirror for package
    pub fn get_mirror_url(&self, cog_name: &str) -> Option<Text> {
        let mut applicable_mirrors: List<_> = self
            .config
            .mirrors
            .iter()
            .filter(|m| {
                if let Some(packages) = &m.packages {
                    packages.contains(&cog_name.into())
                } else {
                    true
                }
            })
            .collect();

        // Sort by priority
        applicable_mirrors.sort_by_key(|m| m.priority);

        applicable_mirrors.first().map(|m| m.url.clone())
    }

    /// Check if in offline mode
    pub fn is_offline(&self) -> bool {
        self.config.offline
    }

    /// Get HTTP client
    pub fn client(&self) -> Result<&Client> {
        self.client
            .as_ref()
            .ok_or_else(|| CliError::Custom("Client not available in offline mode".into()))
    }

    /// Load enterprise config from file
    pub fn load_config(path: &Path) -> Result<EnterpriseConfig> {
        if !path.exists() {
            return Ok(EnterpriseConfig::default());
        }

        let content = std::fs::read_to_string(path)?;
        let config: EnterpriseConfig = toml::from_str(&content)?;

        Ok(config)
    }

    /// Save enterprise config to file
    pub fn save_config(config: &EnterpriseConfig, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(config)?;
        std::fs::write(path, content)?;

        Ok(())
    }
}

/// SBOM generator
pub struct SbomGenerator {
    pub format: SbomFormat,
}

impl SbomGenerator {
    /// Create new SBOM generator
    pub fn new(format: SbomFormat) -> Self {
        Self { format }
    }

    /// Generate SBOM for packages
    pub fn generate(
        &self,
        packages: &[super::types::CogMetadata],
        output_path: &Path,
    ) -> Result<()> {
        match self.format {
            SbomFormat::Spdx => self.generate_spdx(packages, output_path),
            SbomFormat::CycloneDx => self.generate_cyclonedx(packages, output_path),
        }
    }

    /// Generate SPDX format SBOM
    fn generate_spdx(
        &self,
        packages: &[super::types::CogMetadata],
        output_path: &Path,
    ) -> Result<()> {
        use serde_json::json;

        let mut components = List::new();

        for pkg in packages {
            components.push(json!({
                "SPDXID": format!("SPDXRef-Package-{}", pkg.name),
                "name": pkg.name,
                "versionInfo": pkg.version,
                "supplier": pkg.authors.join(", "),
                "downloadLocation": pkg.repository.as_ref().unwrap_or(&"NOASSERTION".into()),
                "filesAnalyzed": false,
                "licenseConcluded": pkg.license.as_ref().unwrap_or(&"NOASSERTION".into()),
                "copyrightText": "NOASSERTION",
                "checksums": [{
                    "algorithm": "SHA256",
                    "checksumValue": pkg.checksum
                }]
            }));
        }

        let sbom = json!({
            "spdxVersion": "SPDX-2.3",
            "dataLicense": "CC0-1.0",
            "SPDXID": "SPDXRef-DOCUMENT",
            "name": "Verum Package SBOM",
            "documentNamespace": format!("https://verum.lang/sbom/{}", uuid::Uuid::new_v4()),
            "creationInfo": {
                "created": chrono::Utc::now().to_rfc3339(),
                "creators": ["Tool: verum-cli-1.0.0"]
            },
            "packages": components
        });

        let content = serde_json::to_string_pretty(&sbom)?;
        std::fs::write(output_path, content)?;

        Ok(())
    }

    /// Generate CycloneDX format SBOM
    fn generate_cyclonedx(
        &self,
        packages: &[super::types::CogMetadata],
        output_path: &Path,
    ) -> Result<()> {
        use serde_json::json;

        let mut components = List::new();

        for pkg in packages {
            components.push(json!({
                "type": "library",
                "name": pkg.name,
                "version": pkg.version,
                "description": pkg.description.as_ref().unwrap_or(&"".into()),
                "licenses": pkg.license.as_ref().map(|l| vec![json!({"license": {"id": l}})]),
                "hashes": [{
                    "alg": "SHA-256",
                    "content": pkg.checksum
                }]
            }));
        }

        let sbom = json!({
            "bomFormat": "CycloneDX",
            "specVersion": "1.4",
            "version": 1,
            "metadata": {
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "tools": [{
                    "name": "verum-cli",
                    "version": "1.0.0"
                }]
            },
            "components": components
        });

        let content = serde_json::to_string_pretty(&sbom)?;
        std::fs::write(output_path, content)?;

        Ok(())
    }
}

// Add uuid dependency for SPDX
use uuid;

#[cfg(test)]
mod tests {
    use super::*;

    fn enterprise_config_with_signature_required(required: bool) -> EnterpriseConfig {
        let mut cfg = EnterpriseConfig::default();
        cfg.access_control.require_signature = required;
        cfg.offline = true; // skip HTTP client construction in tests
        cfg
    }

    #[test]
    fn requires_signature_mirrors_config() {
        // Pin: the read accessor surfaces the configured stance
        // verbatim. Lets driver code branch on the policy without
        // having to re-read EnterpriseConfig internals.
        let cfg = enterprise_config_with_signature_required(true);
        let client = EnterpriseClient::new(cfg).expect("offline client builds");
        assert!(client.requires_signature());

        let cfg = enterprise_config_with_signature_required(false);
        let client = EnterpriseClient::new(cfg).expect("offline client builds");
        assert!(!client.requires_signature());
    }

    #[test]
    fn signature_required_rejects_unsigned_cog() {
        // Pin: with `require_signature = true`, the combined check
        // rejects an otherwise-allowed cog whose signature isn't
        // verified. The name-only `is_cog_allowed` would still pass
        // — proving the new combined API is doing real work, not
        // just delegating.
        let cfg = enterprise_config_with_signature_required(true);
        let client = EnterpriseClient::new(cfg).expect("offline client builds");

        assert!(
            client.is_cog_allowed("anything"),
            "name-only check passes — no allow / deny list configured",
        );
        assert!(
            !client.is_cog_allowed_with_signature("anything", false),
            "require_signature=true must reject unsigned cogs even when name-only check passes",
        );
        assert!(
            client.is_cog_allowed_with_signature("anything", true),
            "require_signature=true accepts signed cogs",
        );
    }

    #[test]
    fn signature_optional_accepts_unsigned_cog() {
        // Pin: with `require_signature = false` (the default), the
        // combined check accepts an unsigned cog as long as the
        // name passes the allow / deny lists.
        let cfg = enterprise_config_with_signature_required(false);
        let client = EnterpriseClient::new(cfg).expect("offline client builds");

        assert!(
            client.is_cog_allowed_with_signature("anything", false),
            "default policy accepts unsigned cogs",
        );
    }

    #[test]
    fn signature_check_still_honours_deny_list() {
        // Pin: signature verification doesn't bypass the deny list
        // — a signed cog on the deny list is still rejected. The
        // signature gate is ADDITIONAL to the existing checks, not
        // a replacement.
        let mut cfg = enterprise_config_with_signature_required(true);
        cfg.access_control.deny_list.push("forbidden-cog".into());
        let client = EnterpriseClient::new(cfg).expect("offline client builds");

        assert!(
            !client.is_cog_allowed_with_signature("forbidden-cog", true),
            "deny list trumps signature verification",
        );
    }

    #[test]
    fn license_compliance_disabled_by_default() {
        // Pin: the default `enterprise.toml` shape carries
        // `compliance.license_compliance = false`, so the gate is
        // OFF for projects that ship enterprise.toml only for proxy
        // / audit purposes. Without this gate the install pipeline
        // would fire the license check on EVERY enterprise project.
        let mut cfg = EnterpriseConfig::default();
        cfg.offline = true;
        let client = EnterpriseClient::new(cfg).expect("offline client builds");
        assert!(
            !client.license_compliance_enabled(),
            "license compliance must default to OFF",
        );
    }

    #[test]
    fn license_compliance_enabled_mirrors_config() {
        // Pin: when `compliance.license_compliance = true` is set
        // in enterprise.toml, the accessor reflects it.
        let mut cfg = EnterpriseConfig::default();
        cfg.offline = true;
        cfg.compliance.license_compliance = true;
        let client = EnterpriseClient::new(cfg).expect("offline client builds");
        assert!(client.license_compliance_enabled());
    }

    #[test]
    fn vulnerability_policy_default_high_blocks_only_critical() {
        // Pin: with the default `max_severity = "high"`, only
        // Critical vulnerabilities exceed the policy. This is the
        // documented permissive default — Low/Medium/High pass
        // through with a warning, only Critical hard-blocks.
        use super::super::types::Severity;
        let mut cfg = EnterpriseConfig::default();
        cfg.offline = true;
        let client = EnterpriseClient::new(cfg).expect("offline client builds");
        assert!(!client.vulnerability_exceeds_policy(&Severity::Low));
        assert!(!client.vulnerability_exceeds_policy(&Severity::Medium));
        assert!(!client.vulnerability_exceeds_policy(&Severity::High));
        assert!(client.vulnerability_exceeds_policy(&Severity::Critical));
    }

    #[test]
    fn vulnerability_policy_low_blocks_everything_above() {
        // Pin: with `max_severity = "low"`, only Low-severity vulns
        // pass through; Medium/High/Critical all block. Strictest
        // operator-configurable policy.
        use super::super::types::Severity;
        let mut cfg = EnterpriseConfig::default();
        cfg.offline = true;
        cfg.compliance.max_severity = "low".into();
        let client = EnterpriseClient::new(cfg).expect("offline client builds");
        assert!(!client.vulnerability_exceeds_policy(&Severity::Low));
        assert!(client.vulnerability_exceeds_policy(&Severity::Medium));
        assert!(client.vulnerability_exceeds_policy(&Severity::High));
        assert!(client.vulnerability_exceeds_policy(&Severity::Critical));
    }

    #[test]
    fn vulnerability_policy_critical_allows_everything() {
        // Pin: with `max_severity = "critical"`, no severity
        // exceeds the ceiling — `require_vulnerability_scan`
        // becomes a soft warning only.
        use super::super::types::Severity;
        let mut cfg = EnterpriseConfig::default();
        cfg.offline = true;
        cfg.compliance.max_severity = "critical".into();
        let client = EnterpriseClient::new(cfg).expect("offline client builds");
        for sev in &[Severity::Low, Severity::Medium, Severity::High, Severity::Critical] {
            assert!(
                !client.vulnerability_exceeds_policy(sev),
                "max_severity=critical must accept {:?}",
                sev,
            );
        }
    }

    #[test]
    fn vulnerability_policy_unrecognised_string_fails_closed() {
        // Pin: a typo in `max_severity` (e.g. "hgih") does NOT
        // silently disable the gate — the policy fails closed and
        // every severity is blocked. This is the safer failure
        // mode for a security policy.
        use super::super::types::Severity;
        let mut cfg = EnterpriseConfig::default();
        cfg.offline = true;
        cfg.compliance.max_severity = "hgih".into(); // typo
        let client = EnterpriseClient::new(cfg).expect("offline client builds");
        for sev in &[Severity::Low, Severity::Medium, Severity::High, Severity::Critical] {
            assert!(
                client.vulnerability_exceeds_policy(sev),
                "unrecognised max_severity must fail closed for {:?}",
                sev,
            );
        }
    }

    #[test]
    fn vulnerability_policy_case_insensitive() {
        // Pin: `max_severity` parsing is case-insensitive — uppercase
        // / mixed-case values accepted so operators don't get bitten
        // by `Critical` vs `critical` discrepancies.
        use super::super::types::Severity;
        let mut cfg = EnterpriseConfig::default();
        cfg.offline = true;
        cfg.compliance.max_severity = "Medium".into();
        let client = EnterpriseClient::new(cfg).expect("offline client builds");
        assert!(!client.vulnerability_exceeds_policy(&Severity::Low));
        assert!(!client.vulnerability_exceeds_policy(&Severity::Medium));
        assert!(client.vulnerability_exceeds_policy(&Severity::High));
    }
}
