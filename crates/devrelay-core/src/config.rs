//! Local user configuration schema and file helpers.
//!
//! Local config belongs under `DEVRELAY_HOME` and stores user/device preferences
//! plus a project registry index. Project manifests remain portable and
//! repository-owned; this config is machine-local.

use crate::{DevRelayError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const LOCAL_CONFIG_VERSION: u32 = 1;
pub const WORKSPACE_ID_PREFIX: &str = "w_";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LocalConfig {
    pub version: u32,
    pub fabric_name: String,
    #[serde(default = "default_device_id")]
    pub device_id: String,
    pub device_name: String,
    pub editor: EditorPreference,
    pub resource_profile: ResourceProfile,
    pub anchor_mode: AnchorMode,
    #[serde(default)]
    pub project_registry: ProjectRegistryIndex,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EditorPreference {
    pub command: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceProfile {
    Eco,
    Balanced,
    Performance,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AnchorMode {
    LocalOnly,
    UserSelected,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AgentRole {
    LocalOnly,
    Anchor,
}

impl AgentRole {
    pub fn from_anchor_mode(anchor_mode: AnchorMode) -> Self {
        match anchor_mode {
            AnchorMode::LocalOnly => Self::LocalOnly,
            AnchorMode::UserSelected => Self::Anchor,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::LocalOnly => "local-only",
            Self::Anchor => "anchor",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectRegistryIndex {
    #[serde(default)]
    pub projects: BTreeMap<String, ProjectRegistryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectRegistryEntry {
    pub project_id: String,
    pub display_name: String,
    pub local_path: PathBuf,
    #[serde(default)]
    pub workspaces: BTreeMap<String, WorkspaceRegistryEntry>,
    #[serde(default)]
    pub manifest_path: Option<PathBuf>,
    #[serde(default)]
    pub remote_url_fingerprint: Option<String>,
    #[serde(default)]
    pub root_commit_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRegistryEntry {
    pub workspace_id: String,
    pub project_id: String,
    pub device_id: String,
    pub local_path: PathBuf,
    pub platform_profile: String,
    pub state: WorkspaceState,
    #[serde(default)]
    pub last_seen_head: Option<String>,
    #[serde(default)]
    pub last_checkpoint_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceState {
    Active,
    Inactive,
    Stale,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RedactedLocalConfig {
    pub version: u32,
    pub fabric_name: String,
    pub device_id: String,
    pub device_name: String,
    pub editor: EditorPreference,
    pub resource_profile: ResourceProfile,
    pub anchor_mode: AnchorMode,
    pub project_count: usize,
    pub projects: BTreeMap<String, RedactedProjectRegistryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RedactedProjectRegistryEntry {
    pub project_id: String,
    pub display_name: String,
    pub local_path: String,
    pub workspace_count: usize,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            version: LOCAL_CONFIG_VERSION,
            fabric_name: "Personal Fabric".to_string(),
            device_id: default_device_id(),
            device_name: "this-device".to_string(),
            editor: EditorPreference {
                command: "system".to_string(),
            },
            resource_profile: ResourceProfile::Balanced,
            anchor_mode: AnchorMode::LocalOnly,
            project_registry: ProjectRegistryIndex::default(),
        }
    }
}

impl LocalConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let raw = fs::read_to_string(path)?;
        Self::parse(&raw)
    }

    pub fn parse(raw: &str) -> Result<Self> {
        let migrated = migrate_local_config(raw)?;
        let mut config: Self = toml::from_str(&migrated)?;
        config.backfill_legacy_workspaces();
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        self.validate()?;
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.to_toml_string()?)?;
        Ok(())
    }

    pub fn to_toml_string(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != LOCAL_CONFIG_VERSION {
            return Err(DevRelayError::Config(format!(
                "unsupported config version {}, expected {}",
                self.version, LOCAL_CONFIG_VERSION
            )));
        }
        validate_non_empty("fabric_name", &self.fabric_name)?;
        validate_non_empty("device_id", &self.device_id)?;
        validate_non_empty("device_name", &self.device_name)?;
        validate_non_empty("editor.command", &self.editor.command)?;

        for (key, project) in &self.project_registry.projects {
            validate_non_empty("project_registry.projects key", key)?;
            validate_non_empty("project_registry.projects.project_id", &project.project_id)?;
            validate_non_empty(
                "project_registry.projects.display_name",
                &project.display_name,
            )?;
            if key != &project.project_id {
                return Err(DevRelayError::Config(format!(
                    "project registry key {key} must match project_id {}",
                    project.project_id
                )));
            }
            if project.local_path.as_os_str().is_empty() {
                return Err(DevRelayError::Config(
                    "project_registry.projects.local_path must not be empty".to_string(),
                ));
            }
            for (workspace_key, workspace) in &project.workspaces {
                validate_non_empty("project_registry.projects.workspaces key", workspace_key)?;
                validate_non_empty(
                    "project_registry.projects.workspaces.workspace_id",
                    &workspace.workspace_id,
                )?;
                if !workspace.workspace_id.starts_with(WORKSPACE_ID_PREFIX) {
                    return Err(DevRelayError::Config(format!(
                        "workspace_id {} must start with {}",
                        workspace.workspace_id, WORKSPACE_ID_PREFIX
                    )));
                }
                if workspace_key != &workspace.workspace_id {
                    return Err(DevRelayError::Config(format!(
                        "workspace registry key {workspace_key} must match workspace_id {}",
                        workspace.workspace_id
                    )));
                }
                if workspace.project_id != project.project_id {
                    return Err(DevRelayError::Config(format!(
                        "workspace {} project_id {} must match project_id {}",
                        workspace.workspace_id, workspace.project_id, project.project_id
                    )));
                }
                validate_non_empty(
                    "project_registry.projects.workspaces.device_id",
                    &workspace.device_id,
                )?;
                validate_non_empty(
                    "project_registry.projects.workspaces.platform_profile",
                    &workspace.platform_profile,
                )?;
                if workspace.local_path.as_os_str().is_empty() {
                    return Err(DevRelayError::Config(
                        "project_registry.projects.workspaces.local_path must not be empty"
                            .to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn redacted_for_diagnostics(&self) -> RedactedLocalConfig {
        RedactedLocalConfig {
            version: self.version,
            fabric_name: self.fabric_name.clone(),
            device_id: self.device_id.clone(),
            device_name: self.device_name.clone(),
            editor: self.editor.clone(),
            resource_profile: self.resource_profile,
            anchor_mode: self.anchor_mode,
            project_count: self.project_registry.projects.len(),
            projects: self
                .project_registry
                .projects
                .iter()
                .map(|(key, project)| {
                    (
                        key.clone(),
                        RedactedProjectRegistryEntry {
                            project_id: project.project_id.clone(),
                            display_name: project.display_name.clone(),
                            local_path: "<redacted>".to_string(),
                            workspace_count: project.workspaces.len(),
                        },
                    )
                })
                .collect(),
        }
    }

    fn backfill_legacy_workspaces(&mut self) {
        let device_id = self.device_id.clone();
        for project in self.project_registry.projects.values_mut() {
            if project.workspaces.is_empty() && !project.local_path.as_os_str().is_empty() {
                let workspace_id =
                    workspace_id_for(&project.project_id, &device_id, &project.local_path);
                project.workspaces.insert(
                    workspace_id.clone(),
                    WorkspaceRegistryEntry {
                        workspace_id,
                        project_id: project.project_id.clone(),
                        device_id: device_id.clone(),
                        local_path: project.local_path.clone(),
                        platform_profile: "unknown".to_string(),
                        state: WorkspaceState::Active,
                        last_seen_head: None,
                        last_checkpoint_id: None,
                    },
                );
            }
        }
    }
}

impl ProjectRegistryIndex {
    pub fn workspace_by_path(
        &self,
        local_path: &Path,
    ) -> Option<(&ProjectRegistryEntry, &WorkspaceRegistryEntry)> {
        self.projects.values().find_map(|project| {
            project
                .workspaces
                .values()
                .find(|workspace| workspace.local_path == local_path)
                .map(|workspace| (project, workspace))
        })
    }

    pub fn workspace_by_project_device_path(
        &self,
        project_id: &str,
        device_id: &str,
        local_path: &Path,
    ) -> Option<&WorkspaceRegistryEntry> {
        self.projects.get(project_id).and_then(|project| {
            project.workspaces.values().find(|workspace| {
                workspace.device_id == device_id && workspace.local_path == local_path
            })
        })
    }

    pub fn workspace_by_id(
        &self,
        workspace_id: &str,
    ) -> Option<(&ProjectRegistryEntry, &WorkspaceRegistryEntry)> {
        self.projects.values().find_map(|project| {
            project
                .workspaces
                .get(workspace_id)
                .map(|workspace| (project, workspace))
        })
    }
}

pub fn workspace_id_for(project_id: &str, device_id: &str, local_path: &Path) -> String {
    let digest = blake3::hash(
        format!(
            "{project_id}\0{device_id}\0{}",
            local_path.to_string_lossy()
        )
        .as_bytes(),
    );
    format!("{WORKSPACE_ID_PREFIX}{}", &digest.to_hex()[..16])
}

pub fn migrate_local_config(raw: &str) -> Result<String> {
    let value: toml::Value = toml::from_str(raw)?;
    let version = value
        .get("version")
        .and_then(toml::Value::as_integer)
        .ok_or_else(|| DevRelayError::Config("config version is required".to_string()))?;
    if version == i64::from(LOCAL_CONFIG_VERSION) {
        return Ok(raw.to_string());
    }
    Err(DevRelayError::Config(format!(
        "no migration path for config version {version}"
    )))
}

fn validate_non_empty(field: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(DevRelayError::Config(format!("{field} must not be empty")));
    }
    Ok(())
}

fn default_device_id() -> String {
    "local-device".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid_and_serializes() {
        let config = LocalConfig::default();
        config.validate().unwrap();

        let encoded = config.to_toml_string().unwrap();
        let decoded = LocalConfig::parse(&encoded).unwrap();

        assert_eq!(decoded.version, LOCAL_CONFIG_VERSION);
        assert_eq!(decoded.fabric_name, "Personal Fabric");
        assert_eq!(decoded.device_id, "local-device");
        assert_eq!(decoded.device_name, "this-device");
        assert_eq!(decoded.editor.command, "system");
        assert_eq!(decoded.resource_profile, ResourceProfile::Balanced);
        assert_eq!(decoded.anchor_mode, AnchorMode::LocalOnly);
    }

    #[test]
    fn agent_role_is_derived_from_anchor_mode() {
        assert_eq!(
            AgentRole::from_anchor_mode(AnchorMode::LocalOnly),
            AgentRole::LocalOnly
        );
        assert_eq!(
            AgentRole::from_anchor_mode(AnchorMode::UserSelected),
            AgentRole::Anchor
        );
        assert_eq!(AgentRole::Anchor.label(), "anchor");
    }

    #[test]
    fn saves_and_loads_config_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let config = LocalConfig::default();

        config.save(&path).unwrap();
        let loaded = LocalConfig::load(&path).unwrap();

        assert_eq!(loaded, config);
    }

    #[test]
    fn validates_config_fields() {
        let mut config = LocalConfig::default();
        config.device_name.clear();

        let err = config.validate().unwrap_err();
        assert!(matches!(err, DevRelayError::Config(_)));
        assert!(err.to_string().contains("device_name"));
    }

    #[test]
    fn migration_placeholder_rejects_unknown_versions() {
        let raw = r#"
version = 99
fabric_name = "Personal Fabric"
device_name = "this-device"

[editor]
command = "system"
resource_profile = "balanced"
anchor_mode = "local-only"
"#;

        let err = migrate_local_config(raw).unwrap_err();
        assert!(err.to_string().contains("no migration path"));
    }

    #[test]
    fn redacts_project_paths_for_diagnostics() {
        let mut config = LocalConfig::default();
        config.project_registry.projects.insert(
            "project123".to_string(),
            ProjectRegistryEntry {
                project_id: "project123".to_string(),
                display_name: "Demo".to_string(),
                local_path: PathBuf::from("/Users/dev/private/project"),
                workspaces: BTreeMap::new(),
                manifest_path: Some(PathBuf::from("/Users/dev/private/project/devrelay.toml")),
                remote_url_fingerprint: Some("remote123".to_string()),
                root_commit_fingerprint: Some("root123".to_string()),
            },
        );

        let redacted = config.redacted_for_diagnostics();

        assert_eq!(redacted.project_count, 1);
        assert_eq!(
            redacted.projects["project123"].local_path,
            "<redacted>".to_string()
        );
    }

    #[test]
    fn workspace_ids_have_stable_prefix_and_hash() {
        let first = workspace_id_for("project123", "device123", Path::new("/repo"));
        let second = workspace_id_for("project123", "device123", Path::new("/repo"));
        let different_path = workspace_id_for("project123", "device123", Path::new("/other"));

        assert!(first.starts_with(WORKSPACE_ID_PREFIX));
        assert_eq!(first.len(), WORKSPACE_ID_PREFIX.len() + 16);
        assert_eq!(first, second);
        assert_ne!(first, different_path);
    }

    #[test]
    fn looks_up_workspaces_by_path_and_project_device_path() {
        let mut config = LocalConfig::default();
        let local_path = PathBuf::from("/Users/dev/project");
        let workspace_id = workspace_id_for("project123", &config.device_id, &local_path);
        let workspace = WorkspaceRegistryEntry {
            workspace_id: workspace_id.clone(),
            project_id: "project123".to_string(),
            device_id: config.device_id.clone(),
            local_path: local_path.clone(),
            platform_profile: "macos-aarch64".to_string(),
            state: WorkspaceState::Active,
            last_seen_head: Some("abc123".to_string()),
            last_checkpoint_id: None,
        };
        config.project_registry.projects.insert(
            "project123".to_string(),
            ProjectRegistryEntry {
                project_id: "project123".to_string(),
                display_name: "Demo".to_string(),
                local_path: local_path.clone(),
                workspaces: BTreeMap::from([(workspace_id.clone(), workspace)]),
                manifest_path: None,
                remote_url_fingerprint: None,
                root_commit_fingerprint: None,
            },
        );

        let (_, by_path) = config
            .project_registry
            .workspace_by_path(&local_path)
            .unwrap();
        assert_eq!(by_path.workspace_id, workspace_id);

        let by_project = config
            .project_registry
            .workspace_by_project_device_path("project123", &config.device_id, &local_path)
            .unwrap();
        assert_eq!(by_project.local_path, local_path);
    }

    #[test]
    fn backfills_workspace_map_for_legacy_project_entries() {
        let legacy = r#"
version = 1
fabric_name = "Personal Fabric"
device_name = "this-device"
resource_profile = "balanced"
anchor_mode = "local-only"

[editor]
command = "system"

[project_registry.projects.project123]
project_id = "project123"
display_name = "Demo"
local_path = "/Users/dev/project"
"#;

        let config = LocalConfig::parse(legacy).unwrap();
        let project = &config.project_registry.projects["project123"];
        assert_eq!(project.workspaces.len(), 1);
        let workspace = project.workspaces.values().next().unwrap();
        assert_eq!(workspace.device_id, "local-device");
        assert_eq!(workspace.platform_profile, "unknown");
        assert_eq!(workspace.last_checkpoint_id, None);
    }

    #[test]
    fn validates_workspace_id_format() {
        let mut config = LocalConfig::default();
        config.project_registry.projects.insert(
            "project123".to_string(),
            ProjectRegistryEntry {
                project_id: "project123".to_string(),
                display_name: "Demo".to_string(),
                local_path: PathBuf::from("/Users/dev/project"),
                workspaces: BTreeMap::from([(
                    "bad123".to_string(),
                    WorkspaceRegistryEntry {
                        workspace_id: "bad123".to_string(),
                        project_id: "project123".to_string(),
                        device_id: config.device_id.clone(),
                        local_path: PathBuf::from("/Users/dev/project"),
                        platform_profile: "macos-aarch64".to_string(),
                        state: WorkspaceState::Active,
                        last_seen_head: None,
                        last_checkpoint_id: None,
                    },
                )]),
                manifest_path: None,
                remote_url_fingerprint: None,
                root_commit_fingerprint: None,
            },
        );

        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("must start with"));
    }
}
