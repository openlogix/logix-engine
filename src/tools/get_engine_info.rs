//! The get_engine_info tool (docs/tool-api.md, Tables 16 and 17).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    /// The operating system and its version, such as "macOS 26.5" or
    /// "Windows 10.0.26100.2314", for system info; on Linux with the desktop
    /// session when known, such as "Ubuntu 22.04.5 LTS, GNOME on Wayland".
    pub operating_system: String,
    /// The commit the engine was built from, short (7 hex digits); omitted
    /// for a build made outside a git checkout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_commit: Option<String>,
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
    /// The file extensions of the HDL files the tools take, without the
    /// dot, so a frontend knows which files are sources: source (design
    /// and testbench files: v, sv) and include (files they include: vh,
    /// svh). Pin files are per FPGA family: get_catalog's families give
    /// each its constraint_extension and constraint_comment.
    pub hdl_extensions: HdlExtensions,
    /// Verible's Verilog language server, for a host that runs it itself
    /// for live errors and other help while the user types: the command
    /// (its absolute path), its arguments and the environment variables to
    /// set for it, on top of the host's own (none today). Its arguments
    /// check syntax only (Verible's style rules would flood beginners;
    /// lint finds design problems) and turn hover info on. Omitted until
    /// the FPGA tools (logix-core) are downloaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language_server: Option<LanguageServer>,
    /// The developer settings in effect: the environment variables the
    /// engine reads for development and diagnostics (LOGIX_ENGINE_CHANNEL,
    /// LOGIX_ENGINE_DEV_REMOTE_CONFIG, LOGIX_ENGINE_DEV_CATALOG_FOLDER), each
    /// with its value, when set. Omitted when none is, as for users.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub developer_settings: Vec<crate::versions::DeveloperSetting>,
    /// The version tree, for system info: the engine, its packages under
    /// it, and under logix-core the third-party releases it bundles. A
    /// frontend shows it under its own entry (see Show system info in the
    /// tool API's recipes).
    pub versions: crate::versions::VersionEntry,
    /// The engine's system info for people, as plain text (for bug reports,
    /// when the engine is the product people use, such as with an AI
    /// agent): the version, operating system, log file, engine folder,
    /// automatic download and command line, then the developer settings
    /// when any is set, then the version tree. Show it as is; its wording
    /// can change in any version, so programs read the fields above.
    pub report: String,
}

/// A program a host runs itself, with its arguments and environment.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LanguageServer {
    /// The program, as an absolute path, such as
    /// .../packages/logix-core/2026-11-22/verible/bin/verible-verilog-ls.
    pub command: String,
    /// Its arguments, in order: --ruleset=none --lsp_enable_hover=true.
    pub args: Vec<String>,
    /// Environment variables to set for it, on top of the host's own.
    /// Empty today.
    pub env: BTreeMap<String, String>,
}

/// The language server's program in the verible component.
const LANGUAGE_SERVER: &str = "verible-verilog-ls";

/// Its arguments: syntax errors only, and hover info.
const LANGUAGE_SERVER_ARGS: &[&str] = &["--ruleset=none", "--lsp_enable_hover=true"];

/// Verible's language server, from a ready logix-core's verible
/// component, when its program is there.
pub fn language_server(packages: &[PackageInfo]) -> Option<LanguageServer> {
    let bin = packages
        .iter()
        .find(|p| p.name == crate::packages::CORE && p.status == PackageStatus::Ready)?
        .components
        .iter()
        .find(|c| c.name == crate::format::VERIBLE)?
        .bin_folder
        .clone();
    let program = std::path::Path::new(&bin)
        .join(format!("{LANGUAGE_SERVER}{}", std::env::consts::EXE_SUFFIX));
    program.is_file().then(|| LanguageServer {
        command: program.display().to_string(),
        args: LANGUAGE_SERVER_ARGS.iter().map(|a| a.to_string()).collect(),
        env: BTreeMap::new(),
    })
}

/// HDL file extensions, without the dot.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HdlExtensions {
    /// Design and testbench files: v (Verilog) and sv (SystemVerilog).
    pub source: Vec<String>,
    /// Files they include: vh and svh.
    pub include: Vec<String>,
}

impl HdlExtensions {
    pub fn get() -> Self {
        let list = |l: &[&str]| l.iter().map(|e| e.to_string()).collect();
        HdlExtensions {
            source: list(crate::format::SOURCE_EXTENSIONS),
            include: list(crate::format::INCLUDE_EXTENSIONS),
        }
    }
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
    /// Why it's on that channel: listed (assign.jsonc lists this release),
    /// default (it doesn't: dev), local build (always dev) or
    /// LOGIX_ENGINE_CHANNEL (a diagnostic setting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_reason: Option<String>,
    /// The config file read, such as dev-channel.jsonc, or prod-channel.jsonc when the
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
    /// hosts that run one of its programs directly (for Verible's language
    /// server, use language_server, which has its arguments too).
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
        operating_system: state::operating_system(),
        engine_commit: Some(state::ENGINE_COMMIT.to_string()).filter(|c| !c.is_empty()),
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
        hdl_extensions: HdlExtensions::get(),
        language_server: None,
        developer_settings: crate::versions::developer_settings(),
        versions: crate::versions::engine_entry(state),
        report: String::new(),
    };
    out.language_server = language_server(&out.packages);
    out.report = report(&out);
    Ok(out)
}

/// The engine's system info as text (`report`), in the layout every Open
/// Logix product uses: its own facts, the developer settings when any is
/// set, then the version tree.
fn report(info: &GetEngineInfoOutput) -> String {
    let mut lines = vec![
        format!("Logix Engine {}", info.engine_version),
        format!(
            "Operating system: {} ({})",
            info.operating_system, info.platform
        ),
        format!("Log file: {}", info.log_file),
        format!("Engine folder: {}", info.engine_folder),
        format!(
            "Automatic package download: {}",
            if info.auto_download { "on" } else { "off" }
        ),
        format!("Command line: {}", info.command_line.join(" ")),
    ];
    if !info.developer_settings.is_empty() {
        lines.push(String::new());
        lines.push("Developer settings:".into());
        for s in &info.developer_settings {
            lines.push(format!("  {} = {}", s.name, s.value));
        }
    }
    lines.push(String::new());
    lines.push("Versions:".into());
    lines.extend(crate::versions::tree_lines(&info.versions));
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
    fn the_language_server_comes_from_a_ready_core() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("verible/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let program = bin.join(format!(
            "verible-verilog-ls{}",
            std::env::consts::EXE_SUFFIX
        ));
        let core = |status| PackageInfo {
            name: crate::packages::CORE.into(),
            version: "2026.11.22".into(),
            status,
            components: vec![ComponentInfo {
                name: "verible".into(),
                description: String::new(),
                bin_folder: bin.display().to_string(),
            }],
        };
        // Not there yet.
        assert!(language_server(&[core(PackageStatus::Ready)]).is_none());
        std::fs::write(&program, "").unwrap();
        let ls = language_server(&[core(PackageStatus::Ready)]).unwrap();
        assert_eq!(ls.command, program.display().to_string());
        assert_eq!(ls.args, ["--ruleset=none", "--lsp_enable_hover=true"]);
        assert!(ls.env.is_empty());
        assert!(language_server(&[core(PackageStatus::Missing)]).is_none());
        assert!(language_server(&[]).is_none());
    }

    #[test]
    fn hdl_extensions_are_what_the_formatter_takes() {
        let e = HdlExtensions::get();
        assert_eq!(e.source, ["v", "sv"]);
        assert_eq!(e.include, ["vh", "svh"]);
        let all: Vec<String> = e
            .source
            .iter()
            .chain(&e.include)
            .map(|x| format!(".{x}"))
            .collect();
        assert_eq!(all, crate::format::EXTENSIONS);
    }

    #[test]
    fn the_report_lists_the_facts_then_the_versions() {
        use crate::versions::{DeveloperSetting, VersionEntry, VersionKind};
        let leaf = |name: &str, version: &str, notes: &[&str]| VersionEntry {
            name: name.into(),
            version: version.into(),
            kind: VersionKind::Package,
            channel: None,
            overridden_by: None,
            notes: notes.iter().map(|n| n.to_string()).collect(),
            details: Vec::new(),
            children: Vec::new(),
        };
        let mut info = GetEngineInfoOutput {
            engine_version: "2026.11.08".into(),
            protocol_versions: vec!["2025-11-25".into()],
            platform: "darwin-arm64".into(),
            operating_system: "macOS 26.5".into(),
            engine_commit: Some("abc1234".into()),
            engine_folder: "/Users/you/.logix/ide/engine-data".into(),
            auto_download: true,
            command_line: vec![
                "--engine-folder".into(),
                "/Users/you/.logix/ide/engine-data".into(),
            ],
            log_file: "/Users/you/.logix/ide/engine-data/logs/20261108-1.log".into(),
            packages: Vec::new(),
            remote_config: RemoteConfigInfo {
                config_version: "001".into(),
                source: "https://raw.githubusercontent.com/openlogix/logix-engine/main/remote-config/logix-engine/001/".into(),
                is_override: false,
                channel: Some("dev".into()),
                channel_reason: Some("LOGIX_ENGINE_CHANNEL".into()),
                config_file: Some("prod-channel.jsonc".into()),
                read_at: Some("2026-11-08T09:30:00Z".into()),
            },
            boards_schema: crate::catalog::BOARDS_SCHEMA.into(),
            hdl_extensions: HdlExtensions::get(),
            language_server: None,
            catalog: CatalogInfo {
                version: "2026.11.08".into(),
                local: false,
            },
            developer_settings: vec![DeveloperSetting {
                name: "LOGIX_ENGINE_CHANNEL".into(),
                value: "dev".into(),
            }],
            versions: VersionEntry {
                name: "Logix Engine".into(),
                version: "2026.11.08".into(),
                kind: VersionKind::Program,
                channel: Some("dev".into()),
                overridden_by: None,
                notes: vec!["channel dev, commit abc1234, remote config 001".into()],
                details: Vec::new(),
                children: vec![
                    leaf("logix-core", "2026.11.10", &["not downloaded yet"]),
                    leaf("logix-catalog", "2026.11.08", &[]),
                ],
            },
            report: String::new(),
        };
        info.report = report(&info);
        assert_eq!(
            info.report,
            "Logix Engine 2026.11.08
Operating system: macOS 26.5 (darwin-arm64)
Log file: /Users/you/.logix/ide/engine-data/logs/20261108-1.log
Engine folder: /Users/you/.logix/ide/engine-data
Automatic package download: on
Command line: --engine-folder /Users/you/.logix/ide/engine-data

Developer settings:
  LOGIX_ENGINE_CHANNEL = dev

Versions:
Logix Engine 2026.11.08
│    channel dev, commit abc1234, remote config 001
├─ logix-core 2026.11.10
│       not downloaded yet
└─ logix-catalog 2026.11.08"
        );
    }
}
