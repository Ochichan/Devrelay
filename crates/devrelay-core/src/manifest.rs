//! `devrelay.toml` schema types, loading, and validation.
//!
//! The manifest captures portable project intent: workspace policy, environment
//! profiles, tasks, secrets, sync hints, and handoff preferences. Validation in
//! this module rejects unsupported schemas, empty executable commands, duplicate
//! pattern lists, and other inputs that would make later snapshot behavior
//! ambiguous.

use crate::error::{DevRelayError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub project_id: String,
    pub name: String,
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub environment: Option<EnvironmentConfig>,
    #[serde(default)]
    pub secrets: BTreeMap<String, SecretConfig>,
    #[serde(default)]
    pub tasks: BTreeMap<String, TaskConfig>,
    #[serde(default)]
    pub sync: Option<SyncConfig>,
    #[serde(default)]
    pub handoff: Option<HandoffConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
    pub untracked: UntrackedPolicy,
    pub portable_paths: PortablePathsPolicy,
    #[serde(default = "default_large_file_threshold_mib")]
    pub large_file_threshold_mib: u64,
    #[serde(default = "default_true")]
    pub preserve_editor_context: bool,
    #[serde(default = "default_true")]
    pub preserve_unsaved_buffers: bool,
    #[serde(default)]
    pub exclude: PatternConfig,
    #[serde(default)]
    pub include: PatternConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatternConfig {
    #[serde(default)]
    pub patterns: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UntrackedPolicy {
    None,
    Safe,
    AllNonignored,
    Explicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PortablePathsPolicy {
    Strict,
    Warn,
    Off,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentConfig {
    #[serde(default)]
    pub profiles: BTreeMap<String, EnvironmentProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentProfile {
    pub kind: EnvironmentKind,
    pub targets: Vec<String>,
    pub command: Vec<String>,
    #[serde(default)]
    pub fingerprint_files: Vec<String>,
    #[serde(default)]
    pub healthcheck: Option<Vec<String>>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnvironmentKind {
    Nix,
    Devcontainer,
    Script,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretConfig {
    pub target: String,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default)]
    pub mode: SecretMode,
    #[serde(default)]
    pub environment_variable: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SecretMode {
    #[default]
    File,
    Environment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskConfig {
    pub profile: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub cpu: Option<u64>,
    #[serde(default)]
    pub memory_mib: Option<u64>,
    #[serde(default)]
    pub disk_mib: Option<u64>,
    #[serde(default)]
    pub interactive: bool,
    #[serde(default)]
    pub cache: Option<TaskCacheMode>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub sandbox: Option<TaskSandbox>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskCacheMode {
    Off,
    Read,
    Write,
    ReadWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskSandbox {
    Host,
    Sandbox,
    Container,
    Vm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncConfig {
    #[serde(default)]
    pub mode: Option<SyncMode>,
    #[serde(default)]
    pub checkpoint_quiet_ms: Option<u64>,
    #[serde(default)]
    pub publish_quiet_ms: Option<u64>,
    #[serde(default)]
    pub max_publish_interval_s: Option<u64>,
    #[serde(default)]
    pub background_bandwidth_mib_s: Option<f64>,
    #[serde(default)]
    pub device_cache_quota_mib: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncMode {
    Adaptive,
    Instant,
    Eco,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffConfig {
    #[serde(default = "default_true")]
    pub restore_editor: bool,
    #[serde(default)]
    pub restore_terminals: Option<RestoreTerminals>,
    #[serde(default = "default_true")]
    pub open_editor: bool,
    #[serde(default)]
    pub target_dirty_policy: DirtyTargetPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestoreTerminals {
    None,
    LayoutOnly,
    ApprovedTasks,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DirtyTargetPolicy {
    #[default]
    SnapshotAndFork,
    NewWorkspace,
    Block,
}

fn default_true() -> bool {
    true
}

fn default_large_file_threshold_mib() -> u64 {
    32
}

impl Manifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let raw = fs::read_to_string(path)?;
        Self::parse(&raw)
    }

    pub fn parse(raw: &str) -> Result<Self> {
        let manifest: Self = toml::from_str(raw)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != 1 {
            return Err(DevRelayError::Manifest(format!(
                "unsupported schema {}, expected 1",
                self.schema
            )));
        }
        if self.project_id.len() < 8 {
            return Err(DevRelayError::Manifest(
                "project_id must be at least 8 characters".to_string(),
            ));
        }
        if self.name.is_empty() || self.name.len() > 128 {
            return Err(DevRelayError::Manifest(
                "name must be between 1 and 128 characters".to_string(),
            ));
        }
        if self.workspace.large_file_threshold_mib == 0
            || self.workspace.large_file_threshold_mib > 102_400
        {
            return Err(DevRelayError::Manifest(
                "large_file_threshold_mib must be between 1 and 102400".to_string(),
            ));
        }
        validate_unique(
            "workspace.exclude.patterns",
            &self.workspace.exclude.patterns,
        )?;
        validate_unique(
            "workspace.include.patterns",
            &self.workspace.include.patterns,
        )?;

        if let Some(environment) = &self.environment {
            for (name, profile) in &environment.profiles {
                validate_non_empty_string_array(
                    &format!("environment.profiles.{name}.targets"),
                    &profile.targets,
                )?;
                validate_command(
                    &format!("environment.profiles.{name}.command"),
                    &profile.command,
                )?;
                validate_unique(
                    &format!("environment.profiles.{name}.fingerprint_files"),
                    &profile.fingerprint_files,
                )?;
                if let Some(healthcheck) = &profile.healthcheck {
                    validate_command(
                        &format!("environment.profiles.{name}.healthcheck"),
                        healthcheck,
                    )?;
                }
            }
        }

        for (name, secret) in &self.secrets {
            if secret.target.is_empty() {
                return Err(DevRelayError::Manifest(format!(
                    "secrets.{name}.target must not be empty"
                )));
            }
        }

        for (name, task) in &self.tasks {
            if task.profile.is_empty() {
                return Err(DevRelayError::Manifest(format!(
                    "tasks.{name}.profile must not be empty"
                )));
            }
            validate_command(&format!("tasks.{name}.command"), &task.command)?;
            validate_unique(&format!("tasks.{name}.platforms"), &task.platforms)?;
            validate_unique(&format!("tasks.{name}.outputs"), &task.outputs)?;
            validate_unique(&format!("tasks.{name}.features"), &task.features)?;
        }

        Ok(())
    }
}

fn validate_command(field: &str, values: &[String]) -> Result<()> {
    if values.is_empty() || values.iter().any(|value| value.is_empty()) {
        return Err(DevRelayError::Manifest(format!(
            "{field} must contain at least one non-empty argument"
        )));
    }
    Ok(())
}

fn validate_non_empty_string_array(field: &str, values: &[String]) -> Result<()> {
    if values.iter().any(|value| value.is_empty()) {
        return Err(DevRelayError::Manifest(format!(
            "{field} must not contain empty values"
        )));
    }
    validate_unique(field, values)
}

fn validate_unique(field: &str, values: &[String]) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(DevRelayError::Manifest(format!(
                "{field} contains duplicate value {value:?}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bundled_manifest() {
        let raw = include_str!("../../../devrelay_spec_bundle/devrelay.toml");
        let manifest = Manifest::parse(raw).expect("bundled manifest should parse");
        assert_eq!(manifest.schema, 1);
        assert_eq!(manifest.name, "payments-api");
        assert_eq!(manifest.workspace.untracked, UntrackedPolicy::Safe);
        assert!(manifest.tasks.contains_key("test"));
    }

    #[test]
    fn rejects_duplicate_patterns() {
        let raw = r#"
schema = 1
project_id = "12345678"
name = "bad"

[workspace]
untracked = "safe"
portable_paths = "strict"

[workspace.exclude]
patterns = ["target/**", "target/**"]
"#;
        let err = Manifest::parse(raw).expect_err("duplicate pattern should fail");
        assert!(err.to_string().contains("duplicate"));
    }
}
