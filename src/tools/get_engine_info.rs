//! The get_engine_info tool (docs/tool-api.md, Tables 16 and 17).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::errors::ToolError;
use crate::packages::PackageManager;
use crate::packages::manifest::PackageSpec;
use crate::state::{self, EngineState};

pub struct GetEngineInfo;

impl EngineTool for GetEngineInfo {
    const NAME: &'static str = "get_engine_info";
    const DESCRIPTION: &'static str = "Returns the engine's version, platform, launch settings and \
        package status. Call it first, to check that the engine is new enough and to see which \
        toolchain packages are downloaded. It is fast and local: it never uses the network. \
        Missing packages are downloaded automatically by the tools that need them, unless \
        auto_download is off.";
    const READ_ONLY: bool = true;
    type Input = GetEngineInfoInput;
    type Output = GetEngineInfoOutput;
}

/// The get_engine_info tool takes no inputs.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEngineInfoInput {}

/// The engine's version, platform, launch settings and package status.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEngineInfoOutput {
    /// The engine version: its release date, as yyyy.mm.dd (its release tag
    /// is the same date as yyyy-mm-dd). Compare versions by date.
    #[schemars(pattern(r"^[0-9]{4}\.[0-9]{2}\.[0-9]{2}$"))]
    pub engine_version: String,
    /// The MCP protocol versions the engine accepts, newest first.
    pub protocol_versions: Vec<String>,
    /// The platform the engine runs on, such as darwin-arm64.
    pub platform: String,
    /// The engine folder, as the host set it with the --engine-folder
    /// command-line argument. The engine keeps its packages and logs there.
    pub engine_folder: String,
    /// Whether the engine downloads and repairs packages automatically when a
    /// tool needs them. On by default. The host turns it off with the
    /// --no-auto-download command-line argument.
    pub auto_download: bool,
    /// The command-line arguments the engine was launched with, as given.
    pub command_line: Vec<String>,
    /// The log file this engine process writes, in the logs subfolder of the
    /// engine folder. Useful when reporting a problem.
    pub log_file: String,
    /// Every package the engine uses, from its remote config, with its
    /// status. Empty until the engine has read its remote config (before
    /// its first operation that needs packages).
    pub packages: Vec<PackageInfo>,
    /// The engine's remote config: where its package releases come from.
    pub remote_config: RemoteConfigInfo,
    /// The address of the board schema this engine reads (Logix Catalog's):
    /// the JSON Schema of each board file, in the catalog and in custom
    /// boards folders, which may name it in a "$schema" key so editors can
    /// check them. A file that names another schema is refused.
    pub boards_schema: String,
    /// The board catalog the engine reads: its Logix Catalog version.
    pub catalog: CatalogInfo,
    /// The facts above as plain text lines for people, the engine's part of
    /// a frontend's system info (for bug reports), so every frontend shows
    /// them the same way: the version, platform, engine folder, log file,
    /// automatic download, command line, each package with its version and
    /// status, the board catalog and the boards schema. Show it as is; its wording can change
    /// in any version, so read the fields above, not this text.
    pub report: String,
}

/// The engine's remote config: which config file it reads, from where, and
/// when it last read it. It reads it again before an operation when that
/// was more than 24 hours ago, or when a host calls check_updates.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteConfigInfo {
    /// The config version this engine reads, such as 001 (the version
    /// folder remote-config/logix-engine/001/ in the engine's repo).
    pub config_version: String,
    /// Where the version folder is read: a URL, or a local folder for the
    /// diagnostic override.
    pub source: String,
    /// The release channel the engine reads, such as prod or dev; omitted
    /// before the config has been read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// Why it's on that channel: listed (channels.jsonc lists this release),
    /// default (it doesn't: dev), local build (always dev) or
    /// LOGIX_ENGINE_CHANNEL (a diagnostic setting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_reason: Option<String>,
    /// The config file read, such as dev.jsonc, or prod.jsonc when the
    /// channel has none of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_file: Option<String>,
    /// True when the source is LOGIX_ENGINE_DEV_REMOTE_CONFIG (a diagnostic
    /// setting); omitted otherwise.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_override: bool,
    /// When the config in use was read, in UTC (yyyy-mm-ddThh:mm:ssZ);
    /// omitted when it hasn't been read yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_at: Option<String>,
}

/// The board catalog (Logix Catalog, github.com/openlogix/logix-catalog),
/// downloaded as the logix-catalog package.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogInfo {
    /// The catalog's version: the date of the logix-catalog release this
    /// engine release pins, as yyyy.mm.dd, or "local" for a catalog folder
    /// from LOGIX_ENGINE_DEV_CATALOG_FOLDER.
    pub version: String,
    /// True when the engine reads a catalog folder from
    /// LOGIX_ENGINE_DEV_CATALOG_FOLDER (a diagnostic setting) instead of
    /// its package; omitted otherwise.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub local: bool,
}

/// A package (logix-catalog or logix-core) and whether it's downloaded.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageInfo {
    /// The package name: logix-catalog or logix-core.
    pub name: String,
    /// The package version: its release date, as yyyy.mm.dd.
    #[schemars(pattern(r"^[0-9]{4}\.[0-9]{2}\.[0-9]{2}$"))]
    pub version: String,
    pub status: PackageStatus,
    /// The tools in logix-core, each its upstream release in a folder of
    /// its own, when the core is downloaded; empty otherwise, and for
    /// logix-catalog.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<ComponentInfo>,
}

/// One tool in logix-core, such as verible.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentInfo {
    /// The component name: oss-cad-suite, openxc7, verible or elkjs
    /// (Windows).
    pub name: String,
    pub description: String,
    /// The folder with the component's programs, as an absolute path. For
    /// hosts that run one of its programs directly, such as Verible's
    /// language server verible-verilog-ls from the verible component.
    pub bin_folder: String,
}

/// Whether a package is ready to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PackageStatus {
    /// Downloaded and intact.
    Ready,
    /// Not downloaded.
    Missing,
    /// Downloaded but incomplete or damaged. It is repaired when a tool needs
    /// it, unless auto-download is off.
    Broken,
}

pub fn run(
    state: &EngineState,
    _input: GetEngineInfoInput,
) -> Result<GetEngineInfoOutput, ToolError> {
    let mut out = GetEngineInfoOutput {
        engine_version: state::ENGINE_VERSION.to_string(),
        protocol_versions: state::PROTOCOL_VERSIONS
            .iter()
            .map(|v| v.to_string())
            .collect(),
        platform: state::platform(),
        engine_folder: state.engine_folder.clone(),
        auto_download: state.auto_download,
        command_line: state.command_line.clone(),
        log_file: state.log_file.display().to_string(),
        packages: package_list(state),
        remote_config: remote_config_info(&state.packages),
        boards_schema: crate::catalog::BOARDS_SCHEMA.to_string(),
        catalog: match crate::catalog::source() {
            Some(s) if s.local => CatalogInfo {
                version: s.version,
                local: true,
            },
            _ => CatalogInfo {
                version: state
                    .packages
                    .manifest()
                    .get(crate::packages::CATALOG)
                    .map(|s| crate::packages::manifest::shown_version(&s.version))
                    .unwrap_or_default(),
                local: false,
            },
        },
        report: String::new(),
    };
    out.report = report(&out);
    Ok(out)
}

/// The engine's part of a system info report (`report`).
fn report(info: &GetEngineInfoOutput) -> String {
    let mut lines = vec![
        format!("Logix Engine {}", info.engine_version),
        format!("Platform: {}", info.platform),
        format!("Engine folder: {}", info.engine_folder),
        format!("Log file: {}", info.log_file),
        format!(
            "Automatic package download: {}",
            if info.auto_download { "on" } else { "off" }
        ),
        format!("Command line: {}", info.command_line.join(" ")),
        "Packages:".to_string(),
    ];
    // In columns: name, version, status.
    let name_width = info
        .packages
        .iter()
        .map(|p| p.name.len())
        .max()
        .unwrap_or(0);
    let version_width = info
        .packages
        .iter()
        .map(|p| p.version.len())
        .max()
        .unwrap_or(0);
    for p in &info.packages {
        let status = match p.status {
            PackageStatus::Ready => "ready",
            PackageStatus::Missing => "missing",
            PackageStatus::Broken => "broken",
        };
        lines.push(format!(
            "  {:name_width$}  {:version_width$}  {status}",
            p.name, p.version
        ));
    }
    let c = &info.catalog;
    lines.push(format!(
        "Board catalog: Logix Catalog {}{}",
        c.version,
        if c.local {
            " (a local folder, LOGIX_ENGINE_DEV_CATALOG_FOLDER)"
        } else {
            ""
        }
    ));
    let r = &info.remote_config;
    let channel = match (&r.channel, &r.channel_reason, &r.config_file) {
        (Some(c), Some(why), Some(file)) => {
            let fallback = if *file != format!("{c}.jsonc") {
                format!(", using {file}")
            } else {
                String::new()
            };
            format!(", channel {c} ({why}{fallback})")
        }
        _ => String::new(),
    };
    lines.push(format!(
        "Remote config: {} from {}{}{channel}, {}",
        r.config_version,
        r.source,
        if r.is_override {
            " (LOGIX_ENGINE_DEV_REMOTE_CONFIG)"
        } else {
            ""
        },
        r.read_at
            .as_ref()
            .map_or("not read yet".to_string(), |t| format!("read {t}"))
    ));
    lines.push(format!("Boards schema: {}", info.boards_schema));
    lines.join("\n")
}

/// The remote config the package manager uses.
pub fn remote_config_info(packages: &PackageManager) -> RemoteConfigInfo {
    use crate::packages::remote_config;
    let r = packages.remote_info();
    RemoteConfigInfo {
        config_version: remote_config::CONFIG_VERSION.to_string(),
        source: r
            .as_ref()
            .map_or_else(remote_config::default_url, |r| r.location.clone()),
        is_override: r.as_ref().is_some_and(|r| r.is_override),
        channel: r
            .as_ref()
            .and_then(|r| r.channel.as_ref())
            .map(|c| c.name.clone()),
        channel_reason: r
            .as_ref()
            .and_then(|r| r.channel.as_ref())
            .map(|c| c.how.clone()),
        config_file: r
            .as_ref()
            .and_then(|r| r.channel.as_ref())
            .map(|c| c.file.clone()),
        read_at: r.and_then(|r| r.fetched).map(remote_config::utc_time),
    }
}

/// Every package the engine uses, with its status.
pub fn package_list(state: &EngineState) -> Vec<PackageInfo> {
    state
        .packages
        .manifest()
        .packages
        .iter()
        .map(|spec| package_info(&state.packages, spec))
        .collect()
}

/// One package's name, version, status and, for a downloaded core, its
/// components.
pub fn package_info(packages: &PackageManager, spec: &PackageSpec) -> PackageInfo {
    let status = packages.status(spec);
    PackageInfo {
        name: spec.name.clone(),
        version: crate::packages::manifest::shown_version(&spec.version),
        status,
        components: match (status, spec.name.as_str()) {
            (PackageStatus::Ready, crate::packages::CORE) => packages
                .core_components()
                .unwrap_or_default()
                .into_iter()
                .map(|(name, c)| ComponentInfo {
                    bin_folder: packages
                        .package_dir(spec)
                        .join(c.run.path.first().unwrap_or(&c.folder))
                        .display()
                        .to_string(),
                    name,
                    description: c.description,
                })
                .collect(),
            _ => Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_lists_the_facts_for_people() {
        let mut info = GetEngineInfoOutput {
            engine_version: "2026.11.08".into(),
            protocol_versions: vec!["2025-11-25".into()],
            platform: "darwin-arm64".into(),
            engine_folder: "/Users/you/.logix/ide/engine".into(),
            auto_download: true,
            command_line: vec![
                "--engine-folder".into(),
                "/Users/you/.logix/ide/engine".into(),
            ],
            log_file: "/Users/you/.logix/ide/engine/logs/20261108-1.log".into(),
            packages: vec![
                PackageInfo {
                    name: "logix-catalog".into(),
                    version: "2026.11.08".into(),
                    status: PackageStatus::Ready,
                    components: Vec::new(),
                },
                PackageInfo {
                    name: "logix-core".into(),
                    version: "2026.11.10".into(),
                    status: PackageStatus::Missing,
                    components: Vec::new(),
                },
            ],
            remote_config: RemoteConfigInfo {
                config_version: "001".into(),
                source: "https://raw.githubusercontent.com/openlogix/logix-engine/main/remote-config/logix-engine/001/".into(),
                is_override: false,
                channel: Some("dev".into()),
                channel_reason: Some("default".into()),
                config_file: Some("prod.jsonc".into()),
                read_at: Some("2026-11-08T09:30:00Z".into()),
            },
            boards_schema: crate::catalog::BOARDS_SCHEMA.into(),
            catalog: CatalogInfo {
                version: "2026.11.08".into(),
                local: false,
            },
            report: String::new(),
        };
        info.report = report(&info);
        assert_eq!(
            info.report,
            "Logix Engine 2026.11.08
Platform: darwin-arm64
Engine folder: /Users/you/.logix/ide/engine
Log file: /Users/you/.logix/ide/engine/logs/20261108-1.log
Automatic package download: on
Command line: --engine-folder /Users/you/.logix/ide/engine
Packages:
  logix-catalog  2026.11.08  ready
  logix-core     2026.11.10  missing
Board catalog: Logix Catalog 2026.11.08
Remote config: 001 from https://raw.githubusercontent.com/openlogix/logix-engine/main/remote-config/logix-engine/001/, channel dev (default, using prod.jsonc), read 2026-11-08T09:30:00Z
Boards schema: https://www.openlogix.org/schemas/logix-board-v1.json"
        );
    }
}
