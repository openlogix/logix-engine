//! Deleting build folders (docs/tool-api.md, "clean_build tool"). Only
//! folders with the engine's marker are deleted, so a wrong path can't remove
//! the user's files.

use std::fs;
use std::path::{Path, PathBuf};

use super::MARKER;
use crate::errors::ToolError;

fn is_build_folder(p: &Path) -> bool {
    // symlink_metadata: a link is never treated as a build folder, so the
    // engine never deletes through it.
    fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()) && p.join(MARKER).is_file()
}

fn delete(p: &Path) -> Result<(), ToolError> {
    fs::remove_dir_all(p)
        .map_err(|e| ToolError::setup(format!("Can't delete {}: {e}", p.display())))
}

/// Deletes `folder` if it's a build folder, or else the build folders
/// directly inside it and then `folder` itself if that leaves it empty.
/// `given` is the folder as the host gave it, used in the returned paths;
/// `real` is its real path. Returns the deleted folders.
pub fn clean(given: &Path, real: &Path) -> Result<Vec<PathBuf>, ToolError> {
    if !real.exists() {
        return Ok(Vec::new());
    }
    if !real.is_dir() {
        return Err(ToolError::request(format!(
            "{} isn't a folder.",
            given.display()
        )));
    }
    if is_build_folder(real) {
        delete(real)?;
        log::info!("Deleted build folder {}", real.display());
        return Ok(vec![given.to_path_buf()]);
    }
    let mut deleted = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(real)
        .map_err(|e| ToolError::setup(format!("Can't read {}: {e}", given.display())))?
        .flatten()
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let p = entry.path();
        if is_build_folder(&p) {
            delete(&p)?;
            log::info!("Deleted build folder {}", p.display());
            deleted.push(given.join(entry.file_name()));
        }
    }
    let empty = fs::read_dir(real).is_ok_and(|mut d| d.next().is_none());
    if !deleted.is_empty() && empty {
        fs::remove_dir(real)
            .map_err(|e| ToolError::setup(format!("Can't delete {}: {e}", given.display())))?;
        deleted.push(given.to_path_buf());
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_folder(p: &Path) {
        fs::create_dir_all(p).unwrap();
        fs::write(p.join(MARKER), "").unwrap();
        fs::write(p.join("hardware.bin"), "x").unwrap();
    }

    #[test]
    fn deletes_a_build_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let b = tmp.path().join("_build/default");
        build_folder(&b);
        assert_eq!(clean(&b, &b).unwrap(), vec![b.clone()]);
        assert!(!b.exists());
        assert!(tmp.path().join("_build").exists(), "only the given folder");
    }

    #[test]
    fn deletes_the_build_folders_inside_a_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("_build");
        build_folder(&root.join("default"));
        build_folder(&root.join("icezum"));
        let got = clean(&root, &root).unwrap();
        assert_eq!(
            got,
            vec![root.join("default"), root.join("icezum"), root.clone()]
        );
        assert!(!root.exists());
    }

    #[test]
    fn keeps_everything_without_the_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("_build");
        build_folder(&root.join("default"));
        fs::create_dir_all(root.join("notes")).unwrap();
        fs::write(root.join("notes/n.txt"), "keep").unwrap();
        fs::write(root.join("keep.txt"), "keep").unwrap();
        let got = clean(&root, &root).unwrap();
        assert_eq!(got, vec![root.join("default")]);
        assert!(root.join("notes/n.txt").exists() && root.join("keep.txt").exists());

        // A project folder with a build folder two levels down: nothing,
        // since only direct subfolders are looked at.
        let proj = tmp.path().join("proj");
        build_folder(&proj.join("_build/default"));
        fs::write(proj.join("main.v"), "").unwrap();
        assert!(clean(&proj, &proj).unwrap().is_empty());
        assert!(proj.join("main.v").exists());
    }

    #[test]
    fn missing_folder_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("_build");
        assert!(clean(&p, &p).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn links_are_not_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let elsewhere = tmp.path().join("elsewhere");
        build_folder(&elsewhere);
        let root = tmp.path().join("_build");
        fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join("default")).unwrap();
        assert!(clean(&root, &root).unwrap().is_empty());
        assert!(elsewhere.join("hardware.bin").exists());
    }
}
