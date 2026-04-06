// Cog registry security: signature verification, supply chain integrity, vulnerability scanning

use super::types::*;
use crate::error::{CliError, Result};
use semver::Version;
use std::path::Path;
use verum_common::{List, Map, Set, Text};

// Re-export types for tests
pub use super::types::{Severity, Vulnerability, VulnerabilityReport};

/// Security scanner for package vulnerabilities
pub struct SecurityScanner {
    /// Known vulnerability database
    pub vulnerability_db: VulnerabilityDatabase,

    /// Audit log
    pub audit_log: List<AuditEntry>,
}

/// Vulnerability database
#[derive(Debug, Clone)]
pub struct VulnerabilityDatabase {
    /// Cog vulnerabilities
    vulnerabilities: Map<Text, List<Vulnerability>>,

    /// Last updated timestamp
    last_updated: i64,
}

/// Audit log entry
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditEntry {
    pub timestamp: i64,
    pub action: AuditAction,
    pub package: Option<Text>,
    pub version: Option<Text>,
    pub user: Option<Text>,
    pub details: Text,
}

/// Audit action
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditAction {
    Install,
    Update,
    Remove,
    Publish,
    Yank,
    SecurityScan,
    VulnerabilityFound,
}

/// Security scan result
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub vulnerabilities: List<VulnerabilityMatch>,
    pub licenses: List<LicenseIssue>,
    pub supply_chain_risks: List<SupplyChainRisk>,
    pub total_severity_score: f64,
}

/// Vulnerability match
#[derive(Debug, Clone)]
pub struct VulnerabilityMatch {
    pub cog: Text,
    pub version: Text,
    pub vulnerability: Vulnerability,
}

/// License issue
#[derive(Debug, Clone)]
pub struct LicenseIssue {
    pub package: Text,
    pub license: Text,
    pub issue: LicenseIssueType,
}

/// License issue type
#[derive(Debug, Clone)]
pub enum LicenseIssueType {
    Incompatible,
    Unknown,
    Copyleft,
    Proprietary,
}

/// Supply chain risk
#[derive(Debug, Clone)]
pub struct SupplyChainRisk {
    pub package: Text,
    pub risk_type: RiskType,
    pub severity: Severity,
    pub description: Text,
}

/// Risk type
#[derive(Debug, Clone)]
pub enum RiskType {
    UnverifiedPublisher,
    MissingSignature,
    RecentlyCreated,
    LowDownloads,
    ManyDependencies,
    UnusualBehavior,
}

impl SecurityScanner {
    /// Create new security scanner
    pub fn new() -> Self {
        Self {
            vulnerability_db: VulnerabilityDatabase {
                vulnerabilities: Map::new(),
                last_updated: 0,
            },
            audit_log: List::new(),
        }
    }

    /// Load vulnerability database
    pub fn load_database(&mut self, path: &Path) -> Result<()> {
        if !path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(path)?;
        self.vulnerability_db = serde_json::from_str(&content)?;

        Ok(())
    }

    /// Update vulnerability database from registry
    pub fn update_database(&mut self, registry_url: &str) -> Result<()> {
        use reqwest::blocking::Client;

        let client = Client::new();
        let url = format!("{}/security/database", registry_url);

        let response = client
            .get(&url)
            .send()
            .map_err(|e| CliError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CliError::Custom(
                "Failed to update vulnerability database".into(),
            ));
        }

        self.vulnerability_db = response
            .json()
            .map_err(|e| CliError::Custom(format!("Failed to parse database: {}", e)))?;

        self.vulnerability_db.last_updated = chrono::Utc::now().timestamp();

        Ok(())
    }

    /// Scan package for vulnerabilities
    pub fn scan_cog(&mut self, metadata: &CogMetadata) -> Result<ScanResult> {
        let mut result = ScanResult {
            vulnerabilities: List::new(),
            licenses: List::new(),
            supply_chain_risks: List::new(),
            total_severity_score: 0.0,
        };

        // Check for known vulnerabilities
        if let Some(vulns) = self.vulnerability_db.vulnerabilities.get(&metadata.name) {
            let version = Version::parse(metadata.version.as_str())
                .map_err(|e| CliError::Custom(format!("Invalid version: {}", e)))?;

            for vuln in vulns {
                if self.is_vulnerable(&version, &vuln.patched_versions) {
                    result.total_severity_score += self.severity_score(&vuln.severity);

                    result.vulnerabilities.push(VulnerabilityMatch {
                        cog: metadata.name.clone(),
                        version: metadata.version.clone(),
                        vulnerability: vuln.clone(),
                    });
                }
            }
        }

        // Check license
        if let Some(license) = &metadata.license {
            if let Some(issue) = self.check_license(license.as_str()) {
                result.licenses.push(LicenseIssue {
                    package: metadata.name.clone(),
                    license: license.clone(),
                    issue,
                });
            }
        } else {
            result.licenses.push(LicenseIssue {
                package: metadata.name.clone(),
                license: "UNKNOWN".into(),
                issue: LicenseIssueType::Unknown,
            });
        }

        // Check supply chain risks
        result
            .supply_chain_risks
            .extend(self.check_supply_chain_risks(metadata));

        // Log audit entry
        if !result.vulnerabilities.is_empty() {
            self.audit_log.push(AuditEntry {
                timestamp: chrono::Utc::now().timestamp(),
                action: AuditAction::VulnerabilityFound,
                package: Some(metadata.name.clone()),
                version: Some(metadata.version.clone()),
                user: None,
                details: format!("Found {} vulnerabilities", result.vulnerabilities.len()).into(),
            });
        }

        Ok(result)
    }

    /// Scan all dependencies
    pub fn scan_dependencies(&mut self, packages: &[CogMetadata]) -> Result<List<ScanResult>> {
        let mut results = List::new();

        for package in packages {
            let result = self.scan_cog(package)?;
            results.push(result);
        }

        Ok(results)
    }

    /// Check if version is vulnerable
    fn is_vulnerable(&self, version: &Version, patched_versions: &[Text]) -> bool {
        for patched in patched_versions {
            if let Ok(patched_ver) = Version::parse(patched.as_str())
                && version >= &patched_ver
            {
                return false;
            }
        }

        true
    }

    /// Get severity score
    pub fn severity_score(&self, severity: &Severity) -> f64 {
        match severity {
            Severity::Low => 1.0,
            Severity::Medium => 5.0,
            Severity::High => 10.0,
            Severity::Critical => 20.0,
        }
    }

    /// Check license compatibility
    pub fn check_license(&self, license: &str) -> Option<LicenseIssueType> {
        // Check for incompatible licenses
        let incompatible = ["GPL-3.0", "AGPL-3.0", "SSPL"];

        if incompatible.iter().any(|&l| license.contains(l)) {
            return Some(LicenseIssueType::Incompatible);
        }

        // Check for copyleft
        let copyleft = ["GPL", "LGPL", "MPL", "EPL"];

        if copyleft.iter().any(|&l| license.contains(l)) {
            return Some(LicenseIssueType::Copyleft);
        }

        None
    }

    /// Check supply chain risks
    fn check_supply_chain_risks(&self, metadata: &CogMetadata) -> List<SupplyChainRisk> {
        let mut risks = List::new();

        // Check for missing signature
        if metadata.signature.is_none() {
            risks.push(SupplyChainRisk {
                package: metadata.name.clone(),
                risk_type: RiskType::MissingSignature,
                severity: Severity::Medium,
                description: "Cog is not cryptographically signed".into(),
            });
        }

        // Check for many dependencies (potential attack surface)
        if metadata.dependencies.len() > 20 {
            risks.push(SupplyChainRisk {
                package: metadata.name.clone(),
                risk_type: RiskType::ManyDependencies,
                severity: Severity::Low,
                description: format!(
                    "Cog has {} dependencies (large attack surface)",
                    metadata.dependencies.len()
                )
                .into(),
            });
        }

        // Check for recently created packages
        let now = chrono::Utc::now().timestamp();
        let one_month_ago = now - (30 * 24 * 60 * 60);

        if metadata.published_at > one_month_ago {
            risks.push(SupplyChainRisk {
                package: metadata.name.clone(),
                risk_type: RiskType::RecentlyCreated,
                severity: Severity::Low,
                description: "Cog was recently published (< 1 month)".into(),
            });
        }

        risks
    }

    /// Add audit log entry
    pub fn log_action(
        &mut self,
        action: AuditAction,
        package: Option<Text>,
        version: Option<Text>,
        details: Text,
    ) {
        self.audit_log.push(AuditEntry {
            timestamp: chrono::Utc::now().timestamp(),
            action,
            package,
            version,
            user: std::env::var("USER").ok().map(|s| s.into()),
            details,
        });
    }

    /// Get audit log
    pub fn get_audit_log(&self) -> &[AuditEntry] {
        &self.audit_log
    }

    /// Save audit log to file
    pub fn save_audit_log(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.audit_log)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Generate security report
    pub fn generate_report(&self, results: &[ScanResult]) -> SecurityReport {
        let mut total_vulnerabilities = 0;
        let mut critical_count = 0;
        let mut high_count = 0;
        let mut medium_count = 0;
        let mut low_count = 0;

        let mut affected_cogs = Set::new();

        for result in results {
            total_vulnerabilities += result.vulnerabilities.len();

            for vuln_match in &result.vulnerabilities {
                affected_cogs.insert(vuln_match.cog.clone());

                match vuln_match.vulnerability.severity {
                    Severity::Critical => critical_count += 1,
                    Severity::High => high_count += 1,
                    Severity::Medium => medium_count += 1,
                    Severity::Low => low_count += 1,
                }
            }
        }

        let max_score = results
            .iter()
            .map(|r| r.total_severity_score)
            .fold(0.0f64, f64::max);

        SecurityReport {
            total_vulnerabilities,
            affected_cogs: affected_cogs.len(),
            critical_count,
            high_count,
            medium_count,
            low_count,
            max_severity_score: max_score,
            license_issues: results.iter().map(|r| r.licenses.len()).sum(),
            supply_chain_risks: results.iter().map(|r| r.supply_chain_risks.len()).sum(),
        }
    }
}

/// Security report summary
#[derive(Debug, Clone)]
pub struct SecurityReport {
    pub total_vulnerabilities: usize,
    pub affected_cogs: usize,
    pub critical_count: usize,
    pub high_count: usize,
    pub medium_count: usize,
    pub low_count: usize,
    pub max_severity_score: f64,
    pub license_issues: usize,
    pub supply_chain_risks: usize,
}

impl SecurityReport {
    /// Check if report is clean
    pub fn is_clean(&self) -> bool {
        self.total_vulnerabilities == 0 && self.critical_count == 0 && self.high_count == 0
    }

    /// Get risk level
    pub fn risk_level(&self) -> RiskLevel {
        if self.critical_count > 0 {
            RiskLevel::Critical
        } else if self.high_count > 0 {
            RiskLevel::High
        } else if self.medium_count > 0 {
            RiskLevel::Medium
        } else if self.low_count > 0 {
            RiskLevel::Low
        } else {
            RiskLevel::None
        }
    }
}

/// Risk level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    None,
    Low,
    Medium,
    High,
    Critical,
}

impl Default for SecurityScanner {
    fn default() -> Self {
        Self::new()
    }
}

// Implement Serialize for VulnerabilityDatabase
impl serde::Serialize for VulnerabilityDatabase {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("VulnerabilityDatabase", 2)?;
        state.serialize_field("vulnerabilities", &self.vulnerabilities)?;
        state.serialize_field("last_updated", &self.last_updated)?;
        state.end()
    }
}

// Implement Deserialize for VulnerabilityDatabase
impl<'de> serde::Deserialize<'de> for VulnerabilityDatabase {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Helper {
            vulnerabilities: Map<Text, List<Vulnerability>>,
            last_updated: i64,
        }

        let helper = Helper::deserialize(deserializer)?;
        Ok(VulnerabilityDatabase {
            vulnerabilities: helper.vulnerabilities,
            last_updated: helper.last_updated,
        })
    }
}
