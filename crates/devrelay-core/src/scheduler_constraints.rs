//! Scheduler device resource collection and task constraint filtering.
//!
//! This module is intentionally limited to the M10.2 contract: collect a
//! conservative device snapshot and reject candidates that cannot satisfy a
//! task definition before later scheduler scoring chooses among them.

use crate::{
    ForegroundLoad, ResourcePolicyContext, ResourcePowerSource, TaskDefinition,
    current_platform_device_scope_key, detect_platform_identity,
};
use globset::Glob;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

#[cfg(target_family = "unix")]
const MIB: u128 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerDeviceSnapshot {
    pub device_id: String,
    pub platform_key: String,
    pub os: String,
    pub architecture: String,
    pub cpu_cores: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_total_mib: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_total_mib: Option<u64>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub policy: SchedulerDevicePolicy,
    pub dynamic: SchedulerDynamicResources,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerDevicePolicy {
    pub allow_task_execution: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Default for SchedulerDevicePolicy {
    fn default() -> Self {
        Self {
            allow_task_execution: true,
            reason: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerDynamicResources {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_load_1m_milli: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_free_mib: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_free_mib: Option<u64>,
    pub power_source: ResourcePowerSource,
    pub low_power_mode: bool,
    pub foreground_load: ForegroundLoad,
    pub network_route_quality: SchedulerNetworkRouteQuality,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SchedulerNetworkRouteQuality {
    #[default]
    Unknown,
    Poor,
    Fair,
    Good,
    Excellent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerConstraintDecision {
    pub device_id: String,
    pub eligible: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejections: Vec<SchedulerConstraintRejection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "code", rename_all = "kebab-case")]
pub enum SchedulerConstraintRejection {
    IncompatiblePlatform {
        actual: String,
        required: Vec<String>,
    },
    MissingFeatures {
        missing: Vec<String>,
    },
    InsufficientCpu {
        required_cores: u64,
        available_cores: u64,
    },
    InsufficientMemory {
        required_mib: u64,
        available_mib: Option<u64>,
        capacity_mib: Option<u64>,
    },
    InsufficientDisk {
        required_mib: u64,
        available_mib: Option<u64>,
        capacity_mib: Option<u64>,
    },
    PolicyDisallowed {
        reason: Option<String>,
    },
}

pub fn collect_local_scheduler_device(workspace_path: impl AsRef<Path>) -> SchedulerDeviceSnapshot {
    let identity = detect_platform_identity();
    let cpu_cores = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .max(1) as u64;
    let context = ResourcePolicyContext::detect_current();
    let cpu_load_1m_milli = cpu_load_1m_milli();
    let foreground_load = match context.foreground_load {
        ForegroundLoad::Unknown => cpu_load_1m_milli
            .map(|load| ForegroundLoad::from_load_average(load as f32 / 1000.0, cpu_cores as usize))
            .unwrap_or(ForegroundLoad::Unknown),
        load => load,
    };
    let (disk_total_mib, disk_free_mib) = disk_mib(workspace_path.as_ref());

    SchedulerDeviceSnapshot {
        device_id: current_platform_device_scope_key(),
        platform_key: identity.platform_key,
        os: identity.family,
        architecture: identity.architecture,
        cpu_cores,
        memory_total_mib: memory_total_mib(),
        disk_total_mib,
        features: platform_features(&identity.capabilities),
        policy: SchedulerDevicePolicy::default(),
        dynamic: SchedulerDynamicResources {
            cpu_load_1m_milli,
            memory_free_mib: memory_free_mib(),
            disk_free_mib,
            power_source: context.power_source,
            low_power_mode: context.low_power_mode,
            foreground_load,
            network_route_quality: SchedulerNetworkRouteQuality::Unknown,
        },
    }
}

pub fn evaluate_scheduler_constraints(
    definition: &TaskDefinition,
    device: &SchedulerDeviceSnapshot,
) -> SchedulerConstraintDecision {
    let mut rejections = Vec::new();

    if !device.policy.allow_task_execution {
        rejections.push(SchedulerConstraintRejection::PolicyDisallowed {
            reason: device.policy.reason.clone(),
        });
    }

    if !definition.platforms.is_empty()
        && !definition
            .platforms
            .iter()
            .any(|pattern| platform_pattern_matches(pattern, &device.platform_key))
    {
        rejections.push(SchedulerConstraintRejection::IncompatiblePlatform {
            actual: device.platform_key.clone(),
            required: definition.platforms.clone(),
        });
    }

    let missing = missing_features(&definition.features, &device.features);
    if !missing.is_empty() {
        rejections.push(SchedulerConstraintRejection::MissingFeatures { missing });
    }

    if let Some(required_cores) = definition.cpu
        && required_cores > device.cpu_cores
    {
        rejections.push(SchedulerConstraintRejection::InsufficientCpu {
            required_cores,
            available_cores: device.cpu_cores,
        });
    }

    if let Some(required_mib) = definition.memory_mib
        && resource_is_insufficient(
            required_mib,
            device.dynamic.memory_free_mib,
            device.memory_total_mib,
        )
    {
        rejections.push(SchedulerConstraintRejection::InsufficientMemory {
            required_mib,
            available_mib: device.dynamic.memory_free_mib,
            capacity_mib: device.memory_total_mib,
        });
    }

    if let Some(required_mib) = definition.disk_mib
        && required_mib > 0
        && resource_is_insufficient(
            required_mib,
            device.dynamic.disk_free_mib,
            device.disk_total_mib,
        )
    {
        rejections.push(SchedulerConstraintRejection::InsufficientDisk {
            required_mib,
            available_mib: device.dynamic.disk_free_mib,
            capacity_mib: device.disk_total_mib,
        });
    }

    SchedulerConstraintDecision {
        device_id: device.device_id.clone(),
        eligible: rejections.is_empty(),
        rejections,
    }
}

pub fn filter_scheduler_candidates(
    definition: &TaskDefinition,
    devices: &[SchedulerDeviceSnapshot],
) -> Vec<SchedulerConstraintDecision> {
    devices
        .iter()
        .map(|device| evaluate_scheduler_constraints(definition, device))
        .filter(|decision| decision.eligible)
        .collect()
}

fn resource_is_insufficient(
    required_mib: u64,
    available_mib: Option<u64>,
    capacity_mib: Option<u64>,
) -> bool {
    match available_mib {
        Some(available_mib) => available_mib < required_mib,
        None => capacity_mib.is_none_or(|capacity_mib| capacity_mib < required_mib),
    }
}

fn missing_features(required: &[String], actual: &[String]) -> Vec<String> {
    let actual = actual.iter().collect::<BTreeSet<_>>();
    let mut missing = required
        .iter()
        .filter(|feature| !actual.contains(feature))
        .cloned()
        .collect::<Vec<_>>();
    missing.sort();
    missing
}

fn platform_pattern_matches(pattern: &str, actual: &str) -> bool {
    if pattern == actual || actual.starts_with(&format!("{pattern}-")) {
        return true;
    }
    Glob::new(pattern)
        .map(|glob| glob.compile_matcher().is_match(actual))
        .unwrap_or(false)
}

fn platform_features(capabilities: &crate::PlatformCapabilities) -> Vec<String> {
    let mut features = Vec::new();
    push_feature(&mut features, capabilities.anchor, "anchor");
    push_feature(
        &mut features,
        capabilities.local_snapshots,
        "local-snapshots",
    );
    push_feature(
        &mut features,
        capabilities.filesystem_events,
        "filesystem-events",
    );
    push_feature(&mut features, capabilities.fsmonitor, "fsmonitor");
    push_feature(&mut features, capabilities.symlinks, "symlinks");
    push_feature(&mut features, capabilities.executable_bit, "executable-bit");
    push_feature(
        &mut features,
        capabilities.case_sensitive_paths,
        "case-sensitive-paths",
    );
    push_feature(&mut features, capabilities.wsl, "wsl");
    features
}

fn push_feature(features: &mut Vec<String>, enabled: bool, feature: &str) {
    if enabled {
        features.push(feature.to_string());
    }
}

#[cfg(target_family = "unix")]
fn bytes_to_mib(bytes: u128) -> Option<u64> {
    u64::try_from(bytes / MIB).ok()
}

#[cfg(target_family = "unix")]
fn disk_mib(path: &Path) -> (Option<u64>, Option<u64>) {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;

    let c_path = match CString::new(path.as_os_str().as_bytes()) {
        Ok(path) => path,
        Err(_) => return (None, None),
    };
    let mut stats = MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(c_path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return (None, None);
    }
    let stats = unsafe { stats.assume_init() };
    let block_size = if stats.f_frsize > 0 {
        stats.f_frsize
    } else {
        stats.f_bsize
    } as u128;
    (
        bytes_to_mib(stats.f_blocks as u128 * block_size),
        bytes_to_mib(stats.f_bavail as u128 * block_size),
    )
}

#[cfg(not(target_family = "unix"))]
fn disk_mib(_path: &Path) -> (Option<u64>, Option<u64>) {
    (None, None)
}

#[cfg(target_family = "unix")]
fn cpu_load_1m_milli() -> Option<u64> {
    let mut loads = [0.0_f64; 3];
    let count = unsafe { libc::getloadavg(loads.as_mut_ptr(), loads.len() as libc::c_int) };
    (count >= 1 && loads[0].is_finite()).then(|| (loads[0].max(0.0) * 1000.0).round() as u64)
}

#[cfg(not(target_family = "unix"))]
fn cpu_load_1m_milli() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn memory_total_mib() -> Option<u64> {
    linux_sysinfo_bytes(|info| info.totalram).and_then(bytes_to_mib)
}

#[cfg(target_os = "linux")]
fn memory_free_mib() -> Option<u64> {
    linux_sysinfo_bytes(|info| info.freeram).and_then(bytes_to_mib)
}

#[cfg(target_os = "linux")]
fn linux_sysinfo_bytes(value: impl FnOnce(libc::sysinfo) -> libc::c_ulong) -> Option<u128> {
    let mut info = std::mem::MaybeUninit::<libc::sysinfo>::uninit();
    if unsafe { libc::sysinfo(info.as_mut_ptr()) } != 0 {
        return None;
    }
    let info = unsafe { info.assume_init() };
    Some(value(info) as u128 * info.mem_unit as u128)
}

#[cfg(target_os = "macos")]
fn memory_total_mib() -> Option<u64> {
    sysctl_u64("hw.memsize").and_then(|bytes| bytes_to_mib(bytes as u128))
}

#[cfg(target_os = "macos")]
fn memory_free_mib() -> Option<u64> {
    let free_pages = sysctl_u64("vm.page_free_count")?;
    let page_size = sysctl_u64("hw.pagesize")?;
    bytes_to_mib(free_pages as u128 * page_size as u128)
}

#[cfg(target_os = "macos")]
fn sysctl_u64(name: &str) -> Option<u64> {
    use std::ffi::CString;

    let name = CString::new(name).ok()?;
    let mut value = 0_u64;
    let mut size = std::mem::size_of::<u64>();
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut value as *mut u64 as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (status == 0 && size > 0).then_some(value)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn memory_total_mib() -> Option<u64> {
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn memory_free_mib() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EnvironmentKind, TaskCacheMode};

    #[test]
    fn local_scheduler_device_collects_static_and_dynamic_shape() {
        let device = collect_local_scheduler_device(".");

        assert!(!device.device_id.is_empty());
        assert!(!device.platform_key.is_empty());
        assert!(!device.os.is_empty());
        assert!(!device.architecture.is_empty());
        assert!(device.cpu_cores >= 1);
        assert_eq!(
            device.dynamic.network_route_quality,
            SchedulerNetworkRouteQuality::Unknown
        );
    }

    #[test]
    fn accepts_matching_device_with_enough_resources() {
        let definition = task_definition();
        let device = scheduler_device();

        let decision = evaluate_scheduler_constraints(&definition, &device);

        assert!(decision.eligible);
        assert!(decision.rejections.is_empty());
        assert_eq!(
            filter_scheduler_candidates(&definition, std::slice::from_ref(&device)),
            vec![decision]
        );
    }

    #[test]
    fn rejects_incompatible_platforms_and_missing_features() {
        let mut definition = task_definition();
        definition.platforms = vec!["linux-*".to_string()];
        definition.features = vec!["gpu".to_string(), "python".to_string()];
        let mut device = scheduler_device();
        device.platform_key = "darwin-arm64".to_string();
        device.features = vec!["python".to_string()];

        let decision = evaluate_scheduler_constraints(&definition, &device);

        assert!(!decision.eligible);
        assert_eq!(
            decision.rejections,
            vec![
                SchedulerConstraintRejection::IncompatiblePlatform {
                    actual: "darwin-arm64".to_string(),
                    required: vec!["linux-*".to_string()],
                },
                SchedulerConstraintRejection::MissingFeatures {
                    missing: vec!["gpu".to_string()],
                },
            ]
        );
    }

    #[test]
    fn rejects_insufficient_resources_and_policy() {
        let mut definition = task_definition();
        definition.cpu = Some(8);
        definition.memory_mib = Some(4096);
        definition.disk_mib = Some(20_000);
        let mut device = scheduler_device();
        device.cpu_cores = 4;
        device.memory_total_mib = Some(2048);
        device.dynamic.memory_free_mib = Some(1024);
        device.disk_total_mib = Some(100_000);
        device.dynamic.disk_free_mib = Some(10_000);
        device.policy = SchedulerDevicePolicy {
            allow_task_execution: false,
            reason: Some("manual pause".to_string()),
        };

        let decision = evaluate_scheduler_constraints(&definition, &device);

        assert!(!decision.eligible);
        assert_eq!(
            decision.rejections,
            vec![
                SchedulerConstraintRejection::PolicyDisallowed {
                    reason: Some("manual pause".to_string()),
                },
                SchedulerConstraintRejection::InsufficientCpu {
                    required_cores: 8,
                    available_cores: 4,
                },
                SchedulerConstraintRejection::InsufficientMemory {
                    required_mib: 4096,
                    available_mib: Some(1024),
                    capacity_mib: Some(2048),
                },
                SchedulerConstraintRejection::InsufficientDisk {
                    required_mib: 20_000,
                    available_mib: Some(10_000),
                    capacity_mib: Some(100_000),
                },
            ]
        );
    }

    #[test]
    fn treats_unknown_resource_metrics_as_rejecting_required_resources() {
        let mut definition = task_definition();
        definition.memory_mib = Some(4096);
        definition.disk_mib = Some(8192);
        let mut device = scheduler_device();
        device.memory_total_mib = None;
        device.dynamic.memory_free_mib = None;
        device.disk_total_mib = None;
        device.dynamic.disk_free_mib = None;

        let decision = evaluate_scheduler_constraints(&definition, &device);

        assert!(!decision.eligible);
        assert_eq!(
            decision.rejections,
            vec![
                SchedulerConstraintRejection::InsufficientMemory {
                    required_mib: 4096,
                    available_mib: None,
                    capacity_mib: None,
                },
                SchedulerConstraintRejection::InsufficientDisk {
                    required_mib: 8192,
                    available_mib: None,
                    capacity_mib: None,
                },
            ]
        );
    }

    fn task_definition() -> TaskDefinition {
        TaskDefinition {
            project_id: "12345678".to_string(),
            task_name: "test".to_string(),
            profile_name: "dev".to_string(),
            profile_kind: EnvironmentKind::Native,
            command: vec!["bash".to_string(), "-lc".to_string(), "true".to_string()],
            platforms: vec!["darwin-*".to_string()],
            cpu: Some(2),
            memory_mib: Some(1024),
            disk_mib: Some(2048),
            interactive: false,
            cache: Some(TaskCacheMode::ReadWrite),
            outputs: Vec::new(),
            features: vec!["python".to_string()],
            sandbox: None,
            command_definition_hash: "a".repeat(64),
        }
    }

    fn scheduler_device() -> SchedulerDeviceSnapshot {
        SchedulerDeviceSnapshot {
            device_id: "dev_local".to_string(),
            platform_key: "darwin-arm64".to_string(),
            os: "darwin".to_string(),
            architecture: "arm64".to_string(),
            cpu_cores: 8,
            memory_total_mib: Some(16_384),
            disk_total_mib: Some(1_000_000),
            features: vec!["python".to_string(), "symlinks".to_string()],
            policy: SchedulerDevicePolicy::default(),
            dynamic: SchedulerDynamicResources {
                cpu_load_1m_milli: Some(1500),
                memory_free_mib: Some(8192),
                disk_free_mib: Some(500_000),
                power_source: ResourcePowerSource::Ac,
                low_power_mode: false,
                foreground_load: ForegroundLoad::Idle,
                network_route_quality: SchedulerNetworkRouteQuality::Unknown,
            },
        }
    }
}
