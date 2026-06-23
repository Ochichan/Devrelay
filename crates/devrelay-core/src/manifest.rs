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
    #[serde(default)]
    pub secret_scanner: SecretScannerConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatternConfig {
    #[serde(default)]
    pub patterns: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretScannerConfig {
    #[serde(default)]
    pub filename_patterns: Vec<String>,
    #[serde(default)]
    pub content_markers: Vec<String>,
    #[serde(default)]
    pub token_prefixes: Vec<String>,
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
        validate_optional_string_array(
            "workspace.secret_scanner.filename_patterns",
            &self.workspace.secret_scanner.filename_patterns,
        )?;
        validate_optional_string_array(
            "workspace.secret_scanner.content_markers",
            &self.workspace.secret_scanner.content_markers,
        )?;
        validate_optional_string_array(
            "workspace.secret_scanner.token_prefixes",
            &self.workspace.secret_scanner.token_prefixes,
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

    /// Returns a canonical hash of manifest fields that can execute commands.
    ///
    /// Non-executable manifest edits, such as project display name or path
    /// policy changes, do not affect this hash. Command edits, task/profile
    /// remapping, and healthcheck edits do affect it.
    pub fn execution_trust_hash(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        update_hash_field(&mut hasher, "devrelay.execution-trust.v1");

        if let Some(environment) = &self.environment {
            for (name, profile) in &environment.profiles {
                update_hash_field(&mut hasher, "environment.profile");
                update_hash_field(&mut hasher, name);
                update_hash_command(&mut hasher, "command", &profile.command);
                if let Some(healthcheck) = &profile.healthcheck {
                    update_hash_command(&mut hasher, "healthcheck", healthcheck);
                }
            }
        }

        for (name, task) in &self.tasks {
            update_hash_field(&mut hasher, "task");
            update_hash_field(&mut hasher, name);
            update_hash_field(&mut hasher, &task.profile);
            update_hash_command(&mut hasher, "command", &task.command);
        }

        hasher.finalize().to_hex().to_string()
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
    if values.is_empty() || values.iter().any(|value| value.is_empty()) {
        return Err(DevRelayError::Manifest(format!(
            "{field} must contain at least one non-empty value"
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

fn validate_optional_string_array(field: &str, values: &[String]) -> Result<()> {
    if values.iter().any(|value| value.is_empty()) {
        return Err(DevRelayError::Manifest(format!(
            "{field} must not contain empty values"
        )));
    }
    validate_unique(field, values)
}

fn update_hash_command(hasher: &mut blake3::Hasher, field: &str, values: &[String]) {
    update_hash_field(hasher, field);
    for value in values {
        update_hash_field(hasher, value);
    }
}

fn update_hash_field(hasher: &mut blake3::Hasher, value: &str) {
    hasher.update(value.as_bytes());
    hasher.update(&[0]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_manifest() -> String {
        r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#
        .to_string()
    }

    fn manifest_with(extra: &str) -> String {
        format!("{}\n{extra}", minimal_manifest())
    }

    #[test]
    fn golden_bundled_manifest_parses_and_validates() {
        let raw = include_str!("../../../devrelay_spec_bundle/devrelay.toml");
        let manifest = Manifest::parse(raw).expect("bundled manifest should parse");
        assert_eq!(manifest.schema, 1);
        assert_eq!(manifest.name, "payments-api");
        assert_eq!(manifest.workspace.untracked, UntrackedPolicy::Safe);
        assert!(manifest.tasks.contains_key("test"));
    }

    #[test]
    fn rejects_invalid_manifest_inputs() {
        let cases = vec![
            (
                "missing schema",
                r#"
project_id = "12345678"
name = "bad"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#
                .to_string(),
            ),
            (
                "unsupported schema",
                r#"
schema = 99
project_id = "12345678"
name = "bad"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#
                .to_string(),
            ),
            (
                "short project_id",
                r#"
schema = 1
project_id = "short"
name = "bad"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#
                .to_string(),
            ),
            (
                "empty project name",
                r#"
schema = 1
project_id = "12345678"
name = ""

[workspace]
untracked = "safe"
portable_paths = "strict"
"#
                .to_string(),
            ),
            (
                "overlong project name",
                format!(
                    r#"
schema = 1
project_id = "12345678"
name = "{}"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#,
                    "x".repeat(129)
                ),
            ),
            (
                "invalid untracked policy",
                r#"
schema = 1
project_id = "12345678"
name = "bad"

[workspace]
untracked = "maybe"
portable_paths = "strict"
"#
                .to_string(),
            ),
            (
                "invalid portable paths policy",
                r#"
schema = 1
project_id = "12345678"
name = "bad"

[workspace]
untracked = "safe"
portable_paths = "maybe"
"#
                .to_string(),
            ),
            (
                "invalid large file threshold",
                r#"
schema = 1
project_id = "12345678"
name = "bad"

[workspace]
untracked = "safe"
portable_paths = "strict"
large_file_threshold_mib = 0
"#
                .to_string(),
            ),
            (
                "duplicate exclude pattern",
                r#"
schema = 1
project_id = "12345678"
name = "bad"

[workspace]
untracked = "safe"
portable_paths = "strict"

[workspace.exclude]
patterns = ["target/**", "target/**"]
"#
                .to_string(),
            ),
            (
                "duplicate include pattern",
                r#"
schema = 1
project_id = "12345678"
name = "bad"

[workspace]
untracked = "safe"
portable_paths = "strict"

[workspace.include]
patterns = ["notes/**", "notes/**"]
"#
                .to_string(),
            ),
            (
                "duplicate secret scanner filename pattern",
                r#"
schema = 1
project_id = "12345678"
name = "bad"

[workspace]
untracked = "safe"
portable_paths = "strict"

[workspace.secret_scanner]
filename_patterns = ["*.secret", "*.secret"]
"#
                .to_string(),
            ),
            (
                "empty secret scanner token prefix",
                r#"
schema = 1
project_id = "12345678"
name = "bad"

[workspace]
untracked = "safe"
portable_paths = "strict"

[workspace.secret_scanner]
token_prefixes = [""]
"#
                .to_string(),
            ),
            (
                "empty environment command",
                manifest_with(
                    r#"
[environment.profiles.dev]
kind = "script"
targets = ["local"]
command = []
"#,
                ),
            ),
            (
                "empty environment target",
                manifest_with(
                    r#"
[environment.profiles.dev]
kind = "script"
targets = [""]
command = ["cargo", "test"]
"#,
                ),
            ),
            (
                "missing environment target",
                manifest_with(
                    r#"
[environment.profiles.dev]
kind = "script"
targets = []
command = ["cargo", "test"]
"#,
                ),
            ),
            (
                "empty task command",
                manifest_with(
                    r#"
[tasks.test]
profile = "dev"
command = []
"#,
                ),
            ),
            (
                "empty task profile",
                manifest_with(
                    r#"
[tasks.test]
profile = ""
command = ["cargo", "test"]
"#,
                ),
            ),
            (
                "empty secret target",
                manifest_with(
                    r#"
[secrets.local_env]
target = ""
"#,
                ),
            ),
        ];

        for (name, raw) in cases {
            assert!(
                Manifest::parse(&raw).is_err(),
                "case should fail manifest validation: {name}"
            );
        }
    }

    #[test]
    fn manifest_structs_round_trip_through_serde() {
        let raw = include_str!("../../../devrelay_spec_bundle/devrelay.toml");
        let manifest = Manifest::parse(raw).expect("bundled manifest should parse");
        let encoded = toml::to_string(&manifest).expect("manifest should serialize");
        let reparsed = Manifest::parse(&encoded).expect("serialized manifest should parse");

        assert_eq!(manifest.schema, reparsed.schema);
        assert_eq!(manifest.project_id, reparsed.project_id);
        assert_eq!(manifest.name, reparsed.name);
        assert_eq!(manifest.workspace.untracked, reparsed.workspace.untracked);
        assert_eq!(
            manifest.workspace.exclude.patterns,
            reparsed.workspace.exclude.patterns
        );
        assert_eq!(
            manifest.workspace.secret_scanner.filename_patterns,
            reparsed.workspace.secret_scanner.filename_patterns
        );
        assert_eq!(
            manifest.tasks.keys().collect::<Vec<_>>(),
            reparsed.tasks.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn parses_workspace_secret_scanner_config() {
        let manifest = Manifest::parse(&manifest_with(
            r#"
[workspace.secret_scanner]
filename_patterns = ["*.local-secret"]
content_markers = ["BEGIN CUSTOM SECRET"]
token_prefixes = ["customtok_"]
"#,
        ))
        .unwrap();

        assert_eq!(
            manifest.workspace.secret_scanner.filename_patterns,
            vec!["*.local-secret"]
        );
        assert_eq!(
            manifest.workspace.secret_scanner.content_markers,
            vec!["BEGIN CUSTOM SECRET"]
        );
        assert_eq!(
            manifest.workspace.secret_scanner.token_prefixes,
            vec!["customtok_"]
        );
    }

    #[test]
    fn execution_trust_hash_ignores_non_executable_manifest_edits() {
        let raw = manifest_with(
            r#"
[environment.profiles.dev]
kind = "script"
targets = ["local"]
command = ["cargo", "test"]
healthcheck = ["cargo", "check"]

[tasks.test]
profile = "dev"
command = ["cargo", "test"]
"#,
        );
        let original = Manifest::parse(&raw).unwrap();
        let renamed = Manifest::parse(&raw.replace("name = \"demo\"", "name = \"demo renamed\""))
            .expect("renamed manifest should parse");

        assert_eq!(
            original.execution_trust_hash(),
            renamed.execution_trust_hash()
        );
    }

    #[test]
    fn execution_trust_hash_changes_for_command_edits() {
        let raw = manifest_with(
            r#"
[environment.profiles.dev]
kind = "script"
targets = ["local"]
command = ["cargo", "test"]

[tasks.test]
profile = "dev"
command = ["cargo", "test"]
"#,
        );
        let original = Manifest::parse(&raw).unwrap();
        let changed =
            Manifest::parse(&raw.replace("[\"cargo\", \"test\"]", "[\"cargo\", \"clippy\"]"))
                .expect("changed command manifest should parse");

        assert_ne!(
            original.execution_trust_hash(),
            changed.execution_trust_hash()
        );
    }
}
