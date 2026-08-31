// SPDX-License-Identifier: AGPL-3.0-only
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

#[derive(
    Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum IntegritySeverity {
    Ok,
    Info,
    Warning,
    Error,
}

impl IntegritySeverity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityCategory {
    Connectivity,
    Version,
    Configuration,
    Database,
    Docker,
    Storage,
    Backup,
    Schema,
}

impl IntegrityCategory {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connectivity => "connectivity",
            Self::Version => "version",
            Self::Configuration => "configuration",
            Self::Database => "database",
            Self::Docker => "docker",
            Self::Storage => "storage",
            Self::Backup => "backup",
            Self::Schema => "schema",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IntegrityFinding {
    pub severity: IntegritySeverity,
    pub message: String,
    pub subject: Option<String>,
    pub details: BTreeMap<String, String>,
}

impl IntegrityFinding {
    #[must_use]
    pub const fn new(severity: IntegritySeverity, message: String) -> Self {
        Self {
            severity,
            message,
            subject: None,
            details: BTreeMap::new(),
        }
    }

    #[must_use]
    pub const fn info(message: String) -> Self {
        Self::new(IntegritySeverity::Info, message)
    }

    #[must_use]
    pub const fn warning(message: String) -> Self {
        Self::new(IntegritySeverity::Warning, message)
    }

    #[must_use]
    pub const fn error(message: String) -> Self {
        Self::new(IntegritySeverity::Error, message)
    }

    #[must_use]
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    #[must_use]
    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.details.insert(key.into(), value.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IntegrityCheckInfo {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: IntegrityCategory,
    pub deep: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IntegrityCheckResult {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: IntegrityCategory,
    pub deep: bool,
    pub severity: IntegritySeverity,
    pub duration_ms: u64,
    pub findings: Vec<IntegrityFinding>,
    pub skipped: Option<String>,
}

impl IntegrityCheckResult {
    #[must_use]
    pub fn severity_of(findings: &[IntegrityFinding]) -> IntegritySeverity {
        findings
            .iter()
            .map(|finding| finding.severity)
            .max()
            .unwrap_or(IntegritySeverity::Ok)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IntegrityReport {
    pub version: String,
    pub generated: DateTime<Utc>,
    pub duration_ms: u64,
    pub severity: IntegritySeverity,
    pub checks: Vec<IntegrityCheckResult>,
}

impl IntegrityReport {
    #[must_use]
    pub fn new(version: String, duration_ms: u64, checks: Vec<IntegrityCheckResult>) -> Self {
        let severity = checks
            .iter()
            .map(|check| check.severity)
            .max()
            .unwrap_or(IntegritySeverity::Ok);
        Self {
            version,
            generated: Utc::now(),
            duration_ms,
            severity,
            checks,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IntegrityStatus {
    pub version: String,
    #[serde(default)]
    pub api_hash: String,
    pub uptime: u64,
    pub server_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IntegritySchemaTable {
    pub name: String,
    pub definition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IntegritySchema {
    pub engine: String,
    pub database: String,
    pub tables: Vec<IntegritySchemaTable>,
}
