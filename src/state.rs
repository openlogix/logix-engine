//! What an engine process knows from its launch. Requests are stateless: the
//! only state is the launch settings, which are fixed for the life of the
//! process.

use std::path::PathBuf;

use rmcp::model::ProtocolVersion;

/// The engine release, a date tag. Set by build.rs.
pub const ENGINE_VERSION: &str = env!("LOGIX_ENGINE_VERSION");

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

#[derive(Debug, Clone)]
pub struct EngineState {
    /// The engine folder as given on the command line.
    pub engine_folder: String,
    /// The engine folder after resolving symbolic links. All disk changes
    /// use this path.
    pub engine_folder_resolved: PathBuf,
    pub auto_install: bool,
    /// The command-line arguments, as given, without the program name.
    pub command_line: Vec<String>,
    pub log_file: PathBuf,
    /// Installs and finds the toolchain packages in the engine folder.
    pub packages: crate::packages::PackageManager,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_version_is_a_date_tag() {
        let v = ENGINE_VERSION;
        assert!(v.len() >= 10, "{v}");
        assert!(
            v[..4].parse::<u32>().is_ok() && &v[4..5] == "-" && &v[7..8] == "-",
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
