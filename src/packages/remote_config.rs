//! The engine's remote config (Remote updates, in the website repo's
//! `CLAUDE.md`): which release of each package the engine uses. It's the
//! version folder `remote-config/logix-engine/<NNN>/` in this repo, by the
//! config version this engine hard-codes, read from GitHub and cached in the
//! engine folder: `channels.jsonc` says which releases are on which channel,
//! and the engine reads its channel's config file, `<channel>.jsonc`
//! (`prod.jsonc` when its channel has none). Its channel: `LOGIX_ENGINE_CHANNEL`
//! when set; else, for a release build, the channel `channels.jsonc` lists
//! its own release under (dev when unlisted); a local build is always dev.
//! There's no built-in list: the repo's files are the pin lists.
//!
//! A config names, per package, a GitHub repo, a release tag and a file
//! template (`${PLATFORM}`, `${YYYYMMDD}`). Resolving it for this platform
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
/// Each version folder's channel lists.
pub const CHANNELS_FILE: &str = "channels.jsonc";
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
        parse_channels(&std::fs::read_to_string(folder.join(CHANNELS_FILE)).unwrap()).unwrap();
    let channel = choose_channel(&channels, channel_variable().unwrap().as_deref()).unwrap();
    let file = [channel.name.clone(), "prod".into()]
        .iter()
        .map(|c| folder.join(format!("{c}.jsonc")))
        .find(|f| f.is_file())
        .expect("the channel's config file, or prod.jsonc");
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
    /// Why: "listed" (channels.jsonc lists this release), "default" (it
    /// doesn't: dev), "local build" (always dev) or "LOGIX_ENGINE_CHANNEL".
    pub how: String,
    /// The config file read: `<channel>.jsonc`, or `prod.jsonc` when the
    /// channel has none.
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

/// channels.jsonc: channel name to its release tags.
pub fn parse_channels(text: &str) -> Result<BTreeMap<String, Vec<String>>, String> {
    let json: serde_json::Value = serde_json::from_str(&strip_comments(text)?)
        .map_err(|e| format!("{CHANNELS_FILE} isn't valid JSONC: {e}"))?;
    let obj = json
        .as_object()
        .ok_or(format!("{CHANNELS_FILE} isn't a JSON object"))?;
    let mut out = BTreeMap::new();
    for (name, tags) in obj.iter().filter(|(k, _)| *k != "$schema") {
        let tags = tags
            .as_array()
            .ok_or(format!("{CHANNELS_FILE}: {name} isn't a list"))?
            .iter()
            .map(|t| t.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .ok_or(format!(
                "{CHANNELS_FILE}: {name} lists something that isn't a release tag"
            ))?;
        out.insert(name.clone(), tags);
    }
    Ok(out)
}

/// The engine's channel, by the rules in the module's comment (`file` is
/// the channel's own; reading falls back to prod.jsonc).
pub fn choose_channel(
    channels: &BTreeMap<String, Vec<String>>,
    variable: Option<&str>,
) -> Result<Channel, String> {
    let (name, how) = if let Some(v) = variable {
        if v != "prod" && v != "dev" && !channels.contains_key(v) {
            return Err(format!(
                "{CHANNEL_VAR}: {v} isn't a channel (prod, dev or one in {CHANNELS_FILE})"
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
        file: format!("{name}.jsonc"),
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
/// the schema (logix-remote-config-v1) does that matters here, and that it
/// names exactly the packages this engine needs.
pub fn parse(text: &str) -> Result<BTreeMap<String, Entry>, String> {
    let json: serde_json::Value = serde_json::from_str(&strip_comments(text)?)
        .map_err(|e| format!("not valid JSONC: {e}"))?;
    let obj = json.as_object().ok_or("not a JSON object")?;
    if obj.get("$schema").and_then(|v| v.as_str()) != Some(SCHEMA) {
        return Err(format!("its $schema must be {SCHEMA}"));
    }
    let mut out = BTreeMap::new();
    for (name, dep) in obj {
        if name == "$schema" {
            continue;
        }
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
        if !manifest::is_date_tag(&release) {
            return Err(format!(
                "{name}: release must be yyyy-mm-dd, not {release:?}"
            ));
        }
        let file = field("file")?;
        let plain = file.replace("${PLATFORM}", "x").replace("${YYYYMMDD}", "x");
        if !safe(&plain) {
            return Err(format!("{name}: bad file {file:?}"));
        }
        out.insert(
            name.clone(),
            Entry {
                owner: owner.into(),
                repo: repo.into(),
                release,
                file,
            },
        );
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

/// Reads a version folder: `channels.jsonc`, then the channel's config file,
/// or `prod.jsonc` when the channel has none (a rule, not an error: dev uses
/// what users get when it needs nothing else).
pub async fn read(source: &ConfigSource, client: &reqwest::Client) -> Result<Read, String> {
    let channels_text = read_file(source, CHANNELS_FILE, client)
        .await?
        .ok_or_else(|| format!("{}: there's no {CHANNELS_FILE}", source.location))?;
    let channels =
        parse_channels(&channels_text).map_err(|e| format!("{}: {e}", source.location))?;
    let mut channel = choose_channel(&channels, channel_variable()?.as_deref())?;
    let text = match read_file(source, &channel.file, client).await? {
        Some(t) => t,
        None if channel.name != "prod" => {
            channel.file = "prod.jsonc".into();
            read_file(source, "prod.jsonc", client)
                .await?
                .ok_or_else(|| {
                    format!(
                        "{}: there's neither {}.jsonc nor prod.jsonc",
                        source.location, channel.name
                    )
                })?
        }
        None => return Err(format!("{}: there's no prod.jsonc", source.location)),
    };
    Ok(Read { channel, text })
}

/// Resolves a config for a platform: each package's file, with its SHA-256
/// from its release's `SHA256SUMS`, under `release_base` (manifest::GITHUB,
/// or a test server).
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
        let e = &entries[name];
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
    /// When it was read, in seconds since 1970 (UTC).
    pub fetched: u64,
    pub set: Manifest,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Cached {
    /// Whether it's from this source and config version, read with the same
    /// channel override (none, or the same LOGIX_ENGINE_CHANNEL).
    pub fn matches(&self, source: &ConfigSource) -> bool {
        let variable = channel_variable().ok().flatten();
        let cached_variable = (self.channel.how == CHANNEL_VAR).then(|| self.channel.name.clone());
        self.source == source.location
            && self.config_version == CONFIG_VERSION
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

    const CONFIG: &str = r#"// What Logix Engine on config 001 uses.
{
  "$schema": "https://www.openlogix.org/schemas/logix-remote-config-v1.json",

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
    fn bad_configs_are_refused() {
        for (from, to, why) in [
            (
                "2026-11-11\",\n    \"file\": \"logix-core",
                "latest\",\n    \"file\": \"logix-core",
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
        parse_channels(&std::fs::read_to_string(folder.join(CHANNELS_FILE)).unwrap()).unwrap();
        // Every config file there is valid: prod.jsonc, dev.jsonc, others.
        let mut files = 0;
        for f in std::fs::read_dir(&folder).unwrap().flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            if name != CHANNELS_FILE && name.ends_with(".jsonc") {
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
            ("dev", "local build", "dev.jsonc")
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
}
