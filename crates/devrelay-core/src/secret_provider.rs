//! Secret provider mapping and local materialization helpers.
//!
//! This module models how a local device maps manifest-declared secrets to
//! local providers. It keeps provider resolution testable through a trait so
//! callers can use real keychains/CLIs later without changing manifest safety
//! rules.

use crate::{DevRelayError, LogRedactor, Manifest, Result, SecretConfig, SecretMode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretProviderLocalConfig {
    #[serde(default)]
    pub mappings: BTreeMap<String, SecretProviderMapping>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretProviderMapping {
    pub provider: SecretProviderKind,
    pub reference: String,
    #[serde(default)]
    pub command: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SecretProviderKind {
    OsKeychain,
    OnePasswordCli,
    BitwardenCli,
    SopsAge,
    UserScript,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretProviderCommandPlan {
    pub program: String,
    pub args: Vec<String>,
}

impl SecretProviderMapping {
    pub fn command_plan(&self) -> Result<Option<SecretProviderCommandPlan>> {
        let plan = match self.provider {
            SecretProviderKind::OsKeychain => return Ok(None),
            SecretProviderKind::OnePasswordCli => SecretProviderCommandPlan {
                program: "op".to_string(),
                args: vec!["read".to_string(), self.reference.clone()],
            },
            SecretProviderKind::BitwardenCli => SecretProviderCommandPlan {
                program: "bw".to_string(),
                args: vec![
                    "get".to_string(),
                    "password".to_string(),
                    self.reference.clone(),
                ],
            },
            SecretProviderKind::SopsAge => SecretProviderCommandPlan {
                program: "sops".to_string(),
                args: vec!["--decrypt".to_string(), self.reference.clone()],
            },
            SecretProviderKind::UserScript => {
                let Some((program, args)) = self.command.split_first() else {
                    return Err(DevRelayError::Config(
                        "user script secret provider requires a command".to_string(),
                    ));
                };
                SecretProviderCommandPlan {
                    program: program.clone(),
                    args: args.to_vec(),
                }
            }
        };
        Ok(Some(plan))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretProviderRequest {
    pub secret_name: String,
    pub mapping: SecretProviderMapping,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretValue {
    pub value: String,
}

impl SecretValue {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }
}

pub trait SecretProvider {
    fn resolve_secret(&self, request: &SecretProviderRequest) -> Result<Option<SecretValue>>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretFileMaterialization {
    pub secret_name: String,
    pub target: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretMaterializationReport {
    pub files: Vec<SecretFileMaterialization>,
    pub environment_variables: BTreeMap<String, String>,
    pub missing_optional: Vec<String>,
    pub hard_exclude_patterns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedSecretMaterializationReport {
    pub files: Vec<SecretFileMaterialization>,
    pub environment_variables: BTreeMap<String, String>,
    pub missing_optional: Vec<String>,
    pub hard_exclude_patterns: Vec<String>,
}

impl SecretMaterializationReport {
    pub fn redacted_for_logs(&self) -> RedactedSecretMaterializationReport {
        let redactor = LogRedactor::new();
        RedactedSecretMaterializationReport {
            files: self.files.clone(),
            environment_variables: self
                .environment_variables
                .iter()
                .map(|(key, value)| (key.clone(), redactor.redact_field(key, value)))
                .collect(),
            missing_optional: self.missing_optional.clone(),
            hard_exclude_patterns: self.hard_exclude_patterns.clone(),
        }
    }
}

pub fn materialize_project_secrets(
    root: &Path,
    manifest: &Manifest,
    local_config: &SecretProviderLocalConfig,
    provider: &(impl SecretProvider + ?Sized),
) -> Result<SecretMaterializationReport> {
    let mut report = SecretMaterializationReport {
        files: Vec::new(),
        environment_variables: BTreeMap::new(),
        missing_optional: Vec::new(),
        hard_exclude_patterns: secret_hard_exclude_patterns(manifest),
    };

    for (secret_name, secret) in &manifest.secrets {
        let Some(mapping) = local_config.mappings.get(secret_name).cloned() else {
            handle_missing_secret(secret_name, secret, &mut report)?;
            continue;
        };
        let request = SecretProviderRequest {
            secret_name: secret_name.clone(),
            mapping,
        };
        let Some(value) = provider.resolve_secret(&request)? else {
            handle_missing_secret(secret_name, secret, &mut report)?;
            continue;
        };

        match secret.mode {
            SecretMode::File => {
                let relative = normalize_secret_target(&secret.target)?;
                let target = root.join(&relative);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&target, value.value.as_bytes())?;
                set_secret_file_permissions(&target)?;
                report.files.push(SecretFileMaterialization {
                    secret_name: secret_name.clone(),
                    target,
                });
            }
            SecretMode::Environment => {
                let key = secret
                    .environment_variable
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(&secret.target)
                    .to_string();
                report.environment_variables.insert(key, value.value);
            }
        }
    }

    Ok(report)
}

pub fn secret_hard_exclude_patterns(manifest: &Manifest) -> Vec<String> {
    manifest
        .secrets
        .values()
        .filter(|secret| secret.mode == SecretMode::File)
        .filter_map(|secret| normalize_secret_target(&secret.target).ok())
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect()
}

fn handle_missing_secret(
    secret_name: &str,
    secret: &SecretConfig,
    report: &mut SecretMaterializationReport,
) -> Result<()> {
    if secret.required {
        return Err(DevRelayError::Config(format!(
            "missing required secret {secret_name}"
        )));
    }
    report.missing_optional.push(secret_name.to_string());
    Ok(())
}

fn normalize_secret_target(target: &str) -> Result<PathBuf> {
    let path = Path::new(target);
    if target.trim().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(DevRelayError::Config(format!(
            "secret target must stay inside the workspace: {target:?}"
        )));
    }
    Ok(path.to_path_buf())
}

#[cfg(unix)]
fn set_secret_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_secret_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PathDecision, UntrackedPolicy, classify_untracked_paths};

    #[derive(Default)]
    struct FakeSecretProvider {
        values: BTreeMap<String, SecretValue>,
    }

    impl FakeSecretProvider {
        fn with_value(mut self, reference: &str, value: &str) -> Self {
            self.values
                .insert(reference.to_string(), SecretValue::new(value));
            self
        }
    }

    impl SecretProvider for FakeSecretProvider {
        fn resolve_secret(&self, request: &SecretProviderRequest) -> Result<Option<SecretValue>> {
            Ok(self.values.get(&request.mapping.reference).cloned())
        }
    }

    fn manifest_with_secrets(extra: &str) -> Manifest {
        Manifest::parse(&format!(
            r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "safe"
portable_paths = "strict"

{extra}
"#
        ))
        .unwrap()
    }

    #[test]
    fn provider_mappings_build_cli_command_plans() {
        let op = SecretProviderMapping {
            provider: SecretProviderKind::OnePasswordCli,
            reference: "op://vault/item/password".to_string(),
            command: Vec::new(),
        };
        assert_eq!(op.command_plan().unwrap().unwrap().program, "op");

        let bw = SecretProviderMapping {
            provider: SecretProviderKind::BitwardenCli,
            reference: "item-id".to_string(),
            command: Vec::new(),
        };
        assert_eq!(
            bw.command_plan().unwrap().unwrap().args,
            vec!["get", "password", "item-id"]
        );

        let sops = SecretProviderMapping {
            provider: SecretProviderKind::SopsAge,
            reference: "secrets.enc.yaml".to_string(),
            command: Vec::new(),
        };
        assert_eq!(sops.command_plan().unwrap().unwrap().program, "sops");

        let script = SecretProviderMapping {
            provider: SecretProviderKind::UserScript,
            reference: "ignored".to_string(),
            command: vec!["./secret.sh".to_string(), "api".to_string()],
        };
        assert_eq!(
            script.command_plan().unwrap().unwrap().program,
            "./secret.sh"
        );

        let keychain = SecretProviderMapping {
            provider: SecretProviderKind::OsKeychain,
            reference: "service/account".to_string(),
            command: Vec::new(),
        };
        assert_eq!(keychain.command_plan().unwrap(), None);
    }

    #[test]
    fn materializes_secret_file_and_environment_with_redacted_report() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = manifest_with_secrets(
            r#"
[secrets.env_file]
target = ".devrelay/secrets/.env"

[secrets.api_token]
target = "API_TOKEN"
mode = "environment"
environment_variable = "API_TOKEN"
"#,
        );
        let local_config = SecretProviderLocalConfig {
            mappings: BTreeMap::from([
                (
                    "env_file".to_string(),
                    SecretProviderMapping {
                        provider: SecretProviderKind::OnePasswordCli,
                        reference: "env".to_string(),
                        command: Vec::new(),
                    },
                ),
                (
                    "api_token".to_string(),
                    SecretProviderMapping {
                        provider: SecretProviderKind::BitwardenCli,
                        reference: "api".to_string(),
                        command: Vec::new(),
                    },
                ),
            ]),
        };
        let provider = FakeSecretProvider::default()
            .with_value("env", "DATABASE_URL=postgres://user:pass@example/db\n")
            .with_value("api", "secret-api-token");

        let report =
            materialize_project_secrets(temp.path(), &manifest, &local_config, &provider).unwrap();

        assert_eq!(
            fs::read_to_string(temp.path().join(".devrelay/secrets/.env")).unwrap(),
            "DATABASE_URL=postgres://user:pass@example/db\n"
        );
        assert_eq!(
            report.environment_variables.get("API_TOKEN").unwrap(),
            "secret-api-token"
        );
        assert_eq!(
            report.hard_exclude_patterns,
            vec![".devrelay/secrets/.env".to_string()]
        );
        assert_eq!(
            report
                .redacted_for_logs()
                .environment_variables
                .get("API_TOKEN")
                .unwrap(),
            "<redacted>"
        );
    }

    #[test]
    fn missing_required_secret_returns_error_but_optional_can_be_skipped() {
        let manifest = manifest_with_secrets(
            r#"
[secrets.required]
target = ".devrelay/secrets/required"

[secrets.optional]
target = ".devrelay/secrets/optional"
required = false
"#,
        );

        let err = materialize_project_secrets(
            Path::new("."),
            &manifest,
            &SecretProviderLocalConfig::default(),
            &FakeSecretProvider::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("missing required secret required"));

        let optional_only = manifest_with_secrets(
            r#"
[secrets.optional]
target = ".devrelay/secrets/optional"
required = false
"#,
        );
        let report = materialize_project_secrets(
            Path::new("."),
            &optional_only,
            &SecretProviderLocalConfig::default(),
            &FakeSecretProvider::default(),
        )
        .unwrap();
        assert_eq!(report.missing_optional, vec!["optional"]);
    }

    #[test]
    fn manifest_secret_file_targets_are_hard_excluded_from_snapshots() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".devrelay/secrets")).unwrap();
        fs::write(temp.path().join(".devrelay/secrets/.env"), "SECRET=1\n").unwrap();
        let mut manifest = manifest_with_secrets(
            r#"
[secrets.env_file]
target = ".devrelay/secrets/.env"
"#,
        );
        manifest.workspace.untracked = UntrackedPolicy::Explicit;
        manifest.workspace.include.patterns = vec![".devrelay/secrets/.env".to_string()];

        let decisions =
            classify_untracked_paths(temp.path(), &manifest, [".devrelay/secrets/.env"]).unwrap();

        assert_eq!(decisions[0].decision, PathDecision::Exclude);
    }
}
