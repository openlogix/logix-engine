//! The package manifest (docs/requirements.md, "Versioning" and "Packages"):
//! the exact release and checksum of every package this engine release was
//! tested with, for each platform. It is built into the engine from
//! manifest.json.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

/// Where package files are downloaded from: GitHub releases.
pub const GITHUB: &str = "https://github.com";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub packages: Vec<PackageSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageSpec {
    /// The package name, a catalog-style ID such as oss-cad-suite.
    pub name: String,
    /// The package release, a date tag.
    pub version: String,
    pub description: String,
    /// The package file for each platform, such as darwin-arm64.
    pub files: BTreeMap<String, PackageFile>,
}

/// One package file, a GitHub release asset.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageFile {
    pub owner: String,
    pub repo: String,
    pub tag: String,
    /// The asset file name, a .tgz.
    pub file: String,
    /// The file size in bytes.
    pub size: u64,
    /// The SHA-256 checksum of the file, as lowercase hex.
    pub sha256: String,
}

impl PackageFile {
    /// The download URL, under `base` (GITHUB, or a test server).
    pub fn url(&self, base: &str) -> String {
        format!(
            "{}/{}/{}/releases/download/{}/{}",
            base.trim_end_matches('/'),
            self.owner,
            self.repo,
            self.tag,
            self.file
        )
    }
}

impl Manifest {
    pub fn parse(json: &str) -> Result<Manifest, String> {
        let manifest: Manifest = serde_json::from_str(json).map_err(|e| e.to_string())?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn get(&self, name: &str) -> Option<&PackageSpec> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// Checks the rules that keep package paths and versions safe.
    pub fn validate(&self) -> Result<(), String> {
        let mut names = std::collections::BTreeSet::new();
        for p in &self.packages {
            if !is_id(&p.name) {
                return Err(format!("bad package name {:?}", p.name));
            }
            if !names.insert(&p.name) {
                return Err(format!("package {} is listed twice", p.name));
            }
            if !is_date_tag(&p.version) {
                return Err(format!("{}: bad version {:?}", p.name, p.version));
            }
            for (platform, f) in &p.files {
                let ctx = format!("{} ({platform})", p.name);
                if !is_id(platform) {
                    return Err(format!("{ctx}: bad platform name"));
                }
                if f.sha256.len() != 64
                    || !f
                        .sha256
                        .bytes()
                        .all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
                {
                    return Err(format!("{ctx}: sha256 must be 64 lowercase hex digits"));
                }
                if !f.file.ends_with(".tgz") || !is_safe_name(&f.file) {
                    return Err(format!("{ctx}: bad file name {:?}", f.file));
                }
                for (what, v) in [("owner", &f.owner), ("repo", &f.repo), ("tag", &f.tag)] {
                    if !is_safe_name(v) {
                        return Err(format!("{ctx}: bad {what} {v:?}"));
                    }
                }
                if f.size == 0 {
                    return Err(format!("{ctx}: size must be set"));
                }
            }
        }
        Ok(())
    }
}

/// Lowercase letters, digits and hyphens, like a catalog ID.
fn is_id(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// A single path component made of safe characters.
fn is_safe_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}

/// `yyyy-mm-dd`, optionally with `-N` for a later release on the same day.
pub fn is_date_tag(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    match &s[10..] {
        "" => true,
        rest => rest.strip_prefix('-').is_some_and(|n| {
            !n.is_empty() && !n.starts_with('0') && n.bytes().all(|c| c.is_ascii_digit())
        }),
    }
}

/// The manifest built into this engine release.
pub fn builtin() -> &'static Manifest {
    static MANIFEST: OnceLock<Manifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        Manifest::parse(include_str!("manifest.json")).expect("the built-in manifest is valid")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_manifest_is_valid_and_covers_darwin_arm64() {
        let m = builtin();
        let oss = m.get("oss-cad-suite").expect("oss-cad-suite is listed");
        let f = &oss.files["darwin-arm64"];
        assert_eq!(
            f.url(GITHUB),
            "https://github.com/fpgawars/tools-oss-cad-suite/releases/download/2026-10-22/apio-oss-cad-suite-darwin-arm64-20261022.tgz"
        );
        // The file name carries the same date as the package version.
        for p in &m.packages {
            let compact = p.version[..10].replace('-', "");
            for f in p.files.values() {
                assert!(f.file.contains(&compact), "{} {}", p.name, f.file);
            }
        }
    }

    #[test]
    fn date_tags() {
        assert!(is_date_tag("2026-10-22"));
        assert!(is_date_tag("2026-10-22-2"));
        assert!(!is_date_tag("2026-10-22-0"));
        assert!(!is_date_tag("2026.10.22"));
        assert!(!is_date_tag("20261022"));
        assert!(!is_date_tag("2026-10-2x"));
    }

    #[test]
    fn unsafe_values_are_refused() {
        let ok = include_str!("manifest.json");
        for (from, to) in [
            ("\"oss-cad-suite\"", "\"../x\""),
            (
                "\"2026-10-22\",\n      \"description\"",
                "\"latest\",\n      \"description\"",
            ),
            (
                "apio-oss-cad-suite-darwin-arm64-20261022.tgz",
                "../evil.tgz",
            ),
            ("\"fpgawars\"", "\"a/b\""),
            ("5fdbffd3", "5FDBFFD3"),
        ] {
            assert!(ok.contains(from), "{from}");
            let bad = ok.replacen(from, to, 1);
            assert!(Manifest::parse(&bad).is_err(), "{to} should be refused");
        }
    }
}
