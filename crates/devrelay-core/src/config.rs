//! Local user configuration schema and file helpers.
//!
//! Local config belongs under `DEVRELAY_HOME` and stores user/device preferences
//! plus a project registry index. Project manifests remain portable and
//! repository-owned; this config is machine-local.

use crate::{
    DevRelayError, Result, current_platform_architecture, current_platform_capabilities_json,
    current_platform_device_scope_key, current_platform_key,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub const LOCAL_CONFIG_VERSION: u32 = 1;
pub const DEVICE_ID_PREFIX: &str = "d_";
pub const WORKSPACE_ID_PREFIX: &str = "w_";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LocalConfig {
    pub version: u32,
    pub fabric_name: String,
    #[serde(default = "default_device_id")]
    pub device_id: String,
    pub device_name: String,
    #[serde(default = "default_platform_key")]
    pub platform_key: String,
    #[serde(default = "default_architecture")]
    pub architecture: String,
    #[serde(default = "default_capabilities_json")]
    pub capabilities_json: String,
    #[serde(default)]
    pub paired_at_unix_seconds: Option<u64>,
    #[serde(default)]
    pub last_seen_unix_seconds: u64,
    pub editor: EditorPreference,
    pub resource_profile: ResourceProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_policy_limits: Option<ResourcePolicyLimits>,
    pub anchor_mode: AnchorMode,
    #[serde(default = "default_mdns_enabled")]
    pub mdns_enabled: bool,
    #[serde(default)]
    pub manual_discovery_address: Option<String>,
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
    Adaptive,
    Instant,
    Eco,
    Custom,
    Balanced,
    Performance,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ResourcePowerSource {
    Ac,
    Battery,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ForegroundLoad {
    Idle,
    Busy,
    Unknown,
}

impl ForegroundLoad {
    pub fn from_load_average(load_average_1m: f32, parallelism: usize) -> Self {
        if !load_average_1m.is_finite() {
            return Self::Unknown;
        }
        let busy_threshold = parallelism.max(1) as f32 * 0.75;
        if load_average_1m >= busy_threshold {
            Self::Busy
        } else {
            Self::Idle
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourcePolicyContext {
    pub parallelism: usize,
    pub power_source: ResourcePowerSource,
    pub low_power_mode: bool,
    pub foreground_load: ForegroundLoad,
}

impl ResourcePolicyContext {
    pub fn for_parallelism(parallelism: usize) -> Self {
        Self {
            parallelism: parallelism.max(1),
            power_source: ResourcePowerSource::Unknown,
            low_power_mode: false,
            foreground_load: ForegroundLoad::Unknown,
        }
    }

    pub fn detect_current() -> Self {
        detect_resource_policy_context()
    }
}

impl Default for ResourcePolicyContext {
    fn default() -> Self {
        let parallelism = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        Self::for_parallelism(parallelism)
    }
}

impl ResourceProfile {
    pub fn canonical(self) -> Self {
        match self {
            Self::Balanced => Self::Adaptive,
            Self::Performance => Self::Instant,
            other => other,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourcePolicyLimits {
    pub cpu_slot_limit: usize,
    pub hashing_concurrency_limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_bandwidth_kib_per_second: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourcePolicy {
    pub profile: ResourceProfile,
    pub limits: ResourcePolicyLimits,
}

impl ResourcePolicy {
    pub fn for_profile(profile: ResourceProfile) -> Self {
        let parallelism = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        Self::for_profile_with_parallelism(profile, parallelism)
    }

    pub fn for_profile_with_parallelism(profile: ResourceProfile, parallelism: usize) -> Self {
        let parallelism = parallelism.max(1);
        let adaptive_slots = (parallelism / 2).max(1);
        match profile.canonical() {
            ResourceProfile::Adaptive => Self {
                profile: ResourceProfile::Adaptive,
                limits: ResourcePolicyLimits {
                    cpu_slot_limit: adaptive_slots,
                    hashing_concurrency_limit: adaptive_slots,
                    network_bandwidth_kib_per_second: None,
                },
            },
            ResourceProfile::Instant => Self {
                profile: ResourceProfile::Instant,
                limits: ResourcePolicyLimits {
                    cpu_slot_limit: parallelism,
                    hashing_concurrency_limit: parallelism,
                    network_bandwidth_kib_per_second: None,
                },
            },
            ResourceProfile::Eco => Self {
                profile: ResourceProfile::Eco,
                limits: ResourcePolicyLimits {
                    cpu_slot_limit: 1,
                    hashing_concurrency_limit: 1,
                    network_bandwidth_kib_per_second: Some(1024),
                },
            },
            ResourceProfile::Custom => Self {
                profile: ResourceProfile::Custom,
                limits: ResourcePolicyLimits {
                    cpu_slot_limit: adaptive_slots,
                    hashing_concurrency_limit: adaptive_slots,
                    network_bandwidth_kib_per_second: None,
                },
            },
            ResourceProfile::Balanced | ResourceProfile::Performance => unreachable!(),
        }
    }

    pub fn for_profile_with_context(
        profile: ResourceProfile,
        context: ResourcePolicyContext,
    ) -> Self {
        Self::for_profile_with_parallelism(profile, context.parallelism)
            .adapted_for_context(context)
    }

    pub fn custom(limits: ResourcePolicyLimits) -> Result<Self> {
        limits.validate()?;
        Ok(Self {
            profile: ResourceProfile::Custom,
            limits,
        })
    }

    pub fn adapted_for_context(mut self, context: ResourcePolicyContext) -> Self {
        if context.power_source == ResourcePowerSource::Battery {
            let battery_cap = (context.parallelism / 4).max(1);
            self.limits.cap_cpu_and_hash(battery_cap, battery_cap);
            self.limits.cap_network_bandwidth(2048);
        }

        if context.foreground_load == ForegroundLoad::Busy {
            let foreground_cpu_cap = (self.limits.cpu_slot_limit / 2).max(1);
            let foreground_hash_cap = (self.limits.hashing_concurrency_limit / 2).max(1);
            self.limits
                .cap_cpu_and_hash(foreground_cpu_cap, foreground_hash_cap);
            self.limits.cap_network_bandwidth(4096);
        }

        if context.low_power_mode {
            self.limits.cap_cpu_and_hash(1, 1);
            self.limits.cap_network_bandwidth(1024);
        }

        self
    }
}

impl ResourcePolicyLimits {
    pub fn validate(self) -> Result<()> {
        if self.cpu_slot_limit == 0 {
            return Err(DevRelayError::Config(
                "resource policy cpu_slot_limit must be greater than zero".to_string(),
            ));
        }
        if self.hashing_concurrency_limit == 0 {
            return Err(DevRelayError::Config(
                "resource policy hashing_concurrency_limit must be greater than zero".to_string(),
            ));
        }
        if matches!(self.network_bandwidth_kib_per_second, Some(0)) {
            return Err(DevRelayError::Config(
                "resource policy network_bandwidth_kib_per_second must be greater than zero"
                    .to_string(),
            ));
        }
        Ok(())
    }

    fn cap_cpu_and_hash(&mut self, cpu_slot_limit: usize, hashing_concurrency_limit: usize) {
        self.cpu_slot_limit = self.cpu_slot_limit.min(cpu_slot_limit.max(1)).max(1);
        self.hashing_concurrency_limit = self
            .hashing_concurrency_limit
            .min(hashing_concurrency_limit.max(1))
            .max(1);
    }

    fn cap_network_bandwidth(&mut self, cap_kib_per_second: u64) {
        self.network_bandwidth_kib_per_second = Some(
            self.network_bandwidth_kib_per_second
                .map(|current| current.min(cap_kib_per_second))
                .unwrap_or(cap_kib_per_second),
        );
    }
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
    pub platform_key: String,
    pub architecture: String,
    pub capabilities_json: String,
    pub paired_at_unix_seconds: Option<u64>,
    pub last_seen_unix_seconds: u64,
    pub editor: EditorPreference,
    pub resource_profile: ResourceProfile,
    pub anchor_mode: AnchorMode,
    pub mdns_enabled: bool,
    pub manual_discovery_address_configured: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub device_id: String,
    pub display_name: String,
    pub platform_key: String,
    pub architecture: String,
    pub capabilities_json: String,
    pub paired_at_unix_seconds: Option<u64>,
    pub last_seen_unix_seconds: u64,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            version: LOCAL_CONFIG_VERSION,
            fabric_name: "Personal Fabric".to_string(),
            device_id: default_device_id(),
            device_name: "this-device".to_string(),
            platform_key: default_platform_key(),
            architecture: default_architecture(),
            capabilities_json: default_capabilities_json(),
            paired_at_unix_seconds: None,
            last_seen_unix_seconds: 0,
            editor: EditorPreference {
                command: "system".to_string(),
            },
            resource_profile: ResourceProfile::Balanced,
            resource_policy_limits: None,
            anchor_mode: AnchorMode::LocalOnly,
            mdns_enabled: true,
            manual_discovery_address: None,
            project_registry: ProjectRegistryIndex::default(),
        }
    }
}

impl LocalConfig {
    pub fn new_for_local_device() -> Self {
        Self {
            device_id: generate_device_id(),
            device_name: default_device_display_name(),
            platform_key: default_platform_key(),
            architecture: default_architecture(),
            capabilities_json: default_capabilities_json(),
            paired_at_unix_seconds: None,
            last_seen_unix_seconds: unix_now_seconds(),
            ..Self::default()
        }
    }

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
        validate_non_empty("platform_key", &self.platform_key)?;
        validate_non_empty("architecture", &self.architecture)?;
        validate_capabilities_json(&self.capabilities_json)?;
        validate_non_empty("editor.command", &self.editor.command)?;
        if let Some(limits) = self.resource_policy_limits {
            limits.validate()?;
        }
        if let Some(address) = &self.manual_discovery_address
            && address.trim().is_empty()
        {
            return Err(DevRelayError::Config(
                "manual_discovery_address must not be empty".to_string(),
            ));
        }

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
            platform_key: self.platform_key.clone(),
            architecture: self.architecture.clone(),
            capabilities_json: self.capabilities_json.clone(),
            paired_at_unix_seconds: self.paired_at_unix_seconds,
            last_seen_unix_seconds: self.last_seen_unix_seconds,
            editor: self.editor.clone(),
            resource_profile: self.resource_profile,
            anchor_mode: self.anchor_mode,
            mdns_enabled: self.mdns_enabled,
            manual_discovery_address_configured: self.manual_discovery_address.is_some(),
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

    pub fn device_identity(&self) -> DeviceIdentity {
        DeviceIdentity {
            device_id: self.device_id.clone(),
            display_name: self.device_name.clone(),
            platform_key: self.platform_key.clone(),
            architecture: self.architecture.clone(),
            capabilities_json: self.capabilities_json.clone(),
            paired_at_unix_seconds: self.paired_at_unix_seconds,
            last_seen_unix_seconds: self.last_seen_unix_seconds,
        }
    }

    pub fn resource_policy(&self) -> ResourcePolicy {
        self.resource_policy_for_context(ResourcePolicyContext::default())
    }

    pub fn resource_policy_for_context(&self, context: ResourcePolicyContext) -> ResourcePolicy {
        let base = if self.resource_profile.canonical() == ResourceProfile::Custom {
            self.resource_policy_limits
                .and_then(|limits| ResourcePolicy::custom(limits).ok())
                .unwrap_or_else(|| {
                    ResourcePolicy::for_profile_with_parallelism(
                        ResourceProfile::Custom,
                        context.parallelism,
                    )
                })
        } else {
            ResourcePolicy::for_profile_with_parallelism(self.resource_profile, context.parallelism)
        };
        base.adapted_for_context(context)
    }

    pub fn mark_device_seen_now(&mut self) {
        self.last_seen_unix_seconds = unix_now_seconds();
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

fn validate_capabilities_json(value: &str) -> Result<()> {
    let parsed: serde_json::Value = serde_json::from_str(value).map_err(|err| {
        DevRelayError::Config(format!("capabilities_json must be valid JSON: {err}"))
    })?;
    if !parsed.is_object() {
        return Err(DevRelayError::Config(
            "capabilities_json must encode a JSON object".to_string(),
        ));
    }
    Ok(())
}

pub fn detect_resource_policy_context() -> ResourcePolicyContext {
    let mut context = ResourcePolicyContext::default();
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = Command::new("pmset").args(["-g", "batt"]).output()
            && output.status.success()
        {
            context.power_source =
                parse_macos_power_source(&String::from_utf8_lossy(&output.stdout));
        }

        if let Ok(output) = Command::new("pmset").arg("-g").output()
            && output.status.success()
            && let Some(low_power_mode) =
                parse_macos_low_power_mode(&String::from_utf8_lossy(&output.stdout))
        {
            context.low_power_mode = low_power_mode;
        }

        if let Ok(output) = Command::new("sysctl").args(["-n", "vm.loadavg"]).output()
            && output.status.success()
            && let Some(load_average_1m) =
                parse_macos_load_average(&String::from_utf8_lossy(&output.stdout))
        {
            context.foreground_load =
                ForegroundLoad::from_load_average(load_average_1m, context.parallelism);
        }
    }
    context
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_power_source(raw: &str) -> ResourcePowerSource {
    if raw.contains("Battery Power") {
        ResourcePowerSource::Battery
    } else if raw.contains("AC Power") {
        ResourcePowerSource::Ac
    } else {
        ResourcePowerSource::Unknown
    }
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_low_power_mode(raw: &str) -> Option<bool> {
    raw.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        (parts.next()? == "lowpowermode").then(|| parts.next().map(|value| value == "1"))?
    })
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_load_average(raw: &str) -> Option<f32> {
    raw.trim()
        .trim_start_matches('{')
        .split_whitespace()
        .next()
        .and_then(|value| value.parse().ok())
}

fn default_device_id() -> String {
    "local-device".to_string()
}

fn default_device_display_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "this-device".to_string())
}

fn default_platform_key() -> String {
    current_platform_key()
}

fn default_architecture() -> String {
    current_platform_architecture()
}

fn default_capabilities_json() -> String {
    current_platform_capabilities_json()
}

fn default_mdns_enabled() -> bool {
    true
}

pub fn generate_device_id() -> String {
    let seed = format!(
        "{}\0{}\0{}\0{}\0{}",
        default_device_display_name(),
        current_platform_device_scope_key(),
        default_architecture(),
        std::process::id(),
        unix_now_nanos()
    );
    let digest = blake3::hash(seed.as_bytes());
    format!("{DEVICE_ID_PREFIX}{}", &digest.to_hex()[..24])
}

fn unix_now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn unix_now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid_and_serializes() {
        let config = LocalConfig::default();
        let platform = crate::detect_platform_identity();
        config.validate().unwrap();

        let encoded = config.to_toml_string().unwrap();
        let decoded = LocalConfig::parse(&encoded).unwrap();

        assert_eq!(decoded.version, LOCAL_CONFIG_VERSION);
        assert_eq!(decoded.fabric_name, "Personal Fabric");
        assert_eq!(decoded.device_id, "local-device");
        assert_eq!(decoded.device_name, "this-device");
        assert_eq!(decoded.platform_key, platform.platform_key);
        assert_eq!(decoded.architecture, platform.architecture);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&decoded.capabilities_json).unwrap(),
            serde_json::to_value(platform.capabilities).unwrap()
        );
        assert_eq!(decoded.paired_at_unix_seconds, None);
        assert_eq!(decoded.last_seen_unix_seconds, 0);
        assert_eq!(decoded.editor.command, "system");
        assert_eq!(decoded.resource_profile, ResourceProfile::Balanced);
        assert_eq!(decoded.anchor_mode, AnchorMode::LocalOnly);
        assert!(decoded.mdns_enabled);
        assert_eq!(decoded.manual_discovery_address, None);
    }

    #[test]
    fn new_local_config_generates_device_identity() {
        let config = LocalConfig::new_for_local_device();
        let platform = crate::detect_platform_identity();
        config.validate().unwrap();

        assert!(config.device_id.starts_with(DEVICE_ID_PREFIX));
        assert_eq!(config.device_id.len(), DEVICE_ID_PREFIX.len() + 24);
        assert!(!config.device_name.is_empty());
        assert_eq!(config.platform_key, platform.platform_key);
        assert_eq!(config.architecture, platform.architecture);
        assert_eq!(config.paired_at_unix_seconds, None);
        assert!(config.last_seen_unix_seconds > 0);

        let identity = config.device_identity();
        assert_eq!(identity.device_id, config.device_id);
        assert_eq!(identity.display_name, config.device_name);
        assert_eq!(identity.capabilities_json, config.capabilities_json);
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
    fn resource_profiles_resolve_to_canonical_limits() {
        assert_eq!(
            ResourcePolicy::for_profile_with_parallelism(ResourceProfile::Adaptive, 8),
            ResourcePolicy {
                profile: ResourceProfile::Adaptive,
                limits: ResourcePolicyLimits {
                    cpu_slot_limit: 4,
                    hashing_concurrency_limit: 4,
                    network_bandwidth_kib_per_second: None,
                },
            }
        );
        assert_eq!(
            ResourcePolicy::for_profile_with_parallelism(ResourceProfile::Instant, 8),
            ResourcePolicy {
                profile: ResourceProfile::Instant,
                limits: ResourcePolicyLimits {
                    cpu_slot_limit: 8,
                    hashing_concurrency_limit: 8,
                    network_bandwidth_kib_per_second: None,
                },
            }
        );
        assert_eq!(
            ResourcePolicy::for_profile_with_parallelism(ResourceProfile::Eco, 8),
            ResourcePolicy {
                profile: ResourceProfile::Eco,
                limits: ResourcePolicyLimits {
                    cpu_slot_limit: 1,
                    hashing_concurrency_limit: 1,
                    network_bandwidth_kib_per_second: Some(1024),
                },
            }
        );
        assert_eq!(
            ResourcePolicy::for_profile_with_parallelism(ResourceProfile::Balanced, 8).profile,
            ResourceProfile::Adaptive
        );
        assert_eq!(
            ResourcePolicy::for_profile_with_parallelism(ResourceProfile::Performance, 8).profile,
            ResourceProfile::Instant
        );
    }

    #[test]
    fn resource_policy_context_reduces_limits_for_power_and_foreground_load() {
        let ac_context = ResourcePolicyContext {
            parallelism: 8,
            power_source: ResourcePowerSource::Ac,
            low_power_mode: false,
            foreground_load: ForegroundLoad::Idle,
        };
        assert_eq!(
            ResourcePolicy::for_profile_with_context(ResourceProfile::Instant, ac_context).limits,
            ResourcePolicyLimits {
                cpu_slot_limit: 8,
                hashing_concurrency_limit: 8,
                network_bandwidth_kib_per_second: None,
            }
        );

        let battery_context = ResourcePolicyContext {
            power_source: ResourcePowerSource::Battery,
            ..ac_context
        };
        assert_eq!(
            ResourcePolicy::for_profile_with_context(ResourceProfile::Instant, battery_context)
                .limits,
            ResourcePolicyLimits {
                cpu_slot_limit: 2,
                hashing_concurrency_limit: 2,
                network_bandwidth_kib_per_second: Some(2048),
            }
        );

        let busy_context = ResourcePolicyContext {
            foreground_load: ForegroundLoad::Busy,
            ..ac_context
        };
        assert_eq!(
            ResourcePolicy::for_profile_with_context(ResourceProfile::Instant, busy_context).limits,
            ResourcePolicyLimits {
                cpu_slot_limit: 4,
                hashing_concurrency_limit: 4,
                network_bandwidth_kib_per_second: Some(4096),
            }
        );

        let low_power_context = ResourcePolicyContext {
            low_power_mode: true,
            ..ac_context
        };
        assert_eq!(
            ResourcePolicy::for_profile_with_context(ResourceProfile::Instant, low_power_context)
                .limits,
            ResourcePolicyLimits {
                cpu_slot_limit: 1,
                hashing_concurrency_limit: 1,
                network_bandwidth_kib_per_second: Some(1024),
            }
        );
    }

    #[test]
    fn custom_resource_policy_validates_limits() {
        let custom = ResourcePolicy::custom(ResourcePolicyLimits {
            cpu_slot_limit: 2,
            hashing_concurrency_limit: 3,
            network_bandwidth_kib_per_second: Some(2048),
        })
        .unwrap();

        assert_eq!(custom.profile, ResourceProfile::Custom);
        assert_eq!(custom.limits.cpu_slot_limit, 2);
        assert_eq!(custom.limits.hashing_concurrency_limit, 3);
        assert_eq!(custom.limits.network_bandwidth_kib_per_second, Some(2048));

        let err = ResourcePolicy::custom(ResourcePolicyLimits {
            cpu_slot_limit: 0,
            hashing_concurrency_limit: 1,
            network_bandwidth_kib_per_second: None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("cpu_slot_limit"));
    }

    #[test]
    fn custom_resource_policy_limits_persist_in_local_config() {
        let config = LocalConfig {
            resource_profile: ResourceProfile::Custom,
            resource_policy_limits: Some(ResourcePolicyLimits {
                cpu_slot_limit: 2,
                hashing_concurrency_limit: 3,
                network_bandwidth_kib_per_second: Some(2048),
            }),
            ..LocalConfig::default()
        };

        let encoded = config.to_toml_string().unwrap();
        assert!(encoded.contains("[resource_policy_limits]"));
        let decoded = LocalConfig::parse(&encoded).unwrap();

        assert_eq!(decoded, config);
        assert_eq!(
            decoded
                .resource_policy_for_context(ResourcePolicyContext::for_parallelism(8))
                .limits,
            ResourcePolicyLimits {
                cpu_slot_limit: 2,
                hashing_concurrency_limit: 3,
                network_bandwidth_kib_per_second: Some(2048),
            }
        );
    }

    #[test]
    fn rejects_invalid_persisted_resource_policy_limits() {
        let config = LocalConfig {
            resource_policy_limits: Some(ResourcePolicyLimits {
                cpu_slot_limit: 0,
                hashing_concurrency_limit: 1,
                network_bandwidth_kib_per_second: None,
            }),
            ..LocalConfig::default()
        };

        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("cpu_slot_limit"));
    }

    #[test]
    fn parses_macos_resource_observation_output() {
        assert_eq!(
            parse_macos_power_source("Now drawing from 'Battery Power'\n"),
            ResourcePowerSource::Battery
        );
        assert_eq!(
            parse_macos_power_source("Now drawing from 'AC Power'\n"),
            ResourcePowerSource::Ac
        );
        assert_eq!(
            parse_macos_low_power_mode(" sleep 1\n lowpowermode 1\n"),
            Some(true)
        );
        assert_eq!(parse_macos_low_power_mode(" lowpowermode 0\n"), Some(false));
        assert_eq!(parse_macos_load_average("{ 6.50 4.0 3.0 }"), Some(6.5));
        assert_eq!(
            ForegroundLoad::from_load_average(6.5, 8),
            ForegroundLoad::Busy
        );
        assert_eq!(
            ForegroundLoad::from_load_average(1.0, 8),
            ForegroundLoad::Idle
        );
    }

    #[test]
    fn parses_all_resource_profile_labels() {
        for (raw, expected) in [
            ("adaptive", ResourceProfile::Adaptive),
            ("instant", ResourceProfile::Instant),
            ("eco", ResourceProfile::Eco),
            ("custom", ResourceProfile::Custom),
            ("balanced", ResourceProfile::Balanced),
            ("performance", ResourceProfile::Performance),
        ] {
            let config = LocalConfig::parse(&format!(
                r#"
version = 1
fabric_name = "Personal Fabric"
device_name = "this-device"
resource_profile = "{raw}"
anchor_mode = "local-only"

[editor]
command = "system"
"#
            ))
            .unwrap();
            assert_eq!(config.resource_profile, expected);
        }
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
        assert!(redacted.mdns_enabled);
        assert!(!redacted.manual_discovery_address_configured);
    }

    #[test]
    fn parses_mdns_disable_config_and_manual_fallback() {
        let raw = r#"
version = 1
fabric_name = "Personal Fabric"
device_name = "this-device"
resource_profile = "balanced"
anchor_mode = "local-only"
mdns_enabled = false
manual_discovery_address = "192.0.2.10:7000"

[editor]
command = "system"
"#;

        let config = LocalConfig::parse(raw).unwrap();

        assert!(!config.mdns_enabled);
        assert_eq!(
            config.manual_discovery_address,
            Some("192.0.2.10:7000".to_string())
        );
    }

    #[test]
    fn rejects_blank_manual_discovery_address() {
        let config = LocalConfig {
            manual_discovery_address: Some("   ".to_string()),
            ..LocalConfig::default()
        };

        let err = config.validate().unwrap_err();

        assert!(err.to_string().contains("manual_discovery_address"));
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
