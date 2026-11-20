//! The check_updates tool (docs/tool-api.md, check_updates): reads the
//! engine's remote config now, downloads the package releases it names and
//! switches to them (Remote updates, in the website repo's CLAUDE.md).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::get_engine_info::{PackageInfo, RemoteConfigInfo, package_list, remote_config_info};
use super::{Call, EngineTool};
use crate::errors::ToolError;
use crate::packages::{Update, remote_config};
use crate::progress::Phases;
use crate::state::EngineState;

pub struct CheckUpdates;

impl EngineTool for CheckUpdates {
    const NAME: &'static str = "check_updates";
    const DESCRIPTION: &'static str = "Checks now for newer releases of the engine's packages \
        (the board catalog and the FPGA tools): reads the engine's remote config, downloads \
        what it names that isn't downloaded yet, and uses it from the next call. The engine \
        also checks on its own, before an operation that needs packages, when it last checked \
        more than 24 hours ago, so call this only to check now, such as when the user asks for \
        updates. It works even when automatic download is off. A download can take a few \
        minutes; the tool reports progress and can be cancelled. When newer_engine is in the \
        result, tell the user that a newer Logix Engine is available and give its page: the \
        engine never updates itself.";
    const READ_ONLY: bool = false;
    const OPEN_WORLD: bool = true;
    type Input = CheckUpdatesInput;
    type Output = CheckUpdatesOutput;
}

/// The check_updates tool takes no inputs.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckUpdatesInput {}

/// What changed, and every package's status afterwards.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckUpdatesOutput {
    /// The packages whose release changed, each with the release used before
    /// (from) and now (to). Empty when nothing changed.
    pub updates: Vec<UpdateInfo>,
    /// Names of the packages this call downloaded or repaired.
    pub downloaded: Vec<String>,
    /// Every package the engine uses, with its status.
    pub packages: Vec<PackageInfo>,
    /// The remote config just read.
    pub remote_config: RemoteConfigInfo,
    /// A newer Logix Engine release on this engine's channel, when there is
    /// one. The engine never updates itself: tell the user, with its page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newer_engine: Option<NewerEngine>,
}

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
    _input: CheckUpdatesInput,
    call: &Call,
) -> Result<CheckUpdatesOutput, ToolError> {
    let updates = state.packages.refresh(true, true).await?;
    let names: Vec<String> = state
        .packages
        .manifest()
        .packages
        .iter()
        .map(|p| p.name.clone())
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let needed = state.packages.needing_download(&names).await?;
    let phases = Phases::new(call.progress.clone(), needed.len());
    // The host asked, so this downloads even when automatic download is off.
    state
        .packages
        .ensure(&needed, true, &phases, &call.cancel)
        .await?;
    let config = remote_config_info(&state.packages);
    // A newer engine: reported, never an error (offline, a broken file).
    // Only with a remote config (tests may give the engine a fixed set).
    let source = state
        .packages
        .remote_info()
        .map(|r| remote_config::ConfigSource {
            location: r.location.clone(),
            is_override: r.is_override,
        });
    let channel = config.channel.clone().unwrap_or_else(|| "dev".into());
    let newer_engine = match (source, remote_config::client()) {
        (None, _) => None,
        (Some(source), Ok(client)) => {
            match remote_config::newer_release(&source, &channel, &client).await {
                Ok(latest) => newer(latest.as_ref(), crate::state::ENGINE_VERSION),
                Err(e) => {
                    log::info!("Couldn't check for a newer engine: {e}");
                    None
                }
            }
        }
        (_, Err(e)) => {
            log::info!("Couldn't check for a newer engine: {e}");
            None
        }
    };
    Ok(CheckUpdatesOutput {
        updates: updates.into_iter().map(UpdateInfo::from).collect(),
        downloaded: needed.into_iter().map(|s| s.name).collect(),
        packages: package_list(state),
        remote_config: config,
        newer_engine,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
