//! WSL and Windows-native workspace diagnostics.

use crate::{GitRepo, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WslFilesystemDoctorReport {
    pub repo: PathBuf,
    pub platform_key: String,
    pub path_kind: WslFilesystemPathKind,
    pub warnings: Vec<WslFilesystemWarning>,
    pub guidance: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WslFilesystemPathKind {
    WslLinuxFilesystem,
    WslWindowsMount,
    WindowsWslShare,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WslFilesystemWarning {
    pub code: WslFilesystemWarningCode,
    pub message: String,
    pub safe_actions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WslFilesystemWarningCode {
    SharedTreeMutationRisk,
}

pub fn run_wsl_filesystem_doctor(
    repo: &GitRepo,
    platform_key: &str,
) -> Result<WslFilesystemDoctorReport> {
    repo.run(&["rev-parse", "--git-dir"])?;
    Ok(analyze_wsl_filesystem(repo.path(), platform_key))
}

fn analyze_wsl_filesystem(path: &Path, platform_key: &str) -> WslFilesystemDoctorReport {
    let path_kind = classify_wsl_filesystem_path(path);
    let warnings = wsl_warnings(path_kind, platform_key);
    WslFilesystemDoctorReport {
        repo: path.to_path_buf(),
        platform_key: platform_key.to_string(),
        path_kind,
        warnings,
        guidance: device_mapping_guidance(),
    }
}

fn classify_wsl_filesystem_path(path: &Path) -> WslFilesystemPathKind {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    if is_wsl_windows_mount(&lower) {
        WslFilesystemPathKind::WslWindowsMount
    } else if lower.starts_with("//wsl$/") || lower.starts_with("//wsl.localhost/") {
        WslFilesystemPathKind::WindowsWslShare
    } else if lower.starts_with('/') {
        WslFilesystemPathKind::WslLinuxFilesystem
    } else {
        WslFilesystemPathKind::Other
    }
}

fn is_wsl_windows_mount(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with("/mnt/")
        && bytes.get(5).is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(6) == Some(&b'/')
}

fn wsl_warnings(path_kind: WslFilesystemPathKind, platform_key: &str) -> Vec<WslFilesystemWarning> {
    let risky = (platform_key.starts_with("wsl2-linux-gnu-")
        && path_kind == WslFilesystemPathKind::WslWindowsMount)
        || (platform_key.starts_with("windows-native-")
            && path_kind == WslFilesystemPathKind::WindowsWslShare);
    if !risky {
        return Vec::new();
    }
    vec![WslFilesystemWarning {
        code: WslFilesystemWarningCode::SharedTreeMutationRisk,
        message:
            "Workspace path crosses the Windows-native/WSL filesystem boundary for this platform."
                .to_string(),
        safe_actions: vec![
            "Use a separate DevRelay device identity for Windows native and each WSL distro."
                .to_string(),
            "Keep WSL-mutated workspaces under the distro filesystem, such as /home/<user>."
                .to_string(),
            "Use a separate Windows-native clone instead of mutating a WSL tree through \\\\wsl$."
                .to_string(),
        ],
    }]
}

fn device_mapping_guidance() -> Vec<String> {
    vec![
        "Treat windows-native-* and wsl2-linux-gnu-* as different DevRelay devices.".to_string(),
        "Treat each WSL distro as its own device boundary, even on the same physical machine."
            .to_string(),
        "Do not share one mutable checkout between Windows native tools and WSL tools.".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_for_wsl_workspace_on_windows_mount() {
        let report = analyze_wsl_filesystem(
            Path::new("/mnt/c/Users/dev/project"),
            "wsl2-linux-gnu-x86_64",
        );

        assert_eq!(report.path_kind, WslFilesystemPathKind::WslWindowsMount);
        assert_eq!(
            report.warnings[0].code,
            WslFilesystemWarningCode::SharedTreeMutationRisk
        );
    }

    #[test]
    fn warns_for_windows_native_wsl_share() {
        let report = analyze_wsl_filesystem(
            Path::new("//wsl$/Ubuntu-24.04/home/dev/project"),
            "windows-native-x86_64",
        );

        assert_eq!(report.path_kind, WslFilesystemPathKind::WindowsWslShare);
        assert_eq!(
            report.warnings[0].code,
            WslFilesystemWarningCode::SharedTreeMutationRisk
        );
    }
}
