//! The check_updates tool (docs/tool-api.md, check_updates): reads the
//! engine's remote config, downloads the package releases it names and
//! switches to them (Remote updates, in the website repo's CLAUDE.md). It's
//! the only tool that switches releases: other tools only download a
//! missing or broken package of the releases in use.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::download_packages::{PackageDownload, to_download_lines};
use super::get_engine_info::{PackageInfo, RemoteConfigInfo, package_list, remote_config_info};
use super::{Call, EngineTool};
use crate::errors::ToolError;
use crate::packages::manifest::PackageSpec;
use crate::packages::{Update, remote_config};
use crate::progress::Phases;
use crate::state::EngineState;

pub struct CheckUpdates;

impl EngineTool for CheckUpdates {
    const NAME: &'static str = "check_updates";
    const DESCRIPTION: &'static str = "Downloads and updates the engine's packages (the board \
        catalog and the FPGA tools): reads the engine's remote config, downloads what it names \
        that isn't downloaded yet (or is broken), and uses it from the next call. It's the only \
        tool that moves the engine to newer releases: other tools download a missing or broken \
        package of the releases in use, inside their own call, but never switch releases. \
        cached_config_ok uses the saved remote config when it was read less than 24 hours \
        ago, so the call is quick and works offline; without it, the config is read now. \
        check_only downloads nothing and says what would download, with sizes. AI agents: \
        before the first other call in a session, call this with cached_config_ok and \
        check_only; when its to_download and updates are both empty, go on without mentioning \
        it; otherwise download as a step of its own in your plan, telling the user how much \
        (its summary says), by calling it again with cached_config_ok and without check_only, \
        then show that call's summary lines. Also call it when the user asks for updates, to \
        download ahead of time (such as before going offline), or when a tool returns a setup \
        error with the download_packages action. It works even when automatic download is \
        off. A download can take a few minutes; the tool reports progress and can be \
        cancelled. When newer_engine is in the result, tell the user that a newer Logix \
        Engine is available and give its page: the engine never updates itself.";
    const READ_ONLY: bool = false;
    const OPEN_WORLD: bool = true;
    type Input = CheckUpdatesInput;
    type Output = CheckUpdatesOutput;
}

/// How to check.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckUpdatesInput {
    /// Use the saved remote config when it was read less than 24 hours ago,
    /// instead of reading it now: quick, and works offline. The config is
    /// read anyway when there's no saved one, or it's older. Use it when the
    /// host starts and before an AI session's first call; leave it off when
    /// the user asks for updates. Defaults to false.
    #[serde(default)]
    pub cached_config_ok: bool,
    /// Download and switch nothing: only say what a call without it would
    /// do, in to_download (with sizes) and updates. Quick: use it to tell
    /// the user how much will download before downloading. It keeps the
    /// saved remote config as it was, so call again with the same
    /// cached_config_ok to download. Defaults to false.
    #[serde(default)]
    pub check_only: bool,
}

/// What changed, and every package's status afterwards.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckUpdatesOutput {
    /// The packages whose release changed, each with the release used before
    /// (from) and now (to); with check_only, those whose release would
    /// change. Empty when nothing changed.
    pub updates: Vec<UpdateInfo>,
    /// Names of the packages this call downloaded or repaired. Empty with
    /// check_only.
    pub downloaded: Vec<String>,
    /// The packages that needed downloading, with their versions and sizes:
    /// with check_only, what a call without it would download; otherwise
    /// what this call downloaded. Empty when everything is ready.
    #[serde(default)]
    pub to_download: Vec<PackageDownload>,
    /// Every package the engine uses, with its status.
    pub packages: Vec<PackageInfo>,
    /// The remote config in use: the one just read, or the saved one (with
    /// cached_config_ok, or when reading failed and the engine kept it).
    pub remote_config: RemoteConfigInfo,
    /// A newer Logix Engine release on this engine's channel, when there is
    /// one, looked for when the call read the remote config. The engine
    /// never updates itself: tell the user, with its page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newer_engine: Option<NewerEngine>,
    /// What changed, for people, a sentence each: "Updated logix-core to
    /// 2026.11.22." for each package whose release changed, "Downloaded
    /// logix-catalog 2026.11.22." for one only downloaded or repaired, or
    /// "Logix Engine's packages are up to date." when nothing changed.
    /// Packages are named by their package names. A newer engine isn't in
    /// it: see newer_engine. With check_only, what a download would get,
    /// such as "To download: logix-core 2026.11.22 (412 MB)." and, for more
    /// than one, a total; empty when there's nothing to download.
    pub summary: Vec<String>,
}

/// The summary of a check: each update, then each other download, else
/// that nothing changed.
fn summary(updates: &[UpdateInfo], downloaded: &[String], packages: &[PackageInfo]) -> Vec<String> {
    let mut lines: Vec<String> = updates
        .iter()
        .map(|u| format!("Updated {} to {}.", u.name, u.to))
        .collect();
    let others: Vec<String> = downloaded
        .iter()
        .filter(|d| !updates.iter().any(|u| &u.name == *d))
        .cloned()
        .collect();
    lines.extend(super::download_packages::downloaded_lines(
        &others, packages,
    ));
    if lines.is_empty() {
        lines.push(UP_TO_DATE.into());
    }
    lines
}

/// The summary when nothing changed.
const UP_TO_DATE: &str = "Logix Engine's packages are up to date.";

/// A newer Logix Engine release than this one.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewerEngine {
    /// Its version, as yyyy.mm.dd.
    pub version: String,
    /// Its release page, where its install steps are.
    pub page: String,
}

/// The newer engine named by `latest`, when it's newer than `own`
/// (yyyy.mm.dd).
fn newer(latest: Option<&remote_config::Entry>, own: &str) -> Option<NewerEngine> {
    let latest = latest.filter(|l| l.release > own.replace('.', "-"))?;
    Some(NewerEngine {
        version: latest.release.replace('-', "."),
        page: format!(
            "https://github.com/{}/{}/releases/tag/{}",
            latest.owner, latest.repo, latest.release
        ),
    })
}

/// A package whose release changed.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateInfo {
    /// The package name, such as logix-catalog.
    pub name: String,
    /// The release used before, as yyyy.mm.dd; omitted for a new package.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The release used now, as yyyy.mm.dd.
    pub to: String,
}

impl From<Update> for UpdateInfo {
    fn from(u: Update) -> Self {
        UpdateInfo {
            name: u.name,
            from: u.from,
            to: u.to,
        }
    }
}

pub async fn run(
    state: &EngineState,
    input: CheckUpdatesInput,
    call: &Call,
) -> Result<CheckUpdatesOutput, ToolError> {
    let force = !input.cached_config_ok;
    let fetched = || state.packages.remote_info().and_then(|r| r.fetched);
    let before = fetched();
    // The host asked, so this reads the config and downloads even when
    // automatic download is off.
    let (updates, needed, config_read) = if input.check_only {
        let preview = state.packages.preview(force, true).await?;
        (preview.updates, preview.to_download, preview.config_read)
    } else {
        let updates = state.packages.refresh(force, true).await?;
        let names: Vec<String> = state
            .packages
            .manifest()
            .packages
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let needed = state.packages.needing_download(&names).await?;
        (updates, needed, fetched() != before)
    };
    let to_download = sizes(state, &needed).await;
    if !input.check_only {
        let phases = Phases::new(call.progress.clone(), needed.len());
        state
            .packages
            .ensure(&needed, true, &phases, &call.cancel)
            .await?;
    }
    let config = remote_config_info(&state.packages);
    let newer_engine = if config_read {
        newer_engine(state, &config).await
    } else {
        None
    };
    let updates: Vec<UpdateInfo> = updates.into_iter().map(UpdateInfo::from).collect();
    let packages = package_list(state);
    if input.check_only {
        return Ok(CheckUpdatesOutput {
            summary: to_download_lines(&to_download),
            updates,
            downloaded: vec![],
            to_download,
            packages,
            remote_config: config,
            newer_engine,
        });
    }
    let downloaded: Vec<String> = needed.into_iter().map(|s| s.name).collect();
    Ok(CheckUpdatesOutput {
        summary: summary(&updates, &downloaded, &packages),
        updates,
        downloaded,
        to_download,
        packages,
        remote_config: config,
        newer_engine,
    })
}

/// The packages to download, with their versions and sizes.
async fn sizes(state: &EngineState, specs: &[PackageSpec]) -> Vec<PackageDownload> {
    let mut out = Vec::new();
    for spec in specs {
        out.push(PackageDownload {
            name: spec.name.clone(),
            version: crate::packages::manifest::shown_version(&spec.version),
            size: state.packages.download_size(spec).await,
        });
    }
    out
}

/// A newer engine on this engine's channel: reported, never an error
/// (offline, a broken file). Only with a remote config (tests may give the
/// engine a fixed set).
async fn newer_engine(state: &EngineState, config: &RemoteConfigInfo) -> Option<NewerEngine> {
    let source = state
        .packages
        .remote_info()
        .map(|r| remote_config::ConfigSource {
            location: r.location.clone(),
            is_override: r.is_override,
        })?;
    let channel = config.channel.clone().unwrap_or_else(|| "dev".into());
    let client = match remote_config::client() {
        Ok(client) => client,
        Err(e) => {
            log::info!("Couldn't check for a newer engine: {e}");
            return None;
        }
    };
    match remote_config::newer_release(&source, &channel, &client).await {
        Ok(latest) => newer(latest.as_ref(), crate::state::ENGINE_VERSION),
        Err(e) => {
            log::info!("Couldn't check for a newer engine: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_says_what_changed() {
        use super::super::get_engine_info::PackageStatus;
        let package = |name: &str, version: &str| PackageInfo {
            name: name.into(),
            version: version.into(),
            status: PackageStatus::Ready,
            components: Vec::new(),
        };
        let packages = [
            package("logix-catalog", "2026.11.30"),
            package("logix-core", "2026.11.22"),
        ];
        let update = UpdateInfo {
            name: "logix-catalog".into(),
            from: Some("2026.11.24".into()),
            to: "2026.11.30".into(),
        };
        assert_eq!(
            summary(
                &[update],
                &["logix-catalog".into(), "logix-core".into()],
                &packages
            ),
            [
                "Updated logix-catalog to 2026.11.30.",
                "Downloaded logix-core 2026.11.22.",
            ]
        );
        assert_eq!(
            summary(&[], &[], &packages),
            ["Logix Engine's packages are up to date."]
        );
        assert_eq!(
            super::super::download_packages::downloaded_lines(&["logix-core".into()], &packages),
            ["Downloaded logix-core 2026.11.22."]
        );
    }

    #[test]
    fn a_newer_engine_is_reported_with_its_page() {
        let latest = remote_config::Entry {
            owner: "openlogix".into(),
            repo: "logix-engine".into(),
            release: "2026-12-01".into(),
            file: String::new(),
        };
        let n = newer(Some(&latest), "2026.11.17").unwrap();
        assert_eq!(n.version, "2026.12.01");
        assert_eq!(
            n.page,
            "https://github.com/openlogix/logix-engine/releases/tag/2026-12-01"
        );
        assert!(newer(Some(&latest), "2026.12.01").is_none());
        assert!(newer(None, "2026.11.17").is_none());
    }
}
