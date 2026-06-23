//! Git object data-plane safety guards.
//!
//! These helpers define the ref namespace and object checks that any future
//! network-facing Git transport must enforce before serving or accepting data.

use crate::{DevRelayError, GitRepo, ProjectRegistryIndex, Result, WorkspaceState};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEVRELAY_REF_NAMESPACE: &str = "refs/devrelay/";
pub const DEVRELAY_SNAPSHOT_REF_NAMESPACE: &str = "refs/devrelay/snapshots/";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitDataPlanePolicy {
    pub allowed_ref_namespace: String,
    pub snapshot_ref_namespace: String,
    pub max_object_bytes: u64,
    pub repository_quota_bytes: u64,
}

impl Default for GitDataPlanePolicy {
    fn default() -> Self {
        Self {
            allowed_ref_namespace: DEVRELAY_REF_NAMESPACE.to_string(),
            snapshot_ref_namespace: DEVRELAY_SNAPSHOT_REF_NAMESPACE.to_string(),
            max_object_bytes: 128 * 1024 * 1024,
            repository_quota_bytes: 10 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitDataPlaneRefSpec {
    pub source: String,
    pub destination: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GitDataPlaneOperation {
    FetchSnapshot,
    PushSnapshot,
}

impl GitDataPlaneOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FetchSnapshot => "fetch-snapshot",
            Self::PushSnapshot => "push-snapshot",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GitDataPlaneImplementationStrategy {
    #[default]
    LocalBareRepo,
}

impl GitDataPlaneImplementationStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalBareRepo => "local-bare-repo",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitDataPlaneAuthorizationRequest<'a> {
    pub project_id: &'a str,
    pub device_id: &'a str,
    pub operation: GitDataPlaneOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitDataPlaneAuthorization {
    pub project_id: String,
    pub device_id: String,
    pub operation: GitDataPlaneOperation,
    pub workspace_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitDataPlaneServePlan {
    pub strategy: GitDataPlaneImplementationStrategy,
    pub authorization: GitDataPlaneAuthorization,
    pub project_id: String,
    pub repo_path: PathBuf,
    pub allowed_ref_namespace: String,
    pub refspecs: Vec<GitDataPlaneRefSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitObjectInspection {
    pub object_id: String,
    pub object_type: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitRepositorySize {
    pub loose_objects_bytes: u64,
    pub packed_objects_bytes: u64,
    pub total_bytes: u64,
}

impl GitDataPlanePolicy {
    pub fn validate_fetch_refspec(&self, refspec: &GitDataPlaneRefSpec) -> Result<()> {
        self.validate_allowed_ref(&refspec.source, "fetch source")?;
        self.validate_allowed_ref(&refspec.destination, "fetch destination")
    }

    pub fn validate_push_refspec(&self, refspec: &GitDataPlaneRefSpec) -> Result<()> {
        self.validate_allowed_ref(&refspec.source, "push source")?;
        self.validate_snapshot_ref(&refspec.destination, "push destination")
    }

    pub fn validate_allowed_ref(&self, refname: &str, label: &str) -> Result<()> {
        validate_refname(refname, label)?;
        if !refname.starts_with(&self.allowed_ref_namespace) {
            return Err(DevRelayError::Config(format!(
                "{label} {refname} must stay under {}",
                self.allowed_ref_namespace
            )));
        }
        Ok(())
    }

    pub fn validate_snapshot_ref(&self, refname: &str, label: &str) -> Result<()> {
        self.validate_allowed_ref(refname, label)?;
        let Some(suffix) = refname.strip_prefix(&self.snapshot_ref_namespace) else {
            return Err(DevRelayError::Config(format!(
                "{label} {refname} must stay under {}",
                self.snapshot_ref_namespace
            )));
        };
        let mut parts = suffix.split('/');
        let snapshot_id = parts.next().unwrap_or_default();
        let leaf = parts.next().unwrap_or_default();
        if snapshot_id.is_empty() || !matches!(leaf, "index" | "work") || parts.next().is_some() {
            return Err(DevRelayError::Config(format!(
                "{label} {refname} must target a snapshot index or work ref"
            )));
        }
        Ok(())
    }

    pub fn validate_object_size(&self, object: &GitObjectInspection) -> Result<()> {
        if object.size_bytes > self.max_object_bytes {
            return Err(DevRelayError::Config(format!(
                "git object {} is {} bytes, above limit {}",
                object.object_id, object.size_bytes, self.max_object_bytes
            )));
        }
        Ok(())
    }

    pub fn validate_repository_quota(&self, size: &GitRepositorySize) -> Result<()> {
        if size.total_bytes > self.repository_quota_bytes {
            return Err(DevRelayError::Config(format!(
                "git object store is {} bytes, above quota {}",
                size.total_bytes, self.repository_quota_bytes
            )));
        }
        Ok(())
    }
}

pub fn authorize_git_data_plane_project(
    registry: &ProjectRegistryIndex,
    request: GitDataPlaneAuthorizationRequest<'_>,
) -> Result<GitDataPlaneAuthorization> {
    if request.project_id.is_empty() {
        return Err(DevRelayError::Config(
            "data-plane project authorization requires project_id".to_string(),
        ));
    }
    if request.device_id.is_empty() {
        return Err(DevRelayError::Config(
            "data-plane project authorization requires device_id".to_string(),
        ));
    }

    let project = registry.projects.get(request.project_id).ok_or_else(|| {
        DevRelayError::Config(format!(
            "data-plane {} rejected: project {} is not registered",
            request.operation.as_str(),
            request.project_id
        ))
    })?;
    let mut workspace_ids = project
        .workspaces
        .values()
        .filter(|workspace| {
            workspace.project_id == project.project_id
                && workspace.device_id == request.device_id
                && matches!(
                    workspace.state,
                    WorkspaceState::Active | WorkspaceState::Inactive
                )
        })
        .map(|workspace| workspace.workspace_id.clone())
        .collect::<Vec<_>>();
    workspace_ids.sort();

    if workspace_ids.is_empty() {
        return Err(DevRelayError::Config(format!(
            "data-plane {} rejected: device {} is not authorized for project {}",
            request.operation.as_str(),
            request.device_id,
            request.project_id
        )));
    }

    Ok(GitDataPlaneAuthorization {
        project_id: project.project_id.clone(),
        device_id: request.device_id.to_string(),
        operation: request.operation,
        workspace_ids,
    })
}

pub fn inspect_git_object(
    repo: &GitRepo,
    object_id: &str,
    policy: &GitDataPlanePolicy,
) -> Result<GitObjectInspection> {
    validate_object_id(object_id)?;
    let object_type = repo.run(&["cat-file", "-t", object_id])?;
    let size_raw = repo.run(&["cat-file", "-s", object_id])?;
    let size_bytes = size_raw.parse::<u64>().map_err(|err| {
        DevRelayError::Config(format!(
            "git object {object_id} reported invalid size {size_raw}: {err}"
        ))
    })?;
    let inspection = GitObjectInspection {
        object_id: object_id.to_string(),
        object_type,
        size_bytes,
    };
    policy.validate_object_size(&inspection)?;
    Ok(inspection)
}

pub fn ensure_git_object_available(
    repo: &GitRepo,
    object_id: &str,
    policy: &GitDataPlanePolicy,
) -> Result<GitObjectInspection> {
    validate_object_id(object_id)?;
    repo.run(&["cat-file", "-e", object_id])?;
    inspect_git_object(repo, object_id, policy)
}

pub fn verify_git_repository_integrity(repo: &GitRepo) -> Result<()> {
    repo.run(&["fsck", "--strict", "--no-progress"])?;
    Ok(())
}

pub fn inspect_git_repository_size(repo: &GitRepo) -> Result<GitRepositorySize> {
    let output = repo.run(&["count-objects", "-v"])?;
    let mut loose_objects = 0_u64;
    let mut loose_kib = 0_u64;
    let mut packed_kib = 0_u64;

    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = parse_count_objects_value(key, value.trim())?;
        match key {
            "count" => loose_objects = value,
            "size" => loose_kib = value,
            "size-pack" => packed_kib = value,
            _ => {}
        }
    }

    let loose_objects_bytes = loose_kib.saturating_mul(1024).max(loose_objects);
    let packed_objects_bytes = packed_kib.saturating_mul(1024);
    Ok(GitRepositorySize {
        loose_objects_bytes,
        packed_objects_bytes,
        total_bytes: loose_objects_bytes.saturating_add(packed_objects_bytes),
    })
}

fn parse_count_objects_value(key: &str, value: &str) -> Result<u64> {
    value.parse::<u64>().map_err(|err| {
        DevRelayError::Config(format!(
            "git count-objects reported invalid {key} value {value}: {err}"
        ))
    })
}

fn validate_object_id(object_id: &str) -> Result<()> {
    if !matches!(object_id.len(), 40 | 64)
        || !object_id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(DevRelayError::Config(format!(
            "git object id {object_id} must be a SHA-1 or SHA-256 hex object id"
        )));
    }
    Ok(())
}

fn validate_refname(refname: &str, label: &str) -> Result<()> {
    if refname.is_empty() {
        return Err(DevRelayError::Config(format!(
            "{label} ref must not be empty"
        )));
    }
    if refname.starts_with('/') || refname.ends_with('/') {
        return Err(DevRelayError::Config(format!(
            "{label} ref {refname} must not start or end with /"
        )));
    }
    if refname.contains("..")
        || refname.ends_with(".lock")
        || refname.contains("//")
        || refname.bytes().any(|byte| byte <= 0x20 || byte == 0x7f)
        || refname
            .bytes()
            .any(|byte| matches!(byte, b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\'))
        || refname.split('/').any(|part| part.starts_with('.'))
    {
        return Err(DevRelayError::Config(format!(
            "{label} ref {refname} is not a permitted Git ref name"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProjectRegistryEntry, WorkspaceRegistryEntry};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    fn init_repo(path: &Path) -> GitRepo {
        let repo = GitRepo::new(path);
        repo.run(&["init", "-b", "main"]).unwrap();
        repo.run(&["config", "user.name", "DevRelay Test"]).unwrap();
        repo.run(&["config", "user.email", "devrelay-test@example.local"])
            .unwrap();
        fs::write(path.join("README.md"), "hello\n").unwrap();
        repo.run(&["add", "."]).unwrap();
        repo.run(&["commit", "-m", "base"]).unwrap();
        repo
    }

    fn registry_with_workspaces() -> ProjectRegistryIndex {
        ProjectRegistryIndex {
            projects: BTreeMap::from([(
                "project-a".to_string(),
                ProjectRegistryEntry {
                    project_id: "project-a".to_string(),
                    display_name: "Project A".to_string(),
                    local_path: Path::new("/tmp/project-a").to_path_buf(),
                    manifest_path: None,
                    remote_url_fingerprint: None,
                    root_commit_fingerprint: None,
                    workspaces: BTreeMap::from([
                        (
                            "ws-active".to_string(),
                            WorkspaceRegistryEntry {
                                workspace_id: "ws-active".to_string(),
                                project_id: "project-a".to_string(),
                                device_id: "device-a".to_string(),
                                local_path: Path::new("/tmp/project-a").to_path_buf(),
                                platform_profile: "macos-arm64".to_string(),
                                state: WorkspaceState::Active,
                                last_seen_head: None,
                                last_checkpoint_id: None,
                            },
                        ),
                        (
                            "ws-stale".to_string(),
                            WorkspaceRegistryEntry {
                                workspace_id: "ws-stale".to_string(),
                                project_id: "project-a".to_string(),
                                device_id: "device-stale".to_string(),
                                local_path: Path::new("/tmp/project-a-old").to_path_buf(),
                                platform_profile: "macos-arm64".to_string(),
                                state: WorkspaceState::Stale,
                                last_seen_head: None,
                                last_checkpoint_id: None,
                            },
                        ),
                    ]),
                },
            )]),
        }
    }

    #[test]
    fn data_plane_authorization_requires_registered_project_device_workspace() {
        let registry = registry_with_workspaces();

        let authorization = authorize_git_data_plane_project(
            &registry,
            GitDataPlaneAuthorizationRequest {
                project_id: "project-a",
                device_id: "device-a",
                operation: GitDataPlaneOperation::FetchSnapshot,
            },
        )
        .unwrap();

        assert_eq!(authorization.project_id, "project-a");
        assert_eq!(authorization.device_id, "device-a");
        assert_eq!(authorization.workspace_ids, vec!["ws-active"]);

        let unknown_project = authorize_git_data_plane_project(
            &registry,
            GitDataPlaneAuthorizationRequest {
                project_id: "project-missing",
                device_id: "device-a",
                operation: GitDataPlaneOperation::FetchSnapshot,
            },
        )
        .unwrap_err();
        assert!(unknown_project.to_string().contains("not registered"));

        let wrong_device = authorize_git_data_plane_project(
            &registry,
            GitDataPlaneAuthorizationRequest {
                project_id: "project-a",
                device_id: "device-b",
                operation: GitDataPlaneOperation::PushSnapshot,
            },
        )
        .unwrap_err();
        assert!(wrong_device.to_string().contains("not authorized"));

        let stale_device = authorize_git_data_plane_project(
            &registry,
            GitDataPlaneAuthorizationRequest {
                project_id: "project-a",
                device_id: "device-stale",
                operation: GitDataPlaneOperation::FetchSnapshot,
            },
        )
        .unwrap_err();
        assert!(stale_device.to_string().contains("not authorized"));
    }

    #[test]
    fn data_plane_first_implementation_strategy_is_local_bare_repo() {
        let strategy = GitDataPlaneImplementationStrategy::default();

        assert_eq!(strategy, GitDataPlaneImplementationStrategy::LocalBareRepo);
        assert_eq!(strategy.as_str(), "local-bare-repo");
    }

    #[test]
    fn data_plane_ref_policy_restricts_fetch_and_push_namespaces() {
        let policy = GitDataPlanePolicy::default();
        policy
            .validate_fetch_refspec(&GitDataPlaneRefSpec {
                source: "refs/devrelay/snapshots/s1/index".to_string(),
                destination: "refs/devrelay/snapshots/s1/index".to_string(),
            })
            .unwrap();
        policy
            .validate_push_refspec(&GitDataPlaneRefSpec {
                source: "refs/devrelay/snapshots/s1/work".to_string(),
                destination: "refs/devrelay/snapshots/s1/work".to_string(),
            })
            .unwrap();

        let fetch_err = policy
            .validate_fetch_refspec(&GitDataPlaneRefSpec {
                source: "refs/heads/main".to_string(),
                destination: "refs/devrelay/snapshots/s1/index".to_string(),
            })
            .unwrap_err();
        assert!(fetch_err.to_string().contains("refs/devrelay/"));

        let push_err = policy
            .validate_push_refspec(&GitDataPlaneRefSpec {
                source: "refs/devrelay/snapshots/s1/work".to_string(),
                destination: "refs/devrelay/snapshots/s1/metadata".to_string(),
            })
            .unwrap_err();
        assert!(push_err.to_string().contains("snapshot index or work ref"));
    }

    #[test]
    fn data_plane_inspects_objects_and_enforces_size_limit() {
        let temp = tempfile::tempdir().unwrap();
        let repo = init_repo(temp.path());
        let blob = repo.run(&["hash-object", "README.md"]).unwrap();
        let policy = GitDataPlanePolicy::default();

        let inspection = ensure_git_object_available(&repo, &blob, &policy).unwrap();

        assert_eq!(inspection.object_type, "blob");
        assert_eq!(inspection.size_bytes, 6);

        let small_policy = GitDataPlanePolicy {
            max_object_bytes: 5,
            ..GitDataPlanePolicy::default()
        };
        let err = inspect_git_object(&repo, &blob, &small_policy).unwrap_err();
        assert!(err.to_string().contains("above limit"));

        let revision_err = inspect_git_object(&repo, "HEAD^{tree}", &policy).unwrap_err();
        assert!(revision_err.to_string().contains("hex object id"));
    }

    #[test]
    fn data_plane_checks_repository_integrity_and_quota() {
        let temp = tempfile::tempdir().unwrap();
        let repo = init_repo(temp.path());
        let policy = GitDataPlanePolicy::default();

        verify_git_repository_integrity(&repo).unwrap();
        let size = inspect_git_repository_size(&repo).unwrap();
        policy.validate_repository_quota(&size).unwrap();

        let tiny_policy = GitDataPlanePolicy {
            repository_quota_bytes: 1,
            ..GitDataPlanePolicy::default()
        };
        let err = tiny_policy.validate_repository_quota(&size).unwrap_err();
        assert!(err.to_string().contains("above quota"));
    }
}
