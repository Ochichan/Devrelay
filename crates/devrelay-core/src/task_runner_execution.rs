//! Task command execution inside prepared runner workspaces.
//!
//! Host execution is implemented here. Sandbox, container, and VM backends are
//! explicit placeholders so callers do not accidentally treat them as isolated.

use crate::{
    DevRelayError, Manifest, Result, SecretMaterializationReport, TaskDefinition, TaskRunState,
    TaskRunnerSecretState, TaskRunnerWorkspace, TaskSandbox,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskExecutionOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TaskExecutionBackend {
    Host,
    SandboxPlaceholder,
    ContainerPlaceholder,
    VmPlaceholder,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskExecutionResult {
    pub task_run_id: String,
    pub task_name: String,
    pub backend: TaskExecutionBackend,
    pub state: TaskRunState,
    pub command: Vec<String>,
    pub working_directory: PathBuf,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub canceled: bool,
    pub duration_millis: u64,
    pub stdout: String,
    pub stderr: String,
    pub logs: Vec<TaskExecutionLogEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskExecutionLogEvent {
    pub stream: TaskExecutionLogStream,
    pub chunk: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TaskExecutionLogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskCommandOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub canceled: bool,
}

impl TaskCommandOutput {
    pub fn success(stdout: impl Into<String>) -> Self {
        Self {
            exit_code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
            timed_out: false,
            canceled: false,
        }
    }

    pub fn failure(exit_code: i32, stderr: impl Into<String>) -> Self {
        Self {
            exit_code: Some(exit_code),
            stdout: String::new(),
            stderr: stderr.into(),
            timed_out: false,
            canceled: false,
        }
    }

    pub fn timed_out(stderr: impl Into<String>) -> Self {
        Self {
            exit_code: None,
            stdout: String::new(),
            stderr: stderr.into(),
            timed_out: true,
            canceled: true,
        }
    }
}

pub trait TaskCommandRunner {
    fn run(
        &mut self,
        cwd: &Path,
        command: &[String],
        environment: &BTreeMap<String, String>,
        timeout_seconds: Option<u64>,
        sink: &mut dyn TaskExecutionLogSink,
    ) -> Result<TaskCommandOutput>;
}

pub trait TaskExecutionLogSink {
    fn on_log(&mut self, event: &TaskExecutionLogEvent) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct NoopTaskExecutionLogSink;

impl TaskExecutionLogSink for NoopTaskExecutionLogSink {
    fn on_log(&mut self, _event: &TaskExecutionLogEvent) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct VecTaskExecutionLogSink {
    pub events: Vec<TaskExecutionLogEvent>,
}

impl TaskExecutionLogSink for VecTaskExecutionLogSink {
    fn on_log(&mut self, event: &TaskExecutionLogEvent) -> Result<()> {
        self.events.push(event.clone());
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemTaskCommandRunner;

impl TaskCommandRunner for SystemTaskCommandRunner {
    fn run(
        &mut self,
        cwd: &Path,
        command: &[String],
        environment: &BTreeMap<String, String>,
        timeout_seconds: Option<u64>,
        sink: &mut dyn TaskExecutionLogSink,
    ) -> Result<TaskCommandOutput> {
        let Some((program, args)) = command.split_first() else {
            return Err(DevRelayError::Config(
                "task command must contain at least one argument".to_string(),
            ));
        };

        let mut process = Command::new(program);
        process
            .args(args)
            .current_dir(cwd)
            .envs(environment)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_process_group(&mut process);
        let mut child = process.spawn()?;
        let stdout = child.stdout.take().expect("stdout should be piped");
        let stderr = child.stderr.take().expect("stderr should be piped");
        let (tx, rx) = mpsc::channel();
        let stdout_reader = spawn_log_reader(stdout, TaskExecutionLogStream::Stdout, tx.clone());
        let stderr_reader = spawn_log_reader(stderr, TaskExecutionLogStream::Stderr, tx);

        let started = Instant::now();
        let timeout = timeout_seconds.map(|seconds| Duration::from_secs(seconds.max(1)));
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut timed_out = false;
        loop {
            drain_log_messages(&rx, &mut stdout, &mut stderr, sink)?;
            if child.try_wait()?.is_some() {
                break;
            }
            if timeout.is_some_and(|timeout| started.elapsed() >= timeout) {
                timed_out = true;
                kill_process_tree(&mut child)?;
                break;
            }
            match rx.recv_timeout(Duration::from_millis(10)) {
                Ok(message) => append_log_message(message, &mut stdout, &mut stderr, sink)?,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {}
            }
        }

        let status = child.wait()?;
        let _ = stdout_reader.join();
        let _ = stderr_reader.join();
        drain_log_messages(&rx, &mut stdout, &mut stderr, sink)?;

        Ok(TaskCommandOutput {
            exit_code: if timed_out { None } else { status.code() },
            stdout,
            stderr,
            timed_out,
            canceled: timed_out,
        })
    }
}

pub fn execute_task_in_runner_workspace(
    manifest: &Manifest,
    workspace: &TaskRunnerWorkspace,
    definition: &TaskDefinition,
    options: TaskExecutionOptions,
    runner: &mut impl TaskCommandRunner,
    sink: &mut dyn TaskExecutionLogSink,
) -> Result<TaskExecutionResult> {
    let backend = task_execution_backend(definition);
    let working_directory =
        resolve_task_working_directory(manifest, workspace, definition, &options)?;
    let environment = task_execution_environment(workspace, options.environment);
    let timeout_seconds = options.timeout_seconds.or_else(|| {
        manifest
            .tasks
            .get(&definition.task_name)
            .and_then(|task| task.timeout_seconds)
    });

    if backend != TaskExecutionBackend::Host {
        return placeholder_execution_result(
            workspace,
            definition,
            backend,
            working_directory,
            sink,
        );
    }

    let started = Instant::now();
    let mut tee = TeeTaskLogSink {
        captured: Vec::new(),
        downstream: sink,
    };
    let output = runner.run(
        &working_directory,
        &definition.command,
        &environment,
        timeout_seconds,
        &mut tee,
    )?;
    let state = if output.exit_code == Some(0) {
        TaskRunState::Succeeded
    } else if output.canceled {
        TaskRunState::Canceled
    } else {
        TaskRunState::Failed
    };

    Ok(TaskExecutionResult {
        task_run_id: workspace.task_run_id.clone(),
        task_name: definition.task_name.clone(),
        backend,
        state,
        command: definition.command.clone(),
        working_directory,
        exit_code: output.exit_code,
        timed_out: output.timed_out,
        canceled: output.canceled,
        duration_millis: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        stdout: output.stdout,
        stderr: output.stderr,
        logs: tee.captured,
    })
}

pub fn task_execution_backend(definition: &TaskDefinition) -> TaskExecutionBackend {
    match definition.sandbox {
        None | Some(TaskSandbox::Host) => TaskExecutionBackend::Host,
        Some(TaskSandbox::Sandbox) => TaskExecutionBackend::SandboxPlaceholder,
        Some(TaskSandbox::Container) => TaskExecutionBackend::ContainerPlaceholder,
        Some(TaskSandbox::Vm) => TaskExecutionBackend::VmPlaceholder,
    }
}

fn placeholder_execution_result(
    workspace: &TaskRunnerWorkspace,
    definition: &TaskDefinition,
    backend: TaskExecutionBackend,
    working_directory: PathBuf,
    sink: &mut dyn TaskExecutionLogSink,
) -> Result<TaskExecutionResult> {
    let stderr = format!("{backend:?} runner is a placeholder and cannot execute tasks yet");
    let event = TaskExecutionLogEvent {
        stream: TaskExecutionLogStream::Stderr,
        chunk: stderr.clone(),
    };
    sink.on_log(&event)?;
    Ok(TaskExecutionResult {
        task_run_id: workspace.task_run_id.clone(),
        task_name: definition.task_name.clone(),
        backend,
        state: TaskRunState::Failed,
        command: definition.command.clone(),
        working_directory,
        exit_code: None,
        timed_out: false,
        canceled: false,
        duration_millis: 0,
        stdout: String::new(),
        stderr,
        logs: vec![event],
    })
}

fn resolve_task_working_directory(
    manifest: &Manifest,
    workspace: &TaskRunnerWorkspace,
    definition: &TaskDefinition,
    options: &TaskExecutionOptions,
) -> Result<PathBuf> {
    let configured = options.working_directory.as_deref().or_else(|| {
        manifest
            .environment
            .as_ref()
            .and_then(|environment| environment.profiles.get(&definition.profile_name))
            .and_then(|profile| profile.working_directory.as_deref())
    });
    let Some(configured) = configured else {
        return Ok(workspace.path.clone());
    };
    let relative = Path::new(configured);
    if configured.trim().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(DevRelayError::Config(format!(
            "task working directory must stay inside runner workspace: {configured:?}"
        )));
    }
    let resolved = workspace.path.join(relative);
    if !resolved.is_dir() {
        return Err(DevRelayError::Config(format!(
            "task working directory does not exist: {}",
            resolved.display()
        )));
    }
    Ok(resolved)
}

fn task_execution_environment(
    workspace: &TaskRunnerWorkspace,
    mut environment: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    if let TaskRunnerSecretState::Materialized {
        report:
            SecretMaterializationReport {
                environment_variables,
                ..
            },
    } = &workspace.secrets
    {
        for (key, value) in environment_variables {
            environment.insert(key.clone(), value.clone());
        }
    }
    environment
}

struct TeeTaskLogSink<'a> {
    captured: Vec<TaskExecutionLogEvent>,
    downstream: &'a mut dyn TaskExecutionLogSink,
}

impl TaskExecutionLogSink for TeeTaskLogSink<'_> {
    fn on_log(&mut self, event: &TaskExecutionLogEvent) -> Result<()> {
        self.captured.push(event.clone());
        self.downstream.on_log(event)
    }
}

struct LogMessage {
    stream: TaskExecutionLogStream,
    chunk: String,
}

fn spawn_log_reader<R: Read + Send + 'static>(
    mut reader: R,
    stream: TaskExecutionLogStream,
    tx: mpsc::Sender<LogMessage>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    let chunk = String::from_utf8_lossy(&buffer[..count]).to_string();
                    if tx.send(LogMessage { stream, chunk }).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    })
}

fn drain_log_messages(
    rx: &mpsc::Receiver<LogMessage>,
    stdout: &mut String,
    stderr: &mut String,
    sink: &mut dyn TaskExecutionLogSink,
) -> Result<()> {
    while let Ok(message) = rx.try_recv() {
        append_log_message(message, stdout, stderr, sink)?;
    }
    Ok(())
}

fn append_log_message(
    message: LogMessage,
    stdout: &mut String,
    stderr: &mut String,
    sink: &mut dyn TaskExecutionLogSink,
) -> Result<()> {
    match message.stream {
        TaskExecutionLogStream::Stdout => stdout.push_str(&message.chunk),
        TaskExecutionLogStream::Stderr => stderr.push_str(&message.chunk),
    }
    sink.on_log(&TaskExecutionLogEvent {
        stream: message.stream,
        chunk: message.chunk,
    })
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn kill_process_tree(child: &mut Child) -> std::io::Result<()> {
    let pid = child.id() as libc::pid_t;
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    match child.kill() {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::InvalidInput => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(not(unix))]
fn kill_process_tree(child: &mut Child) -> std::io::Result<()> {
    match child.kill() {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::InvalidInput => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EnvironmentKind, TaskRunnerEnvironmentState, TaskRunnerWorkspaceRetentionPolicy,
        VerificationDetails,
    };
    use std::cell::RefCell;
    use std::fs;

    #[test]
    fn runs_host_task_with_working_directory_environment_and_logs() {
        let temp = tempfile::tempdir().unwrap();
        let workspace_path = temp.path().join("runner");
        fs::create_dir_all(workspace_path.join("subdir")).unwrap();
        let manifest = manifest(
            r#"
working_directory = "subdir"
"#,
            "",
        );
        let definition = crate::task_definition(&manifest, "test").unwrap();
        let workspace = workspace(
            &workspace_path,
            TaskRunnerSecretState::Materialized {
                report: SecretMaterializationReport {
                    files: Vec::new(),
                    environment_variables: BTreeMap::from([(
                        "SECRET_TOKEN".to_string(),
                        "from-secret".to_string(),
                    )]),
                    missing_optional: Vec::new(),
                    hard_exclude_patterns: Vec::new(),
                },
            },
        );
        let mut runner = FakeTaskRunner::new(TaskCommandOutput::success("done\n"));
        let mut sink = VecTaskExecutionLogSink::default();

        let result = execute_task_in_runner_workspace(
            &manifest,
            &workspace,
            &definition,
            TaskExecutionOptions {
                working_directory: None,
                environment: BTreeMap::from([("EXTRA".to_string(), "1".to_string())]),
                timeout_seconds: Some(9),
            },
            &mut runner,
            &mut sink,
        )
        .unwrap();

        let call = runner.calls.borrow()[0].clone();
        assert_eq!(call.cwd, workspace_path.join("subdir"));
        assert_eq!(call.command, vec!["fake", "--ok"]);
        assert_eq!(call.environment.get("EXTRA").map(String::as_str), Some("1"));
        assert_eq!(
            call.environment.get("SECRET_TOKEN").map(String::as_str),
            Some("from-secret")
        );
        assert_eq!(call.timeout_seconds, Some(9));
        assert_eq!(result.state, TaskRunState::Succeeded);
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.stdout, "done\n");
        assert_eq!(
            result.logs,
            vec![TaskExecutionLogEvent {
                stream: TaskExecutionLogStream::Stdout,
                chunk: "done\n".to_string(),
            }]
        );
        assert_eq!(sink.events, result.logs);
    }

    #[test]
    fn failed_and_timed_out_tasks_map_to_run_state() {
        let temp = tempfile::tempdir().unwrap();
        let workspace_path = temp.path().join("runner");
        fs::create_dir_all(&workspace_path).unwrap();
        let manifest = manifest("", "timeout_seconds = 1");
        let definition = crate::task_definition(&manifest, "test").unwrap();
        let workspace = workspace(
            &workspace_path,
            TaskRunnerSecretState::SkippedNotPermitted {
                required: Vec::new(),
            },
        );
        let mut runner = FakeTaskRunner::new(TaskCommandOutput::timed_out("timeout\n"));
        let mut sink = NoopTaskExecutionLogSink;

        let result = execute_task_in_runner_workspace(
            &manifest,
            &workspace,
            &definition,
            TaskExecutionOptions::default(),
            &mut runner,
            &mut sink,
        )
        .unwrap();

        assert_eq!(runner.calls.borrow()[0].timeout_seconds, Some(1));
        assert_eq!(result.state, TaskRunState::Canceled);
        assert!(result.timed_out);
        assert!(result.canceled);
        assert_eq!(result.exit_code, None);
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_child_process_group() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("survived");
        let command = vec![
            "sh".to_string(),
            "-c".to_string(),
            "(sleep 2; printf survived > \"$1\") & wait".to_string(),
            "_".to_string(),
            marker.to_string_lossy().to_string(),
        ];
        let mut runner = SystemTaskCommandRunner;
        let mut sink = NoopTaskExecutionLogSink;

        let output = runner
            .run(temp.path(), &command, &BTreeMap::new(), Some(1), &mut sink)
            .unwrap();
        thread::sleep(Duration::from_millis(2300));

        assert!(output.timed_out);
        assert!(output.canceled);
        assert_eq!(output.exit_code, None);
        assert!(!marker.exists());
    }

    #[test]
    fn sandbox_container_and_vm_backends_are_explicit_placeholders() {
        for (sandbox, backend) in [
            ("sandbox", TaskExecutionBackend::SandboxPlaceholder),
            ("container", TaskExecutionBackend::ContainerPlaceholder),
            ("vm", TaskExecutionBackend::VmPlaceholder),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let workspace_path = temp.path().join("runner");
            fs::create_dir_all(&workspace_path).unwrap();
            let manifest = manifest("", &format!("sandbox = \"{sandbox}\""));
            let definition = crate::task_definition(&manifest, "test").unwrap();
            let workspace = workspace(
                &workspace_path,
                TaskRunnerSecretState::SkippedNotPermitted {
                    required: Vec::new(),
                },
            );
            let mut runner = PanicTaskRunner;
            let mut sink = VecTaskExecutionLogSink::default();

            let result = execute_task_in_runner_workspace(
                &manifest,
                &workspace,
                &definition,
                TaskExecutionOptions::default(),
                &mut runner,
                &mut sink,
            )
            .unwrap();

            assert_eq!(result.backend, backend);
            assert_eq!(result.state, TaskRunState::Failed);
            assert!(result.stderr.contains("placeholder"));
            assert_eq!(sink.events, result.logs);
        }
    }

    #[test]
    fn rejects_working_directory_escape() {
        let temp = tempfile::tempdir().unwrap();
        let workspace_path = temp.path().join("runner");
        fs::create_dir_all(&workspace_path).unwrap();
        let manifest = manifest("", "");
        let definition = crate::task_definition(&manifest, "test").unwrap();
        let workspace = workspace(
            &workspace_path,
            TaskRunnerSecretState::SkippedNotPermitted {
                required: Vec::new(),
            },
        );
        let mut runner = PanicTaskRunner;
        let mut sink = NoopTaskExecutionLogSink;

        let err = execute_task_in_runner_workspace(
            &manifest,
            &workspace,
            &definition,
            TaskExecutionOptions {
                working_directory: Some("../outside".to_string()),
                environment: BTreeMap::new(),
                timeout_seconds: None,
            },
            &mut runner,
            &mut sink,
        )
        .unwrap_err();

        assert!(err.to_string().contains("working directory"));
    }

    fn manifest(profile_extra: &str, task_extra: &str) -> Manifest {
        Manifest::parse(&format!(
            r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "safe"
portable_paths = "strict"

[environment.profiles.dev]
kind = "native"
targets = ["darwin-*", "linux-*"]
command = ["bash", "-lc", "true"]
{profile_extra}

[tasks.test]
profile = "dev"
command = ["fake", "--ok"]
platforms = ["darwin-*", "linux-*"]
cpu = 1
memory_mib = 64
disk_mib = 64
cache = "read"
{task_extra}
"#
        ))
        .unwrap()
    }

    fn workspace(path: &Path, secrets: TaskRunnerSecretState) -> TaskRunnerWorkspace {
        TaskRunnerWorkspace {
            task_run_id: "tr_exec".to_string(),
            project_id: "12345678".to_string(),
            task_name: "test".to_string(),
            path: path.to_path_buf(),
            snapshot_id: "snap_123".to_string(),
            canonical_session: false,
            environment: TaskRunnerEnvironmentState {
                profile_name: "dev".to_string(),
                kind: EnvironmentKind::Native,
                command_scope: "environment.profile.dev".to_string(),
                hydrated: true,
                explanation: Vec::new(),
            },
            sidecars: crate::TaskRunnerSidecarState::NotRequired,
            secrets,
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

    #[derive(Clone)]
    struct RecordedCall {
        cwd: PathBuf,
        command: Vec<String>,
        environment: BTreeMap<String, String>,
        timeout_seconds: Option<u64>,
    }

    struct FakeTaskRunner {
        output: TaskCommandOutput,
        calls: RefCell<Vec<RecordedCall>>,
    }

    impl FakeTaskRunner {
        fn new(output: TaskCommandOutput) -> Self {
            Self {
                output,
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl TaskCommandRunner for FakeTaskRunner {
        fn run(
            &mut self,
            cwd: &Path,
            command: &[String],
            environment: &BTreeMap<String, String>,
            timeout_seconds: Option<u64>,
            sink: &mut dyn TaskExecutionLogSink,
        ) -> Result<TaskCommandOutput> {
            self.calls.borrow_mut().push(RecordedCall {
                cwd: cwd.to_path_buf(),
                command: command.to_vec(),
                environment: environment.clone(),
                timeout_seconds,
            });
            if !self.output.stdout.is_empty() {
                sink.on_log(&TaskExecutionLogEvent {
                    stream: TaskExecutionLogStream::Stdout,
                    chunk: self.output.stdout.clone(),
                })?;
            }
            if !self.output.stderr.is_empty() {
                sink.on_log(&TaskExecutionLogEvent {
                    stream: TaskExecutionLogStream::Stderr,
                    chunk: self.output.stderr.clone(),
                })?;
            }
            Ok(self.output.clone())
        }
    }

    struct PanicTaskRunner;

    impl TaskCommandRunner for PanicTaskRunner {
        fn run(
            &mut self,
            _cwd: &Path,
            _command: &[String],
            _environment: &BTreeMap<String, String>,
            _timeout_seconds: Option<u64>,
            _sink: &mut dyn TaskExecutionLogSink,
        ) -> Result<TaskCommandOutput> {
            panic!("placeholder backends must not execute host commands");
        }
    }
}
