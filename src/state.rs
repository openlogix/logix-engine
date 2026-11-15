//! What an engine process knows from its launch. Requests are stateless: the
//! only state is the launch settings, which are fixed for the life of the
//! process.

use std::path::PathBuf;

use rmcp::model::ProtocolVersion;

/// The engine version as people see it, yyyy.mm.dd: its release date. Set
/// by build.rs.
pub const ENGINE_VERSION: &str = env!("LOGIX_ENGINE_BUILD_VERSION");

/// The commit the engine was built from, short (7 hex digits); empty when
/// built outside a git checkout. Set by build.rs.
pub const ENGINE_COMMIT: &str = env!("LOGIX_ENGINE_BUILD_COMMIT");

/// The MCP protocol versions the engine accepts, newest first. The engine is
/// pinned to 2026-07-28 and also accepts 2025-11-25 clients.
pub const PROTOCOL_VERSIONS: &[ProtocolVersion] =
    &[ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25];

/// The platform the engine was built for, such as `darwin-arm64`.
pub fn platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x86-64",
        other => other,
    };
    format!("{os}-{arch}")
}

/// The operating system and its version, such as "macOS 26.5" or "Windows
/// 10.0.26100.2314", for system info; the platform's name when the version
/// can't be read. Read once.
pub fn operating_system() -> String {
    static OS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    OS.get_or_init(|| os_version().unwrap_or_else(platform))
        .clone()
}

fn os_version() -> Option<String> {
    let run = |cmd: &mut std::process::Command| {
        cmd.output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    if cfg!(target_os = "macos") {
        let v = run(std::process::Command::new("/usr/bin/sw_vers").arg("-productVersion"))?;
        return Some(format!("macOS {v}"));
    }
    if cfg!(windows) {
        // "Microsoft Windows [Version 10.0.26100.2314]" -> "Windows 10.0.26100.2314".
        let mut cmd = std::process::Command::new("cmd.exe");
        cmd.args(["/c", "ver"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let text = run(&mut cmd)?;
        let v = text.split("Version ").nth(1)?.trim().trim_end_matches(']');
        return Some(format!("Windows {v}"));
    }
    // Linux: PRETTY_NAME from /etc/os-release, such as "Ubuntu 24.04.1 LTS".
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim_matches('"').to_string())
}

#[derive(Debug, Clone)]
pub struct EngineState {
    /// The engine folder as given on the command line.
    pub engine_folder: String,
    /// The engine folder after resolving symbolic links. All disk changes
    /// use this path.
    pub engine_folder_resolved: PathBuf,
    pub auto_download: bool,
    /// The command-line arguments, as given, without the program name.
    pub command_line: Vec<String>,
    pub log_file: PathBuf,
    /// Downloads and finds the packages in the engine folder.
    pub packages: crate::packages::PackageManager,
}

impl EngineState {
    /// Makes sure the board catalog is loaded: the logix-catalog release the
    /// remote config names, downloaded first when needed and auto-download
    /// is on.
    /// A catalog folder from LOGIX_ENGINE_DEV_CATALOG_FOLDER, loaded at
    /// startup, stays.
    pub async fn ensure_catalog(
        &self,
        call: &crate::tools::Call,
    ) -> Result<(), crate::errors::ToolError> {
        use crate::packages::CATALOG;
        if crate::catalog::source().is_some_and(|s| s.local) {
            return Ok(());
        }
        let needed = self.packages.needing_download(&[CATALOG]).await?;
        let spec = self.packages.spec(CATALOG)?;
        if !needed.is_empty() {
            let phases = crate::progress::Phases::new(call.progress.clone(), 1);
            self.packages
                .ensure(&needed, self.auto_download, &phases, &call.cancel)
                .await?;
        }
        let folder = self.packages.package_dir(&spec);
        crate::catalog::load(&folder, &spec.version, false).map_err(|e| {
            crate::errors::ToolError::setup(format!("Can't read the board catalog: {e}"))
                .with_action(
                    crate::errors::ActionCode::DownloadPackages,
                    format!("Download the {CATALOG} package again, which may be damaged."),
                )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_version_is_a_dotted_date() {
        let v = ENGINE_VERSION;
        assert_eq!(v.len(), 10, "{v}");
        assert!(
            crate::packages::manifest::is_date_tag(&v.replace('.', "-")) && !v.contains('-'),
            "{v}"
        );
    }

    #[test]
    fn platform_name() {
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            assert_eq!(platform(), "darwin-arm64");
        }
    }
}
