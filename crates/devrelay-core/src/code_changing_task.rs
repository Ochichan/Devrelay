//! Code-changing task planning and result summarization.
//!
//! Code-changing agent tasks are allowed to mutate only their isolated runner
//! workspace. This module creates a separate task session, captures the commit
//! chain or diff from that workspace, runs declared test commands there, and
//! returns a summary without auto-merging into any canonical session.

use crate::{
    DevRelayError, GitRepo, NoopTaskExecutionLogSink, Result, StatusEntryKind, TaskCommandRunner,
    TaskExecutionLogSink, TaskRunnerWorkspace, generate_session_id,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeChangingTaskPlan {
    pub task_run_id: String,
    pub task_name: String,
    pub code_changing: bool,
    pub parent_session_id: Option<String>,
    pub task_session_id: String,
    pub isolated_workspace: PathBuf,
    pub auto_merge: bool,
    pub test_commands: Vec<CodeChangingTaskTestCommand>,
    pub explanation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeChangingTaskChangeSet {
    pub base_ref: String,
    pub head_oid: String,
    pub commit_chain: Vec<CodeChangingTaskCommit>,
    pub diff: String,
    pub changed_files: Vec<CodeChangingTaskChangedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeChangingTaskCommit {
    pub oid: String,
    pub subject: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeChangingTaskChangedFile {
    pub path: String,
    pub status: String,
    pub source: CodeChangingTaskChangeSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CodeChangingTaskChangeSource {
    CommitRange,
    WorkingTree,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeChangingTaskTestCommand {
    pub name: String,
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeChangingTaskTestResult {
    pub name: String,
    pub command: Vec<String>,
    pub passed: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeChangingTaskSummary {
    pub task_run_id: String,
    pub task_name: String,
    pub code_changing: bool,
    pub task_session_id: String,
    pub isolated_workspace: PathBuf,
    pub auto_merge: bool,
    pub changed_files: Vec<CodeChangingTaskChangedFile>,
    pub commit_chain: Vec<CodeChangingTaskCommit>,
    pub tests: Vec<CodeChangingTaskTestResult>,
    pub success: bool,
    pub summary: String,
}

pub fn plan_code_changing_task(
    workspace: &TaskRunnerWorkspace,
    parent_session_id: Option<String>,
    test_commands: Vec<CodeChangingTaskTestCommand>,
) -> Result<CodeChangingTaskPlan> {
    ensure_isolated_workspace(workspace)?;
    let task_session_id = generate_session_id();
    Ok(CodeChangingTaskPlan {
        task_run_id: workspace.task_run_id.clone(),
        task_name: workspace.task_name.clone(),
        code_changing: true,
        parent_session_id,
        task_session_id,
        isolated_workspace: workspace.path.clone(),
        auto_merge: false,
        test_commands,
        explanation: vec![
            "task is marked as code-changing".to_string(),
            "a separate fork session is reserved for task output".to_string(),
            "runner workspace is non-canonical and cannot take writer ownership".to_string(),
            "auto-merge into the active session is disabled".to_string(),
        ],
    })
}

pub fn capture_code_changing_task_changes(
    workspace: &TaskRunnerWorkspace,
    base_ref: &str,
) -> Result<CodeChangingTaskChangeSet> {
    ensure_isolated_workspace(workspace)?;
    validate_non_empty("base_ref", base_ref)?;
    let repo = GitRepo::new(&workspace.path);
    let head_oid = repo.run(&["rev-parse", "HEAD"])?;
    let commit_chain = capture_commit_chain(&repo, base_ref)?;
    let mut changed_files = committed_changed_files(&repo, base_ref)?;
    changed_files.extend(working_tree_changed_files(&repo)?);
    changed_files.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.status.cmp(&right.status))
            .then(change_source_order(&left.source).cmp(&change_source_order(&right.source)))
    });
    changed_files.dedup();

    let mut diff = if commit_chain.is_empty() {
        String::new()
    } else {
        repo.run(&["diff", "--binary", base_ref, "HEAD"])?
    };
    let working_tree_diff = repo.run(&["diff", "--binary", "HEAD"])?;
    if !working_tree_diff.is_empty() {
        if !diff.is_empty() {
            diff.push('\n');
        }
        diff.push_str(&working_tree_diff);
    }

    Ok(CodeChangingTaskChangeSet {
        base_ref: base_ref.to_string(),
        head_oid,
        commit_chain,
        diff,
        changed_files,
    })
}

pub fn run_code_changing_task_tests(
    workspace: &TaskRunnerWorkspace,
    tests: &[CodeChangingTaskTestCommand],
    runner: &mut impl TaskCommandRunner,
) -> Result<Vec<CodeChangingTaskTestResult>> {
    let mut sink = NoopTaskExecutionLogSink;
    run_code_changing_task_tests_with_sink(workspace, tests, runner, &mut sink)
}

pub fn run_code_changing_task_tests_with_sink(
    workspace: &TaskRunnerWorkspace,
    tests: &[CodeChangingTaskTestCommand],
    runner: &mut impl TaskCommandRunner,
    sink: &mut dyn TaskExecutionLogSink,
) -> Result<Vec<CodeChangingTaskTestResult>> {
    ensure_isolated_workspace(workspace)?;
    let environment = BTreeMap::new();
    tests
        .iter()
        .map(|test| {
            if test.name.trim().is_empty() || test.command.is_empty() {
                return Err(DevRelayError::Config(
                    "code-changing task test command requires a name and command".to_string(),
                ));
            }
            let output = runner.run(
                &workspace.path,
                &test.command,
                &environment,
                test.timeout_seconds,
                sink,
            )?;
            Ok(CodeChangingTaskTestResult {
                name: test.name.clone(),
                command: test.command.clone(),
                passed: output.exit_code == Some(0) && !output.timed_out && !output.canceled,
                exit_code: output.exit_code,
                timed_out: output.timed_out,
                stdout: output.stdout,
                stderr: output.stderr,
            })
        })
        .collect()
}

pub fn summarize_code_changing_task(
    plan: &CodeChangingTaskPlan,
    changes: &CodeChangingTaskChangeSet,
    tests: Vec<CodeChangingTaskTestResult>,
) -> CodeChangingTaskSummary {
    let success = tests.iter().all(|test| test.passed);
    let summary = format!(
        "{} changed file(s), {} commit(s), {} test(s) {}",
        changes.changed_files.len(),
        changes.commit_chain.len(),
        tests.len(),
        if success { "passed" } else { "failed" }
    );
    CodeChangingTaskSummary {
        task_run_id: plan.task_run_id.clone(),
        task_name: plan.task_name.clone(),
        code_changing: plan.code_changing,
        task_session_id: plan.task_session_id.clone(),
        isolated_workspace: plan.isolated_workspace.clone(),
        auto_merge: false,
        changed_files: changes.changed_files.clone(),
        commit_chain: changes.commit_chain.clone(),
        tests,
        success,
        summary,
    }
}

fn capture_commit_chain(repo: &GitRepo, base_ref: &str) -> Result<Vec<CodeChangingTaskCommit>> {
    let range = format!("{base_ref}..HEAD");
    let raw = repo.run(&["log", "--reverse", "--format=%H%x09%s", range.as_str()])?;
    Ok(raw
        .lines()
        .filter_map(|line| {
            let (oid, subject) = line.split_once('\t')?;
            Some(CodeChangingTaskCommit {
                oid: oid.to_string(),
                subject: subject.to_string(),
            })
        })
        .collect())
}

fn committed_changed_files(
    repo: &GitRepo,
    base_ref: &str,
) -> Result<Vec<CodeChangingTaskChangedFile>> {
    let raw = repo.run(&["diff", "--name-status", base_ref, "HEAD"])?;
    Ok(raw
        .lines()
        .filter_map(parse_name_status_line)
        .map(|(status, path)| CodeChangingTaskChangedFile {
            path,
            status,
            source: CodeChangingTaskChangeSource::CommitRange,
        })
        .collect())
}

fn working_tree_changed_files(repo: &GitRepo) -> Result<Vec<CodeChangingTaskChangedFile>> {
    Ok(repo
        .status()?
        .entries
        .into_iter()
        .map(|entry| {
            let status = match entry.kind {
                StatusEntryKind::Untracked => "??".to_string(),
                _ => entry.xy.unwrap_or_else(|| "??".to_string()),
            };
            CodeChangingTaskChangedFile {
                path: entry.path,
                status,
                source: CodeChangingTaskChangeSource::WorkingTree,
            }
        })
        .collect())
}

fn parse_name_status_line(line: &str) -> Option<(String, String)> {
    let mut parts = line.split('\t');
    let status = parts.next()?.to_string();
    let first_path = parts.next()?.to_string();
    let path = parts.next().map(str::to_string).unwrap_or(first_path);
    Some((status, path))
}

fn ensure_isolated_workspace(workspace: &TaskRunnerWorkspace) -> Result<()> {
    if workspace.canonical_session {
        return Err(DevRelayError::Config(
            "code-changing task cannot run in a canonical session workspace".to_string(),
        ));
    }
    Ok(())
}

fn validate_non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(DevRelayError::Config(format!(
            "code-changing task {field} must not be empty"
        )));
    }
    Ok(())
}

fn change_source_order(source: &CodeChangingTaskChangeSource) -> u8 {
    match source {
        CodeChangingTaskChangeSource::CommitRange => 0,
        CodeChangingTaskChangeSource::WorkingTree => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EnvironmentKind, SESSION_ID_PREFIX, TaskCommandOutput, TaskRunnerEnvironmentState,
        TaskRunnerSecretState, TaskRunnerSidecarState, TaskRunnerWorkspaceRetentionPolicy,
        VerificationDetails,
    };
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    #[test]
    fn plans_code_changing_task_in_separate_noncanonical_session() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = runner_workspace(temp.path().join("runner"), false);
        let tests = vec![CodeChangingTaskTestCommand {
            name: "unit".to_string(),
            command: vec!["cargo".to_string(), "test".to_string()],
            timeout_seconds: Some(60),
        }];

        let plan =
            plan_code_changing_task(&workspace, Some("se_parent".to_string()), tests.clone())
                .unwrap();

        assert!(plan.code_changing);
        assert!(plan.task_session_id.starts_with(SESSION_ID_PREFIX));
        assert_ne!(plan.task_session_id, "se_parent");
        assert_eq!(plan.parent_session_id.as_deref(), Some("se_parent"));
        assert_eq!(plan.test_commands, tests);
        assert!(!plan.auto_merge);

        let canonical = runner_workspace(temp.path().join("canonical"), true);
        let err = plan_code_changing_task(&canonical, None, Vec::new()).unwrap_err();
        assert!(err.to_string().contains("canonical session"));
    }

    #[test]
    fn captures_commit_chain_diff_and_changed_files_from_runner_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let runner_path = temp.path().join("runner");
        init_repo(&runner_path);
        let base_ref = git(&runner_path, &["rev-parse", "HEAD"]);
        fs::write(runner_path.join("src.rs"), "changed\n").unwrap();
        git(&runner_path, &["add", "src.rs"]);
        git(&runner_path, &["commit", "-m", "agent change"]);
        fs::write(runner_path.join("notes.txt"), "uncommitted\n").unwrap();
        let workspace = runner_workspace(&runner_path, false);

        let changes = capture_code_changing_task_changes(&workspace, &base_ref).unwrap();

        assert_eq!(changes.commit_chain.len(), 1);
        assert_eq!(changes.commit_chain[0].subject, "agent change");
        assert!(changes.diff.contains("changed"));
        assert!(
            changes
                .changed_files
                .iter()
                .any(|file| file.path == "src.rs"
                    && file.source == CodeChangingTaskChangeSource::CommitRange)
        );
        assert!(
            changes
                .changed_files
                .iter()
                .any(|file| file.path == "notes.txt"
                    && file.source == CodeChangingTaskChangeSource::WorkingTree)
        );
    }

    #[test]
    fn runs_declared_tests_and_returns_summary_without_touching_canonical() {
        let temp = tempfile::tempdir().unwrap();
        let runner_path = temp.path().join("runner");
        let canonical_path = temp.path().join("canonical");
        init_repo(&runner_path);
        init_repo(&canonical_path);
        let canonical_before = fs::read_to_string(canonical_path.join("src.rs")).unwrap();
        let base_ref = git(&runner_path, &["rev-parse", "HEAD"]);
        fs::write(runner_path.join("src.rs"), "changed\n").unwrap();
        let workspace = runner_workspace(&runner_path, false);
        let plan = plan_code_changing_task(
            &workspace,
            Some("se_parent".to_string()),
            vec![CodeChangingTaskTestCommand {
                name: "unit".to_string(),
                command: vec!["cargo".to_string(), "test".to_string()],
                timeout_seconds: Some(30),
            }],
        )
        .unwrap();
        let mut runner = FakeTaskRunner::default();

        let tests =
            run_code_changing_task_tests(&workspace, &plan.test_commands, &mut runner).unwrap();
        let changes = capture_code_changing_task_changes(&workspace, &base_ref).unwrap();
        let summary = summarize_code_changing_task(&plan, &changes, tests);

        assert!(changes.diff.contains("changed"));
        assert_eq!(
            runner.commands,
            vec![vec!["cargo".to_string(), "test".to_string()]]
        );
        assert!(summary.code_changing);
        assert!(summary.success);
        assert!(!summary.auto_merge);
        assert!(
            summary
                .changed_files
                .iter()
                .any(|file| file.path == "src.rs")
        );
        assert!(summary.summary.contains("changed file"));
        assert_eq!(
            fs::read_to_string(canonical_path.join("src.rs")).unwrap(),
            canonical_before
        );
    }

    #[derive(Default)]
    struct FakeTaskRunner {
        commands: Vec<Vec<String>>,
    }

    impl TaskCommandRunner for FakeTaskRunner {
        fn run(
            &mut self,
            cwd: &Path,
            command: &[String],
            _environment: &BTreeMap<String, String>,
            _timeout_seconds: Option<u64>,
            _sink: &mut dyn TaskExecutionLogSink,
        ) -> Result<TaskCommandOutput> {
            assert!(cwd.ends_with("runner"));
            self.commands.push(command.to_vec());
            Ok(TaskCommandOutput::success("ok"))
        }
    }

    fn runner_workspace(path: impl Into<PathBuf>, canonical_session: bool) -> TaskRunnerWorkspace {
        TaskRunnerWorkspace {
            task_run_id: "tr_code".to_string(),
            project_id: "12345678".to_string(),
            task_name: "agent-edit".to_string(),
            path: path.into(),
            snapshot_id: "s1_0123456789abcdef01234567".to_string(),
            canonical_session,
            environment: TaskRunnerEnvironmentState {
                profile_name: "dev".to_string(),
                kind: EnvironmentKind::Native,
                command_scope: "environment.profile.dev".to_string(),
                hydrated: true,
                explanation: Vec::new(),
            },
            sidecars: TaskRunnerSidecarState::NotRequired,
            secrets: TaskRunnerSecretState::SkippedNotPermitted {
                required: Vec::new(),
            },
            verification: VerificationDetails {
                head_oid: "h".to_string(),
                index_tree_oid: "i".to_string(),
                work_tree_oid: "w".to_string(),
                state_hash: "s".to_string(),
                included_untracked: Vec::new(),
                excluded_paths: Vec::new(),
            },
            retention_policy: TaskRunnerWorkspaceRetentionPolicy::delete_on_cleanup(),
        }
    }

    fn init_repo(path: &Path) {
        fs::create_dir_all(path).unwrap();
        git(path, &["init", "-b", "main"]);
        git(path, &["config", "user.name", "DevRelay Test"]);
        git(
            path,
            &["config", "user.email", "devrelay-test@example.local"],
        );
        fs::write(path.join("src.rs"), "base\n").unwrap();
        git(path, &["add", "src.rs"]);
        git(path, &["commit", "-m", "base"]);
    }

    fn git(path: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git command failed: {args:?}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }
}
