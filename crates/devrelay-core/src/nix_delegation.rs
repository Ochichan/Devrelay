//! Nix task delegation planning.
//!
//! This module does not execute remote Nix builds. It produces the deterministic
//! builder file and command plan that the later remote runner can execute, while
//! reusing scheduler constraints and Nix adapter health to fail closed.

use crate::{
    DevRelayError, EnvironmentKind, NixAdapterReport, Result, SchedulerConstraintRejection,
    SchedulerDeviceSnapshot, TaskDefinition, evaluate_scheduler_constraints,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};

const NIX_DELEGATION_DIR: &str = ".devrelay/nix";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NixDelegationOptions {
    pub flake_reference: String,
    pub builder_name: String,
    pub lan_binary_cache: Option<NixLanBinaryCacheTarget>,
}

impl NixDelegationOptions {
    pub fn default_for_task(definition: &TaskDefinition) -> Self {
        Self {
            flake_reference: ".".to_string(),
            builder_name: format!(
                "devrelay-{}",
                sanitize_nix_identifier(&definition.task_name)
            ),
            lan_binary_cache: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NixLanBinaryCacheTarget {
    pub store_uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NixDelegationDecision {
    Delegated,
    NotNixTask,
    DeviceRejected,
    NixUnavailable,
    NixHealthcheckFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NixDelegationPlan {
    pub decision: NixDelegationDecision,
    pub delegated: bool,
    pub task_name: String,
    pub device_id: String,
    pub platform_key: String,
    pub builder_set: Option<NixTemporaryBuilderSet>,
    pub remote_logs: NixRemoteBuilderLogPlan,
    pub lan_binary_cache: NixLanBinaryCachePublishPlan,
    pub explanation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NixTemporaryBuilderSet {
    pub file_name: String,
    pub attr_name: String,
    pub source_reference: String,
    pub expression: String,
    pub build_command: Vec<String>,
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NixRemoteBuilderLogPlan {
    pub enabled: bool,
    pub command: Vec<String>,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NixLanBinaryCachePublishPlan {
    pub enabled: bool,
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_uri: Option<String>,
    pub explanation: String,
}

pub fn plan_nix_delegation(
    definition: &TaskDefinition,
    device: &SchedulerDeviceSnapshot,
    nix: &NixAdapterReport,
    options: NixDelegationOptions,
) -> Result<NixDelegationPlan> {
    validate_non_empty("flake_reference", &options.flake_reference)?;
    validate_non_empty("builder_name", &options.builder_name)?;

    if definition.profile_kind != EnvironmentKind::Nix {
        return Ok(disabled_plan(
            NixDelegationDecision::NotNixTask,
            definition,
            device,
            vec![format!(
                "task {} uses {:?}, so Nix delegation is not applicable",
                definition.task_name, definition.profile_kind
            )],
        ));
    }

    let constraint_decision = evaluate_scheduler_constraints(definition, device);
    if !constraint_decision.eligible {
        return Ok(disabled_plan(
            NixDelegationDecision::DeviceRejected,
            definition,
            device,
            vec![format!(
                "device {} cannot satisfy task constraints: {}",
                device.device_id,
                format_constraint_rejections(&constraint_decision.rejections)
            )],
        ));
    }

    if !nix.nix_available {
        return Ok(disabled_plan(
            NixDelegationDecision::NixUnavailable,
            definition,
            device,
            vec![format!(
                "device {} cannot run Nix because nix is unavailable",
                device.device_id
            )],
        ));
    }

    if !nix.healthcheck.shell_ready {
        return Ok(disabled_plan(
            NixDelegationDecision::NixHealthcheckFailed,
            definition,
            device,
            vec![format!(
                "device {} failed Nix healthcheck: {}",
                device.device_id, nix.healthcheck.failure_logs
            )],
        ));
    }

    let builder_set = temporary_builder_set(definition, &options);
    let remote_logs = NixRemoteBuilderLogPlan {
        enabled: true,
        command: vec![
            "nix".to_string(),
            "build".to_string(),
            "--print-build-logs".to_string(),
            "--file".to_string(),
            builder_set.file_name.clone(),
            builder_set.attr_name.clone(),
        ],
        explanation: "remote Nix build logs are streamed through nix build --print-build-logs"
            .to_string(),
    };
    let lan_binary_cache = lan_binary_cache_publish_plan(&builder_set, &options);
    let mut explanation = vec![
        format!("task {} uses a Nix profile", definition.task_name),
        format!(
            "device {} satisfies scheduler constraints for {}",
            device.device_id, device.platform_key
        ),
        "Nix healthcheck reports a ready shell".to_string(),
        format!(
            "temporary builder {} will run the task command from {} and expose declared outputs",
            builder_set.file_name, builder_set.source_reference
        ),
        remote_logs.explanation.clone(),
        lan_binary_cache.explanation.clone(),
    ];
    if let Some(fingerprint) = &nix.flake_fingerprint {
        explanation.push(format!(
            "flake fingerprint {fingerprint} participates in scheduling"
        ));
    }

    Ok(NixDelegationPlan {
        decision: NixDelegationDecision::Delegated,
        delegated: true,
        task_name: definition.task_name.clone(),
        device_id: device.device_id.clone(),
        platform_key: device.platform_key.clone(),
        builder_set: Some(builder_set),
        remote_logs,
        lan_binary_cache,
        explanation,
    })
}

pub fn write_nix_temporary_builder_set(
    root: impl AsRef<Path>,
    builder_set: &NixTemporaryBuilderSet,
) -> Result<PathBuf> {
    validate_builder_file_name(&builder_set.file_name)?;
    let path = root
        .as_ref()
        .join(NIX_DELEGATION_DIR)
        .join(&builder_set.file_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, builder_set.expression.as_bytes())?;
    Ok(path)
}

fn temporary_builder_set(
    definition: &TaskDefinition,
    options: &NixDelegationOptions,
) -> NixTemporaryBuilderSet {
    let attr_name = sanitize_nix_identifier(&options.builder_name);
    let file_name = format!("{attr_name}.nix");
    let command = shell_command(&definition.command);
    let outputs = definition.outputs.clone();
    let expression = format!(
        r#"# Generated by DevRelay. This file is safe to delete.
{{ pkgs ? import <nixpkgs> {{}} }}:

{{
  {attr_name} = pkgs.stdenv.mkDerivation {{
    name = {name};
    src = ./.;
    dontConfigure = true;
    dontBuild = false;
    buildPhase = ''
      set -euo pipefail
      {command}
    '';
    installPhase = ''
      mkdir -p "$out/devrelay-artifacts"
      cat > "$out/devrelay-artifacts/outputs.txt" <<'DEVRELAY_OUTPUTS'
{outputs_text}
DEVRELAY_OUTPUTS
    '';
    passthru.devrelayOutputs = {outputs};
  }};
}}
"#,
        attr_name = attr_name,
        name = nix_string(&attr_name),
        command = command,
        outputs_text = outputs.join("\n"),
        outputs = nix_string_list(&outputs),
    );
    NixTemporaryBuilderSet {
        file_name,
        attr_name,
        source_reference: options.flake_reference.clone(),
        expression,
        build_command: definition.command.clone(),
        outputs,
    }
}

fn lan_binary_cache_publish_plan(
    builder_set: &NixTemporaryBuilderSet,
    options: &NixDelegationOptions,
) -> NixLanBinaryCachePublishPlan {
    let Some(target) = &options.lan_binary_cache else {
        return NixLanBinaryCachePublishPlan {
            enabled: false,
            command: Vec::new(),
            store_uri: None,
            explanation:
                "LAN binary cache publish is skipped because no cache target was configured"
                    .to_string(),
        };
    };
    NixLanBinaryCachePublishPlan {
        enabled: true,
        command: vec![
            "nix".to_string(),
            "copy".to_string(),
            "--to".to_string(),
            target.store_uri.clone(),
            "--file".to_string(),
            builder_set.file_name.clone(),
            builder_set.attr_name.clone(),
        ],
        store_uri: Some(target.store_uri.clone()),
        explanation: format!(
            "successful Nix result will be copied to LAN binary cache {}",
            target.store_uri
        ),
    }
}

fn disabled_plan(
    decision: NixDelegationDecision,
    definition: &TaskDefinition,
    device: &SchedulerDeviceSnapshot,
    explanation: Vec<String>,
) -> NixDelegationPlan {
    NixDelegationPlan {
        decision,
        delegated: false,
        task_name: definition.task_name.clone(),
        device_id: device.device_id.clone(),
        platform_key: device.platform_key.clone(),
        builder_set: None,
        remote_logs: NixRemoteBuilderLogPlan {
            enabled: false,
            command: Vec::new(),
            explanation: "remote builder logs are disabled because delegation was not selected"
                .to_string(),
        },
        lan_binary_cache: NixLanBinaryCachePublishPlan {
            enabled: false,
            command: Vec::new(),
            store_uri: None,
            explanation: "LAN binary cache publish is disabled because delegation was not selected"
                .to_string(),
        },
        explanation,
    }
}

fn format_constraint_rejections(rejections: &[SchedulerConstraintRejection]) -> String {
    rejections
        .iter()
        .map(|rejection| format!("{rejection:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn shell_command(command: &[String]) -> String {
    command
        .iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/' | b':' | b'=' | b',' | b'+'))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn nix_string(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    format!("\"{escaped}\"")
}

fn nix_string_list(values: &[String]) -> String {
    format!(
        "[ {} ]",
        values
            .iter()
            .map(|value| nix_string(value))
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn sanitize_nix_identifier(value: &str) -> String {
    let mut sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while sanitized.contains("--") {
        sanitized = sanitized.replace("--", "-");
    }
    sanitized = sanitized.trim_matches('-').to_string();
    if sanitized.is_empty() {
        sanitized.push_str("task");
    }
    if sanitized
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_digit())
    {
        sanitized.insert_str(0, "task-");
    }
    sanitized
}

fn validate_builder_file_name(file_name: &str) -> Result<()> {
    let path = Path::new(file_name);
    let mut components = path.components();
    let single_normal_component =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if file_name.trim().is_empty() || !single_normal_component || !file_name.ends_with(".nix") {
        return Err(DevRelayError::Config(format!(
            "Nix builder file name must be a safe .nix file name: {file_name:?}"
        )));
    }
    Ok(())
}

fn validate_non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(DevRelayError::Config(format!(
            "Nix delegation {field} must not be empty"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ForegroundLoad, NixCacheWarmth, NixDevelopHealthcheck, NixFlakeFiles, NixHealthState,
        NixPlaceholderPlan, ResourcePowerSource, SchedulerDevicePolicy, SchedulerDynamicResources,
        SchedulerNetworkRouteQuality, TaskCacheMode,
    };
    use std::path::PathBuf;

    #[test]
    fn rejects_non_nix_tasks() {
        let definition = task_definition(EnvironmentKind::Native);

        let plan = plan_nix_delegation(
            &definition,
            &device(),
            &nix_report(true, true),
            NixDelegationOptions::default_for_task(&definition),
        )
        .unwrap();

        assert_eq!(plan.decision, NixDelegationDecision::NotNixTask);
        assert!(!plan.delegated);
        assert!(plan.builder_set.is_none());
    }

    #[test]
    fn plans_nix_delegation_with_builder_logs_and_binary_cache() {
        let temp = tempfile::tempdir().unwrap();
        let definition = task_definition(EnvironmentKind::Nix);
        let mut options = NixDelegationOptions::default_for_task(&definition);
        options.lan_binary_cache = Some(NixLanBinaryCacheTarget {
            store_uri: "ssh://anchor-cache".to_string(),
            public_key: Some("anchor-cache.pub".to_string()),
        });

        let plan =
            plan_nix_delegation(&definition, &device(), &nix_report(true, true), options).unwrap();

        assert_eq!(plan.decision, NixDelegationDecision::Delegated);
        assert!(plan.delegated);
        let builder = plan.builder_set.as_ref().unwrap();
        assert_eq!(builder.file_name, "devrelay-build.nix");
        assert_eq!(builder.source_reference, ".");
        assert!(builder.expression.contains("cargo build --locked"));
        assert!(builder.expression.contains("dist/**"));
        assert!(plan.remote_logs.enabled);
        assert!(
            plan.remote_logs
                .command
                .iter()
                .any(|arg| arg == "--print-build-logs")
        );
        assert!(plan.lan_binary_cache.enabled);
        assert_eq!(
            plan.lan_binary_cache.command,
            vec![
                "nix",
                "copy",
                "--to",
                "ssh://anchor-cache",
                "--file",
                "devrelay-build.nix",
                "devrelay-build"
            ]
        );
        assert!(
            plan.explanation
                .iter()
                .any(|line| line.contains("Nix healthcheck"))
        );

        let path = write_nix_temporary_builder_set(temp.path(), builder).unwrap();
        assert_eq!(
            path.strip_prefix(temp.path()).unwrap(),
            PathBuf::from(".devrelay/nix/devrelay-build.nix")
        );
        assert_eq!(fs::read_to_string(path).unwrap(), builder.expression);

        let mut unsafe_builder = builder.clone();
        unsafe_builder.file_name = "nested/builder.nix".to_string();
        assert!(write_nix_temporary_builder_set(temp.path(), &unsafe_builder).is_err());
    }

    #[test]
    fn respects_scheduler_constraints_before_delegating() {
        let mut definition = task_definition(EnvironmentKind::Nix);
        definition.platforms = vec!["linux-*".to_string()];
        let mut device = device();
        device.platform_key = "darwin-arm64".to_string();

        let plan = plan_nix_delegation(
            &definition,
            &device,
            &nix_report(true, true),
            NixDelegationOptions::default_for_task(&definition),
        )
        .unwrap();

        assert_eq!(plan.decision, NixDelegationDecision::DeviceRejected);
        assert!(!plan.delegated);
        assert!(
            plan.explanation
                .iter()
                .any(|line| line.contains("IncompatiblePlatform"))
        );
    }

    #[test]
    fn rejects_unavailable_or_unhealthy_nix_devices() {
        let definition = task_definition(EnvironmentKind::Nix);

        let unavailable = plan_nix_delegation(
            &definition,
            &device(),
            &nix_report(false, false),
            NixDelegationOptions::default_for_task(&definition),
        )
        .unwrap();
        assert_eq!(unavailable.decision, NixDelegationDecision::NixUnavailable);

        let unhealthy = plan_nix_delegation(
            &definition,
            &device(),
            &nix_report(true, false),
            NixDelegationOptions::default_for_task(&definition),
        )
        .unwrap();
        assert_eq!(
            unhealthy.decision,
            NixDelegationDecision::NixHealthcheckFailed
        );
        assert!(
            unhealthy
                .explanation
                .iter()
                .any(|line| line.contains("failed Nix healthcheck"))
        );
    }

    fn task_definition(kind: EnvironmentKind) -> TaskDefinition {
        TaskDefinition {
            project_id: "12345678".to_string(),
            task_name: "build".to_string(),
            profile_name: "dev".to_string(),
            profile_kind: kind,
            command: vec![
                "cargo".to_string(),
                "build".to_string(),
                "--locked".to_string(),
            ],
            platforms: vec!["darwin-*".to_string()],
            cpu: Some(2),
            memory_mib: Some(512),
            disk_mib: Some(512),
            interactive: false,
            cache: Some(TaskCacheMode::ReadWrite),
            outputs: vec!["dist/**".to_string()],
            features: Vec::new(),
            sandbox: None,
            command_definition_hash: "c".repeat(64),
        }
    }

    fn device() -> SchedulerDeviceSnapshot {
        SchedulerDeviceSnapshot {
            device_id: "dev_local".to_string(),
            platform_key: "darwin-arm64".to_string(),
            os: "darwin".to_string(),
            architecture: "arm64".to_string(),
            cpu_cores: 8,
            memory_total_mib: Some(32_768),
            disk_total_mib: Some(512_000),
            features: vec!["local-snapshots".to_string()],
            policy: SchedulerDevicePolicy::default(),
            dynamic: SchedulerDynamicResources {
                cpu_load_1m_milli: Some(100),
                memory_free_mib: Some(16_384),
                disk_free_mib: Some(256_000),
                power_source: ResourcePowerSource::Ac,
                low_power_mode: false,
                foreground_load: ForegroundLoad::Idle,
                network_route_quality: SchedulerNetworkRouteQuality::Good,
            },
        }
    }

    fn nix_report(nix_available: bool, shell_ready: bool) -> NixAdapterReport {
        NixAdapterReport {
            nix_available,
            flake_files: NixFlakeFiles {
                files: vec![PathBuf::from("flake.nix"), PathBuf::from("flake.lock")],
            },
            flake_fingerprint: Some("flake-fingerprint".to_string()),
            healthcheck: NixDevelopHealthcheck {
                state: if shell_ready {
                    NixHealthState::ShellReady
                } else if nix_available {
                    NixHealthState::Failed
                } else {
                    NixHealthState::Unavailable
                },
                command: vec![
                    "develop".to_string(),
                    "--command".to_string(),
                    "true".to_string(),
                ],
                shell_ready,
                exit_code: shell_ready.then_some(0).or(Some(1)),
                stdout: if shell_ready {
                    "ready".to_string()
                } else {
                    String::new()
                },
                stderr: if shell_ready {
                    String::new()
                } else {
                    "failed".to_string()
                },
                failure_logs: if shell_ready {
                    String::new()
                } else {
                    "failed".to_string()
                },
            },
            store_prefetch: NixPlaceholderPlan {
                enabled: false,
                reason: "not needed for test".to_string(),
            },
            lan_binary_cache: NixPlaceholderPlan {
                enabled: true,
                reason: "configured for test".to_string(),
            },
            cache_warmth: NixCacheWarmth {
                platform_key: "darwin-arm64".to_string(),
                score: 80,
                explanation: "warm".to_string(),
            },
        }
    }
}
