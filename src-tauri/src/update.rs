//! "A new version is out."
//!
//! Checked once at launch against the public GitHub releases, with no token and no
//! telemetry: one request, no identifiers, and the app is just as usable if it
//! fails. Note it asks for the release *list* rather than `/releases/latest` —
//! that endpoint skips pre-releases, and every 0.x build of Kakiri is one.

use std::time::Duration;

#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct Update {
    /// Version without the leading `v`.
    pub version: String,
    /// Release page, for whoever clicks "get it".
    pub url: String,
    /// First lines of the release notes, already trimmed to something displayable.
    pub notes: String,
}

const REPO: &str = "0xwagmi/Kakiri";

fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next()?;
    let mut it = core.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next().unwrap_or("0").parse().ok()?;
    let patch = it.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn first_lines(body: &str, max: usize) -> String {
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .take(max)
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(240)
        .collect()
}

pub fn check() -> Option<Update> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .ok()?;
    let raw = client
        .get(format!("https://api.github.com/repos/{REPO}/releases?per_page=10"))
        .header("User-Agent", "Kakiri")
        .header("Accept", "application/vnd.github+json")
        .send()
        .ok()?
        .text()
        .ok()?;
    let list: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let current = parse(env!("CARGO_PKG_VERSION"))?;

    list.as_array()?
        .iter()
        .filter(|r| !r["draft"].as_bool().unwrap_or(false))
        .filter_map(|r| {
            let tag = r["tag_name"].as_str()?;
            let version = parse(tag)?;
            Some((version, r))
        })
        .filter(|(version, _)| *version > current)
        .max_by_key(|(version, _)| *version)
        .map(|(version, r)| Update {
            version: format!("{}.{}.{}", version.0, version.1, version.2),
            url: r["html_url"].as_str().unwrap_or_default().to_string(),
            notes: first_lines(r["body"].as_str().unwrap_or_default(), 3),
        })
}

#[cfg(test)]
mod tests {
    use super::{first_lines, parse};

    #[test]
    fn versions_compare_numerically_not_as_text() {
        assert!(parse("v0.1.10") > parse("0.1.9"));
        assert!(parse("v1.0.0") > parse("0.9.9"));
        assert_eq!(parse("0.1.6"), parse("v0.1.6"));
        assert_eq!(parse("0.1.6-beta.1"), parse("0.1.6"));
    }

    #[test]
    fn notes_drop_headings_and_stay_short() {
        let body = "## What's new\n\n- a thing\n- another\n\n- third\n";
        let out = first_lines(body, 2);
        assert_eq!(out, "- a thing - another");
    }
}
