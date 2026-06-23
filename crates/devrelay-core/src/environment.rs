//! Environment profile selection policy.
//!
//! This module does not execute environment commands. It chooses the profile
//! that a runner should hydrate after checking platform targets, adapter
//! availability, and bootstrap trust state.

use crate::{EnvironmentKind, Manifest, current_platform_key};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentSelectionContext {
    pub platform_key: String,
    pub available_kinds: BTreeSet<EnvironmentKind>,
    pub trusted_command_scopes: BTreeSet<String>,
}

impl EnvironmentSelectionContext {
    pub fn current() -> Self {
        Self {
            platform_key: current_platform_key(),
            available_kinds: BTreeSet::from([EnvironmentKind::Manual]),
            trusted_command_scopes: BTreeSet::new(),
        }
    }

    pub fn with_platform_key(platform_key: impl Into<String>) -> Self {
        Self {
            platform_key: platform_key.into(),
            available_kinds: BTreeSet::from([EnvironmentKind::Manual]),
            trusted_command_scopes: BTreeSet::new(),
        }
    }

    pub fn with_available_kind(mut self, kind: EnvironmentKind) -> Self {
        self.available_kinds.insert(kind);
        self
    }

    pub fn with_trusted_command_scope(mut self, scope: impl Into<String>) -> Self {
        self.trusted_command_scopes.insert(scope.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentProfileSelection {
    pub profile_name: Option<String>,
    pub kind: Option<EnvironmentKind>,
    pub command_scope: Option<String>,
    pub explanation: Vec<String>,
}

pub fn environment_profile_command_scope(profile_name: &str) -> String {
    format!("environment.profile.{profile_name}")
}

pub fn select_environment_profile(
    manifest: &Manifest,
    context: &EnvironmentSelectionContext,
) -> EnvironmentProfileSelection {
    let Some(environment) = &manifest.environment else {
        return EnvironmentProfileSelection {
            profile_name: None,
            kind: None,
            command_scope: None,
            explanation: vec!["manifest has no environment profiles".to_string()],
        };
    };

    let profiles = &environment.profiles;
    let mut explanation = Vec::new();
    for kind in [
        EnvironmentKind::Nix,
        EnvironmentKind::Devcontainer,
        EnvironmentKind::Native,
        EnvironmentKind::Script,
        EnvironmentKind::Manual,
    ] {
        for (name, profile) in profiles.iter().filter(|(_, profile)| profile.kind == kind) {
            let scope = environment_profile_command_scope(name);
            if !profile_targets_platform(&profile.targets, &context.platform_key) {
                explanation.push(format!(
                    "skipped {name}: targets do not match {}",
                    context.platform_key
                ));
                continue;
            }
            if !kind_available(kind, &context.available_kinds) {
                explanation.push(format!("skipped {name}: {:?} adapter unavailable", kind));
                continue;
            }
            if kind == EnvironmentKind::Script && !context.trusted_command_scopes.contains(&scope) {
                explanation.push(format!("skipped {name}: bootstrap command is not trusted"));
                continue;
            }

            explanation.push(format!(
                "selected {name}: {:?} profile matches {}",
                kind, context.platform_key
            ));
            return EnvironmentProfileSelection {
                profile_name: Some(name.clone()),
                kind: Some(kind),
                command_scope: Some(scope),
                explanation,
            };
        }
    }

    if profiles.is_empty() {
        explanation.push("manifest environment has no profiles".to_string());
    } else {
        explanation.push(format!(
            "no environment profile matches {} with available adapters",
            context.platform_key
        ));
    }
    EnvironmentProfileSelection {
        profile_name: None,
        kind: None,
        command_scope: None,
        explanation,
    }
}

pub fn profile_targets_platform(targets: &[String], platform_key: &str) -> bool {
    targets
        .iter()
        .any(|target| target_matches_platform_key(target, platform_key))
}

fn kind_available(kind: EnvironmentKind, available_kinds: &BTreeSet<EnvironmentKind>) -> bool {
    kind == EnvironmentKind::Manual || available_kinds.contains(&kind)
}

fn target_matches_platform_key(target: &str, platform_key: &str) -> bool {
    let target = target.trim();
    if target == "*" || target == "local" || target == platform_key {
        return true;
    }
    if let Some(prefix) = target.strip_suffix("-*") {
        return platform_key.starts_with(prefix);
    }
    platform_aliases(platform_key).contains(target)
}

fn platform_aliases(platform_key: &str) -> BTreeSet<&'static str> {
    if platform_key.starts_with("darwin-") {
        BTreeSet::from(["darwin", "macos"])
    } else if platform_key.starts_with("wsl2-linux-gnu-") {
        BTreeSet::from(["wsl2", "linux", "linux-gnu"])
    } else if platform_key.starts_with("linux-gnu-") {
        BTreeSet::from(["linux", "linux-gnu"])
    } else if platform_key.starts_with("windows-native-") {
        BTreeSet::from(["windows", "windows-native"])
    } else {
        BTreeSet::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EnvironmentConfig, EnvironmentProfile, UntrackedPolicy};
    use std::collections::BTreeMap;

    fn manifest_with_profiles(
        profiles: impl IntoIterator<Item = (&'static str, EnvironmentKind, Vec<&'static str>)>,
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
            .map(|(name, kind, targets)| {
                (
                    name.to_string(),
                    EnvironmentProfile {
                        kind,
                        targets: targets.into_iter().map(str::to_string).collect(),
                        command: vec!["echo".to_string(), name.to_string()],
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

    #[test]
    fn selection_context_detects_current_platform_key() {
        let context = EnvironmentSelectionContext::current();

        assert!(!context.platform_key.is_empty());
        assert!(context.available_kinds.contains(&EnvironmentKind::Manual));
    }

    #[test]
    fn matches_manifest_profile_targets_with_platform_aliases() {
        assert!(profile_targets_platform(
            &["darwin".to_string()],
            "darwin-arm64"
        ));
        assert!(profile_targets_platform(
            &["linux-gnu-*".to_string()],
            "linux-gnu-x86_64"
        ));
        assert!(profile_targets_platform(
            &["local".to_string()],
            "windows-native-x86_64"
        ));
        assert!(!profile_targets_platform(
            &["windows-native".to_string()],
            "darwin-arm64"
        ));
    }

    #[test]
    fn prefers_nix_then_devcontainer_then_native_then_trusted_script_then_manual() {
        let manifest = manifest_with_profiles([
            ("manual", EnvironmentKind::Manual, vec!["darwin"]),
            ("script", EnvironmentKind::Script, vec!["darwin"]),
            ("native", EnvironmentKind::Native, vec!["darwin"]),
            ("dev", EnvironmentKind::Devcontainer, vec!["darwin"]),
            ("nix", EnvironmentKind::Nix, vec!["darwin"]),
        ]);
        let script_scope = environment_profile_command_scope("script");

        let nix = select_environment_profile(
            &manifest,
            &EnvironmentSelectionContext::with_platform_key("darwin-arm64")
                .with_available_kind(EnvironmentKind::Nix)
                .with_available_kind(EnvironmentKind::Devcontainer)
                .with_available_kind(EnvironmentKind::Native)
                .with_available_kind(EnvironmentKind::Script)
                .with_trusted_command_scope(script_scope.clone()),
        );
        assert_eq!(nix.profile_name.as_deref(), Some("nix"));

        let devcontainer = select_environment_profile(
            &manifest,
            &EnvironmentSelectionContext::with_platform_key("darwin-arm64")
                .with_available_kind(EnvironmentKind::Devcontainer)
                .with_available_kind(EnvironmentKind::Native)
                .with_available_kind(EnvironmentKind::Script)
                .with_trusted_command_scope(script_scope.clone()),
        );
        assert_eq!(devcontainer.profile_name.as_deref(), Some("dev"));

        let native = select_environment_profile(
            &manifest,
            &EnvironmentSelectionContext::with_platform_key("darwin-arm64")
                .with_available_kind(EnvironmentKind::Native)
                .with_available_kind(EnvironmentKind::Script)
                .with_trusted_command_scope(script_scope.clone()),
        );
        assert_eq!(native.profile_name.as_deref(), Some("native"));

        let script = select_environment_profile(
            &manifest,
            &EnvironmentSelectionContext::with_platform_key("darwin-arm64")
                .with_available_kind(EnvironmentKind::Script)
                .with_trusted_command_scope(script_scope),
        );
        assert_eq!(script.profile_name.as_deref(), Some("script"));

        let manual = select_environment_profile(
            &manifest,
            &EnvironmentSelectionContext::with_platform_key("darwin-arm64")
                .with_available_kind(EnvironmentKind::Script),
        );
        assert_eq!(manual.profile_name.as_deref(), Some("manual"));
        assert!(
            manual
                .explanation
                .iter()
                .any(|line| line.contains("bootstrap command is not trusted"))
        );
    }

    #[test]
    fn returns_no_selection_when_no_target_matches() {
        let manifest =
            manifest_with_profiles([("manual", EnvironmentKind::Manual, vec!["windows-native"])]);

        let selected = select_environment_profile(
            &manifest,
            &EnvironmentSelectionContext::with_platform_key("darwin-arm64"),
        );

        assert_eq!(selected.profile_name, None);
        assert!(selected.explanation.iter().any(|line| {
            line.contains("targets do not match") || line.contains("no environment profile")
        }));
    }

    #[test]
    fn keeps_manifest_defaults_unchanged() {
        let manifest = Manifest::parse(
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

        assert_eq!(manifest.workspace.untracked, UntrackedPolicy::Safe);
        assert_eq!(
            select_environment_profile(
                &manifest,
                &EnvironmentSelectionContext::with_platform_key("darwin-arm64"),
            )
            .profile_name,
            None
        );
    }
}
