//! Whether a newer release of Wisp has been published. The desktop only
//! says so and points at the release page: nothing is downloaded or
//! installed from here.
//!
//! The request is the WebView desktop's: the latest release of the public
//! repository, with nothing about the user or their work in it.

use anyhow::{Context, Result};
use serde::Deserialize;

const LATEST: &str = "https://api.github.com/repos/xuzhougeng/wisp-science/releases/latest";
const RELEASES: &str = "https://github.com/xuzhougeng/wisp-science/releases";
/// As much of the release notes as is passed on.
const NOTES_LIMIT: usize = 8_000;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: String,
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateCheck {
    /// The version that is running.
    pub current: String,
    /// The latest published version.
    pub latest: String,
    pub newer: bool,
    /// The page to get it from.
    pub url: String,
    /// The release's notes, as Markdown.
    pub notes: String,
}

/// The numbers of a version such as `v1.24.0` or `1.25.0-rc.1`, and whether
/// it is a pre-release. Anything else is not a version.
fn parse(version: &str) -> Option<([u64; 3], bool)> {
    let version = version.trim().trim_start_matches(['v', 'V']);
    let (numbers, pre) = match version.split_once('-') {
        Some((numbers, _)) => (numbers, true),
        None => (version.split('+').next()?, false),
    };
    let mut parts = numbers.split('.').map(|part| part.parse::<u64>().ok());
    let found = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some((found, pre))
}

fn compare(current: &str, release: Release) -> Result<UpdateCheck> {
    let (running, running_pre) =
        parse(current).with_context(|| format!("This build's version, {current}, is not one"))?;
    let (published, published_pre) = parse(&release.tag_name).with_context(|| {
        format!(
            "The latest release is tagged {}, which is not a version",
            release.tag_name
        )
    })?;
    // The same numbers are newer only as the release of a pre-release.
    let newer = published > running || (published == running && running_pre && !published_pre);
    let text = |numbers: [u64; 3]| format!("{}.{}.{}", numbers[0], numbers[1], numbers[2]);
    let url = match release.html_url.starts_with(RELEASES) {
        true => release.html_url,
        // Whatever the answer says, the page opened is the project's own.
        false => RELEASES.to_owned(),
    };
    let mut notes: String = release.body.chars().take(NOTES_LIMIT).collect();
    if release.body.chars().count() > NOTES_LIMIT {
        notes.push('…');
    }
    Ok(UpdateCheck {
        current: text(running),
        latest: text(published),
        newer,
        url,
        notes,
    })
}

/// The WebView desktop's switch for looking when the app starts, saved as
/// `true` or `false`.
const AT_START: &str = "update_check_enabled";

/// Whether a saved switch is on. Nothing saved, or something that is not
/// a switch, is on, as the WebView reads it.
fn at_start(saved: Option<&str>) -> bool {
    saved
        .and_then(|saved| serde_json::from_str::<bool>(saved.trim()).ok())
        .unwrap_or(true)
}

impl crate::Server {
    /// Whether to look for a newer version when the app starts.
    pub async fn update_check_at_start(&self) -> Result<bool> {
        Ok(at_start(
            self.0.store.get_setting(AT_START).await?.as_deref(),
        ))
    }

    pub async fn set_update_check_at_start(&self, enabled: bool) -> Result<()> {
        self.0
            .store
            .set_setting(AT_START, &enabled.to_string())
            .await
    }
}

/// Asks the release page which version is the latest.
pub(crate) async fn check() -> Result<UpdateCheck> {
    let release = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()?
        .get(LATEST)
        .header(reqwest::header::USER_AGENT, "wisp-science-update-check")
        .send()
        .await
        .context("Could not reach the release page")?
        .error_for_status()
        .context("The release page answered with an error")?
        .json::<Release>()
        .await
        .context("The release page's answer could not be read")?;
    compare(env!("CARGO_PKG_VERSION"), release)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> Release {
        Release {
            tag_name: tag.into(),
            html_url: format!("{RELEASES}/tag/{tag}"),
            body: "## Notes".into(),
        }
    }

    #[test]
    fn a_later_version_is_newer_and_the_same_or_an_earlier_one_is_not() {
        let found = compare("1.24.0", release("v1.25.0")).unwrap();
        assert!(found.newer);
        assert_eq!(
            (found.current.as_str(), found.latest.as_str()),
            ("1.24.0", "1.25.0")
        );
        assert_eq!(found.url, format!("{RELEASES}/tag/v1.25.0"));
        assert_eq!(found.notes, "## Notes");
        assert!(!compare("1.24.0", release("v1.24.0")).unwrap().newer);
        assert!(!compare("1.24.0", release("v1.23.9")).unwrap().newer);
        // Numbers, not text: 1.10 is after 1.9.
        assert!(compare("1.9.0", release("1.10.0")).unwrap().newer);
        // The release of a pre-release that is running is newer; a
        // pre-release of what is running is not.
        assert!(compare("1.25.0-rc.1", release("v1.25.0")).unwrap().newer);
        assert!(!compare("1.25.0", release("v1.25.0-rc.2")).unwrap().newer);
    }

    #[test]
    fn looking_at_start_is_on_unless_it_was_turned_off() {
        assert!(at_start(None));
        assert!(at_start(Some("true")));
        assert!(!at_start(Some("false")));
        assert!(!at_start(Some(" false\n")));
        assert!(at_start(Some("maybe")));
    }

    #[test]
    fn what_is_not_a_version_or_not_the_projects_page_is_not_trusted() {
        for tag in ["latest", "v1.2", "1.2.3.4", "v1.x.0", ""] {
            assert!(compare("1.24.0", release(tag)).is_err(), "{tag}");
        }
        assert!(compare("dev", release("v1.0.0")).is_err());
        // A link elsewhere is replaced by the project's release page.
        let mut elsewhere = release("v1.25.0");
        elsewhere.html_url = "https://example.test/download".into();
        assert_eq!(compare("1.24.0", elsewhere).unwrap().url, RELEASES);
        // Long notes are cut.
        let mut long = release("v1.25.0");
        long.body = "é".repeat(NOTES_LIMIT + 5);
        let notes = compare("1.24.0", long).unwrap().notes;
        assert_eq!(notes.chars().count(), NOTES_LIMIT + 1);
        assert!(notes.ends_with('…'));
    }
}
