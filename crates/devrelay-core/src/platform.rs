//! Canonical local platform identity and capability detection.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PLATFORM_KEY_FORMAT: &str =
    "darwin-{arch} | linux-gnu-{arch} | windows-native-{arch} | wsl2-linux-gnu-{arch}";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlatformIdentity {
    pub platform_key: String,
    pub family: String,
    pub architecture: String,
    pub abi: Option<String>,
    pub wsl: Option<WslIdentity>,
    pub capabilities: PlatformCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WslIdentity {
    pub distro: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlatformCapabilities {
    pub anchor: bool,
    pub local_snapshots: bool,
    pub filesystem_events: bool,
    pub fsmonitor: bool,
    pub symlinks: bool,
    pub executable_bit: bool,
    pub case_sensitive_paths: bool,
    pub wsl: bool,
}

pub fn detect_platform_identity() -> PlatformIdentity {
    platform_identity_from_probe(&PlatformProbe::current())
}

pub fn current_platform_key() -> String {
    detect_platform_identity().platform_key
}

pub fn current_platform_architecture() -> String {
    detect_platform_identity().architecture
}

pub fn current_platform_capabilities_json() -> String {
    serde_json::to_string(&detect_platform_identity().capabilities)
        .unwrap_or_else(|_| r#"{"anchor":true,"local_snapshots":true}"#.to_string())
}

pub fn platform_capabilities_for_key(platform_key: &str) -> PlatformCapabilities {
    if platform_key.starts_with("darwin-") {
        capabilities_for("darwin")
    } else if platform_key.starts_with("linux-gnu-") {
        capabilities_for("linux")
    } else if platform_key.starts_with("wsl2-linux-gnu-") {
        capabilities_for("wsl2")
    } else if platform_key.starts_with("windows-native-") {
        capabilities_for("windows")
    } else {
        capabilities_for("unknown")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlatformProbe {
    os: String,
    arch: String,
    env: BTreeMap<String, String>,
    linux_proc_version: Option<String>,
}

impl PlatformProbe {
    fn current() -> Self {
        Self {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            env: std::env::vars().collect(),
            linux_proc_version: read_linux_proc_version(),
        }
    }
}

fn platform_identity_from_probe(probe: &PlatformProbe) -> PlatformIdentity {
    let architecture = canonical_architecture(&probe.os, &probe.arch);
    let wsl = detect_wsl(probe);
    let (family, abi, platform_key) = if wsl.is_some() {
        (
            "wsl2".to_string(),
            Some("gnu".to_string()),
            format!("wsl2-linux-gnu-{architecture}"),
        )
    } else {
        match probe.os.as_str() {
            "macos" | "darwin" => ("darwin".to_string(), None, format!("darwin-{architecture}")),
            "linux" => (
                "linux".to_string(),
                Some("gnu".to_string()),
                format!("linux-gnu-{architecture}"),
            ),
            "windows" => (
                "windows".to_string(),
                Some("native".to_string()),
                format!("windows-native-{architecture}"),
            ),
            other => (other.to_string(), None, format!("{other}-{architecture}")),
        }
    };

    PlatformIdentity {
        platform_key,
        family: family.clone(),
        architecture,
        abi,
        wsl,
        capabilities: capabilities_for(&family),
    }
}

fn capabilities_for(family: &str) -> PlatformCapabilities {
    match family {
        "darwin" => PlatformCapabilities {
            anchor: true,
            local_snapshots: true,
            filesystem_events: true,
            fsmonitor: true,
            symlinks: true,
            executable_bit: true,
            case_sensitive_paths: false,
            wsl: false,
        },
        "linux" => PlatformCapabilities {
            anchor: true,
            local_snapshots: true,
            filesystem_events: true,
            fsmonitor: false,
            symlinks: true,
            executable_bit: true,
            case_sensitive_paths: true,
            wsl: false,
        },
        "wsl2" => PlatformCapabilities {
            anchor: true,
            local_snapshots: true,
            filesystem_events: true,
            fsmonitor: false,
            symlinks: true,
            executable_bit: true,
            case_sensitive_paths: true,
            wsl: true,
        },
        "windows" => PlatformCapabilities {
            anchor: true,
            local_snapshots: true,
            filesystem_events: true,
            fsmonitor: true,
            symlinks: false,
            executable_bit: false,
            case_sensitive_paths: false,
            wsl: false,
        },
        _ => PlatformCapabilities {
            anchor: false,
            local_snapshots: true,
            filesystem_events: false,
            fsmonitor: false,
            symlinks: false,
            executable_bit: false,
            case_sensitive_paths: true,
            wsl: false,
        },
    }
}

fn canonical_architecture(os: &str, arch: &str) -> String {
    match (os, arch) {
        ("macos" | "darwin", "aarch64") => "arm64".to_string(),
        (_, "amd64") => "x86_64".to_string(),
        (_, "arm64") => "aarch64".to_string(),
        _ => arch.to_string(),
    }
}

fn detect_wsl(probe: &PlatformProbe) -> Option<WslIdentity> {
    if probe.os != "linux" {
        return None;
    }
    let proc_version = probe.linux_proc_version.as_deref().unwrap_or_default();
    let has_wsl_marker = probe.env.contains_key("WSL_DISTRO_NAME")
        || probe.env.contains_key("WSL_INTEROP")
        || proc_version.to_ascii_lowercase().contains("microsoft");
    if !has_wsl_marker {
        return None;
    }
    let is_wsl2 = probe.env.contains_key("WSL_INTEROP")
        || proc_version.to_ascii_lowercase().contains("wsl2")
        || proc_version.contains("microsoft-standard");
    if !is_wsl2 {
        return None;
    }
    Some(WslIdentity {
        distro: probe.env.get("WSL_DISTRO_NAME").cloned(),
        version: wsl_version_from_proc(proc_version),
    })
}

fn wsl_version_from_proc(proc_version: &str) -> Option<String> {
    proc_version
        .split_whitespace()
        .find(|part| part.to_ascii_lowercase().contains("microsoft"))
        .map(str::to_string)
}

#[cfg(target_os = "linux")]
fn read_linux_proc_version() -> Option<String> {
    std::fs::read_to_string("/proc/version").ok()
}

#[cfg(not(target_os = "linux"))]
fn read_linux_proc_version() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_darwin_platform_keys() {
        assert_platform("macos", "aarch64", "darwin-arm64", "arm64");
        assert_platform("macos", "x86_64", "darwin-x86_64", "x86_64");
    }

    #[test]
    fn detects_linux_gnu_platform_keys() {
        assert_platform("linux", "x86_64", "linux-gnu-x86_64", "x86_64");
        assert_platform("linux", "aarch64", "linux-gnu-aarch64", "aarch64");
    }

    #[test]
    fn detects_windows_native_platform_key() {
        assert_platform("windows", "x86_64", "windows-native-x86_64", "x86_64");
    }

    #[test]
    fn detects_wsl2_distro_and_version() {
        let mut env = BTreeMap::new();
        env.insert("WSL_DISTRO_NAME".to_string(), "Ubuntu-24.04".to_string());
        env.insert("WSL_INTEROP".to_string(), "/run/WSL/1_interop".to_string());
        let identity = platform_identity_from_probe(&PlatformProbe {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            env,
            linux_proc_version: Some(
                "Linux version 5.15.153.1-microsoft-standard-WSL2".to_string(),
            ),
        });

        assert_eq!(identity.platform_key, "wsl2-linux-gnu-x86_64");
        assert!(identity.capabilities.wsl);
        assert_eq!(
            identity.wsl.as_ref().and_then(|wsl| wsl.distro.as_deref()),
            Some("Ubuntu-24.04")
        );
        assert_eq!(
            identity.wsl.as_ref().and_then(|wsl| wsl.version.as_deref()),
            Some("5.15.153.1-microsoft-standard-WSL2")
        );
    }

    #[test]
    fn serializes_platform_capabilities_as_config_json() {
        let capabilities = serde_json::to_value(capabilities_for("darwin")).unwrap();

        assert_eq!(capabilities["anchor"], true);
        assert_eq!(capabilities["local_snapshots"], true);
        assert_eq!(capabilities["fsmonitor"], true);
    }

    fn assert_platform(os: &str, arch: &str, platform_key: &str, architecture: &str) {
        let identity = platform_identity_from_probe(&PlatformProbe {
            os: os.to_string(),
            arch: arch.to_string(),
            env: BTreeMap::new(),
            linux_proc_version: None,
        });

        assert_eq!(identity.platform_key, platform_key);
        assert_eq!(identity.architecture, architecture);
    }
}
