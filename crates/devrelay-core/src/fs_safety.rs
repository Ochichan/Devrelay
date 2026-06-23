//! Filesystem traversal guards for platform-specific link types.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

pub(crate) fn is_traversal_boundary(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink() || is_windows_reparse_point(metadata)
}

#[cfg(windows)]
pub(crate) fn is_windows_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
pub(crate) fn is_windows_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
pub(crate) fn reparse_points_in_workspace(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut points = Vec::new();
    collect_reparse_points(root, root, &mut points)?;
    points.sort();
    Ok(points)
}

#[cfg(windows)]
fn collect_reparse_points(
    root: &Path,
    directory: &Path,
    points: &mut Vec<PathBuf>,
) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if directory == root && entry.file_name() == ".git" {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if is_windows_reparse_point(&metadata) {
            points.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
            continue;
        }
        if metadata.is_dir() {
            collect_reparse_points(root, &path, points)?;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn reparse_points_in_workspace(_root: &Path) -> io::Result<Vec<PathBuf>> {
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_workspace_reports_junction_reparse_point() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let junction = temp.path().join("junction");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("file.txt"), "demo\n").unwrap();

        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "mklink /J failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let points = reparse_points_in_workspace(temp.path()).unwrap();

        assert_eq!(points, vec![PathBuf::from("junction")]);
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_workspace_has_no_reparse_points() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("file.txt"), "demo\n").unwrap();

        let points = reparse_points_in_workspace(temp.path()).unwrap();

        assert!(points.is_empty());
    }
}
