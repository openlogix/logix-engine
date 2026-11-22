//! System info's version tree and developer settings (docs/tool-api.md,
//! Show system info; the website repo's `CLAUDE.md`, System info and the
//! version tree, owns the layout).
//!
//! The tree starts at the engine and goes down through its packages to the
//! third-party releases logix-core bundles, where it stops. Each entry says
//! only what its name and version don't already tell in `notes` (shown in
//! text reports) and the rest in `details` (for a frontend's hover). Every
//! entry comes from what's in place (receipts, the core's own
//! `logix-core.json`), so nothing here is listed by hand except the names
//! people know the third-party tools by and the programs the engine runs
//! from each (`THIRD_PARTY`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::packages::manifest::{PackageSpec, shown_version};
use crate::packages::{CATALOG, CORE, CoreComponent, PackageManager};
use crate::state::{self, EngineState};
use crate::tools::get_engine_info::PackageStatus;

/// One entry of the version tree: a program, a package or a third-party
/// release, with its own dependencies under it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VersionEntry {
    /// What people call it: a program by its product name (Logix Engine), a
    /// package by its package name (logix-core), a third-party release by
    /// its project's name (OSS CAD Suite).
    pub name: String,
    /// Its version as people see it: yyyy.mm.dd for Open Logix releases,
    /// upstream's own for third-party releases, or a note such as "local
    /// folder" when a developer setting replaces it.
    pub version: String,
    pub kind: VersionKind,
    /// A program's release channel, such as prod or dev; omitted for
    /// packages and before the remote config has been read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// The developer setting (an environment variable) that replaces this
    /// entry, such as LOGIX_ENGINE_DEV_CATALOG_FOLDER; omitted otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overridden_by: Option<String>,
    /// Short lines for people, under the entry in a text report: only what
    /// the name and version don't already tell (a program's channel, commit
    /// and remote config; a third-party release's upstream; an override; a
    /// package that isn't downloaded yet).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// Everything else known about it, as label and value pairs in order,
    /// for a frontend to show on demand (such as on hover): its repo,
    /// release, file, checksum, which config named it, and for a third-party
    /// release the programs Open Logix runs from it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<VersionDetail>,
    /// Its own dependencies, in the order to show them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<VersionEntry>,
}

/// What kind of thing a version tree entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VersionKind {
    /// An Open Logix program, such as Logix Engine.
    Program,
    /// An Open Logix package, such as logix-core.
    Package,
    /// A third-party release bundled unchanged, such as OSS CAD Suite.
    ThirdParty,
}

/// One label and value of a version tree entry's details.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VersionDetail {
    pub label: String,
    pub value: String,
}

/// A developer setting in effect: an environment variable and its value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeveloperSetting {
    /// The variable, such as LOGIX_ENGINE_CHANNEL.
    pub name: String,
    pub value: String,
}

/// The third-party releases in logix-core, by component: the name people
/// know each by and the programs Open Logix runs from it (only those, not
/// everything it holds). Keep it in step with the programs the engine runs
/// (and hosts run from `bin_folder`, such as verible-verilog-ls).
const THIRD_PARTY: &[(&str, &str, &[&str])] = &[
    (
        "oss-cad-suite",
        "OSS CAD Suite",
        &[
            "yosys",
            "nextpnr-ice40",
            "nextpnr-ecp5",
            "nextpnr-himbaechel",
            "icepack",
            "iceprog",
            "ecppack",
            "gowin_pack",
            "iverilog",
            "vvp",
            "verilator",
            "openFPGALoader",
            "dfu-util",
        ],
    ),
    (
        "openxc7",
        "openXC7",
        &["nextpnr-xilinx", "fasm2frames", "xc7frames2bit"],
    ),
    (
        "verible",
        "Verible",
        &["verible-verilog-format", "verible-verilog-ls"],
    ),
    ("elkjs", "elkjs", &["elkjs (lays out design diagrams)"]),
    ("wdi-simple", "wdi-simple (libwdi)", &["wdi-simple"]),
];

/// The developer settings the engine reads, in effect now (set and not
/// empty): docs/command-line.md, Diagnostic settings.
pub fn developer_settings() -> Vec<DeveloperSetting> {
    [
        crate::packages::remote_config::CHANNEL_VAR,
        crate::packages::remote_config::DEV_REMOTE_CONFIG_VAR,
        crate::catalog::DEV_CATALOG_VAR,
        crate::viewer::DEV_VIEWER_VAR,
    ]
    .iter()
    .filter_map(|name| {
        std::env::var(name)
            .ok()
            .filter(|v| !v.is_empty())
            .map(|value| DeveloperSetting {
                name: name.to_string(),
                value,
            })
    })
    .collect()
}

fn detail(label: &str, value: impl Into<String>) -> VersionDetail {
    VersionDetail {
        label: label.into(),
        value: value.into(),
    }
}

/// The engine's own entry, with its packages under it.
pub fn engine_entry(state: &EngineState) -> VersionEntry {
    let remote = crate::tools::get_engine_info::remote_config_info(&state.packages);
    let mut notes = Vec::new();
    let mut summary = Vec::new();
    if let Some(c) = &remote.channel {
        summary.push(format!("channel {c}"));
    }
    if !state::ENGINE_COMMIT.is_empty() {
        summary.push(format!("commit {}", state::ENGINE_COMMIT));
    }
    summary.push(format!("remote config {}", remote.config_version));
    notes.push(summary.join(", "));
    let mut details = vec![detail("Repo", "openlogix/logix-engine")];
    details.push(if crate::packages::remote_config::IS_RELEASE_BUILD {
        detail("Release", state::ENGINE_VERSION.replace('.', "-"))
    } else {
        detail("Build", "local build")
    });
    if !state::ENGINE_COMMIT.is_empty() {
        details.push(detail("Commit", state::ENGINE_COMMIT));
    }
    if let (Some(c), Some(why), Some(file)) =
        (&remote.channel, &remote.channel_reason, &remote.config_file)
    {
        details.push(detail("Channel", format!("{c} ({why}), reads {file}")));
    }
    details.push(detail(
        "Remote config",
        format!("{}, from {}", remote.config_version, remote.source),
    ));
    details.push(detail(
        "Read",
        remote.read_at.clone().unwrap_or_else(|| "not yet".into()),
    ));
    // Where it keeps its packages and writes its log: a frontend's dev
    // settings may move them (a local engine, a test remote config).
    details.push(detail("Engine folder", state.engine_folder.clone()));
    details.push(detail("Log file", state.log_file.display().to_string()));
    let named_by = match &remote.config_file {
        Some(f) => format!(
            "Logix Engine's remote config {}, {f}",
            remote.config_version
        ),
        None => format!("Logix Engine's remote config {}", remote.config_version),
    };
    let manifest = state.packages.manifest();
    // logix-core first, then the rest in the config's order.
    let mut specs: Vec<&PackageSpec> = manifest.packages.iter().collect();
    specs.sort_by_key(|s| s.name != CORE);
    let children = specs
        .into_iter()
        .map(|spec| package_entry(&state.packages, spec, &named_by))
        .collect();
    VersionEntry {
        name: "Logix Engine".into(),
        version: state::ENGINE_VERSION.into(),
        kind: VersionKind::Program,
        channel: remote.channel.clone(),
        overridden_by: None,
        notes,
        details,
        children,
    }
}

/// A package's entry: the release the remote config names, and for a
/// ready logix-core, its third-party releases.
fn package_entry(packages: &PackageManager, spec: &PackageSpec, named_by: &str) -> VersionEntry {
    if spec.name == CATALOG
        && let Some(s) = crate::catalog::source().filter(|s| s.local)
    {
        let var = crate::catalog::DEV_CATALOG_VAR;
        return VersionEntry {
            name: spec.name.clone(),
            version: "local folder".into(),
            kind: VersionKind::Package,
            channel: None,
            overridden_by: Some(var.into()),
            notes: vec![format!("dev override: {var}")],
            details: vec![
                detail("Folder", s.folder.display().to_string()),
                detail("Variable", var),
            ],
            children: Vec::new(),
        };
    }
    let status = packages.status(spec);
    let mut notes = Vec::new();
    match status {
        PackageStatus::Ready => {}
        PackageStatus::Missing => notes.push("not downloaded yet".to_string()),
        PackageStatus::Broken => {
            notes.push("damaged: downloaded again when it's needed".to_string())
        }
    }
    let mut details = Vec::new();
    if let Some(f) = packages.file_for(spec) {
        details.push(detail("Repo", format!("{}/{}", f.owner, f.repo)));
        details.push(detail("Release", f.tag.clone()));
        details.push(detail("File", f.file.clone()));
        details.push(detail("SHA-256", f.sha256.clone()));
    }
    details.push(detail("Pinned in", named_by));
    let children = if status == PackageStatus::Ready && spec.name == CORE {
        third_party_entries(packages.core_components().unwrap_or_default())
    } else {
        Vec::new()
    };
    VersionEntry {
        name: spec.name.clone(),
        version: shown_version(&spec.version),
        kind: VersionKind::Package,
        channel: None,
        overridden_by: None,
        notes,
        details,
        children,
    }
}

/// logix-core's third-party releases, in `THIRD_PARTY`'s order, then any
/// other component by its own name.
pub fn third_party_entries(
    components: std::collections::BTreeMap<String, CoreComponent>,
) -> Vec<VersionEntry> {
    let rank = |id: &str| {
        THIRD_PARTY
            .iter()
            .position(|(c, _, _)| *c == id)
            .unwrap_or(THIRD_PARTY.len())
    };
    let mut list: Vec<(String, CoreComponent)> = components.into_iter().collect();
    list.sort_by_key(|(id, _)| rank(id));
    list.into_iter()
        .map(|(id, c)| {
            let known = THIRD_PARTY.iter().find(|(cid, _, _)| *cid == id);
            let s = &c.source;
            let upstream = s.repo.clone().or_else(|| {
                s.upstream
                    .as_deref()
                    .map(|u| u.trim_start_matches("https://github.com/").to_string())
            });
            let version = s
                .release
                .clone()
                .or_else(|| s.tag.clone())
                .or_else(|| s.version.clone())
                .unwrap_or_else(|| "unknown".into());
            let mut details = Vec::new();
            if let Some(u) = &s.upstream {
                details.push(detail("Upstream", u.clone()));
            }
            if let Some(r) = &s.repo {
                details.push(detail("Repo", r.clone()));
            }
            for (label, v) in [
                ("Release", &s.release),
                ("Tag", &s.tag),
                ("Version", &s.version),
                ("Commit", &s.commit),
                ("File", &s.file),
                ("Built", &s.built),
            ] {
                if let Some(v) = v {
                    details.push(detail(label, v.clone()));
                }
            }
            if let Some((_, _, uses)) = known {
                details.push(detail("Uses", uses.join(", ")));
            }
            VersionEntry {
                name: known.map_or(id.clone(), |(_, name, _)| name.to_string()),
                version,
                kind: VersionKind::ThirdParty,
                channel: None,
                overridden_by: None,
                notes: upstream.map(|u| format!("from {u}")).into_iter().collect(),
                details,
                children: Vec::new(),
            }
        })
        .collect()
}

/// The tree as text lines, one entry a line with its notes indented under
/// it, children drawn with tree lines. A subtree's lines nest under a
/// parent by prefixing them, so frontends draw the same way.
pub fn tree_lines(entry: &VersionEntry) -> Vec<String> {
    let mut lines = vec![format!("{} {}", entry.name, entry.version)];
    let bar = if entry.children.is_empty() {
        "     "
    } else {
        "│    "
    };
    lines.extend(entry.notes.iter().map(|n| format!("{bar}{n}")));
    let count = entry.children.len();
    for (i, child) in entry.children.iter().enumerate() {
        let last = i + 1 == count;
        for (j, line) in tree_lines(child).into_iter().enumerate() {
            let prefix = match (j == 0, last) {
                (true, false) => "├─ ",
                (true, true) => "└─ ",
                (false, false) => "│  ",
                (false, true) => "   ",
            };
            lines.push(format!("{prefix}{line}"));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        name: &str,
        version: &str,
        notes: &[&str],
        children: Vec<VersionEntry>,
    ) -> VersionEntry {
        VersionEntry {
            name: name.into(),
            version: version.into(),
            kind: VersionKind::Package,
            channel: None,
            overridden_by: None,
            notes: notes.iter().map(|n| n.to_string()).collect(),
            details: Vec::new(),
            children,
        }
    }

    #[test]
    fn the_tree_draws_children_and_notes() {
        let tree = entry(
            "Logix Engine",
            "2026.11.14",
            &["channel prod, commit abc1234, remote config 001"],
            vec![
                entry(
                    "logix-core",
                    "2026.11.14",
                    &[],
                    vec![
                        entry(
                            "OSS CAD Suite",
                            "2026-11-04",
                            &["from YosysHQ/oss-cad-suite-build"],
                            vec![],
                        ),
                        entry("elkjs", "0.12.1", &["from kieler/elkjs"], vec![]),
                    ],
                ),
                entry("logix-catalog", "2026.11.14", &[], vec![]),
            ],
        );
        assert_eq!(
            tree_lines(&tree).join("\n"),
            "Logix Engine 2026.11.14
│    channel prod, commit abc1234, remote config 001
├─ logix-core 2026.11.14
│  ├─ OSS CAD Suite 2026-11-04
│  │       from YosysHQ/oss-cad-suite-build
│  └─ elkjs 0.12.1
│          from kieler/elkjs
└─ logix-catalog 2026.11.14"
        );
    }

    #[test]
    fn third_party_entries_come_from_the_cores_own_description() {
        let json = r#"{
            "oss-cad-suite": {"description": "d", "folder": "oss-cad-suite", "run": {"path": ["oss-cad-suite/bin"]},
                "source": {"upstream": "https://github.com/YosysHQ/oss-cad-suite-build", "file": "f.tgz",
                           "repo": "YosysHQ/oss-cad-suite-build", "release": "2026-11-04"}},
            "elkjs": {"description": "d", "folder": "elkjs", "run": {"path": ["elkjs"]},
                "source": {"upstream": "https://github.com/kieler/elkjs", "file": "elkjs-0.12.1.tgz", "version": "0.12.1"}},
            "wdi-simple": {"description": "d", "folder": "wdi-simple", "run": {"path": ["wdi-simple"]},
                "source": {"upstream": "https://github.com/pbatard/libwdi", "repo": "pbatard/libwdi", "tag": "v1.5.1",
                           "commit": "9b23b82", "built": "by logix-core from this source, unmodified"}},
            "newtool": {"description": "d", "folder": "newtool", "run": {"path": ["newtool"]}}
        }"#;
        let components = serde_json::from_str(json).unwrap();
        let entries = third_party_entries(components);
        let lines: Vec<String> = entries
            .iter()
            .map(|e| format!("{} {} {:?}", e.name, e.version, e.notes))
            .collect();
        assert_eq!(
            lines,
            [
                r#"OSS CAD Suite 2026-11-04 ["from YosysHQ/oss-cad-suite-build"]"#,
                r#"elkjs 0.12.1 ["from kieler/elkjs"]"#,
                r#"wdi-simple (libwdi) v1.5.1 ["from pbatard/libwdi"]"#,
                "newtool unknown []",
            ]
        );
        assert!(entries[0].kind == VersionKind::ThirdParty);
        let uses = entries[0]
            .details
            .iter()
            .find(|d| d.label == "Uses")
            .unwrap();
        assert!(
            uses.value.starts_with("yosys, nextpnr-ice40"),
            "{}",
            uses.value
        );
        assert!(entries[3].details.is_empty());
    }
}
