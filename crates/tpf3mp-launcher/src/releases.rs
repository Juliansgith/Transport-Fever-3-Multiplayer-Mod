//! The project's releases as the launcher lists them (D18): their notes,
//! for the release panel and its history, and which one is the latest on
//! the player's track. Read from GitHub's API, which the updater otherwise
//! avoids: only when the player looks at the history, and for the
//! Experimental track's checks, well inside its 60 requests an hour.
//!
//! What a release offers is only ever installed through its signed
//! manifest (`update.rs`); nothing read here is trusted for that.

use serde::{Deserialize, Serialize};

use crate::update::UpdateError;

/// Releases fetched per page of the history.
pub const PAGE: u32 = 10;
/// Most of a release's notes kept, in bytes.
const MAX_NOTES: usize = 16 * 1024;

/// Which releases the player is offered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Track {
    /// Published releases only.
    #[default]
    Stable,
    /// Pre-releases too.
    Experimental,
}

/// A release, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    /// Its version, from the tag `v<version>`.
    pub version: String,
    /// Its title on GitHub.
    pub title: String,
    /// Its notes, as written (Markdown), cut at 16 KiB.
    pub notes: String,
    /// When it was published, as GitHub gives it (RFC 3339).
    pub date: Option<String>,
    pub experimental: bool,
    /// Whether it has a signed manifest to install from.
    pub installable: bool,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
}

/// The releases in a page of GitHub's release list, newest first. Drafts,
/// and tags that are not `v` and a version, are left out.
pub fn parse(json: &[u8]) -> Result<Vec<Release>, UpdateError> {
    let listed: Vec<ApiRelease> = serde_json::from_slice(json)
        .map_err(|error| UpdateError::Malformed(format!("the release list: {error}")))?;
    let mut releases: Vec<Release> = listed
        .into_iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let version = release.tag_name.strip_prefix('v')?;
            semver::Version::parse(version).ok()?;
            let mut notes = release.body.unwrap_or_default();
            if notes.len() > MAX_NOTES {
                let mut end = MAX_NOTES;
                while !notes.is_char_boundary(end) {
                    end -= 1;
                }
                notes.truncate(end);
            }
            let installable = ["release.json", "release.json.sig"]
                .iter()
                .all(|file| release.assets.iter().any(|asset| asset.name == *file));
            Some(Release {
                version: version.to_owned(),
                title: release
                    .name
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| format!("TPF3-MP {version}")),
                notes,
                date: release.published_at,
                experimental: release.prerelease,
                installable,
            })
        })
        .collect();
    releases.sort_by(|a, b| b.date.cmp(&a.date));
    Ok(releases)
}

/// The latest installable release on `track`: the newest published one,
/// by date, as tearded's launcher chose it.
pub fn latest(releases: &[Release], track: Track) -> Option<&Release> {
    releases
        .iter()
        .filter(|release| release.installable)
        .filter(|release| track == Track::Experimental || !release.experimental)
        .max_by(|a, b| a.date.cmp(&b.date))
}

/// Page `page` (from 1) of the project's releases, and whether there are
/// more.
pub fn fetch(api: &str, page: u32) -> Result<(Vec<Release>, bool), UpdateError> {
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .https_only(api.starts_with("https://"))
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .build(),
    );
    let url = format!("{api}/releases?per_page={PAGE}&page={}", page.max(1));
    let mut response = agent
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header(
            "User-Agent",
            format!("tpf3mp-launcher/{}", crate::update::VERSION),
        )
        .call()?;
    let json = response
        .body_mut()
        .with_config()
        .limit(4 * 1024 * 1024)
        .read_to_vec()?;
    let listed: Vec<serde_json::Value> = serde_json::from_slice(&json)
        .map_err(|error| UpdateError::Malformed(format!("the release list: {error}")))?;
    let more = listed.len() == PAGE as usize;
    Ok((parse(&json)?, more))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed() -> Vec<u8> {
        br###"[
          {"tag_name":"v0.3.0-beta.1","name":"Beta","body":"Try this","published_at":"2026-10-05T10:00:00Z",
           "prerelease":true,"draft":false,"assets":[{"name":"release.json"},{"name":"release.json.sig"}]},
          {"tag_name":"v0.2.0","name":"","body":"## Fixes\n- a","published_at":"2026-10-01T10:00:00Z",
           "prerelease":false,"draft":false,"assets":[{"name":"release.json"},{"name":"release.json.sig"},{"name":"x.zip"}]},
          {"tag_name":"v0.2.1","name":"Unsigned","body":null,"published_at":"2026-10-03T10:00:00Z",
           "prerelease":false,"draft":false,"assets":[{"name":"x.zip"}]},
          {"tag_name":"v9.9.9","name":"Draft","published_at":null,"prerelease":false,"draft":true,"assets":[]},
          {"tag_name":"nightly","name":"Not a version","published_at":"2026-10-04T10:00:00Z","prerelease":true,"assets":[]},
          {"tag_name":"v0.1.0","name":"First","body":"Hello","published_at":"2026-09-29T10:00:00Z",
           "prerelease":false,"draft":false,"assets":[{"name":"release.json"},{"name":"release.json.sig"}]}
        ]"###
        .to_vec()
    }

    #[test]
    fn releases_are_read_newest_first_without_drafts() {
        let releases = parse(&listed()).unwrap();
        let versions: Vec<&str> = releases.iter().map(|r| r.version.as_str()).collect();
        assert_eq!(versions, ["0.3.0-beta.1", "0.2.1", "0.2.0", "0.1.0"]);
        assert_eq!(
            releases[2].title, "TPF3-MP 0.2.0",
            "an empty title is named"
        );
        assert_eq!(releases[2].notes, "## Fixes\n- a");
        assert!(releases[0].experimental);
        assert!(!releases[1].installable, "no signed manifest");
    }

    #[test]
    fn the_latest_depends_on_the_track() {
        let releases = parse(&listed()).unwrap();
        assert_eq!(
            latest(&releases, Track::Stable).map(|r| r.version.as_str()),
            Some("0.2.0"),
            "the newest signed stable release"
        );
        assert_eq!(
            latest(&releases, Track::Experimental).map(|r| r.version.as_str()),
            Some("0.3.0-beta.1")
        );
    }

    #[test]
    fn long_notes_are_cut_on_a_character() {
        let body = "é".repeat(MAX_NOTES);
        let json = serde_json::json!([{
            "tag_name": "v1.0.0", "body": body, "published_at": "2026-10-01T00:00:00Z",
            "assets": []
        }]);
        let releases = parse(json.to_string().as_bytes()).unwrap();
        assert!(releases[0].notes.len() <= MAX_NOTES);
        assert!(releases[0].notes.chars().all(|c| c == 'é'));
    }

    #[test]
    fn a_list_that_is_not_one_is_refused() {
        assert!(parse(b"{\"message\":\"API rate limit exceeded\"}").is_err());
    }
}
