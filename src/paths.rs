//! Path checks from docs/requirements.md ("Safe file changes") and the path
//! types in docs/tool-api.md. Every check returns a message that says which
//! rule failed.

use std::path::{Path, PathBuf};

/// Checks the "absolute path" form: a complete path with no `.` or `..`
/// segments and no `~` or environment variables.
pub fn check_absolute_path(s: &str) -> Result<PathBuf, String> {
    if s.is_empty() {
        return Err("the path is empty".into());
    }
    if s.starts_with('~') {
        return Err(format!(
            "{s:?} starts with '~'. Give the full path, with the home folder expanded"
        ));
    }
    if s.contains('$') || (cfg!(windows) && s.contains('%')) {
        return Err(format!(
            "{s:?} contains an environment variable. Give the full path, with variables expanded"
        ));
    }
    let path = PathBuf::from(s);
    if !path.is_absolute() {
        return Err(format!("{s:?} is not an absolute path"));
    }
    let separators: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    if s.split(separators).any(|seg| seg == "." || seg == "..") {
        return Err(format!(
            "{s:?} contains '.' or '..' segments. Give the normalized path"
        ));
    }
    Ok(path)
}

/// Resolves a path to its real location, following symbolic links. The path
/// doesn't need to exist: its deepest existing ancestor is resolved and the
/// rest is appended.
pub fn resolve(path: &Path) -> std::io::Result<PathBuf> {
    let mut existing = path.to_path_buf();
    let mut rest = Vec::new();
    loop {
        match existing.canonicalize() {
            Ok(real) => {
                let mut out = real;
                for name in rest.iter().rev() {
                    out.push(name);
                }
                return Ok(out);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let name = existing.file_name().map(|n| n.to_os_string());
                match (name, existing.parent()) {
                    (Some(name), Some(parent)) => {
                        rest.push(name);
                        existing = parent.to_path_buf();
                    }
                    _ => return Err(e),
                }
            }
            Err(e) => return Err(e),
        }
    }
}

fn last_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn is_forbidden(resolved: &Path, home: Option<&Path>) -> Option<&'static str> {
    if resolved.parent().is_none() {
        return Some("the file system root");
    }
    if let Some(home) = home {
        let home = resolve(home).unwrap_or_else(|_| home.to_path_buf());
        if resolved == home {
            return Some("the user's home folder");
        }
    }
    None
}

/// Checks an engine folder path. Its last folder name must contain "engine",
/// both as given and after resolving symbolic links, and it can't be the file
/// system root or the home folder. Returns the resolved path.
pub fn check_engine_folder(s: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let given = check_absolute_path(s)?;
    if !last_name(&given).contains("engine") {
        return Err(format!(
            "the last folder name of the engine folder {s:?} must contain \"engine\", such as ~/.myhost/engine"
        ));
    }
    let resolved =
        resolve(&given).map_err(|e| format!("can't resolve the engine folder {s:?}: {e}"))?;
    if let Some(what) = is_forbidden(&resolved, home) {
        return Err(format!("the engine folder {s:?} can't be {what}"));
    }
    if !last_name(&resolved).contains("engine") {
        return Err(format!(
            "the engine folder {s:?} resolves to {:?}, whose last folder name doesn't contain \"engine\"",
            resolved.display()
        ));
    }
    Ok(resolved)
}

/// The user's home folder, from the environment.
pub fn home_folder() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_path_rules() {
        assert!(check_absolute_path("/a/b").is_ok());
        assert!(check_absolute_path("").is_err());
        assert!(check_absolute_path("a/b").is_err());
        assert!(check_absolute_path("~/a").is_err());
        assert!(check_absolute_path("/a/$HOME/b").is_err());
        assert!(check_absolute_path("/a/./b").is_err());
        assert!(check_absolute_path("/a/../b").is_err());
        assert!(check_absolute_path("/a/b/..").is_err());
        assert!(check_absolute_path("/a/..b/c.d").is_ok());
    }

    #[test]
    fn engine_folder_keyword_is_checked_on_the_last_name_only() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().to_str().unwrap();
        assert!(check_engine_folder(&format!("{base}/x/engine"), None).is_ok());
        assert!(check_engine_folder(&format!("{base}/x/my-engine-2"), None).is_ok());
        let err = check_engine_folder(&format!("{base}/engine/x"), None).unwrap_err();
        assert!(err.contains("must contain \"engine\""), "{err}");
    }

    #[test]
    fn engine_folder_cannot_be_root_or_home() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("engine-home");
        std::fs::create_dir(&home).unwrap();
        let err = check_engine_folder(home.to_str().unwrap(), Some(&home)).unwrap_err();
        assert!(err.contains("home folder"), "{err}");
        assert!(check_engine_folder("/", None).is_err());
    }

    #[test]
    fn engine_folder_is_checked_after_resolving_links() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("not-it");
        std::fs::create_dir(&target).unwrap();
        let link = tmp.path().join("engine");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(unix)]
        {
            let err = check_engine_folder(link.to_str().unwrap(), None).unwrap_err();
            assert!(err.contains("resolves to"), "{err}");
        }
    }

    #[test]
    fn resolve_handles_missing_tails() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().canonicalize().unwrap();
        assert_eq!(
            resolve(&tmp.path().join("a/b")).unwrap(),
            real.join("a").join("b")
        );
    }
}
