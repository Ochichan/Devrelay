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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LocalConfig {
    pub version: u32,
    pub fabric_name: String,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RedactedLocalConfig {
    pub version: u32,
    pub fabric_name: String,
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
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            version: LOCAL_CONFIG_VERSION,
            fabric_name: "Personal Fabric".to_string(),
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
        let config: Self = toml::from_str(&migrated)?;
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
        }
        Ok(())
    }

    pub fn redacted_for_diagnostics(&self) -> RedactedLocalConfig {
        RedactedLocalConfig {
            version: self.version,
            fabric_name: self.fabric_name.clone(),
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
                        },
                    )
                })
                .collect(),
        }
    }
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
        assert_eq!(decoded.device_name, "this-device");
        assert_eq!(decoded.editor.command, "system");
        assert_eq!(decoded.resource_profile, ResourceProfile::Balanced);
        assert_eq!(decoded.anchor_mode, AnchorMode::LocalOnly);
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
            },
        );

        let redacted = config.redacted_for_diagnostics();

        assert_eq!(redacted.project_count, 1);
        assert_eq!(
            redacted.projects["project123"].local_path,
            "<redacted>".to_string()
        );
    }
}
