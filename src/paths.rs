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

/// Checks the "relative path" form: `/` separators, not starting with `/`,
/// no empty, `.` or `..` segments, and no backslashes.
pub fn check_relative_path(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("the path is empty".into());
    }
    if s.starts_with('/') {
        return Err(format!("{s:?} must be relative, not start with '/'"));
    }
    if s.contains('\\') {
        return Err(format!("{s:?} must use '/' as the separator"));
    }
    if s.split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return Err(format!(
            "{s:?} has an empty, '.' or '..' segment. Give a normalized relative path"
        ));
    }
    Ok(())
}

/// Resolves `rel` under `base` (already resolved) and checks that the real
/// path, after following symbolic links, stays inside `base`. The path must
/// exist.
pub fn resolve_inside(base: &Path, rel: &str) -> Result<PathBuf, String> {
    let joined = base.join(rel);
    let real = joined
        .canonicalize()
        .map_err(|e| format!("{rel:?} can't be found in {:?}: {e}", base.display()))?;
    if !real.starts_with(base) {
        return Err(format!(
            "{rel:?} resolves to {:?}, outside {:?}",
            real.display(),
            base.display()
        ));
    }
    Ok(real)
}

fn has_build_keyword(path: &Path) -> bool {
    let mut names = path.iter().rev().filter_map(|n| n.to_str());
    names.next().is_some_and(|n| n.contains("build"))
        || (path.parent().is_some() && names.next().is_some_and(|n| n.contains("build")))
}

/// Checks a build folder path: absolute, one of its last two folder names
/// contains "build" (as given and after resolving symbolic links), and it
/// isn't the file system root, the home folder or the source folder.
/// Returns the resolved path. The folder doesn't need to exist.
pub fn check_build_folder(
    s: &str,
    home: Option<&Path>,
    source_folder: Option<&Path>,
) -> Result<PathBuf, String> {
    let given = check_absolute_path(s)?;
    if !has_build_keyword(&given) {
        return Err(format!(
            "one of the last two folder names of the build folder {s:?} must contain \"build\", such as _build or _build/default"
        ));
    }
    let resolved =
        resolve(&given).map_err(|e| format!("can't resolve the build folder {s:?}: {e}"))?;
    if let Some(what) = is_forbidden(&resolved, home) {
        return Err(format!("the build folder {s:?} can't be {what}"));
    }
    if source_folder.is_some_and(|src| src == resolved) {
        return Err(format!(
            "the build folder {s:?} can't be the source folder itself. Use a folder inside it, such as _build"
        ));
    }
    if !has_build_keyword(&resolved) {
        return Err(format!(
            "the build folder {s:?} resolves to {:?}, whose last two folder names don't contain \"build\"",
            resolved.display()
        ));
    }
    Ok(resolved)
}

#[cfg(test)]
mod build_folder_tests {
    use super::*;

    #[test]
    fn relative_path_rules() {
        assert!(check_relative_path("main.v").is_ok());
        assert!(check_relative_path("rtl/uart.v").is_ok());
        for bad in [
            "", "/main.v", "a//b.v", "./main.v", "a/../b.v", "a\\b.v", "a/",
        ] {
            assert!(check_relative_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn build_folder_keyword_in_the_last_two_names() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let b = base.to_str().unwrap();
        assert!(check_build_folder(&format!("{b}/p/_build"), None, None).is_ok());
        assert!(check_build_folder(&format!("{b}/p/_build/default"), None, None).is_ok());
        assert!(check_build_folder(&format!("{b}/build/x/y"), None, None).is_err());
        assert!(check_build_folder(&format!("{b}/p/out"), None, None).is_err());
        let src = base.join("build-src");
        std::fs::create_dir(&src).unwrap();
        let err = check_build_folder(src.to_str().unwrap(), None, Some(&src)).unwrap_err();
        assert!(err.contains("source folder"), "{err}");
    }

    #[test]
    fn resolve_inside_refuses_links_out() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap().join("src");
        std::fs::create_dir(&base).unwrap();
        std::fs::write(base.join("a.v"), "").unwrap();
        std::fs::write(tmp.path().join("outside.v"), "").unwrap();
        assert!(resolve_inside(&base, "a.v").is_ok());
        assert!(resolve_inside(&base, "missing.v").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(tmp.path().join("outside.v"), base.join("link.v")).unwrap();
            let err = resolve_inside(&base, "link.v").unwrap_err();
            assert!(err.contains("outside"), "{err}");
        }
    }
}
