//! The engine's remote config (Remote updates, in the website repo's
//! `CLAUDE.md`): which release of each package the engine uses. It's the
//! version folder `remote-config/logix-engine/<NNN>/` in this repo, by the
//! config version this engine hard-codes, read from GitHub and cached in the
//! engine folder: `assign.jsonc` says which releases are on which channel,
//! and the engine reads its channel's config file, `<channel>-channel.jsonc`
//! (`prod-channel.jsonc` when its channel has none). Its channel:
//! `LOGIX_ENGINE_CHANNEL` when set; else, for a release build, the channel
//! `assign.jsonc` lists its own release under (dev when unlisted); a local
//! build is always dev.
//! There's no built-in list: the repo's files are the pin lists.
//!
//! A config names `dependencies`, per package a GitHub repo, a release tag
//! and a file template (`${PLATFORM}`, `${YYYYMMDD}`), and `latest`, the
//! engine's own newest release on the channel (read, not used yet).
//! `remote-config/logix-engine/index.jsonc` lists the version folders. Resolving it for this platform
//! takes each file's SHA-256 from its release's `SHA256SUMS`, which gives the
//! package set the package manager downloads and checks.
//!
//! Cache: `remote-config/config.json` in the engine folder, the resolved set
//! with where it came from and when. It's used for 24 hours, and offline.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::manifest::{self, Manifest, PackageFile, PackageSpec};

/// The repo whose `remote-config/` holds the engine's config.
pub const CONFIG_REPO: &str = "openlogix/logix-engine";
/// The engine's config version: which version folder `remote-config/
/// logix-engine/<NNN>/` it reads. Raise it only when a new engine can't share
/// the current folder (a package added or dropped, a file's format changed,
/// or a package release older engines can't use).
pub const CONFIG_VERSION: &str = "001";
/// The product folder in `remote-config/`.
const PRODUCT: &str = "logix-engine";
/// Each version folder's channel lists: which channel each release is on.
pub const ASSIGN_FILE: &str = "assign.jsonc";
/// The prod channel's config file, which a channel without its own file reads.
pub const PROD_FILE: &str = "prod-channel.jsonc";

/// A channel's config file name.
pub fn channel_file(channel: &str) -> String {
    format!("{channel}-channel.jsonc")
}
/// The local override of the channel (a runtime setting, no `DEV_`, since
/// `dev` is a channel name).
pub const CHANNEL_VAR: &str = "LOGIX_ENGINE_CHANNEL";
/// Whether this is a release build (build.rs): a local build is always dev.
pub const IS_RELEASE_BUILD: bool = env!("LOGIX_ENGINE_BUILD_RELEASE").len() == 4; // "true"
/// Where raw repo files are read from.
pub const RAW_BASE: &str = "https://raw.githubusercontent.com";
/// The diagnostic override: the URL of a version folder, or a local folder,
/// read instead of the regular one (as Apio's). A bad value is an error,
/// never a fallback.
pub const DEV_REMOTE_CONFIG_VAR: &str = "LOGIX_ENGINE_DEV_REMOTE_CONFIG";
/// How long a fetched config is used before it's fetched again.
pub const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
/// The schema every remote config names.
pub const SCHEMA: &str = "https://www.openlogix.org/schemas/logix-remote-config-v1.json";
/// The packages every config of this config version must name, and no others.
pub const PACKAGES: [&str; 2] = [super::CATALOG, super::CORE];

/// The version folder's path in the repo.
pub fn repo_path() -> String {
    format!("remote-config/{PRODUCT}/{CONFIG_VERSION}")
}

/// The regular version folder's address: its raw files on `main`.
pub fn default_url() -> String {
    format!("{RAW_BASE}/{CONFIG_REPO}/main/{}/", repo_path())
}

/// The repo's own version folder, as an override source: what the tests
/// use, so they run with the releases the repo names (the file of the
/// tests' channel: dev, or LOGIX_ENGINE_CHANNEL, once per config file CI
/// tests).
#[doc(hidden)]
pub fn repo_file_source() -> ConfigSource {
    ConfigSource {
        location: Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(repo_path())
            .display()
            .to_string(),
        is_override: true,
    }
}

/// The release the repo's config file names for a package (for tests): the
/// file the tests' channel reads.
#[doc(hidden)]
pub fn repo_release(name: &str) -> String {
    let folder = PathBuf::from(repo_file_source().location);
    let channels =
        parse_channels(&std::fs::read_to_string(folder.join(ASSIGN_FILE)).unwrap()).unwrap();
    let channel = choose_channel(&channels, channel_variable().unwrap().as_deref()).unwrap();
    let file = [channel.name.clone(), "prod".into()]
        .iter()
        .map(|c| folder.join(channel_file(c)))
        .find(|f| f.is_file())
        .expect("the channel's config file, or prod-channel.jsonc");
    parse(&std::fs::read_to_string(file).unwrap()).expect("a valid config")[name]
        .release
        .clone()
}

/// Where the engine reads its remote config.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigSource {
    /// An http(s) URL or a local file path.
    pub location: String,
    /// True for the diagnostic override.
    pub is_override: bool,
}

impl ConfigSource {
    pub fn regular() -> Self {
        ConfigSource {
            location: default_url(),
            is_override: false,
        }
    }

    /// The override from `LOGIX_ENGINE_DEV_REMOTE_CONFIG`, if set.
    pub fn from_env() -> Option<Self> {
        std::env::var(DEV_REMOTE_CONFIG_VAR)
            .ok()
            .filter(|v| !v.is_empty())
            .map(|location| ConfigSource {
                location,
                is_override: true,
            })
    }

    fn is_url(&self) -> bool {
        self.location.starts_with("https://") || self.location.starts_with("http://")
    }

    /// A file of the version folder: a URL, or a local path.
    fn file(&self, name: &str) -> String {
        if self.is_url() {
            format!("{}/{name}", self.location.trim_end_matches('/'))
        } else {
            Path::new(&self.location).join(name).display().to_string()
        }
    }
}

/// The channel the engine reads, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    /// Such as prod or dev.
    pub name: String,
    /// Why: "listed" (assign.jsonc lists this release), "default" (it
    /// doesn't: dev), "local build" (always dev) or "LOGIX_ENGINE_CHANNEL".
    pub how: String,
    /// The config file read: `<channel>-channel.jsonc`, or
    /// `prod-channel.jsonc` when the channel has none.
    pub file: String,
}

/// A valid channel name: lowercase letters, digits and dashes, a letter first.
pub fn is_channel_name(s: &str) -> bool {
    s.bytes().next().is_some_and(|c| c.is_ascii_lowercase())
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// The channel override from `LOGIX_ENGINE_CHANNEL`, if set; a value that
/// isn't a channel name is an error.
pub fn channel_variable() -> Result<Option<String>, String> {
    match std::env::var(CHANNEL_VAR) {
        Ok(v) if !v.is_empty() => {
            if is_channel_name(&v) {
                Ok(Some(v))
            } else {
                Err(format!(
                    "{CHANNEL_VAR} must be a channel name, such as prod or dev, not {v:?}"
                ))
            }
        }
        _ => Ok(None),
    }
}

/// assign.jsonc: channel name to its release tags.
pub fn parse_channels(text: &str) -> Result<BTreeMap<String, Vec<String>>, String> {
    let json: serde_json::Value = serde_json::from_str(&strip_comments(text)?)
        .map_err(|e| format!("{ASSIGN_FILE} isn't valid JSONC: {e}"))?;
    let obj = json
        .as_object()
        .ok_or(format!("{ASSIGN_FILE} isn't a JSON object"))?;
    let mut out = BTreeMap::new();
    for (name, tags) in obj.iter().filter(|(k, _)| *k != "$schema") {
        let tags = tags
            .as_array()
            .ok_or(format!("{ASSIGN_FILE}: {name} isn't a list"))?
            .iter()
            .map(|t| t.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .ok_or(format!(
                "{ASSIGN_FILE}: {name} lists something that isn't a release tag"
            ))?;
        out.insert(name.clone(), tags);
    }
    Ok(out)
}

/// The engine's channel, by the rules in the module's comment (`file` is
/// the channel's own; reading falls back to prod-channel.jsonc).
pub fn choose_channel(
    channels: &BTreeMap<String, Vec<String>>,
    variable: Option<&str>,
) -> Result<Channel, String> {
    let (name, how) = if let Some(v) = variable {
        if v != "prod" && v != "dev" && !channels.contains_key(v) {
            return Err(format!(
                "{CHANNEL_VAR}: {v} isn't a channel (prod, dev or one in {ASSIGN_FILE})"
            ));
        }
        (v.to_string(), CHANNEL_VAR)
    } else if !IS_RELEASE_BUILD {
        ("dev".to_string(), "local build")
    } else {
        let tag = crate::state::ENGINE_VERSION.replace('.', "-");
        match channels.iter().find(|(_, tags)| tags.contains(&tag)) {
            Some((name, _)) => (name.clone(), "listed"),
            None => ("dev".to_string(), "default"),
        }
    };
    Ok(Channel {
        file: channel_file(&name),
        name,
        how: how.to_string(),
    })
}

/// One dependency in a config.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub owner: String,
    pub repo: String,
    pub release: String,
    pub file: String,
}

/// JSONC to JSON: removes `//` and `/* */` comments outside strings.
pub fn strip_comments(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '/' && chars.peek() == Some(&'/') {
            for n in chars.by_ref() {
                if n == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev = ' ';
            let mut closed = false;
            for n in chars.by_ref() {
                if n == '\n' {
                    out.push('\n');
                }
                if prev == '*' && n == '/' {
                    closed = true;
                    break;
                }
                prev = n;
            }
            if !closed {
                return Err("a /* comment isn't closed".into());
            }
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

/// Reads a config's text: its dependencies, by package name. Checks what
/// the schema (logix-remote-config-v1) does that matters here, that `latest`
/// is well formed (not used yet), and that it names exactly the packages
/// this engine needs.
pub fn parse(text: &str) -> Result<BTreeMap<String, Entry>, String> {
    let json: serde_json::Value = serde_json::from_str(&strip_comments(text)?)
        .map_err(|e| format!("not valid JSONC: {e}"))?;
    let obj = json.as_object().ok_or("not a JSON object")?;
    if obj.get("$schema").and_then(|v| v.as_str()) != Some(SCHEMA) {
        return Err(format!("its $schema must be {SCHEMA}"));
    }
    if let Some(extra) = obj
        .keys()
        .find(|k| !["$schema", "latest", "dependencies"].contains(&k.as_str()))
    {
        return Err(format!("unknown key {extra}"));
    }
    if let Some(latest) = obj.get("latest") {
        entry("latest", latest, false)?;
    }
    let deps = obj
        .get("dependencies")
        .and_then(|d| d.as_object())
        .ok_or("no dependencies object")?;
    let mut out = BTreeMap::new();
    for (name, dep) in deps {
        out.insert(name.clone(), entry(name, dep, true)?);
    }
    let names: Vec<&str> = out.keys().map(String::as_str).collect();
    let mut wanted = PACKAGES.to_vec();
    wanted.sort();
    if names != wanted {
        return Err(format!(
            "it must name exactly {}, not {}",
            wanted.join(", "),
            names.join(", ")
        ));
    }
    Ok(out)
}

/// One `repo`, `release`, `file` entry: a dependency (file required) or
/// `latest` (file optional: nothing downloads it).
fn entry(name: &str, dep: &serde_json::Value, needs_file: bool) -> Result<Entry, String> {
    {
        let field = |key: &str| -> Result<String, String> {
            dep.get(key)
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| format!("{name}: no {key}"))
        };
        if let Some(extra) = dep
            .as_object()
            .ok_or_else(|| format!("{name}: not an object"))?
            .keys()
            .find(|k| !["repo", "release", "file"].contains(&k.as_str()))
        {
            return Err(format!("{name}: unknown key {extra}"));
        }
        let repo = field("repo")?;
        let (owner, repo) = repo
            .split_once('/')
            .filter(|(o, r)| safe(o) && safe(r))
            .ok_or_else(|| format!("{name}: repo must be owner/name, not {repo:?}"))?;
        let release = field("release")?;
        // A dependency may follow the newest release; latest names a tag.
        if !manifest::is_date_tag(&release) && !(needs_file && release == NEWEST) {
            return Err(format!(
                "{name}: release must be yyyy-mm-dd, not {release:?}"
            ));
        }
        let file = match dep.get("file") {
            None if !needs_file => String::new(),
            _ => field("file")?,
        };
        let plain = file.replace("${PLATFORM}", "x").replace("${YYYYMMDD}", "x");
        if !file.is_empty() && !safe(&plain) {
            return Err(format!("{name}: bad file {file:?}"));
        }
        Ok(Entry {
            owner: owner.into(),
            repo: repo.into(),
            release,
            file,
        })
    }
}

/// A single path component made of safe characters.
fn safe(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}

/// An HTTP client for the config and its checksums: short timeouts, since
/// these are small files read before an operation.
pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .user_agent(format!("logix-engine/{}", crate::state::ENGINE_VERSION))
        .build()
        .map_err(|e| e.to_string())
}

async fn get_text(client: &reqwest::Client, url: &str) -> Result<String, String> {
    get_optional(client, url)
        .await?
        .ok_or_else(|| format!("{url}: HTTP 404 Not Found"))
}

/// A URL's text; None for a 404 (a file that isn't there).
async fn get_optional(client: &reqwest::Client, url: &str) -> Result<Option<String>, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(format!("{url}: HTTP {status}"));
    }
    response
        .text()
        .await
        .map(Some)
        .map_err(|e| format!("{url}: {e}"))
}

/// A version folder's file; None when it isn't there.
async fn read_file(
    source: &ConfigSource,
    name: &str,
    client: &reqwest::Client,
) -> Result<Option<String>, String> {
    let at = source.file(name);
    if source.is_url() {
        get_optional(client, &at).await
    } else if !Path::new(&at).exists() {
        Ok(None)
    } else {
        std::fs::read_to_string(&at)
            .map(Some)
            .map_err(|e| format!("{at}: {e}"))
    }
}

/// What reading a version folder gives: the channel and its config file.
pub struct Read {
    pub channel: Channel,
    pub text: String,
}

/// Reads a version folder: `assign.jsonc`, then the channel's config file,
/// or `prod-channel.jsonc` when the channel has none (a rule, not an error: dev uses
/// what users get when it needs nothing else).
pub async fn read(source: &ConfigSource, client: &reqwest::Client) -> Result<Read, String> {
    let channels_text = read_file(source, ASSIGN_FILE, client)
        .await?
        .ok_or_else(|| format!("{}: there's no {ASSIGN_FILE}", source.location))?;
    let channels =
        parse_channels(&channels_text).map_err(|e| format!("{}: {e}", source.location))?;
    let mut channel = choose_channel(&channels, channel_variable()?.as_deref())?;
    let text = match read_file(source, &channel.file, client).await? {
        Some(t) => t,
        None if channel.name != "prod" => {
            channel.file = PROD_FILE.into();
            read_file(source, PROD_FILE, client).await?.ok_or_else(|| {
                format!(
                    "{}: there's neither {} nor {PROD_FILE}",
                    source.location,
                    channel_file(&channel.name)
                )
            })?
        }
        None => return Err(format!("{}: there's no {PROD_FILE}", source.location)),
    };
    Ok(Read { channel, text })
}

/// The product folder's list of its version folders.
pub const INDEX_FILE: &str = "index.jsonc";
const INDEX_SCHEMA: &str = "https://www.openlogix.org/schemas/logix-config-index-v1.json";

/// A config's `latest` (the newest engine release on its channel), without
/// checking its dependencies: a newer config version's file is read this
/// way, for its `latest` only.
pub fn parse_latest(text: &str) -> Result<Option<Entry>, String> {
    let json: serde_json::Value = serde_json::from_str(&strip_comments(text)?)
        .map_err(|e| format!("not valid JSONC: {e}"))?;
    match json.get("latest") {
        None => Ok(None),
        Some(l) => entry("latest", l, false).map(Some),
    }
}

/// index.jsonc: the version folders, as listed.
pub fn parse_index(text: &str) -> Result<Vec<String>, String> {
    let json: serde_json::Value = serde_json::from_str(&strip_comments(text)?)
        .map_err(|e| format!("{INDEX_FILE} isn't valid JSONC: {e}"))?;
    if json.get("$schema").and_then(|v| v.as_str()) != Some(INDEX_SCHEMA) {
        return Err(format!("{INDEX_FILE}: its $schema must be {INDEX_SCHEMA}"));
    }
    json.get("versions")
        .and_then(|v| v.as_array())
        .ok_or(format!("{INDEX_FILE} has no versions list"))?
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|v| v.len() >= 3 && v.bytes().all(|c| c.is_ascii_digit()))
                .map(str::to_string)
                .ok_or(format!(
                    "{INDEX_FILE} lists {v}, not a version folder such as 001"
                ))
        })
        .collect()
}

/// The version folders to look in for a newer release: those not older than
/// `own`, largest first, by decimal value (`0100` before `010`), always
/// with `own`.
pub fn folders_to_try(versions: &[String], own: &str) -> Vec<String> {
    let value = |v: &str| v.parse::<u64>().unwrap_or(0);
    let mut out: Vec<String> = versions
        .iter()
        .filter(|v| value(v) >= value(own))
        .cloned()
        .collect();
    out.sort_by_key(|v| std::cmp::Reverse(value(v)));
    if !out.iter().any(|v| value(v) == value(own)) {
        out.push(own.to_string());
    }
    out
}

impl ConfigSource {
    /// The product folder: the version folder's parent, with `index.jsonc`
    /// and the other version folders.
    fn product_folder(&self) -> ConfigSource {
        let trimmed = self.location.trim_end_matches(['/', '\\']);
        let parent = if self.is_url() {
            trimmed
                .rsplit_once('/')
                .map_or(trimmed, |(p, _)| p)
                .to_string()
        } else {
            Path::new(trimmed)
                .parent()
                .map_or(trimmed.to_string(), |p| p.display().to_string())
        };
        ConfigSource {
            location: parent,
            is_override: self.is_override,
        }
    }
}

/// The newest engine release on `channel` (check_updates' newer_engine;
/// the website repo's CLAUDE.md, Remote updates, A newer version of the
/// product): from `index.jsonc` in the product folder, the largest version
/// folder not older than this engine's that has the channel's file (for a
/// channel other than prod, `prod-channel.jsonc` when it has none), and its
/// `latest`. Without the index, this engine's own folder. None when no file
/// names `latest`.
pub async fn newer_release(
    source: &ConfigSource,
    channel: &str,
    client: &reqwest::Client,
) -> Result<Option<Entry>, String> {
    let product = source.product_folder();
    let versions = match read_file(&product, INDEX_FILE, client).await {
        Ok(Some(text)) => parse_index(&text)?,
        Ok(None) => vec![CONFIG_VERSION.to_string()],
        Err(e) => {
            log::info!("Couldn't read {INDEX_FILE}, using config {CONFIG_VERSION}: {e}");
            vec![CONFIG_VERSION.to_string()]
        }
    };
    let mut files = vec![channel_file(channel)];
    if channel != "prod" {
        files.push(PROD_FILE.to_string());
    }
    for version in folders_to_try(&versions, CONFIG_VERSION) {
        let folder = ConfigSource {
            location: product.file(&version),
            is_override: source.is_override,
        };
        for name in &files {
            if let Some(text) = read_file(&folder, name, client).await? {
                return parse_latest(&text).map_err(|e| format!("{}: {e}", folder.file(name)));
            }
        }
    }
    Ok(None)
}

/// A dependency's `release` that stands for its repo's newest release when
/// the config is read (the website repo's `CLAUDE.md`, Remote updates; only
/// outside prod, which the shared check enforces).
pub const NEWEST: &str = "newest";

/// The tag `newest` stands for, from GitHub's release list (JSON): the
/// newest date-tagged release, stable or pre-release, not drafts.
pub fn newest_tag(list: &str) -> Option<String> {
    let releases: Vec<serde_json::Value> = serde_json::from_str(list).ok()?;
    releases
        .iter()
        .filter(|r| r["draft"] != serde_json::Value::Bool(true))
        .filter_map(|r| r["tag_name"].as_str())
        .filter(|t| manifest::is_date_tag(t))
        .max()
        .map(str::to_string)
}

/// Where the release list is read, for `release_base`: GitHub's API, or
/// `<base>/api` on a test server.
fn api_base(release_base: &str) -> String {
    if release_base.trim_end_matches('/') == manifest::GITHUB {
        "https://api.github.com".to_string()
    } else {
        format!("{}/api", release_base.trim_end_matches('/'))
    }
}

/// Resolves a config for a platform: each package's file, with its SHA-256
/// from its release's `SHA256SUMS`, under `release_base` (manifest::GITHUB,
/// or a test server). A package naming `newest` gets its repo's newest
/// release first.
pub async fn resolve(
    entries: &BTreeMap<String, Entry>,
    platform: &str,
    release_base: &str,
    client: &reqwest::Client,
) -> Result<Manifest, String> {
    let mut packages = Vec::new();
    // The catalog first: it's small, so boards can be listed while the
    // tools download.
    for name in PACKAGES {
        let mut e = entries[name].clone();
        if e.release == NEWEST {
            let url = format!(
                "{}/repos/{}/{}/releases?per_page=30",
                api_base(release_base),
                e.owner,
                e.repo
            );
            e.release = newest_tag(&get_text(client, &url).await?)
                .ok_or_else(|| format!("{name}: {}/{} has no releases", e.owner, e.repo))?;
        }
        let e = &e;
        let per_platform = e.file.contains("${PLATFORM}");
        let file = e
            .file
            .replace("${PLATFORM}", platform)
            .replace("${YYYYMMDD}", &e.release.replace('-', ""));
        let sums_url = format!(
            "{}/{}/{}/releases/download/{}/SHA256SUMS",
            release_base.trim_end_matches('/'),
            e.owner,
            e.repo,
            e.release
        );
        let sums = get_text(client, &sums_url).await?;
        let sha256 = sums
            .lines()
            .filter_map(|l| l.split_once(char::is_whitespace))
            .find(|(_, f)| f.trim().trim_start_matches('*') == file)
            .map(|(sha, _)| sha.to_lowercase())
            .ok_or_else(|| format!("{sums_url} doesn't list {file}"))?;
        let key = if per_platform {
            platform.to_string()
        } else {
            super::ANY_PLATFORM.to_string()
        };
        packages.push(PackageSpec {
            name: name.to_string(),
            version: e.release.clone(),
            files: BTreeMap::from([(
                key,
                PackageFile {
                    owner: e.owner.clone(),
                    repo: e.repo.clone(),
                    tag: e.release.clone(),
                    file,
                    size: None,
                    sha256,
                },
            )]),
        });
    }
    let set = Manifest { packages };
    set.validate()?;
    Ok(set)
}

/// The cached, resolved config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cached {
    /// Where it was read: a URL or a local file.
    pub source: String,
    pub config_version: String,
    /// The channel and the config file read.
    pub channel: Channel,
    /// The engine build that read it (`reader()`): the channel depends on
    /// it, so another build (a newer release, a local build sharing the
    /// engine folder) reads the config again.
    pub reader: String,
    /// When it was read, in seconds since 1970 (UTC).
    pub fetched: u64,
    pub set: Manifest,
}

/// This engine build, as the cache records it: its version, and whether
/// it's a release build or a local one, which is what its channel depends
/// on besides channels.jsonc and LOGIX_ENGINE_CHANNEL.
pub fn reader() -> String {
    let kind = if IS_RELEASE_BUILD {
        "release"
    } else {
        "local build"
    };
    format!("logix-engine {} {kind}", crate::state::ENGINE_VERSION)
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Cached {
    /// Whether it's from this source and config version, read by this
    /// engine build (a cache another build wrote may be another channel's:
    /// found 2026-11-16, when a local build shared a prod release's folder)
    /// with the same channel override (none, or the same
    /// LOGIX_ENGINE_CHANNEL).
    pub fn matches(&self, source: &ConfigSource) -> bool {
        let variable = channel_variable().ok().flatten();
        let cached_variable = (self.channel.how == CHANNEL_VAR).then(|| self.channel.name.clone());
        self.source == source.location
            && self.config_version == CONFIG_VERSION
            && self.reader == reader()
            && variable == cached_variable
    }

    /// Whether it's older than MAX_AGE.
    pub fn is_due(&self) -> bool {
        now().saturating_sub(self.fetched) >= MAX_AGE.as_secs()
    }
}

fn cache_file(engine_folder: &Path) -> PathBuf {
    engine_folder.join("remote-config").join("config.json")
}

/// The cached config, if there is a readable one.
pub fn load_cache(engine_folder: &Path) -> Option<Cached> {
    let text = std::fs::read_to_string(cache_file(engine_folder)).ok()?;
    let cached: Cached = serde_json::from_str(&text).ok()?;
    cached.set.validate().ok()?;
    Some(cached)
}

/// Writes the cache, replacing the old one in one step.
pub fn save_cache(engine_folder: &Path, cached: &Cached) -> Result<(), String> {
    let file = cache_file(engine_folder);
    let dir = file.parent().expect("the cache file has a folder");
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(".config-{}.json", std::process::id()));
    let json = serde_json::to_string_pretty(cached).map_err(|e| e.to_string())? + "\n";
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())
}

/// The UTC time of a `fetched` value, as `yyyy-mm-ddThh:mm:ssZ`.
pub fn utc_time(secs: u64) -> String {
    let t = time::OffsetDateTime::from_unix_timestamp(secs as i64)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    t.format(time::macros::format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second]Z"
    ))
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A cache another engine build wrote (another release, or a local build
    // sharing the engine folder) may be another channel's: read again.
    #[test]
    fn a_cache_from_another_build_is_read_again() {
        let mut cached = Cached {
            source: ConfigSource::regular().location,
            config_version: CONFIG_VERSION.to_string(),
            channel: Channel {
                name: "dev".into(),
                how: "local build".into(),
                file: "dev-channel.jsonc".into(),
            },
            reader: reader(),
            fetched: now(),
            set: Manifest {
                packages: Vec::new(),
            },
        };
        if channel_variable().ok().flatten().is_none() {
            assert!(cached.matches(&ConfigSource::regular()));
        }
        cached.reader = "logix-engine 2026.11.15 release".into();
        assert!(!cached.matches(&ConfigSource::regular()));
        assert!(reader().starts_with(&format!("logix-engine {} ", crate::state::ENGINE_VERSION)));
    }

    const CONFIG: &str = r#"// What Logix Engine on config 001 uses.
{
  "$schema": "https://www.openlogix.org/schemas/logix-remote-config-v1.json",
  "latest": { "repo": "openlogix/logix-engine", "release": "2026-11-17" },
  "dependencies": {
    /* The FPGA tools. */
    "logix-core": {
      "repo": "openlogix/logix-core",
      "release": "2026-11-11",
      "file": "logix-core-${PLATFORM}-${YYYYMMDD}.tgz"
    },

    // A "//" in a string isn't a comment: "a//b".
    "logix-catalog": {
      "repo": "openlogix/logix-catalog",
      "release": "2026-11-11",
      "file": "logix-catalog-${YYYYMMDD}.tgz"
    }
  }
}
"#;

    #[test]
    fn a_config_reads_with_its_comments() {
        let e = parse(CONFIG).unwrap();
        assert_eq!(
            e["logix-core"],
            Entry {
                owner: "openlogix".into(),
                repo: "logix-core".into(),
                release: "2026-11-11".into(),
                file: "logix-core-${PLATFORM}-${YYYYMMDD}.tgz".into(),
            }
        );
        assert_eq!(
            strip_comments(r#"{"a": "x//y" /* c */}"#).unwrap(),
            r#"{"a": "x//y" }"#
        );
    }

    #[test]
    fn a_dependency_may_follow_the_newest_release() {
        let e = parse(&CONFIG.replacen("2026-11-11", "newest", 1)).unwrap();
        assert_eq!(e["logix-core"].release, "newest");
    }

    #[test]
    fn newest_is_the_newest_dated_release_not_a_draft() {
        let list = r#"[
            {"tag_name": "2026-11-21", "draft": false, "prerelease": false},
            {"tag_name": "2026-11-23", "draft": true, "prerelease": true},
            {"tag_name": "2026-11-22", "draft": false, "prerelease": true},
            {"tag_name": "v1.0", "draft": false, "prerelease": false}
        ]"#;
        assert_eq!(newest_tag(list).as_deref(), Some("2026-11-22"));
        assert_eq!(newest_tag("[]"), None);
        assert_eq!(newest_tag("not json"), None);
        assert_eq!(api_base(manifest::GITHUB), "https://api.github.com");
        assert_eq!(api_base("http://127.0.0.1:9/"), "http://127.0.0.1:9/api");
    }

    #[test]
    fn bad_configs_are_refused() {
        for (from, to, why) in [
            (
                "2026-11-11\",\n      \"file\": \"logix-core",
                "latest\",\n      \"file\": \"logix-core",
                "release",
            ),
            ("openlogix/logix-core", "logix-core", "owner/name"),
            ("logix-catalog-${YYYYMMDD}.tgz", "../evil.tgz", "bad file"),
            ("\"logix-catalog\": {", "\"logix-other\": {", "exactly"),
            (
                "logix-remote-config-v1",
                "logix-remote-config-v9",
                "$schema",
            ),
            (
                "\"repo\": \"openlogix/logix-core\",",
                "\"repo\": \"openlogix/logix-core\", \"x\": 1,",
                "unknown key",
            ),
            ("\"latest\": {", "\"newest\": {", "unknown key newest"),
            (
                "\"release\": \"2026-11-17\"",
                "\"release\": \"soon\"",
                "latest: release",
            ),
            // latest names a tag, never newest.
            (
                "\"release\": \"2026-11-17\"",
                "\"release\": \"newest\"",
                "latest: release",
            ),
            ("\"dependencies\": {", "\"deps\": {", "unknown key deps"),
        ] {
            assert!(CONFIG.contains(from), "{from}");
            let err = parse(&CONFIG.replacen(from, to, 1)).unwrap_err();
            assert!(err.contains(why), "{why}: {err}");
        }
        assert!(parse("{ /* open").is_err());
    }

    #[test]
    fn the_repo_config_is_valid() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join(repo_path());
        parse_channels(&std::fs::read_to_string(folder.join(ASSIGN_FILE)).unwrap()).unwrap();
        // Every config file there is valid: prod-channel.jsonc, dev-channel.jsonc,
        // others.
        let mut files = 0;
        for f in std::fs::read_dir(&folder).unwrap().flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            if name.ends_with("-channel.jsonc") {
                parse(&std::fs::read_to_string(f.path()).unwrap()).unwrap();
                files += 1;
            }
        }
        assert!(files > 0, "no config file in {}", folder.display());
        assert_eq!(
            default_url(),
            "https://raw.githubusercontent.com/openlogix/logix-engine/main/remote-config/logix-engine/001/"
        );
    }

    #[test]
    fn the_channel_follows_the_rules() {
        let channels = parse_channels(
            r#"{"$schema": "x", /* promoted */ "prod": ["2026-11-20"], "beta": []}"#,
        )
        .unwrap();
        assert_eq!(channels["prod"], ["2026-11-20"]);
        // Tests are local builds: dev, unless the variable says otherwise.
        let c = choose_channel(&channels, None).unwrap();
        assert_eq!(
            (c.name.as_str(), c.how.as_str(), c.file.as_str()),
            ("dev", "local build", "dev-channel.jsonc")
        );
        let c = choose_channel(&channels, Some("beta")).unwrap();
        assert_eq!((c.name.as_str(), c.how.as_str()), ("beta", CHANNEL_VAR));
        assert!(choose_channel(&channels, Some("prod")).is_ok());
        assert!(
            choose_channel(&channels, Some("nightly"))
                .unwrap_err()
                .contains("isn't a channel")
        );
        assert!(is_channel_name("prod") && is_channel_name("beta-2"));
        assert!(!is_channel_name("Prod") && !is_channel_name("2x") && !is_channel_name(""));
        assert!(parse_channels(r#"{"prod": "2026-11-20"}"#).is_err());
    }

    #[test]
    fn times_are_utc() {
        assert_eq!(utc_time(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_time(1_794_441_600), "2026-11-12T00:00:00Z");
    }

    #[test]
    fn newer_folders_are_tried_largest_first() {
        let v = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            folders_to_try(&v(&["001", "002", "010", "0100"]), "002"),
            v(&["0100", "010", "002"])
        );
        assert_eq!(folders_to_try(&v(&["001"]), "002"), v(&["002"]));
        let index = r#"{"$schema": "https://www.openlogix.org/schemas/logix-config-index-v1.json", "versions": ["001", "002"]}"#;
        assert_eq!(parse_index(index).unwrap(), v(&["001", "002"]));
        assert!(parse_index(&index.replace("\"002\"", "\"2\"")).is_err());
    }

    #[tokio::test]
    async fn the_newest_folder_with_the_channel_gives_latest() {
        let tmp = tempfile::tempdir().unwrap();
        let product = tmp.path().join("logix-engine");
        let write = |path: &str, text: &str| {
            let f = product.join(path);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        };
        let config = |release: &str| CONFIG.replace("2026-11-17", release);
        write(
            INDEX_FILE,
            r#"{"$schema": "https://www.openlogix.org/schemas/logix-config-index-v1.json", "versions": ["001", "002", "003"]}"#,
        );
        write("001/prod-channel.jsonc", &config("2026-11-17"));
        write("002/prod-channel.jsonc", &config("2026-12-01"));
        write("003/dev-channel.jsonc", &config("2026-12-05"));
        let own = ConfigSource {
            location: product.join(CONFIG_VERSION).display().to_string(),
            is_override: true,
        };
        let client = client().unwrap();
        let latest = |channel: &'static str| {
            let (own, client) = (own.clone(), client.clone());
            async move {
                newer_release(&own, channel, &client)
                    .await
                    .unwrap()
                    .map(|e| e.release)
            }
        };
        assert_eq!(latest("prod").await.as_deref(), Some("2026-12-01"));
        assert_eq!(latest("dev").await.as_deref(), Some("2026-12-05"));
        assert_eq!(latest("beta").await.as_deref(), Some("2026-12-01"));
        std::fs::remove_file(product.join(INDEX_FILE)).unwrap();
        assert_eq!(latest("prod").await.as_deref(), Some("2026-11-17"));
    }
}
