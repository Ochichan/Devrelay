//! Environment hydration diagnostics.
//!
//! The doctor reports blockers before or during hydration without mutating Git
//! state. Expensive or side-effectful healthchecks are opt-in through
//! `EnvironmentDoctorOptions`.

use crate::{
    CommandTrustEvaluation, CommandTrustStatus, DevContainerHealthState, DevContainerPrepareState,
    DevRelayError, EnvironmentCommand, EnvironmentCommandRunner, EnvironmentKind,
    EnvironmentProfile, EnvironmentSelectionContext, LogRedactor, Manifest, NativeBootstrapShell,
    NativeBootstrapState, NativeHealthState, NixHealthState, Result, SecretMode,
    SecretProviderLocalConfig, classify_native_command, detect_container_engine,
    detect_nix_availability, environment_profile_command_scope, inspect_devcontainer_environment,
    inspect_native_environment, profile_targets_platform, run_nix_develop_healthcheck,
    select_environment_profile,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentDoctorOptions {
    pub platform_key: String,
    pub command_trust: Option<CommandTrustEvaluation>,
    pub run_healthcheck: bool,
    pub allow_devcontainer_prepare: bool,
}

impl EnvironmentDoctorOptions {
    pub fn for_platform(platform_key: impl Into<String>) -> Self {
        Self {
            platform_key: platform_key.into(),
            command_trust: None,
            run_healthcheck: false,
            allow_devcontainer_prepare: false,
        }
    }

    pub fn with_command_trust(mut self, command_trust: CommandTrustEvaluation) -> Self {
        self.command_trust = Some(command_trust);
        self
    }

    pub fn with_run_healthcheck(mut self, run_healthcheck: bool) -> Self {
        self.run_healthcheck = run_healthcheck;
        self
    }

    pub fn with_allow_devcontainer_prepare(mut self, allow_devcontainer_prepare: bool) -> Self {
        self.allow_devcontainer_prepare = allow_devcontainer_prepare;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentDoctorReport {
    pub repo: PathBuf,
    pub platform_key: String,
    pub selected_profile_name: Option<String>,
    pub selected_profile_kind: Option<EnvironmentKind>,
    pub selection_explanation: Vec<String>,
    pub nix_available: bool,
    pub container_engine: Option<String>,
    pub powershell_available: bool,
    pub required_secret_count: usize,
    pub mapped_required_secret_count: usize,
    pub issues: Vec<EnvironmentDoctorIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentDoctorIssue {
    pub code: EnvironmentDoctorIssueCode,
    pub profile_name: Option<String>,
    pub secret_name: Option<String>,
    pub message: String,
    pub detail: Option<String>,
    pub safe_actions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum EnvironmentDoctorIssueCode {
    MissingNix,
    MissingContainerEngine,
    MissingPowerShell,
    ChangedCommandHash,
    MissingRequiredSecret,
    IncompatiblePlatformTarget,
    HealthcheckFailure,
}

pub fn run_environment_doctor(
    root: &Path,
    manifest: &Manifest,
    local_secrets: &SecretProviderLocalConfig,
    options: &EnvironmentDoctorOptions,
    runner: &impl EnvironmentCommandRunner,
) -> Result<EnvironmentDoctorReport> {
    let nix_available = detect_nix_availability(root, runner)?;
    let container_engine = detect_container_engine(root, runner)?;
    let powershell_available = detect_powershell_availability(root, runner)?;
    let mut available_kinds = BTreeSet::from([EnvironmentKind::Manual, EnvironmentKind::Native]);
    if nix_available {
        available_kinds.insert(EnvironmentKind::Nix);
    }
    if container_engine.is_some() {
        available_kinds.insert(EnvironmentKind::Devcontainer);
    }
    if command_trust_is_approved(options.command_trust.as_ref()) {
        available_kinds.insert(EnvironmentKind::Script);
    }

    let mut context = EnvironmentSelectionContext::with_platform_key(options.platform_key.clone());
    for kind in available_kinds {
        context = context.with_available_kind(kind);
    }
    if command_trust_is_approved(options.command_trust.as_ref())
        && let Some(environment) = &manifest.environment
    {
        for name in environment.profiles.keys() {
            context = context.with_trusted_command_scope(environment_profile_command_scope(name));
        }
    }

    let selection = select_environment_profile(manifest, &context);
    let mut issues = Vec::new();
    collect_profile_issues(
        manifest,
        options,
        nix_available,
        container_engine.is_some(),
        powershell_available,
        &mut issues,
    );
    collect_command_trust_issues(options.command_trust.as_ref(), &mut issues);
    let (required_secret_count, mapped_required_secret_count) =
        collect_secret_issues(manifest, local_secrets, &mut issues);
    if options.run_healthcheck
        && let Some(profile_name) = selection.profile_name.as_deref()
        && let Some(profile) = manifest
            .environment
            .as_ref()
            .and_then(|environment| environment.profiles.get(profile_name))
    {
        collect_healthcheck_issue(
            root,
            profile_name,
            profile,
            nix_available,
            options,
            runner,
            &mut issues,
        )?;
    }

    Ok(EnvironmentDoctorReport {
        repo: root.to_path_buf(),
        platform_key: options.platform_key.clone(),
        selected_profile_name: selection.profile_name,
        selected_profile_kind: selection.kind,
        selection_explanation: selection.explanation,
        nix_available,
        container_engine: container_engine.map(|engine| engine.command().to_string()),
        powershell_available,
        required_secret_count,
        mapped_required_secret_count,
        issues,
    })
}

fn collect_profile_issues(
    manifest: &Manifest,
    options: &EnvironmentDoctorOptions,
    nix_available: bool,
    container_engine_available: bool,
    powershell_available: bool,
    issues: &mut Vec<EnvironmentDoctorIssue>,
) {
    let Some(environment) = &manifest.environment else {
        return;
    };
    if !environment.profiles.is_empty()
        && !environment
            .profiles
            .values()
            .any(|profile| profile_targets_platform(&profile.targets, &options.platform_key))
    {
        issues.push(issue(
            EnvironmentDoctorIssueCode::IncompatiblePlatformTarget,
            None,
            None,
            format!(
                "No environment profile targets platform {}.",
                options.platform_key
            ),
            None,
        ));
    }

    for (name, profile) in &environment.profiles {
        if !profile_targets_platform(&profile.targets, &options.platform_key) {
            continue;
        }
        match profile.kind {
            EnvironmentKind::Nix if !nix_available => issues.push(issue(
                EnvironmentDoctorIssueCode::MissingNix,
                Some(name.clone()),
                None,
                format!("Profile {name} requires Nix, but the nix command is unavailable."),
                None,
            )),
            EnvironmentKind::Devcontainer if !container_engine_available => issues.push(issue(
                EnvironmentDoctorIssueCode::MissingContainerEngine,
                Some(name.clone()),
                None,
                format!(
                    "Profile {name} requires a Dev Container, but Docker or Podman is unavailable."
                ),
                None,
            )),
            EnvironmentKind::Native | EnvironmentKind::Script
                if profile_uses_powershell(profile) && !powershell_available =>
            {
                issues.push(issue(
                    EnvironmentDoctorIssueCode::MissingPowerShell,
                    Some(name.clone()),
                    None,
                    format!(
                        "Profile {name} requires PowerShell, but no PowerShell command was found."
                    ),
                    None,
                ));
            }
            _ => {}
        }
    }
}

fn collect_command_trust_issues(
    command_trust: Option<&CommandTrustEvaluation>,
    issues: &mut Vec<EnvironmentDoctorIssue>,
) {
    if command_trust.is_some_and(|evaluation| evaluation.status == CommandTrustStatus::Changed) {
        issues.push(issue(
            EnvironmentDoctorIssueCode::ChangedCommandHash,
            None,
            None,
            "Executable manifest command content changed since the last approval.".to_string(),
            None,
        ));
    }
}

fn collect_secret_issues(
    manifest: &Manifest,
    local_secrets: &SecretProviderLocalConfig,
    issues: &mut Vec<EnvironmentDoctorIssue>,
) -> (usize, usize) {
    let mut required_secret_count = 0;
    let mut mapped_required_secret_count = 0;
    for (name, secret) in &manifest.secrets {
        if !secret.required {
            continue;
        }
        required_secret_count += 1;
        if local_secrets.mappings.contains_key(name) {
            mapped_required_secret_count += 1;
            continue;
        }
        let target = match secret.mode {
            SecretMode::File => format!("file {}", secret.target),
            SecretMode::Environment => secret
                .environment_variable
                .as_deref()
                .unwrap_or(&secret.target)
                .to_string(),
        };
        issues.push(issue(
            EnvironmentDoctorIssueCode::MissingRequiredSecret,
            None,
            Some(name.clone()),
            format!("Required secret {name} has no local provider mapping for {target}."),
            None,
        ));
    }
    (required_secret_count, mapped_required_secret_count)
}

fn collect_healthcheck_issue(
    root: &Path,
    profile_name: &str,
    profile: &EnvironmentProfile,
    nix_available: bool,
    options: &EnvironmentDoctorOptions,
    runner: &impl EnvironmentCommandRunner,
    issues: &mut Vec<EnvironmentDoctorIssue>,
) -> Result<()> {
    let redactor = LogRedactor::new();
    let healthcheck = profile.healthcheck.clone().unwrap_or_default();
    match profile.kind {
        EnvironmentKind::Nix => {
            let report = run_nix_develop_healthcheck(root, &healthcheck, nix_available, runner)?;
            if report.state == NixHealthState::Failed {
                issues.push(issue(
                    EnvironmentDoctorIssueCode::HealthcheckFailure,
                    Some(profile_name.to_string()),
                    None,
                    format!("Profile {profile_name} Nix healthcheck failed."),
                    Some(redactor.redact_text(&report.failure_logs)),
                ));
            }
        }
        EnvironmentKind::Devcontainer => {
            let report = inspect_devcontainer_environment(
                root,
                &healthcheck,
                options.allow_devcontainer_prepare,
                runner,
            )?;
            if report.image_prepare.state == DevContainerPrepareState::Failed {
                issues.push(issue(
                    EnvironmentDoctorIssueCode::HealthcheckFailure,
                    Some(profile_name.to_string()),
                    None,
                    format!("Profile {profile_name} Dev Container preparation failed."),
                    Some(redactor.redact_text(&report.image_prepare.logs)),
                ));
            } else if report.healthcheck.state == DevContainerHealthState::Failed {
                issues.push(issue(
                    EnvironmentDoctorIssueCode::HealthcheckFailure,
                    Some(profile_name.to_string()),
                    None,
                    format!("Profile {profile_name} Dev Container healthcheck failed."),
                    Some(redactor.redact_text(&report.healthcheck.failure_logs)),
                ));
            }
        }
        EnvironmentKind::Native | EnvironmentKind::Script => {
            if !command_trust_is_approved(options.command_trust.as_ref()) {
                return Ok(());
            }
            let report = inspect_native_environment(root, profile, true, runner)?;
            if matches!(
                report.bootstrap.state,
                NativeBootstrapState::Failed | NativeBootstrapState::TimedOut
            ) {
                issues.push(issue(
                    EnvironmentDoctorIssueCode::HealthcheckFailure,
                    Some(profile_name.to_string()),
                    None,
                    format!("Profile {profile_name} native bootstrap failed."),
                    Some(redactor.redact_text(&report.bootstrap.logs)),
                ));
            } else if matches!(
                report.healthcheck.state,
                NativeHealthState::Failed | NativeHealthState::TimedOut
            ) {
                issues.push(issue(
                    EnvironmentDoctorIssueCode::HealthcheckFailure,
                    Some(profile_name.to_string()),
                    None,
                    format!("Profile {profile_name} native healthcheck failed."),
                    Some(redactor.redact_text(&report.healthcheck.failure_logs)),
                ));
            }
        }
        EnvironmentKind::Manual => {}
    }
    Ok(())
}

fn detect_powershell_availability(
    root: &Path,
    runner: &impl EnvironmentCommandRunner,
) -> Result<bool> {
    for program in ["pwsh", "powershell", "powershell.exe", "pwsh.exe"] {
        match runner.run(root, &EnvironmentCommand::new(program, ["--version"])) {
            Ok(output) if output.succeeded() => return Ok(true),
            Ok(_) => {}
            Err(DevRelayError::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(false)
}

fn profile_uses_powershell(profile: &EnvironmentProfile) -> bool {
    classify_native_command(&profile.command) == NativeBootstrapShell::PowerShell
        || profile.healthcheck.as_ref().is_some_and(|command| {
            classify_native_command(command) == NativeBootstrapShell::PowerShell
        })
}

fn command_trust_is_approved(command_trust: Option<&CommandTrustEvaluation>) -> bool {
    command_trust.is_some_and(|evaluation| evaluation.status.approved())
}

fn issue(
    code: EnvironmentDoctorIssueCode,
    profile_name: Option<String>,
    secret_name: Option<String>,
    message: String,
    detail: Option<String>,
) -> EnvironmentDoctorIssue {
    EnvironmentDoctorIssue {
        code,
        profile_name,
        secret_name,
        message,
        detail,
        safe_actions: safe_actions_for(code),
    }
}

fn safe_actions_for(code: EnvironmentDoctorIssueCode) -> Vec<String> {
    match code {
        EnvironmentDoctorIssueCode::MissingNix => vec![
            "Install Nix and ensure `nix --version` succeeds in this shell.".to_string(),
            "Select a compatible Dev Container, native, or manual profile for this device."
                .to_string(),
        ],
        EnvironmentDoctorIssueCode::MissingContainerEngine => vec![
            "Install and start Docker or Podman before hydrating the Dev Container profile."
                .to_string(),
            "Select a compatible Nix, native, or manual profile for this device.".to_string(),
        ],
        EnvironmentDoctorIssueCode::MissingPowerShell => vec![
            "Install PowerShell 7 and ensure `pwsh --version` succeeds in this shell.".to_string(),
            "Change the profile command or healthcheck to a shell available on this platform."
                .to_string(),
        ],
        EnvironmentDoctorIssueCode::ChangedCommandHash => vec![
            "Review executable command, task, healthcheck, and fingerprint file changes."
                .to_string(),
            "Approve the new command hash only if the changed commands are expected.".to_string(),
        ],
        EnvironmentDoctorIssueCode::MissingRequiredSecret => vec![
            "Add a local secret provider mapping for this required secret.".to_string(),
            "Mark the manifest secret optional only if hydration can proceed without it."
                .to_string(),
        ],
        EnvironmentDoctorIssueCode::IncompatiblePlatformTarget => vec![
            "Add this platform key or a compatible alias to an environment profile target."
                .to_string(),
            "Run hydration from a device whose platform matches one of the declared targets."
                .to_string(),
        ],
        EnvironmentDoctorIssueCode::HealthcheckFailure => vec![
            "Inspect the healthcheck logs and fix the failing toolchain or command.".to_string(),
            "Retry hydration after the profile healthcheck succeeds locally.".to_string(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CommandTrustDecision, CommandTrustRecord, EnvironmentCommandOutput, EnvironmentConfig,
        SecretConfig, SecretProviderKind, SecretProviderMapping,
    };
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct FakeRunner {
        outputs: BTreeMap<Vec<String>, EnvironmentCommandOutput>,
    }

    impl FakeRunner {
        fn with_output(
            mut self,
            program: &str,
            args: &[&str],
            output: EnvironmentCommandOutput,
        ) -> Self {
            let mut key = vec![program.to_string()];
            key.extend(args.iter().map(|arg| (*arg).to_string()));
            self.outputs.insert(key, output);
            self
        }
    }

    impl EnvironmentCommandRunner for FakeRunner {
        fn run(
            &self,
            _cwd: &Path,
            command: &EnvironmentCommand,
        ) -> Result<EnvironmentCommandOutput> {
            let mut key = vec![command.program.clone()];
            key.extend(command.args.iter().cloned());
            Ok(self
                .outputs
                .get(&key)
                .cloned()
                .unwrap_or_else(|| EnvironmentCommandOutput::failure(127, "command not found")))
        }
    }

    fn manifest_with_profiles(
        profiles: impl IntoIterator<
            Item = (
                &'static str,
                EnvironmentKind,
                Vec<&'static str>,
                Vec<&'static str>,
            ),
        >,
    ) -> Manifest {
        let mut manifest = Manifest::parse(
            r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#,
        )
        .unwrap();
        let profiles = profiles
            .into_iter()
            .map(|(name, kind, targets, command)| {
                (
                    name.to_string(),
                    EnvironmentProfile {
                        kind,
                        targets: targets.into_iter().map(str::to_string).collect(),
                        command: command.into_iter().map(str::to_string).collect(),
                        fingerprint_files: Vec::new(),
                        healthcheck: None,
                        working_directory: None,
                        timeout_seconds: None,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        manifest.environment = Some(EnvironmentConfig { profiles });
        manifest
    }

    fn changed_trust_evaluation() -> CommandTrustEvaluation {
        CommandTrustEvaluation {
            status: CommandTrustStatus::Changed,
            approval: Some(CommandTrustRecord {
                approval_id: 1,
                project_id: "12345678".to_string(),
                device_id: "device-a".to_string(),
                command_scope: "manifest".to_string(),
                command_hash: "old".to_string(),
                decision: CommandTrustDecision::TrustThisVersion,
                consumed_at_unix_seconds: None,
                created_at_unix_seconds: 1,
            }),
            previous_hash: Some("old".to_string()),
        }
    }

    #[test]
    fn environment_doctor_reports_missing_tools_secrets_and_changed_trust() {
        let mut manifest = manifest_with_profiles([
            ("nix", EnvironmentKind::Nix, vec!["darwin"], vec!["nix"]),
            (
                "dev",
                EnvironmentKind::Devcontainer,
                vec!["darwin"],
                vec!["devcontainer"],
            ),
            (
                "native",
                EnvironmentKind::Native,
                vec!["darwin"],
                vec!["pwsh", "-NoProfile", "-Command", "./bootstrap.ps1"],
            ),
        ]);
        manifest.secrets.insert(
            "api_token".to_string(),
            SecretConfig {
                target: ".devrelay/secrets/api_token".to_string(),
                required: true,
                mode: SecretMode::File,
                environment_variable: None,
            },
        );

        let report = run_environment_doctor(
            Path::new("/repo"),
            &manifest,
            &SecretProviderLocalConfig::default(),
            &EnvironmentDoctorOptions::for_platform("darwin-arm64")
                .with_command_trust(changed_trust_evaluation()),
            &FakeRunner::default(),
        )
        .unwrap();

        let codes = report
            .issues
            .iter()
            .map(|issue| issue.code)
            .collect::<BTreeSet<_>>();
        assert!(codes.contains(&EnvironmentDoctorIssueCode::MissingNix));
        assert!(codes.contains(&EnvironmentDoctorIssueCode::MissingContainerEngine));
        assert!(codes.contains(&EnvironmentDoctorIssueCode::MissingPowerShell));
        assert!(codes.contains(&EnvironmentDoctorIssueCode::ChangedCommandHash));
        assert!(codes.contains(&EnvironmentDoctorIssueCode::MissingRequiredSecret));
        assert!(
            report
                .issues
                .iter()
                .all(|issue| !issue.safe_actions.is_empty())
        );
        assert_eq!(report.required_secret_count, 1);
        assert_eq!(report.mapped_required_secret_count, 0);
    }

    #[test]
    fn environment_doctor_reports_incompatible_platform_target() {
        let manifest = manifest_with_profiles([(
            "manual",
            EnvironmentKind::Manual,
            vec!["windows-native"],
            vec!["echo", "manual"],
        )]);

        let report = run_environment_doctor(
            Path::new("/repo"),
            &manifest,
            &SecretProviderLocalConfig::default(),
            &EnvironmentDoctorOptions::for_platform("darwin-arm64"),
            &FakeRunner::default(),
        )
        .unwrap();

        assert!(
            report.issues.iter().any(|issue| {
                issue.code == EnvironmentDoctorIssueCode::IncompatiblePlatformTarget
            })
        );
        assert_eq!(report.selected_profile_name, None);
    }

    #[test]
    fn environment_doctor_reports_nix_healthcheck_failure() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("flake.nix"), "{ outputs = _: {}; }\n").unwrap();
        let mut manifest =
            manifest_with_profiles([("nix", EnvironmentKind::Nix, vec!["darwin"], vec!["nix"])]);
        manifest
            .environment
            .as_mut()
            .unwrap()
            .profiles
            .get_mut("nix")
            .unwrap()
            .healthcheck = Some(vec!["cargo".to_string(), "test".to_string()]);
        let runner = FakeRunner::default()
            .with_output(
                "nix",
                &["--version"],
                EnvironmentCommandOutput::success("nix"),
            )
            .with_output(
                "nix",
                &["develop", "--command", "cargo", "test"],
                EnvironmentCommandOutput::failure(1, "token=secret failed"),
            );

        let report = run_environment_doctor(
            temp.path(),
            &manifest,
            &SecretProviderLocalConfig::default(),
            &EnvironmentDoctorOptions::for_platform("darwin-arm64").with_run_healthcheck(true),
            &runner,
        )
        .unwrap();

        let issue = report
            .issues
            .iter()
            .find(|issue| issue.code == EnvironmentDoctorIssueCode::HealthcheckFailure)
            .expect("healthcheck issue");
        assert_eq!(issue.profile_name.as_deref(), Some("nix"));
        assert!(
            !issue
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("secret")
        );
    }

    #[test]
    fn environment_doctor_counts_mapped_required_secrets() {
        let mut manifest = manifest_with_profiles([]);
        manifest.secrets.insert(
            "api_token".to_string(),
            SecretConfig {
                target: "API_TOKEN".to_string(),
                required: true,
                mode: SecretMode::Environment,
                environment_variable: Some("API_TOKEN".to_string()),
            },
        );
        let local_secrets = SecretProviderLocalConfig {
            mappings: BTreeMap::from([(
                "api_token".to_string(),
                SecretProviderMapping {
                    provider: SecretProviderKind::OnePasswordCli,
                    reference: "op://vault/item/password".to_string(),
                    command: Vec::new(),
                },
            )]),
        };

        let report = run_environment_doctor(
            Path::new("/repo"),
            &manifest,
            &local_secrets,
            &EnvironmentDoctorOptions::for_platform("darwin-arm64"),
            &FakeRunner::default(),
        )
        .unwrap();

        assert_eq!(report.required_secret_count, 1);
        assert_eq!(report.mapped_required_secret_count, 1);
        assert!(
            !report
                .issues
                .iter()
                .any(|issue| { issue.code == EnvironmentDoctorIssueCode::MissingRequiredSecret })
        );
    }
}
