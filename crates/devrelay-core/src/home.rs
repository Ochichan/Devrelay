//! Local DevRelay data directory resolution and path helpers.
//!
//! M1 local workflows keep user-specific state outside project repositories.
//! `DEVRELAY_HOME` overrides the platform default; otherwise the default is
//! macOS Application Support, XDG data home on Linux, and LocalAppData on
//! Windows.

use crate::{DevRelayError, Result};
use serde::Serialize;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEVRELAY_HOME_ENV: &str = "DEVRELAY_HOME";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevRelayHome {
    root: PathBuf,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AnchorLayout {
    pub data_dir: PathBuf,
    pub metadata_db_path: PathBuf,
    pub snapshot_repo_root: PathBuf,
    pub cas_root: PathBuf,
    pub startup_path: PathBuf,
}

impl DevRelayHome {
    pub fn resolve() -> Result<Self> {
        Self::resolve_from_env_value(std::env::var_os(DEVRELAY_HOME_ENV))
    }

    pub fn resolve_from_env_value(value: Option<OsString>) -> Result<Self> {
        if let Some(value) = value {
            if value.is_empty() {
                return Err(DevRelayError::Config(format!(
                    "{DEVRELAY_HOME_ENV} must not be empty"
                )));
            }
            return Ok(Self::new(PathBuf::from(value)));
        }
        Ok(Self::new(default_root()?))
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.root.join("projects")
    }

    pub fn project_data_dir(&self, project_id: &str) -> PathBuf {
        self.projects_dir().join(project_id)
    }

    pub fn snapshot_bare_repo_path(&self, project_id: &str) -> PathBuf {
        self.project_data_dir(project_id).join("snapshots.git")
    }

    pub fn metadata_db_path(&self, project_id: &str) -> PathBuf {
        self.project_data_dir(project_id).join("metadata.sqlite")
    }

    pub fn hydration_state_path(&self, project_id: &str, workspace_id: Option<&str>) -> PathBuf {
        self.project_data_dir(project_id)
            .join("hydration")
            .join(hydration_state_file_name(workspace_id))
    }

    pub fn cas_dir(&self, project_id: &str) -> PathBuf {
        self.project_data_dir(project_id).join("cas")
    }

    pub fn log_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn diagnostics_dir(&self) -> PathBuf {
        self.root.join("diagnostics")
    }

    pub fn metrics_dir(&self) -> PathBuf {
        self.root.join("metrics")
    }

    pub fn anchor_dir(&self) -> PathBuf {
        self.root.join("anchor")
    }

    pub fn anchor_metadata_db_path(&self) -> PathBuf {
        self.anchor_dir().join("metadata.sqlite")
    }

    pub fn anchor_snapshot_repo_root(&self) -> PathBuf {
        self.anchor_dir().join("snapshots")
    }

    pub fn anchor_cas_root(&self) -> PathBuf {
        self.anchor_dir().join("cas")
    }

    pub fn anchor_startup_path(&self) -> PathBuf {
        self.anchor_dir().join("startup.json")
    }

    pub fn identity_dir(&self) -> PathBuf {
        self.root.join("identity")
    }

    pub fn fabric_secret_path(&self) -> PathBuf {
        self.identity_dir().join("dev-fabric-secret.json")
    }

    pub fn anchor_layout(&self) -> AnchorLayout {
        AnchorLayout {
            data_dir: self.anchor_dir(),
            metadata_db_path: self.anchor_metadata_db_path(),
            snapshot_repo_root: self.anchor_snapshot_repo_root(),
            cas_root: self.anchor_cas_root(),
            startup_path: self.anchor_startup_path(),
        }
    }

    pub fn agent_socket_path(&self) -> PathBuf {
        self.root.join("agent.sock")
    }

    pub fn create_base_dirs(&self) -> Result<()> {
        for dir in [
            self.root.clone(),
            self.projects_dir(),
            self.log_dir(),
            self.diagnostics_dir(),
            self.metrics_dir(),
        ] {
            fs::create_dir_all(dir)?;
        }
        self.check_permissions()
    }

    pub fn create_anchor_dirs(&self) -> Result<()> {
        self.create_base_dirs()?;
        for dir in [
            self.anchor_dir(),
            self.anchor_snapshot_repo_root(),
            self.anchor_cas_root(),
        ] {
            fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    pub fn create_project_dirs(&self, project_id: &str) -> Result<()> {
        fs::create_dir_all(self.project_data_dir(project_id))?;
        fs::create_dir_all(self.cas_dir(project_id))?;
        Ok(())
    }

    pub fn check_permissions(&self) -> Result<()> {
        let metadata = fs::metadata(&self.root)?;
        if !metadata.is_dir() {
            return Err(DevRelayError::Config(format!(
                "{} is not a directory",
                self.root.display()
            )));
        }

        let probe = self.root.join(format!(
            ".permission-check-{}-{}",
            std::process::id(),
            unix_nanos()
        ));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)?;
        fs::remove_file(probe)?;
        Ok(())
    }
}

fn default_root() -> Result<PathBuf> {
    default_root_from_env(
        std::env::var_os("HOME"),
        std::env::var_os("XDG_DATA_HOME"),
        std::env::var_os("LOCALAPPDATA"),
        std::env::var_os("USERPROFILE"),
    )
}

fn default_root_from_env(
    home: Option<OsString>,
    xdg_data_home: Option<OsString>,
    local_app_data: Option<OsString>,
    user_profile: Option<OsString>,
) -> Result<PathBuf> {
    let _ = (&home, &xdg_data_home, &local_app_data, &user_profile);

    #[cfg(target_os = "macos")]
    {
        let home = require_env_path("HOME", home)?;
        return Ok(home
            .join("Library")
            .join("Application Support")
            .join("DevRelay"));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(path) = xdg_data_home {
            if !path.is_empty() {
                return Ok(PathBuf::from(path).join("devrelay"));
            }
        }
        let home = require_env_path("HOME", home)?;
        return Ok(home.join(".local").join("share").join("devrelay"));
    }

    #[cfg(windows)]
    {
        if let Some(path) = local_app_data {
            if !path.is_empty() {
                return Ok(PathBuf::from(path).join("DevRelay"));
            }
        }
        let profile = require_env_path("USERPROFILE", user_profile)?;
        return Ok(profile.join("AppData").join("Local").join("DevRelay"));
    }

    #[allow(unreachable_code)]
    Err(DevRelayError::Config(
        "unsupported platform for DevRelay home resolution".to_string(),
    ))
}

fn require_env_path(name: &str, value: Option<OsString>) -> Result<PathBuf> {
    let value = value.ok_or_else(|| DevRelayError::Config(format!("{name} is not set")))?;
    if value.is_empty() {
        return Err(DevRelayError::Config(format!("{name} must not be empty")));
    }
    Ok(PathBuf::from(value))
}

fn hydration_state_file_name(workspace_id: Option<&str>) -> String {
    match workspace_id {
        Some(workspace_id) => format!("workspace-{}.json", safe_path_component(workspace_id)),
        None => "project.json".to_string(),
    }
}

fn safe_path_component(value: &str) -> String {
    let safe = !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        && value != "."
        && value != "..";
    if safe {
        return value.to_string();
    }

    let mut encoded = String::with_capacity("hex-".len() + value.len().saturating_mul(2));
    encoded.push_str("hex-");
    for byte in value.bytes() {
        use std::fmt::Write;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_custom_devrelay_home_override() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::resolve_from_env_value(Some(temp.path().into())).unwrap();

        assert_eq!(home.root(), temp.path());
        assert_eq!(home.config_file(), temp.path().join("config.toml"));
    }

    #[test]
    fn exposes_project_path_helpers() {
        let home = DevRelayHome::new("/tmp/devrelay-test");

        assert_eq!(
            home.projects_dir(),
            PathBuf::from("/tmp/devrelay-test/projects")
        );
        assert_eq!(
            home.project_data_dir("project123"),
            PathBuf::from("/tmp/devrelay-test/projects/project123")
        );
        assert_eq!(
            home.snapshot_bare_repo_path("project123"),
            PathBuf::from("/tmp/devrelay-test/projects/project123/snapshots.git")
        );
        assert_eq!(
            home.metadata_db_path("project123"),
            PathBuf::from("/tmp/devrelay-test/projects/project123/metadata.sqlite")
        );
        assert_eq!(
            home.hydration_state_path("project123", None),
            PathBuf::from("/tmp/devrelay-test/projects/project123/hydration/project.json")
        );
        assert_eq!(
            home.hydration_state_path("project123", Some("ws_abcdef-123")),
            PathBuf::from(
                "/tmp/devrelay-test/projects/project123/hydration/workspace-ws_abcdef-123.json"
            )
        );
        assert_eq!(
            home.hydration_state_path("project123", Some("../escape")),
            PathBuf::from(
                "/tmp/devrelay-test/projects/project123/hydration/workspace-hex-2e2e2f657363617065.json"
            )
        );
        assert_eq!(
            home.cas_dir("project123"),
            PathBuf::from("/tmp/devrelay-test/projects/project123/cas")
        );
        assert_eq!(home.log_dir(), PathBuf::from("/tmp/devrelay-test/logs"));
        assert_eq!(
            home.diagnostics_dir(),
            PathBuf::from("/tmp/devrelay-test/diagnostics")
        );
        assert_eq!(
            home.agent_socket_path(),
            PathBuf::from("/tmp/devrelay-test/agent.sock")
        );

        let anchor = home.anchor_layout();
        assert_eq!(anchor.data_dir, PathBuf::from("/tmp/devrelay-test/anchor"));
        assert_eq!(
            anchor.metadata_db_path,
            PathBuf::from("/tmp/devrelay-test/anchor/metadata.sqlite")
        );
        assert_eq!(
            anchor.snapshot_repo_root,
            PathBuf::from("/tmp/devrelay-test/anchor/snapshots")
        );
        assert_eq!(
            anchor.cas_root,
            PathBuf::from("/tmp/devrelay-test/anchor/cas")
        );
        assert_eq!(
            anchor.startup_path,
            PathBuf::from("/tmp/devrelay-test/anchor/startup.json")
        );
    }

    #[test]
    fn creates_data_directories_and_checks_permissions() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("devrelay-home"));

        home.create_base_dirs().unwrap();
        home.create_project_dirs("project123").unwrap();
        home.create_anchor_dirs().unwrap();

        assert!(home.root().is_dir());
        assert!(home.projects_dir().is_dir());
        assert!(home.project_data_dir("project123").is_dir());
        assert!(home.cas_dir("project123").is_dir());
        assert!(home.log_dir().is_dir());
        assert!(home.diagnostics_dir().is_dir());
        assert!(home.metrics_dir().is_dir());
        assert!(home.anchor_dir().is_dir());
        assert!(home.anchor_snapshot_repo_root().is_dir());
        assert!(home.anchor_cas_root().is_dir());
    }

    #[test]
    fn rejects_empty_devrelay_home_override() {
        let err = DevRelayHome::resolve_from_env_value(Some(OsString::new())).unwrap_err();
        assert!(matches!(err, DevRelayError::Config(_)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn defaults_to_macos_application_support() {
        let path =
            default_root_from_env(Some(OsString::from("/Users/dev")), None, None, None).unwrap();

        assert_eq!(
            path,
            PathBuf::from("/Users/dev/Library/Application Support/DevRelay")
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn defaults_to_linux_xdg_or_local_share() {
        let xdg = default_root_from_env(
            Some(OsString::from("/home/dev")),
            Some(OsString::from("/data")),
            None,
            None,
        )
        .unwrap();
        assert_eq!(xdg, PathBuf::from("/data/devrelay"));

        let fallback =
            default_root_from_env(Some(OsString::from("/home/dev")), None, None, None).unwrap();
        assert_eq!(fallback, PathBuf::from("/home/dev/.local/share/devrelay"));
    }

    #[cfg(windows)]
    #[test]
    fn defaults_to_windows_local_app_data() {
        let local = default_root_from_env(
            None,
            None,
            Some(OsString::from(r"C:\Users\dev\AppData\Local")),
            None,
        )
        .unwrap();
        assert_eq!(local, PathBuf::from(r"C:\Users\dev\AppData\Local\DevRelay"));

        let fallback =
            default_root_from_env(None, None, None, Some(OsString::from(r"C:\Users\dev"))).unwrap();
        assert_eq!(
            fallback,
            PathBuf::from(r"C:\Users\dev\AppData\Local\DevRelay")
        );
    }
}
