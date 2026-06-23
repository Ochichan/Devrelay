//! Filesystem watcher abstractions for background protection.
//!
//! Watcher events are hints only. The agent must re-run Git status and snapshot
//! verification before making durable state changes.

use crate::{DevRelayError, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::SystemTime;

const ROOT_RELATIVE_PATH: &str = ".";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FilesystemEventKind {
    Create,
    Modify,
    Remove,
    Rename,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesystemRawEvent {
    pub kind: FilesystemEventKind,
    pub paths: Vec<PathBuf>,
}

impl FilesystemRawEvent {
    pub fn new(kind: FilesystemEventKind, paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            kind,
            paths: sorted_paths(paths),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesystemWatchMessage {
    Event(FilesystemRawEvent),
    Error(String),
}

impl FilesystemWatchMessage {
    pub fn event(event: FilesystemRawEvent) -> Self {
        Self::Event(event)
    }

    pub fn error(error: impl Into<String>) -> Self {
        Self::Error(error.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceWatch {
    pub workspace_id: String,
    pub root: PathBuf,
}

impl WorkspaceWatch {
    pub fn new(workspace_id: impl Into<String>, root: impl Into<PathBuf>) -> Result<Self> {
        let watch = Self {
            workspace_id: workspace_id.into(),
            root: root.into(),
        };
        watch.validate()?;
        Ok(watch)
    }

    fn validate(&self) -> Result<()> {
        if self.workspace_id.trim().is_empty() {
            return Err(watcher_error("workspace watch id must not be empty"));
        }
        if self.root.as_os_str().is_empty() {
            return Err(watcher_error("workspace watch root must not be empty"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceChangeHint {
    pub workspace_id: String,
    pub source_generation: u64,
    pub kind: FilesystemEventKind,
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoalescedWorkspaceChange {
    pub workspace_id: String,
    pub source_generation: u64,
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Default)]
pub struct FilesystemWatchState {
    workspaces: BTreeMap<String, WorkspaceWatch>,
    source_generations: BTreeMap<String, u64>,
    pending_paths: BTreeMap<String, BTreeSet<PathBuf>>,
}

impl FilesystemWatchState {
    pub fn register_workspace(&mut self, watch: WorkspaceWatch) -> Result<()> {
        watch.validate()?;
        self.source_generations
            .entry(watch.workspace_id.clone())
            .or_insert(0);
        self.pending_paths
            .entry(watch.workspace_id.clone())
            .or_default();
        self.workspaces.insert(watch.workspace_id.clone(), watch);
        Ok(())
    }

    pub fn unregister_workspace(&mut self, workspace_id: &str) -> Option<WorkspaceWatch> {
        self.source_generations.remove(workspace_id);
        self.pending_paths.remove(workspace_id);
        self.workspaces.remove(workspace_id)
    }

    pub fn workspace_count(&self) -> usize {
        self.workspaces.len()
    }

    pub fn source_generation(&self, workspace_id: &str) -> Option<u64> {
        self.source_generations.get(workspace_id).copied()
    }

    pub fn observe_raw_event(&mut self, event: &FilesystemRawEvent) -> Vec<WorkspaceChangeHint> {
        let mut by_workspace: BTreeMap<String, BTreeSet<PathBuf>> = BTreeMap::new();
        for path in &event.paths {
            if let Some((workspace_id, relative_path)) = self.match_workspace(path) {
                by_workspace
                    .entry(workspace_id)
                    .or_default()
                    .insert(relative_path);
            }
        }

        let mut hints = Vec::new();
        for (workspace_id, paths) in by_workspace {
            let generation = self
                .source_generations
                .entry(workspace_id.clone())
                .or_insert(0);
            *generation = generation.saturating_add(1);

            let pending = self.pending_paths.entry(workspace_id.clone()).or_default();
            pending.extend(paths.iter().cloned());

            hints.push(WorkspaceChangeHint {
                workspace_id,
                source_generation: *generation,
                kind: event.kind,
                paths: paths.into_iter().collect(),
            });
        }
        hints
    }

    pub fn drain_coalesced(&mut self) -> Vec<CoalescedWorkspaceChange> {
        let workspace_ids: Vec<_> = self.pending_paths.keys().cloned().collect();
        let mut changes = Vec::new();
        for workspace_id in workspace_ids {
            let Some(paths) = self.pending_paths.get_mut(&workspace_id) else {
                continue;
            };
            if paths.is_empty() {
                continue;
            }

            let drained = std::mem::take(paths);
            changes.push(CoalescedWorkspaceChange {
                source_generation: self.source_generation(&workspace_id).unwrap_or_default(),
                workspace_id,
                paths: drained.into_iter().collect(),
            });
        }
        changes
    }

    fn match_workspace(&self, path: &Path) -> Option<(String, PathBuf)> {
        self.workspaces
            .values()
            .filter_map(|watch| {
                let relative_path = path.strip_prefix(&watch.root).ok()?;
                let relative_path = if relative_path.as_os_str().is_empty() {
                    PathBuf::from(ROOT_RELATIVE_PATH)
                } else {
                    relative_path.to_path_buf()
                };
                Some((
                    watch.workspace_id.clone(),
                    watch.root.components().count(),
                    relative_path,
                ))
            })
            .max_by_key(|(_, depth, _)| *depth)
            .map(|(workspace_id, _, relative_path)| (workspace_id, relative_path))
    }
}

pub trait FilesystemWatcher: Send {
    fn watch(&mut self, watch: WorkspaceWatch) -> Result<()>;
    fn unwatch(&mut self, workspace_id: &str) -> Result<()>;

    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}

pub fn default_filesystem_watcher(
    sender: Sender<FilesystemWatchMessage>,
) -> Result<Box<dyn FilesystemWatcher>> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(MacOsFilesystemWatcher::new(sender)?))
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(Box::new(PollingFilesystemWatcher::new(sender)))
    }
}

#[cfg(target_os = "macos")]
pub struct MacOsFilesystemWatcher {
    inner: notify::RecommendedWatcher,
    workspaces: BTreeMap<String, PathBuf>,
}

#[cfg(target_os = "macos")]
impl MacOsFilesystemWatcher {
    pub fn new(sender: Sender<FilesystemWatchMessage>) -> Result<Self> {
        use notify::{Config, Watcher as NotifyWatcher};

        let inner = notify::RecommendedWatcher::new(
            move |event: notify::Result<notify::Event>| match event {
                Ok(event) => {
                    let event = raw_event_from_notify(event);
                    if !event.paths.is_empty() {
                        let _ = sender.send(FilesystemWatchMessage::event(event));
                    }
                }
                Err(err) => {
                    let _ = sender.send(FilesystemWatchMessage::error(err.to_string()));
                }
            },
            Config::default(),
        )
        .map_err(|err| watcher_error(format!("failed to start macOS watcher: {err}")))?;

        Ok(Self {
            inner,
            workspaces: BTreeMap::new(),
        })
    }
}

#[cfg(target_os = "macos")]
impl FilesystemWatcher for MacOsFilesystemWatcher {
    fn watch(&mut self, watch: WorkspaceWatch) -> Result<()> {
        use notify::{RecursiveMode, Watcher as NotifyWatcher};

        watch.validate()?;
        self.inner
            .watch(&watch.root, RecursiveMode::Recursive)
            .map_err(|err| {
                watcher_error(format!(
                    "failed to watch workspace {} at {}: {err}",
                    watch.workspace_id,
                    watch.root.display()
                ))
            })?;
        self.workspaces.insert(watch.workspace_id, watch.root);
        Ok(())
    }

    fn unwatch(&mut self, workspace_id: &str) -> Result<()> {
        use notify::Watcher as NotifyWatcher;

        let Some(root) = self.workspaces.remove(workspace_id) else {
            return Ok(());
        };
        self.inner.unwatch(&root).map_err(|err| {
            watcher_error(format!(
                "failed to unwatch workspace {workspace_id} at {}: {err}",
                root.display()
            ))
        })
    }

    fn shutdown(&mut self) -> Result<()> {
        use notify::Watcher as NotifyWatcher;

        for (_, root) in std::mem::take(&mut self.workspaces) {
            let _ = self.inner.unwatch(&root);
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn raw_event_from_notify(event: notify::Event) -> FilesystemRawEvent {
    use notify::event::{EventKind, ModifyKind};

    let kind = match event.kind {
        EventKind::Create(_) => FilesystemEventKind::Create,
        EventKind::Remove(_) => FilesystemEventKind::Remove,
        EventKind::Modify(ModifyKind::Name(_)) => FilesystemEventKind::Rename,
        EventKind::Modify(_) => FilesystemEventKind::Modify,
        _ => FilesystemEventKind::Other,
    };
    FilesystemRawEvent::new(kind, event.paths)
}

#[derive(Debug)]
pub struct PollingFilesystemWatcher {
    sender: Sender<FilesystemWatchMessage>,
    workspaces: BTreeMap<String, WorkspaceWatch>,
    snapshots: BTreeMap<String, BTreeMap<PathBuf, FileFingerprint>>,
}

impl PollingFilesystemWatcher {
    pub fn new(sender: Sender<FilesystemWatchMessage>) -> Self {
        Self {
            sender,
            workspaces: BTreeMap::new(),
            snapshots: BTreeMap::new(),
        }
    }

    pub fn poll_once(&mut self) -> Result<Vec<FilesystemRawEvent>> {
        let mut events = Vec::new();
        for (workspace_id, watch) in &self.workspaces {
            let current = scan_workspace(&watch.root)?;
            let previous = self
                .snapshots
                .insert(workspace_id.clone(), current.clone())
                .unwrap_or_default();
            let paths = changed_paths(&previous, &current);
            if paths.is_empty() {
                continue;
            }
            let event = FilesystemRawEvent::new(FilesystemEventKind::Modify, paths);
            let _ = self
                .sender
                .send(FilesystemWatchMessage::event(event.clone()));
            events.push(event);
        }
        Ok(events)
    }
}

impl FilesystemWatcher for PollingFilesystemWatcher {
    fn watch(&mut self, watch: WorkspaceWatch) -> Result<()> {
        watch.validate()?;
        let snapshot = scan_workspace(&watch.root)?;
        self.snapshots.insert(watch.workspace_id.clone(), snapshot);
        self.workspaces.insert(watch.workspace_id.clone(), watch);
        Ok(())
    }

    fn unwatch(&mut self, workspace_id: &str) -> Result<()> {
        self.snapshots.remove(workspace_id);
        self.workspaces.remove(workspace_id);
        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        self.snapshots.clear();
        self.workspaces.clear();
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFingerprint {
    is_dir: bool,
    is_file: bool,
    len: u64,
    modified: Option<SystemTime>,
}

fn scan_workspace(root: &Path) -> Result<BTreeMap<PathBuf, FileFingerprint>> {
    let metadata = fs::symlink_metadata(root)?;
    let mut snapshot = BTreeMap::new();
    if metadata.file_type().is_dir() {
        for entry in fs::read_dir(root)? {
            scan_path(&entry?.path(), &mut snapshot)?;
        }
    } else {
        snapshot.insert(root.to_path_buf(), fingerprint(metadata));
    }
    Ok(snapshot)
}

fn scan_path(path: &Path, snapshot: &mut BTreeMap<PathBuf, FileFingerprint>) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    let file_type = metadata.file_type();
    snapshot.insert(path.to_path_buf(), fingerprint(metadata));

    if file_type.is_dir() {
        for entry in fs::read_dir(path)? {
            scan_path(&entry?.path(), snapshot)?;
        }
    }
    Ok(())
}

fn fingerprint(metadata: fs::Metadata) -> FileFingerprint {
    let file_type = metadata.file_type();
    FileFingerprint {
        is_dir: file_type.is_dir(),
        is_file: file_type.is_file(),
        len: metadata.len(),
        modified: metadata.modified().ok(),
    }
}

fn changed_paths(
    previous: &BTreeMap<PathBuf, FileFingerprint>,
    current: &BTreeMap<PathBuf, FileFingerprint>,
) -> Vec<PathBuf> {
    let mut paths = BTreeSet::new();
    for path in previous.keys() {
        if !current.contains_key(path) {
            paths.insert(path.clone());
        }
    }
    for (path, fingerprint) in current {
        if previous.get(path) != Some(fingerprint) {
            paths.insert(path.clone());
        }
    }
    paths.into_iter().collect()
}

fn sorted_paths(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    paths
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn watcher_error(detail: impl Into<String>) -> DevRelayError {
    DevRelayError::Watcher(detail.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn watch_state_drops_paths_outside_registered_workspaces() {
        let root = PathBuf::from("/tmp/devrelay-watch-state-drop");
        let mut state = FilesystemWatchState::default();
        state
            .register_workspace(WorkspaceWatch::new("w_main", &root).unwrap())
            .unwrap();

        let hints = state.observe_raw_event(&FilesystemRawEvent::new(
            FilesystemEventKind::Modify,
            [PathBuf::from("/tmp/devrelay-other/file.txt")],
        ));

        assert!(hints.is_empty());
        assert!(state.drain_coalesced().is_empty());
        assert_eq!(state.source_generation("w_main"), Some(0));
    }

    #[test]
    fn watch_state_increments_generation_and_coalesces_relative_paths() {
        let root = PathBuf::from("/tmp/devrelay-watch-state-coalesce");
        let mut state = FilesystemWatchState::default();
        state
            .register_workspace(WorkspaceWatch::new("w_main", &root).unwrap())
            .unwrap();

        let first = state.observe_raw_event(&FilesystemRawEvent::new(
            FilesystemEventKind::Modify,
            [
                root.join("src/lib.rs"),
                root.join("src/main.rs"),
                root.join("src/lib.rs"),
            ],
        ));
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].source_generation, 1);
        assert_eq!(
            first[0].paths,
            vec![PathBuf::from("src/lib.rs"), PathBuf::from("src/main.rs")]
        );

        let second = state.observe_raw_event(&FilesystemRawEvent::new(
            FilesystemEventKind::Create,
            [root.join("README.md")],
        ));
        assert_eq!(second[0].source_generation, 2);

        let coalesced = state.drain_coalesced();
        assert_eq!(coalesced.len(), 1);
        assert_eq!(coalesced[0].workspace_id, "w_main");
        assert_eq!(coalesced[0].source_generation, 2);
        assert_eq!(
            coalesced[0].paths,
            vec![
                PathBuf::from("README.md"),
                PathBuf::from("src/lib.rs"),
                PathBuf::from("src/main.rs")
            ]
        );
        assert!(state.drain_coalesced().is_empty());
    }

    #[test]
    fn watch_state_uses_deepest_workspace_root() {
        let root = PathBuf::from("/tmp/devrelay-watch-state-nested");
        let nested = root.join("child");
        let mut state = FilesystemWatchState::default();
        state
            .register_workspace(WorkspaceWatch::new("w_root", &root).unwrap())
            .unwrap();
        state
            .register_workspace(WorkspaceWatch::new("w_child", &nested).unwrap())
            .unwrap();

        let hints = state.observe_raw_event(&FilesystemRawEvent::new(
            FilesystemEventKind::Modify,
            [nested.join("file.txt")],
        ));

        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].workspace_id, "w_child");
        assert_eq!(hints[0].paths, vec![PathBuf::from("file.txt")]);
        assert_eq!(state.source_generation("w_root"), Some(0));
        assert_eq!(state.source_generation("w_child"), Some(1));
    }

    #[test]
    fn polling_watcher_reports_file_changes_as_hints() {
        let temp = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let mut watcher = PollingFilesystemWatcher::new(tx);
        watcher
            .watch(WorkspaceWatch::new("w_main", temp.path()).unwrap())
            .unwrap();

        let changed = temp.path().join("changed.txt");
        fs::write(&changed, "changed").unwrap();

        let events = watcher.poll_once().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, FilesystemEventKind::Modify);
        assert_eq!(events[0].paths, vec![changed.clone()]);

        let message = rx.recv().unwrap();
        assert_eq!(
            message,
            FilesystemWatchMessage::event(FilesystemRawEvent::new(
                FilesystemEventKind::Modify,
                [changed]
            ))
        );
    }

    #[test]
    fn polling_watcher_unwatch_stops_reports() {
        let temp = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let mut watcher = PollingFilesystemWatcher::new(tx);
        watcher
            .watch(WorkspaceWatch::new("w_main", temp.path()).unwrap())
            .unwrap();
        watcher.unwatch("w_main").unwrap();

        fs::write(temp.path().join("ignored.txt"), "ignored").unwrap();

        assert!(watcher.poll_once().unwrap().is_empty());
        assert!(rx.try_recv().is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_watcher_supports_lifecycle() {
        let temp = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut watcher = MacOsFilesystemWatcher::new(tx).unwrap();

        watcher
            .watch(WorkspaceWatch::new("w_main", temp.path()).unwrap())
            .unwrap();
        watcher.unwatch("w_main").unwrap();
        watcher.shutdown().unwrap();
    }
}
